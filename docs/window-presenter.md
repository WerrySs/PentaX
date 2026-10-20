# Owned game-window presentation

`pentax_platform::presenter` owns a separate Win32 window, event pump and Vulkan
WSI device. It does not attach to the eframe launcher HWND or its Glow surface.
The implementation is original Rust using the documented Windows and Vulkan
APIs linked below. This component alone is not a running title, window transport,
guest event queue or measured game-frame result.

## Startup and pixel profile

Creation and WSI queries use a scoped per-monitor DPI context, restoring the
calling thread's prior context on exit. The owned window class permits small
programmatic extents and verifies the requested physical client dimensions
before creating the Vulkan surface. Nonclient borders use the actual window
DPI; pixel dimensions are not silently scaled or enlarged by caption defaults.

Open with `Config { width, height }` before asking the guest session to take its
first Frame. These dimensions must come from actual registered/ready surface
metadata through a bounded configuration handshake. The first pixel message
cannot be the only configuration source: taking a Frame already requires a
genuine tick. Do not solve that ordering by guessing dimensions or fabricating
a tick. Configuration, supervision and core Frame acknowledgement remain caller
responsibilities.

The first window has a fixed client extent, no implicit scaling, and one
outstanding frame. Submit tight RGBA8, dimensions 1..4096, at most 16 MiB, with
explicit sRGB encoding. Unspecified transfer encoding is unsupported. The
selected surface is `B8G8R8A8_UNORM` / `SRGB_NONLINEAR`; the CPU upload exchanges
red and blue only, without an extra gamma transform. The opaque swapchain does
not blend alpha. Incoming Straight/Premultiplied flags are retained in receipts;
premultiplied RGB stays premultiplied and is not silently unpremultiplied. This
explicit opaque policy is not general transparent-window compositing.

Caller-supplied held CPU bytes must include actual retained Frame/IPC capacities
and at least the borrowed pixel length. The mapped upload allocation's actual
Vulkan requirement size and the event record's actual Vec capacity are added
before allocation and upload. Event metadata is preadmitted before reservation
and its actual capacity is carried into upload admission. The ceiling is 64 MiB.
Pixel work is prebounded; the copy uses no
pixel Vec or fallible growth. Separate device admission counts the actual upload
allocation plus all returned swapchain images' logical RGBA8 bytes, at most
128 MiB. Before swapchain creation it conservatively admits all eight image
slots plus the minimum upload; the actual returned count and upload requirements
are independently checked afterward. Some otherwise valid 16 MiB frames can
therefore exceed this narrower device profile. This is not an exact account of
opaque driver allocations. Enumeration
uses fixed arrays (32 physical devices, 512 extensions/device, 64 queue families,
32 formats, 8 swapchain images) and refuses incomplete/oversized results.

## Two independent capabilities

An owner opens only after querying and enabling the exact Vulkan extensions and
feature bits. The initial profile requires Vulkan 1.1, surface/Win32 surface,
surface-capabilities2, EXT surface-maintenance1, swapchain, KHR present-id, KHR
present-wait and EXT swapchain-maintenance1. Missing functionality is an explicit
unavailable/error result; there is no texture-upload or repaint fallback.

`submit` returns **SwapAccepted** after an actual successful FIFO queue present.
It never returns Presented. Nonzero present IDs strictly increase, only one is
outstanding, and mailbox replacement is not enabled. `poll_present` checks:

1. The exact transfer fence, authorizing staging/command-buffer reuse.
2. The exact maintenance presentation fence, authorizing safe destruction/reuse
   of presentation resources. This fence does not establish display completion.
3. `vkWaitForPresentKHR` for the submitted ID, authorizing a Presented receipt
   only after an unchanged live window/surface/display profile recheck.

All explicit acquire/wait timeouts are zero. Calls into the OS/driver can still
take implementation-dependent time; this is not a driver sandbox. Never call
this API while holding a paused guest-memory or native execution permit.

The present-wait contract has no exact physical pixel timestamp. In particular,
an out-of-date swapchain may report success for an image which might have been
presented. This implementation rejects observed out-of-date/suboptimal results,
loss of the selected surface contract, changed window event epochs, monitor
changes, resize, hidden/minimized/cloaked windows and composition loss. Its
receipt means successful Vulkan presentation wait in that unchanged profile,
not a stronger physical-scanout measurement or proof against arbitrary external
occlusion. A later event-loop repaint is never used as a receipt.

Ticks are independent: `observe_tick` reads actual DWM `cRefresh`, with the
required NULL HWND, only with composition enabled and exactly one active monitor
which is the current window's primary monitor. It checks visibility, monitor,
window event epoch and surface identity around the query. Repeated counts return
None; zero or backwards counts invalidate the owner. Counts are never locally
incremented, interpolated from QPC/refresh rate, or derived from accepted frames.
This is named a single-primary-monitor **compositor refresh observation**, not a
universal hardware-vblank source. Other display configurations are unavailable.

## Ownership, changes and cancellation

The safe owner is neither Send nor Sync and exports no HWND, Vulkan handle or
arbitrary caller-selected title/path. Each instance receives a unique checked
generation; sequences strictly increase and one completion is returned once.
Resize/display/profile change requires reopening from fresh configuration and
cancels any outstanding receipt. It never retags the old frame to a new window.
Each pump processes at most 64 messages from its own window.

Cancel hides and invalidates immediately. It does not call unbounded
`device_wait_idle` or acknowledge a guest frame. Normal destruction follows both
fences and the present wait. If acquisition/submission/presentation remains
unresolved, the whole bounded backend (including loader, HWND and in-use handles)
is retained until process teardown. A process-wide poisoned owner prevents
repeated opens/leaks and returns SessionRestartRequired. Therefore production
integration **must host this presenter in a supervised child process**; the
launcher event loop must not silently leak a backend and reopen. The supervisor
must impose an overall deadline/cancellation policy and terminate/reap the
consumer on terminal loss. The presenter never waits for global GPU idle.

The worker must keep its original opaque core Frame, validate the session,
sequence and generation of a consumer receipt, and acknowledge only that object
at a safe native boundary. These public receipt values do not reconstruct core
authority. Window close/loss/cancellation must stop the supervised session
without acknowledgement. Guest flip events and real vblank integration remain
separate responsibilities.

## Primary API references and validation scope

- [vkWaitForPresentKHR](https://docs.vulkan.org/refpages/latest/refpages/source/vkWaitForPresentKHR.html):
  per-ID wait, timeout, replacement and out-of-date limitations.
- [VkPresentIdKHR](https://docs.vulkan.org/refpages/latest/refpages/source/VkPresentIdKHR.html):
  enabled feature and strictly increasing nonzero IDs per swapchain.
- [VK_EXT_swapchain_maintenance1](https://docs.vulkan.org/refpages/latest/refpages/source/VK_EXT_swapchain_maintenance1.html)
  and [VK_EXT_surface_maintenance1](https://docs.vulkan.org/refpages/latest/refpages/source/VK_EXT_surface_maintenance1.html):
  required instance/device dependencies and features.
- [VkSwapchainPresentFenceInfoKHR/EXT](https://docs.vulkan.org/refpages/latest/refpages/source/VkSwapchainPresentFenceInfoKHR.html)
  and [Khronos semaphore-reuse guide](https://docs.vulkan.org/guide/latest/swapchain_semaphore_reuse.html):
  presentation-resource lifetime, distinct from transfer fence/display wait.
- [vkQueuePresentKHR](https://docs.vulkan.org/refpages/latest/refpages/source/vkQueuePresentKHR.html):
  queue acceptance/order and finite but potentially blocking driver operation.
- [DwmGetCompositionTimingInfo](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/nf-dwmapi-dwmgetcompositiontiminginfo)
  and [DWM_TIMING_INFO](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ns-dwmapi-dwm_timing_info):
  NULL HWND on Windows 8.1+, cbSize, actual cRefresh and QPC fields.
- [DwmIsCompositionEnabled](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/nf-dwmapi-dwmiscompositionenabled),
  [GetSystemMetrics](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getsystemmetrics),
  [MonitorFromWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-monitorfromwindow),
  [GetMonitorInfoW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getmonitorinfow),
  and [DwmGetWindowAttribute](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/nf-dwmapi-dwmgetwindowattribute):
  explicit current display/visibility checks, not elapsed-time vblank synthesis.
- [CreateWindowExW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-createwindowexw)
  and [PeekMessageW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-peekmessagew):
  owned window creation and bounded same-window message dispatch.

Original pure tests cover sequence/generation/invalidation, monotonic actual
tick handling and actual staging-capacity admission. One ignored local-window
test exercises this real present/wait API with an original 64x64 image, exact
sequence receipt and cancellation. Source inclusion or compilation is not
evidence that the window test, extension admission or display was exercised.
