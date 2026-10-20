// SPDX-License-Identifier: GPL-2.0-only

//! Separate, thread-owned game window and truthful presentation receipts.
//!
//! This API must run outside any paused guest-memory/execution permit. It does
//! not borrow the launcher's window, acknowledge a core Frame, or synthesize
//! vblank from elapsed time. See `docs/window-presenter.md` for the deliberately
//! narrow display, color and cancellation profiles and primary API contracts.

// Non-Windows builds retain the same unavailable API and pure state tests;
// Windows-only owner methods have no live backend on those targets.
#![cfg_attr(
    not(all(target_os = "windows", target_arch = "x86_64")),
    allow(dead_code)
)]

use std::{
    fmt,
    marker::PhantomData,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod windows;

const MAX_FRAME: usize = 16 * 1024 * 1024;
const MAX_CPU: usize = 64 * 1024 * 1024;
const MAX_GPU: u64 = 128 * 1024 * 1024;
static GENERATION: AtomicU64 = AtomicU64::new(1);

/// The input flag is retained even though this first opaque surface does not blend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlphaMode {
    Straight,
    Premultiplied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorEncoding {
    Unspecified,
    Srgb,
}

/// Borrowed tight RGBA8, never a guest/host numeric pointer or authority ticket.
pub struct FrameView<'a> {
    pub width: u32,
    pub height: u32,
    pub pixels: &'a [u8],
    pub alpha: AlphaMode,
    pub encoding: ColorEncoding,
}

/// Application limits, not console hardware limits or driver heap accounting.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_frame_bytes: usize,
    pub max_cpu_bytes: usize,
    pub max_device_bytes: u64,
    /// Maximum pixels swizzled in a single submission.
    pub max_pixels: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_frame_bytes: MAX_FRAME,
            max_cpu_bytes: MAX_CPU,
            max_device_bytes: MAX_GPU,
            max_pixels: (MAX_FRAME / 4) as u64,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub width: u32,
    pub height: u32,
}

/// Closed window invalidation categories without host geometry or identifiers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowChange {
    InitialClientExtent,
    CloakingQuery,
    Cloaked,
    ClientExtent,
    EventEpoch,
    Monitor,
    SurfaceImageCount,
    SurfaceExtent,
    SurfaceLimits,
    SurfaceProfile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum CompletionRequirement {
    SystemLoader,
    Vulkan11,
    Swapchain,
    PresentId,
    PresentWait,
    PresentationFence,
    FeatureBits,
    PresentationQueue,
    SurfaceProfile,
    SurfaceUsage,
    SurfaceExtent,
    SurfaceFormats,
    SurfaceImages,
    UploadMemory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    UnsupportedHost,
    UnavailableTick,
    UnavailableCompletion,
    MissingCompletionRequirement(CompletionRequirement),
    UnsupportedColor,
    Limit,
    Allocation,
    InvalidFrame,
    InvalidSequence,
    Busy,
    WouldBlock,
    NeedsReopen,
    WindowChanged(WindowChange),
    Closed,
    Cancelled,
    /// A cancelled backend may still own GPU work; its bounded resources are
    /// retained until process teardown, and another presenter cannot open here.
    SessionRestartRequired,
    OperatingSystem(&'static str, u32),
    Vulkan(&'static str, i32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WindowChanged(change) => write!(f, "presentation window changed: {change:?}"),
            Self::MissingCompletionRequirement(requirement) => {
                write!(
                    f,
                    "presentation completion requirement unavailable: {requirement:?}"
                )
            }
            Self::OperatingSystem(api, code) => write!(f, "{api} failed with code {code}"),
            Self::Vulkan(api, code) => write!(f, "{api} failed with code {code}"),
            other => f.write_str(match other {
                Self::UnsupportedHost => "game presentation requires Windows x86-64",
                Self::UnavailableTick => {
                    "single-primary-monitor compositor tick profile unavailable"
                }
                Self::UnavailableCompletion => {
                    "FIFO Vulkan present-id/present-wait completion unavailable"
                }
                Self::UnsupportedColor => "presentation requires explicitly sRGB-encoded RGBA8",
                Self::Limit => "presentation resource or work limit exceeded",
                Self::Allocation => "presentation staging allocation refused",
                Self::InvalidFrame => "frame dimensions or tight RGBA8 span invalid",
                Self::InvalidSequence => "frame sequence is zero, stale or exhausted",
                Self::Busy => "one presentation is already outstanding",
                Self::WouldBlock => "presentation image not ready; retry without acknowledgement",
                Self::NeedsReopen => "window or display generation changed; reopen required",
                Self::Closed => "game window closed",
                Self::Cancelled => "game presentation cancelled",
                Self::SessionRestartRequired => {
                    "presentation teardown requires a new consumer process"
                }
                Self::OperatingSystem(..)
                | Self::Vulkan(..)
                | Self::WindowChanged(_)
                | Self::MissingCompletionRequirement(_) => unreachable!(),
            }),
        }
    }
}
impl std::error::Error for Error {}

/// Current owner generation. A resize/loss cancels rather than retagging work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowStatus {
    Active,
    NeedsReopen,
    Closed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowState {
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    pub status: WindowStatus,
}

/// Actual DWM cRefresh observation in the admitted single-monitor profile.
/// It is not a physical scanout timestamp or a locally interpolated count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservedTick {
    generation: u64,
    count: u64,
}
impl ObservedTick {
    pub fn generation(self) -> u64 {
        self.generation
    }
    pub fn count(self) -> u64 {
        self.count
    }
}

/// Queue acceptance only. This type never means presentation completed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SwapAccepted {
    generation: u64,
    sequence: u64,
    alpha: AlphaMode,
}
impl SwapAccepted {
    pub fn generation(self) -> u64 {
        self.generation
    }
    pub fn sequence(self) -> u64 {
        self.sequence
    }
    pub fn alpha(self) -> AlphaMode {
        self.alpha
    }
}

/// Returned once only after a successful present wait and unchanged generation.
/// The guarantee is the Vulkan API guarantee, not exact physical pixel timing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Presented {
    accepted: SwapAccepted,
}
impl Presented {
    pub fn generation(self) -> u64 {
        self.accepted.generation
    }
    pub fn sequence(self) -> u64 {
        self.accepted.sequence
    }
    pub fn alpha(self) -> AlphaMode {
        self.accepted.alpha
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Presentation {
    Idle,
    Pending,
    Presented(Presented),
}

struct State {
    window: WindowState,
    last_sequence: u64,
    last_tick: u64,
    pending: Option<SwapAccepted>,
}
impl State {
    fn new(config: Config) -> Result<Self, Error> {
        let generation = GENERATION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| Error::Limit)?;
        Ok(Self {
            window: WindowState {
                generation,
                width: config.width,
                height: config.height,
                status: WindowStatus::Active,
            },
            last_sequence: 0,
            last_tick: 0,
            pending: None,
        })
    }
    fn active(&self) -> Result<(), Error> {
        match self.window.status {
            WindowStatus::Active => Ok(()),
            WindowStatus::NeedsReopen => Err(Error::NeedsReopen),
            WindowStatus::Closed => Err(Error::Closed),
            WindowStatus::Cancelled => Err(Error::Cancelled),
        }
    }
    fn admit(&self, sequence: u64) -> Result<(), Error> {
        self.active()?;
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        if sequence == 0 || sequence <= self.last_sequence {
            return Err(Error::InvalidSequence);
        }
        Ok(())
    }
    fn accepted(&mut self, sequence: u64, alpha: AlphaMode) -> SwapAccepted {
        let accepted = SwapAccepted {
            generation: self.window.generation,
            sequence,
            alpha,
        };
        self.last_sequence = sequence;
        self.pending = Some(accepted);
        accepted
    }
    fn invalidate(&mut self, status: WindowStatus) {
        self.pending = None;
        self.window.status = status;
    }
    fn tick(&mut self, count: u64) -> Result<Option<ObservedTick>, Error> {
        self.active()?;
        if count == 0 || count < self.last_tick {
            self.invalidate(WindowStatus::NeedsReopen);
            return Err(Error::UnavailableTick);
        }
        if count == self.last_tick {
            return Ok(None);
        }
        self.last_tick = count;
        Ok(Some(ObservedTick {
            generation: self.window.generation,
            count,
        }))
    }
    fn completed(&mut self) -> Result<Presentation, Error> {
        self.active()?;
        Ok(match self.pending.take() {
            Some(accepted) => Presentation::Presented(Presented { accepted }),
            None => Presentation::Idle,
        })
    }
}

fn frame_length(width: u32, height: u32, limits: Limits) -> Result<usize, Error> {
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err(Error::InvalidFrame);
    }
    let pixels = u64::from(width) * u64::from(height);
    let bytes = usize::try_from(pixels * 4).map_err(|_| Error::Limit)?;
    if bytes > limits.max_frame_bytes.min(MAX_FRAME) || pixels > limits.max_pixels {
        return Err(Error::Limit);
    }
    Ok(bytes)
}

fn cpu_admit(held: usize, mapped: u64, limits: Limits) -> Result<(), Error> {
    let mapped = usize::try_from(mapped).map_err(|_| Error::Limit)?;
    if held
        .checked_add(mapped)
        .is_none_or(|total| total > limits.max_cpu_bytes.min(MAX_CPU))
    {
        return Err(Error::Limit);
    }
    Ok(())
}

// Swapchain creation can return more images than requested. Reserve all eight
// image slots and the minimum upload before asking the driver to create it;
// after creation the actual count/requirements are checked independently.
fn image_admit(frame_bytes: usize, limits: Limits) -> Result<(), Error> {
    let bytes = (frame_bytes as u64).checked_mul(9).ok_or(Error::Limit)?;
    if bytes > limits.max_device_bytes.min(MAX_GPU) {
        return Err(Error::Limit);
    }
    Ok(())
}

/// Unique game window, surface, device and current-thread event pump. !Send/!Sync.
/// Only one may exist in a process; cancellation of unresolved driver work may
/// require process teardown. No method acknowledges or mutates a guest Frame.
pub struct Presenter {
    state: State,
    limits: Limits,
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    backend: Option<windows::Backend>,
    _thread: PhantomData<Rc<()>>,
}

impl Presenter {
    pub fn open(config: Config, limits: Limits) -> Result<Self, Error> {
        Self::open_with_staging_cap(config, limits, limits.max_cpu_bytes)
    }

    /// Bound the backend's mapped upload and CPU metadata before allocation,
    /// independently of the total external-held-plus-backend submission limit.
    /// This lets a supervised session reserve a future decoder/frame capacity.
    pub fn open_with_staging_cap(
        config: Config,
        limits: Limits,
        max_backend_cpu_bytes: usize,
    ) -> Result<Self, Error> {
        frame_length(config.width, config.height, limits)?;
        if limits.max_device_bytes == 0
            || limits.max_cpu_bytes == 0
            || max_backend_cpu_bytes == 0
            || max_backend_cpu_bytes > limits.max_cpu_bytes.min(MAX_CPU)
        {
            return Err(Error::Limit);
        }
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            // Only construction uses the separate backend cap; submit retains
            // the original global held+mapped ceiling below.
            let backend_limits = Limits {
                max_cpu_bytes: max_backend_cpu_bytes,
                ..limits
            };
            let backend = windows::Backend::open(config, backend_limits)?;
            Ok(Self {
                state: State::new(config)?,
                limits,
                backend: Some(backend),
                _thread: PhantomData,
            })
        }
        #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
        {
            Err(Error::UnsupportedHost)
        }
    }

    /// Actual host-visible retained capacity: mapped Vulkan upload requirements
    /// plus this owner's event metadata. Driver-private heaps are not claimed.
    pub fn retained_cpu_bytes(&self) -> usize {
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            self.backend.as_ref().map_or(0, |backend| {
                usize::try_from(backend.staging_bytes()).unwrap_or(usize::MAX)
            })
        }
        #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
        {
            0
        }
    }

    /// Dispatch at most 64 messages, then recheck live window/profile identity.
    /// Resize, hidden/minimized state or display change is terminal for this owner.
    pub fn pump(&mut self) -> Result<WindowState, Error> {
        self.state.active()?;
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            if let Err(error) = self.backend.as_mut().ok_or(Error::Cancelled)?.check() {
                if error == Error::WouldBlock {
                    return Err(error);
                }
                self.state.invalidate(if error == Error::Closed {
                    WindowStatus::Closed
                } else {
                    WindowStatus::NeedsReopen
                });
                return Err(error);
            }
        }
        Ok(self.state.window)
    }

    pub fn window_state(&self) -> WindowState {
        self.state.window
    }

    /// No waiting or estimated refresh synthesis; equal count produces None.
    pub fn observe_tick(&mut self) -> Result<Option<ObservedTick>, Error> {
        self.pump()?;
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            let result = self.backend.as_mut().ok_or(Error::Cancelled)?.tick();
            match result {
                Ok(count) => self.state.tick(count),
                Err(Error::WouldBlock) => Err(Error::WouldBlock),
                Err(error) => {
                    self.state.invalidate(WindowStatus::NeedsReopen);
                    Err(error)
                }
            }
        }
        #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
        {
            Err(Error::UnsupportedHost)
        }
    }

    /// `held_cpu_bytes` includes the caller's actual retained Frame/IPC capacities.
    /// No scaling, alpha blending or gamma conversion is silently introduced.
    /// Acquire timeout is zero. WouldBlock changes neither sequence nor receipt.
    pub fn submit(
        &mut self,
        sequence: u64,
        frame: FrameView<'_>,
        held_cpu_bytes: usize,
    ) -> Result<SwapAccepted, Error> {
        self.state.admit(sequence)?;
        let length = frame_length(frame.width, frame.height, self.limits)?;
        if frame.width != self.state.window.width
            || frame.height != self.state.window.height
            || frame.pixels.len() != length
        {
            return Err(Error::InvalidFrame);
        }
        if frame.encoding != ColorEncoding::Srgb {
            return Err(Error::UnsupportedColor);
        }
        if held_cpu_bytes < frame.pixels.len() {
            return Err(Error::InvalidFrame);
        }
        self.pump()?;
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            let backend = self.backend.as_mut().ok_or(Error::Cancelled)?;
            cpu_admit(held_cpu_bytes, backend.staging_bytes(), self.limits)?;
            if let Err(error) = backend.submit(sequence, frame.pixels) {
                if error != Error::WouldBlock {
                    self.state.invalidate(WindowStatus::NeedsReopen);
                }
                return Err(error);
            }
            Ok(self.state.accepted(sequence, frame.alpha))
        }
        #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
        {
            let _ = held_cpu_bytes;
            Err(Error::UnsupportedHost)
        }
    }

    /// Zero-timeout fence/present polling. Swap acceptance alone stays Pending.
    /// Rechecks current window generation after the successful Vulkan wait too.
    pub fn poll_present(&mut self) -> Result<Presentation, Error> {
        self.pump()?;
        if self.state.pending.is_none() {
            return Ok(Presentation::Idle);
        }
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            let sequence = self.state.pending.ok_or(Error::InvalidSequence)?.sequence;
            let result = self
                .backend
                .as_mut()
                .ok_or(Error::Cancelled)?
                .poll(sequence);
            match result {
                Ok(false) => Ok(Presentation::Pending),
                Ok(true) => {
                    self.pump()?;
                    self.state.completed()
                }
                Err(error) => {
                    self.state.invalidate(WindowStatus::NeedsReopen);
                    Err(error)
                }
            }
        }
        #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
        {
            Err(Error::UnsupportedHost)
        }
    }

    /// Invalidate immediately, with no queue-idle wait and no successful receipt.
    pub fn cancel(&mut self) {
        self.state.invalidate(WindowStatus::Cancelled);
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        if let Some(backend) = self.backend.as_mut() {
            backend.hide();
        }
    }
}

impl Drop for Presenter {
    fn drop(&mut self) {
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        if let Some(mut backend) = self.backend.take() {
            backend.hide();
            if !backend.safe_to_destroy() {
                // Retain the unique owner, loader and all in-use handles. The
                // process-wide slot remains held, preventing unbounded leaks.
                // An OS-supervised consumer process can now exit without ack.
                backend.retain_for_restart();
                std::mem::forget(backend);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn acceptance_is_not_completion_and_generation_never_retags_pending() {
        let mut a = State::new(Config {
            width: 64,
            height: 64,
        })
        .unwrap();
        let b = State::new(Config {
            width: 64,
            height: 64,
        })
        .unwrap();
        assert_ne!(a.window.generation, b.window.generation);
        assert_eq!(a.admit(0), Err(Error::InvalidSequence));
        a.admit(7).unwrap();
        let accepted = a.accepted(7, AlphaMode::Premultiplied);
        assert_eq!(a.admit(8), Err(Error::Busy));
        assert_eq!(a.pending, Some(accepted));
        let Presentation::Presented(receipt) = a.completed().unwrap() else {
            panic!()
        };
        assert_eq!(
            (receipt.sequence(), receipt.alpha()),
            (7, AlphaMode::Premultiplied)
        );
        assert_eq!(a.completed(), Ok(Presentation::Idle));
        assert_eq!(a.admit(7), Err(Error::InvalidSequence));
        a.accepted(9, AlphaMode::Straight);
        a.invalidate(WindowStatus::NeedsReopen);
        assert_eq!(a.completed(), Err(Error::NeedsReopen));
        assert!(a.pending.is_none());
    }
    #[test]
    fn ticks_are_actual_monotonic_observations_and_never_interpolated() {
        let mut a = State::new(Config {
            width: 1,
            height: 1,
        })
        .unwrap();
        assert_eq!(a.tick(81).unwrap().unwrap().count(), 81);
        assert_eq!(a.tick(81), Ok(None));
        assert_eq!(a.tick(93).unwrap().unwrap().count(), 93);
        assert_eq!(a.tick(92), Err(Error::UnavailableTick));
        assert_eq!(a.tick(94), Err(Error::NeedsReopen));
    }
    #[test]
    fn staging_admits_actual_capacity_and_external_held_before_upload() {
        let limits = Limits::default();
        assert_eq!(frame_length(4096, 1024, limits), Ok(MAX_FRAME));
        assert_eq!(frame_length(4096, 4096, limits), Err(Error::Limit));
        assert_eq!(frame_length(0, 8, limits), Err(Error::InvalidFrame));
        assert_eq!(cpu_admit(MAX_CPU - 4096, 4096, limits), Ok(()));
        assert_eq!(cpu_admit(MAX_CPU - 4096, 4097, limits), Err(Error::Limit));
        assert_eq!(cpu_admit(usize::MAX, 1, limits), Err(Error::Limit));
        assert_eq!(image_admit(1920 * 1080 * 4, limits), Ok(()));
        assert_eq!(
            image_admit(
                64 * 64 * 4,
                Limits {
                    max_device_bytes: 32768,
                    ..limits
                }
            ),
            Err(Error::Limit)
        );
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    #[test]
    #[ignore = "requires a local visible primary monitor and admitted Vulkan presentation extensions"]
    fn actual_owned_window_waits_for_exact_presentation_and_cancel_never_acks() {
        use std::time::{Duration, Instant};
        let mut presenter = Presenter::open(
            Config {
                width: 64,
                height: 64,
            },
            Limits::default(),
        )
        .unwrap();
        let generation = presenter.window_state().generation;
        let tick = presenter.observe_tick().unwrap().unwrap();
        assert_eq!(tick.generation(), generation);
        let mut pixels = [0_u8; 64 * 64 * 4];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[31, 95, 159, 255]);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let accepted = loop {
            let frame = FrameView {
                width: 64,
                height: 64,
                pixels: &pixels,
                alpha: AlphaMode::Straight,
                encoding: ColorEncoding::Srgb,
            };
            match presenter.submit(1, frame, pixels.len()) {
                Ok(accepted) => break accepted,
                Err(Error::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(1))
                }
                other => panic!("presentation admission failed: {other:?}"),
            }
        };
        assert_eq!(
            (accepted.sequence(), accepted.generation()),
            (1, generation)
        );
        let receipt = loop {
            match presenter.poll_present().unwrap() {
                Presentation::Presented(receipt) => break receipt,
                Presentation::Pending if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(1))
                }
                other => panic!("presentation did not complete within the bounded test: {other:?}"),
            }
        };
        assert_eq!((receipt.sequence(), receipt.generation()), (1, generation));
        assert_eq!(presenter.poll_present().unwrap(), Presentation::Idle);
        presenter.cancel();
        assert_eq!(presenter.poll_present(), Err(Error::Cancelled));
        assert_eq!(presenter.observe_tick(), Err(Error::Cancelled));
    }
}
