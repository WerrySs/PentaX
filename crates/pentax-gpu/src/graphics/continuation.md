# Selected flip wait: owned native continuation

The exact public packet `c0051018, handle, buffer, 0, 0, 0, 0` is a sealed
ordered graphics action. The handle and signed buffer index select the real
VideoOut pending requests, including CPU and GPU requests in every incomplete
phase. Taking a Frame does not complete a request. Its actual owned
acknowledgement removes it. Closing/reopening the numeric port changes its
generation and refuses an old continuation.

The driver owns the captured plan, action position, current register state,
source-range exclusions, reusable transfer staging and admitted limits across a
wait. It retains no paused-memory borrow or completed-draw receipt. Before
parking, prior synchronous GPU work has completed its fence and owned writeback.
The saved prefix remains represented by an in-flight execution while the
presentation consumer can take, present, acknowledge and deliver events. A
terminal failure remains poisoned and cannot be cleared by a saved prefix latch.

Opaque process-unique request identities and per-wait generations prevent
foreign-driver, reused-slot and old-step replay. New submissions and frame
boundaries enter the same bounded FIFO. Queued source ranges are captured only
when their admitted request reaches the head. A later submission cannot change
the active context. Dynamic arrays, arguments and payloads retain their normal
current-owner/permissions/physical-alias checks at their actual execution turn.

Retained execution and queued metadata capacities share the existing host and
presentation consumer ceilings with queued Frames and external transport memory.
They remain charged while parked and on failure, until actual owner cleanup.
No existing quota is increased. Wait streams admit flip slots and capture work
at each actual flip turn; no-wait submissions retain their complete preflight.
A wait consumes old draw eligibility, so any later GPU flip still needs its own
actual completed draw.

The scoped WaitUntilSafeForRendering producer emits only this packet through
the existing checked seven-word allocation and actual grow callback. Port and
buffer readiness are resolved at submission time, rather than by the producer.
The original fifty-two builder entries and all earlier provider ordinals remain
stable; the exact producer appends two matching module/library scopes.

Native HLE binds an opaque graphics ticket to the original owner, TLS thread,
marker PC, service ID and six arguments. Before advancing, it checks the active
source lineage and the executor's full private pending syscall token. A new
selected wait returns an explicit replacement PendingThreadWait. The worker
replaces only that wait object, retaining the same original token, complete CPU,
TLS, phase and callback frames; only whole-submission completion retires the
syscall. Readiness scans under another selected TLS only observe the driver.
Existing non-graphics wait completion keeps its Option API. The legacy scalar
submit path refuses a wait stream before guest effects.

When every guest thread parks, the existing FrameBridge pump runs before
scheduler rotation and consumes only actual retained Frame/Presented ownership.
Its actual acknowledgement and notification delivery precede guest re-entry.
No timer, vblank, frame handoff or scheduler Boolean makes a wait ready. Terminal
owner teardown revokes retained execution/FIFO storage without consuming work
budgets; it never turns an abandoned submission into a guest return.

Original ordinary fixtures consume actual VideoOut reservations, recording
snapshots, CPU Frames and opaque acknowledgements. Exit-only native-token and
scheduler fixtures consume successor waits under the same original token.
One separately ignored driver fixture uses an actual GPU draw, flip, window
presentation completion and owned acknowledgement before its driver suffix can
run; it does not exercise native HLE submission. Source review is distinct from
compiling or running these fixtures, and from a rendered game frame.

Primary public provenance: KytyPS5 revision
`7b7f9a70c652afd6ec8ea8599884d9b4b3abc9f3`,
[`agc.cpp`](https://github.com/KytyPS5/KytyPS5/blob/7b7f9a70c652afd6ec8ea8599884d9b4b3abc9f3/src/libs/agc.cpp)
WaitSafe producer and the corresponding `pm4Handlers.cpp`, `graphicsRun.cpp`
and `videoOut.cpp` selected CPU/GPU wait. Existing attribution and GPL-2.0-only
requirements apply. No upstream implementation or guest shader was copied.
