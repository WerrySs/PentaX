// SPDX-License-Identifier: GPL-2.0-only

use super::{Config, Error, Limits, WindowChange};
use std::{
    cell::UnsafeCell,
    mem::size_of,
    ptr::{null, null_mut},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
use windows_sys::Win32::{
    Foundation::{GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::{
        Dwm::{
            DWM_TIMING_INFO, DWMWA_CLOAKED, DwmGetCompositionTimingInfo, DwmGetWindowAttribute,
            DwmIsCompositionEnabled,
        },
        Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromWindow},
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::{
            AdjustWindowRectExForDpi, DPI_AWARENESS_CONTEXT,
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, GetDpiForWindow,
            SetThreadDpiAwarenessContext,
        },
        WindowsAndMessaging::*,
    },
};

mod wsi;
static OPEN: AtomicBool = AtomicBool::new(false);
static POISONED: AtomicBool = AtomicBool::new(false);
static CLASS: OnceLock<Result<(), u32>> = OnceLock::new();
const CLASS_NAME: &[u16] = &[
    80, 101, 110, 116, 97, 88, 46, 71, 97, 109, 101, 86, 105, 101, 119, 46, 49, 0,
];
const TITLE: &[u16] = &[
    80, 101, 110, 116, 97, 88, 32, 71, 97, 109, 101, 32, 86, 105, 101, 119, 0,
];
const WINDOW_STYLE: WINDOW_STYLE = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;

// Scope only this owner's creation/physical-coordinate queries. A created HWND
// retains its DPI awareness after the calling thread's previous context returns;
// no process-wide/launcher DPI policy is changed. Microsoft's mixed-mode rule:
// https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setthreaddpiawarenesscontext
struct PhysicalPixels(DPI_AWARENESS_CONTEXT);
impl PhysicalPixels {
    fn enter() -> Result<Self, Error> {
        // SAFETY: Fixed supported context for this thread only. The exact prior
        // valid context is retained and restored on every ordinary/error exit.
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if previous.is_null() {
            return Err(os("SetThreadDpiAwarenessContext"));
        }
        Ok(Self(previous))
    }
}
impl Drop for PhysicalPixels {
    fn drop(&mut self) {
        // SAFETY: This non-Send scope restores the valid context returned by
        // Windows on this same thread; it never alters another window owner.
        let _ = unsafe { SetThreadDpiAwarenessContext(self.0) };
    }
}

fn outer_extent(width: u32, height: u32, dpi: u32) -> Result<(i32, i32), Error> {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: i32::try_from(width).map_err(|_| Error::Limit)?,
        bottom: i32::try_from(height).map_err(|_| Error::Limit)?,
    };
    // SAFETY: Checked dimensions and a writable local RECT. The supplied DPI
    // comes from Windows; only nonclient borders/caption scale, never pixels.
    if unsafe { AdjustWindowRectExForDpi(&mut rect, WINDOW_STYLE, 0, 0, dpi) } == 0 {
        return Err(os("AdjustWindowRectExForDpi"));
    }
    let width = rect
        .right
        .checked_sub(rect.left)
        .filter(|v| *v > 0)
        .ok_or(Error::Limit)?;
    let height = rect
        .bottom
        .checked_sub(rect.top)
        .filter(|v| *v > 0)
        .ok_or(Error::Limit)?;
    Ok((width, height))
}

struct Slot;
impl Slot {
    fn acquire() -> Result<Self, Error> {
        OPEN.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                if POISONED.load(Ordering::Acquire) {
                    Error::SessionRestartRequired
                } else {
                    Error::Busy
                }
            })?;
        Ok(Self)
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        OPEN.store(false, Ordering::Release);
    }
}

#[derive(Clone, Copy)]
struct Events {
    epoch: u64,
    closed: bool,
}

// The boxed-by-capacity element remains at one stable address until after
// DestroyWindow. Only this thread's window procedure accesses its UnsafeCell.
unsafe extern "system" fn procedure(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if message == WM_GETMINMAXINFO && l != 0 {
        // This fixed-size owned class permits small programmatic pixel extents
        // instead of inheriting the system caption-button tracking minimum.
        // It can arrive before WM_NCCREATE, so no callback state is required.
        // Maximize limits and all caption/system/minimize styles stay intact.
        // SAFETY: Windows supplies this message's writable MINMAXINFO. Only its
        // minimum tracking fields are changed, as specified by WM_GETMINMAXINFO.
        unsafe {
            let info = &mut *(l as *mut MINMAXINFO);
            info.ptMinTrackSize.x = 1;
            info.ptMinTrackSize.y = 1;
        }
        return 0;
    }
    if message == WM_NCCREATE {
        // SAFETY: Windows supplies a live CREATESTRUCTW for WM_NCCREATE. Its
        // lpCreateParams is our retained single UnsafeCell, not guest input.
        let pointer = unsafe { (*(l as *const CREATESTRUCTW)).lpCreateParams };
        // SAFETY: This hwnd is being created by this procedure on the owning
        // thread. The pointer remains live through every subsequent callback.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, pointer as isize) };
    }
    // SAFETY: The slot is zero before WM_NCCREATE and contains only our pointer
    // thereafter; it is cleared before the Window allocation is released.
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut UnsafeCell<Events>;
    if !pointer.is_null() {
        if matches!(
            message,
            WM_SIZE
                | WM_MOVE
                | WM_DISPLAYCHANGE
                | WM_DPICHANGED
                | WM_SHOWWINDOW
                | WM_WINDOWPOSCHANGED
                | WM_CLOSE
                | WM_DESTROY
        ) {
            // SAFETY: Same-thread reentrancy; no Rust borrow of this cell spans
            // a Win32 call. Only a short raw mutation occurs before forwarding.
            unsafe {
                let events = (*pointer).get();
                (*events).epoch = (*events).epoch.saturating_add(1);
                if matches!(message, WM_CLOSE | WM_DESTROY) {
                    (*events).closed = true;
                }
            }
        }
        if message == WM_CLOSE {
            // Keep the HWND alive while unresolved Vulkan work is retained.
            // SAFETY: Owned same-thread HWND; hiding creates no successful ack.
            unsafe { ShowWindow(hwnd, SW_HIDE) };
            return 0;
        }
    }
    // SAFETY: Unhandled original Windows message and its original parameters.
    unsafe { DefWindowProcW(hwnd, message, w, l) }
}

struct Window {
    hwnd: HWND,
    instance: windows_sys::Win32::Foundation::HINSTANCE,
    events: Vec<UnsafeCell<Events>>,
    epoch: u64,
    monitor: windows_sys::Win32::Graphics::Gdi::HMONITOR,
    monitor_rect: RECT,
    width: u32,
    height: u32,
}

impl Window {
    fn open(config: Config, limits: Limits) -> Result<Self, Error> {
        let _physical = PhysicalPixels::enter()?;
        // SAFETY: NULL obtains the current executable module, with no loading.
        let instance = unsafe { GetModuleHandleW(null()) };
        if instance.is_null() {
            return Err(os("GetModuleHandleW"));
        }
        let class = CLASS.get_or_init(|| {
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance,
                lpszClassName: CLASS_NAME.as_ptr(),
                ..WNDCLASSW::default()
            };
            // SAFETY: Static UTF-16 name, static procedure and live module. The
            // process retains this single registration; no per-frame classes.
            if unsafe { RegisterClassW(&class) } == 0 {
                // SAFETY: Read immediately on failure, no intervening OS call.
                Err(unsafe { GetLastError() })
            } else {
                Ok(())
            }
        });
        if let Err(code) = class {
            return Err(Error::OperatingSystem("RegisterClassW", *code));
        }
        let mut events = Vec::new();
        super::cpu_admit(size_of::<UnsafeCell<Events>>(), 0, limits)?;
        events.try_reserve_exact(1).map_err(|_| Error::Allocation)?;
        let metadata = events
            .capacity()
            .checked_mul(size_of::<UnsafeCell<Events>>())
            .ok_or(Error::Limit)?;
        super::cpu_admit(metadata, 0, limits)?;
        events.push(UnsafeCell::new(Events {
            epoch: 1,
            closed: false,
        }));
        // A fixed-size game window avoids implicit scaling; changes still
        // invalidate its generation. The launcher owns its separate HWND.
        // SAFETY: Real current system DPI under the temporary PMv2 context.
        // The HWND's actual selected-monitor DPI is used again after creation.
        let dpi = unsafe { GetDpiForSystem() };
        if dpi == 0 {
            return Err(os("GetDpiForSystem"));
        }
        let (width, height) = outer_extent(config.width, config.height, dpi)?;
        // SAFETY: Only fixed names/style/checked sizes and a stable owned cell
        // enter Windows. It never sees a guest address, title or payload pointer.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                CLASS_NAME.as_ptr(),
                TITLE.as_ptr(),
                WINDOW_STYLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                width,
                height,
                null_mut(),
                null_mut(),
                instance,
                events.as_ptr().cast(),
            )
        };
        if hwnd.is_null() {
            return Err(os("CreateWindowExW"));
        }
        let window = Self {
            hwnd,
            instance,
            events,
            epoch: 0,
            monitor: null_mut(),
            monitor_rect: RECT::default(),
            width: config.width,
            height: config.height,
        };
        // Commit the exact physical client size before Vulkan queries the
        // surface extent. On any mismatch Window drops the owned HWND safely.
        window.set_client_extent()?;
        Ok(window)
    }

    fn set_client_extent(&self) -> Result<(), Error> {
        // The caller holds PhysicalPixels while this window is being created.
        // SAFETY: Real live owned HWND; PMv2 reports its actual monitor DPI.
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        if dpi == 0 {
            return Err(os("GetDpiForWindow"));
        }
        let (mut width, mut height) = outer_extent(self.width, self.height, dpi)?;
        let wanted_width = i32::try_from(self.width).map_err(|_| Error::Limit)?;
        let wanted_height = i32::try_from(self.height).map_err(|_| Error::Limit)?;
        for _ in 0..3 {
            // Suppress only this owned programmatic sizing operation's default
            // WM_WINDOWPOSCHANGING validation/clamping. Caption controls/styles
            // remain intact. The final client pixels are independently checked.
            // https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-windowposchanging
            // SAFETY: Same-thread owned HWND and checked physical dimensions;
            // no move, reorder, show or activation is requested.
            if unsafe {
                SetWindowPos(
                    self.hwnd,
                    null_mut(),
                    0,
                    0,
                    width,
                    height,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSENDCHANGING,
                )
            } == 0
            {
                return Err(os("SetWindowPos"));
            }
            let mut client = RECT::default();
            // SAFETY: Writable local RECT, queried under PMv2 coordinates.
            if unsafe { GetClientRect(self.hwnd, &mut client) } == 0 {
                return Err(os("GetClientRect"));
            }
            let client_width = client.right.checked_sub(client.left).ok_or(Error::Limit)?;
            let client_height = client.bottom.checked_sub(client.top).ok_or(Error::Limit)?;
            if client_width == wanted_width && client_height == wanted_height {
                return Ok(());
            }
            // AdjustWindowRectExForDpi is only an initial nonclient estimate.
            // Correct from actual physical rectangles; do not guess a theme's
            // border width, change the requested pixel geometry, or loop forever.
            let mut outer = RECT::default();
            // SAFETY: Same live HWND and exact writable RECT under PMv2.
            if unsafe { GetWindowRect(self.hwnd, &mut outer) } == 0 {
                return Err(os("GetWindowRect"));
            }
            width = outer
                .right
                .checked_sub(outer.left)
                .and_then(|v| v.checked_add(wanted_width.checked_sub(client_width)?))
                .filter(|v| *v > 0)
                .ok_or(Error::Limit)?;
            height = outer
                .bottom
                .checked_sub(outer.top)
                .and_then(|v| v.checked_add(wanted_height.checked_sub(client_height)?))
                .filter(|v| *v > 0)
                .ok_or(Error::Limit)?;
        }
        Err(Error::WindowChanged(WindowChange::InitialClientExtent))
    }

    fn show_and_bind(&mut self) -> Result<(), Error> {
        // SAFETY: Owned HWND, same thread. No foreign/launcher window is used.
        unsafe {
            ShowWindow(self.hwnd, SW_SHOW);
        }
        self.messages()?;
        let (monitor, rect) = self.profile()?;
        self.monitor = monitor;
        self.monitor_rect = rect;
        self.epoch = self.events().epoch;
        self.check()
    }

    fn events(&self) -> Events {
        // SAFETY: Stable single cell, same owning thread; no simultaneous
        // mutation or borrow. This copy does not span a Windows callback.
        unsafe { *self.events[0].get() }
    }

    fn messages(&self) -> Result<(), Error> {
        for _ in 0..64 {
            let mut message = MSG::default();
            // SAFETY: Writable local MSG and this owner's HWND filter. Do not
            // steal launcher/thread messages or run an unbounded event loop.
            if unsafe { PeekMessageW(&mut message, self.hwnd, 0, 0, PM_REMOVE) } == 0 {
                return Ok(());
            }
            // SAFETY: Dispatch precisely the retrieved same-window message.
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        Err(Error::WouldBlock)
    }

    fn profile(&self) -> Result<(windows_sys::Win32::Graphics::Gdi::HMONITOR, RECT), Error> {
        let _physical = PhysicalPixels::enter()?;
        // SAFETY: Queries only the live owned HWND and local scalar outputs.
        let visible = unsafe {
            IsWindow(self.hwnd) != 0 && IsWindowVisible(self.hwnd) != 0 && IsIconic(self.hwnd) == 0
        };
        if !visible || self.events().closed {
            return Err(Error::Closed);
        }
        let mut cloaked = 0_u32;
        // SAFETY: Owned HWND and exact scalar output. A cloaked window is not
        // eligible even if its Win32 visible style remains set.
        if unsafe {
            DwmGetWindowAttribute(
                self.hwnd,
                DWMWA_CLOAKED as u32,
                &mut cloaked as *mut u32 as *mut _,
                size_of::<u32>() as u32,
            )
        } < 0
        {
            return Err(Error::WindowChanged(WindowChange::CloakingQuery));
        }
        if cloaked != 0 {
            return Err(Error::WindowChanged(WindowChange::Cloaked));
        }
        let mut rect = RECT::default();
        // SAFETY: Owned HWND and a writable output of the correct type.
        if unsafe { GetClientRect(self.hwnd, &mut rect) } == 0 {
            return Err(os("GetClientRect"));
        }
        if rect.right - rect.left != self.width as i32
            || rect.bottom - rect.top != self.height as i32
        {
            return Err(Error::WindowChanged(WindowChange::ClientExtent));
        }
        let mut enabled = 0;
        // SAFETY: Finite local output, no window handle required by this query.
        if unsafe { DwmIsCompositionEnabled(&mut enabled) } < 0 || enabled == 0 {
            return Err(Error::UnavailableTick);
        }
        // Only this explicit single-monitor profile may interpret the global
        // DWM counter as the current window's compositor refresh observation.
        // SAFETY: Read-only display query with no caller-provided index.
        if unsafe { GetSystemMetrics(SM_CMONITORS) } != 1 {
            return Err(Error::UnavailableTick);
        }
        // SAFETY: Query this HWND's actual current monitor; no nearest fallback.
        let monitor = unsafe { MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONULL) };
        if monitor.is_null() {
            return Err(Error::UnavailableTick);
        }
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..MONITORINFO::default()
        };
        // SAFETY: Actual queried monitor and exact initialized output size.
        if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 || info.dwFlags & 1 == 0 {
            return Err(Error::UnavailableTick);
        }
        Ok((monitor, info.rcMonitor))
    }

    fn check(&self) -> Result<(), Error> {
        self.messages()?;
        if self.events().closed {
            return Err(Error::Closed);
        }
        let (monitor, rect) = self.profile()?;
        let same_rect = rect.left == self.monitor_rect.left
            && rect.top == self.monitor_rect.top
            && rect.right == self.monitor_rect.right
            && rect.bottom == self.monitor_rect.bottom;
        if self.events().epoch != self.epoch {
            return Err(Error::WindowChanged(WindowChange::EventEpoch));
        }
        if monitor != self.monitor || !same_rect {
            return Err(Error::WindowChanged(WindowChange::Monitor));
        }
        Ok(())
    }

    fn tick(&self) -> Result<u64, Error> {
        self.check()?;
        let mut timing = DWM_TIMING_INFO {
            cbSize: size_of::<DWM_TIMING_INFO>() as u32,
            ..DWM_TIMING_INFO::default()
        };
        // SAFETY: Windows >=8.1 requires HWND=NULL. We checked the explicit
        // single-primary-monitor profile; cbSize and output storage are exact.
        let code = unsafe { DwmGetCompositionTimingInfo(null_mut(), &mut timing) };
        if code < 0 {
            return Err(Error::UnavailableTick);
        }
        self.check()?;
        // Copy packed fields by value, never form unaligned references.
        if timing.qpcVBlank == 0 || timing.rateRefresh.uiDenominator == 0 || timing.cRefresh == 0 {
            return Err(Error::UnavailableTick);
        }
        Ok(timing.cRefresh)
    }

    fn hide(&self) {
        // SAFETY: Owned HWND remains live even when its GPU work is retained.
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: Called only after the Vulkan surface owner is destroyed, or
        // before one exists. Detach callback storage before freeing the cell.
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            DestroyWindow(self.hwnd);
        }
    }
}

fn os(operation: &'static str) -> Error {
    // SAFETY: Thread-local last-error query; callers use immediately on failure.
    Error::OperatingSystem(operation, unsafe { GetLastError() })
}

pub(super) struct Backend {
    // Declaration/drop order is intentional: Vulkan surface before HWND/slot.
    gpu: wsi::Wsi,
    window: Window,
    _slot: Slot,
}
impl Backend {
    pub fn open(config: Config, limits: Limits) -> Result<Self, Error> {
        let slot = Slot::acquire()?;
        // Keep WSI's Win32 surface queries in the same physical coordinate
        // space as this owner's creation, then restore the caller on return.
        let _physical = PhysicalPixels::enter()?;
        let mut window = Window::open(config, limits)?;
        let metadata = window
            .events
            .capacity()
            .checked_mul(size_of::<UnsafeCell<Events>>())
            .ok_or(Error::Limit)?;
        // Bind the actual visible profile before Vulkan captures its surface
        // capabilities. A hidden HWND can advertise a different surface extent
        // or image-count profile; showing it afterwards would stale that proof
        // before the first tick or submission.
        window.show_and_bind()?;
        let gpu = wsi::Wsi::open(window.hwnd, window.instance, config, limits, metadata)?;
        super::cpu_admit(
            window.events.capacity() * size_of::<UnsafeCell<Events>>(),
            gpu.staging_bytes(),
            limits,
        )?;
        window.check()?;
        gpu.check_surface()?;
        Ok(Self {
            gpu,
            window,
            _slot: slot,
        })
    }
    pub fn check(&mut self) -> Result<(), Error> {
        let _physical = PhysicalPixels::enter()?;
        self.window.check()?;
        self.gpu.check_surface()
    }
    pub fn tick(&mut self) -> Result<u64, Error> {
        self.check()?;
        let tick = self.window.tick()?;
        self.check()?;
        Ok(tick)
    }
    pub fn staging_bytes(&self) -> u64 {
        self.gpu.staging_bytes()
            + self.window.events.capacity() as u64 * size_of::<UnsafeCell<Events>>() as u64
    }
    pub fn submit(&mut self, id: u64, pixels: &[u8]) -> Result<(), Error> {
        let _physical = PhysicalPixels::enter()?;
        self.gpu.submit(id, pixels)
    }
    pub fn poll(&mut self, id: u64) -> Result<bool, Error> {
        let _physical = PhysicalPixels::enter()?;
        self.gpu.poll(id)
    }
    pub fn hide(&mut self) {
        self.window.hide();
    }
    pub fn safe_to_destroy(&self) -> bool {
        self.gpu.safe_to_destroy()
    }
    pub fn retain_for_restart(&mut self) {
        POISONED.store(true, Ordering::Release);
    }
}
