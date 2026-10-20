# Native worker frame bridge

The explicit `probe-entry --present-window yes` consumer path pumps outside native entry and every paused-memory
borrow. Ordinary probes pass no consumer and retain their execution, scheduling
and capture policy. A consumer supplies bounded byte transport; transport
acceptance, export, window creation and texture upload do not acknowledge a
guest flip.

The window option also selects the finite
[presented-startup graphics work policy](presented-startup-graphics-work.md).
It changes only per-draw preparation and graphics backend work ceilings;
compute, memory, cumulative driver work and supervisor limits remain separate.

The core exposes an allocation-free peek at the oldest READY request. An older
incomplete or already handed-off request blocks later frames. The peek checks
the actual native owner and returns dimensions, declared color/alpha intent,
actual retained capacity and port generation. It supplies no frame ticket,
pixel address, display tick or acknowledgement authority.

The bridge sends Configure from that genuine metadata before requesting a
frame. It requires the exact Ready echo, then a separately observed increasing
display tick. A take moves the existing owned core Frame into the bridge; its
numeric wire identity cannot recreate core authority. One frame remains owned
until an actual Presented receipt and successful core acknowledgement. A
retryable process-clock refusal retains the same Frame and receipt without
resending or presenting again. EOF, cancellation, protocol failure and timeout
never acknowledge it. Returning from guest execution with an unfinished
consumer lifecycle is an explicit cancellation outcome.

Each control header passes identity, phase, dimensions and sequence checks
before payload allocation. Controls use bounded inline input and at most one
decoder payload. Writes advance exactly once when the transport owns a copied
chunk, then wait for that write's completion. The transport reserves its
maximum simultaneous decoded-frame, presenter or endpoint capacity in advance
and reports actual retained capacity. Original core limits are only tightened:
the common ceiling includes queued captures, the external original Frame,
transport, decoder and staging. Moving a frame out of the queue transfers its
capacity charge atomically; by-reference acknowledgement alone does not release
the external allocation charge.

The native loop pumps at bounded block intervals and while parked. A genuine
pending consumer deadline provides a finite external wake before the scheduler
declares an indefinite guest deadlock. Requested idle slices have their own
cumulative budget and never satisfy a guest wait, consume its timeout quota or
enter a parked CPU. Handshake, frame, session and pump ceilings prevent an
unresponsive consumer from extending the run indefinitely.

The local presenter adapter runs on the same
supervised worker thread, outside guest execution, and may emit Ready, observed
ticks and Presented only from its actual platform lifecycle. The bridge does
not infer OS presentation from successful transport I/O. The local nonce is a
checked process-unique, nonsecret serial identity; it makes no cross-process
security claim. Launcher Play integration remains separate from this explicit
supervised-worker option.

Actual observed ticks publish distinct VideoOut vblank events; the first
observed refresh establishes a port-local observation epoch. Subsequent count
increments use the actual observed refresh delta, not a guessed refresh rate.
Actual acknowledged Frames yield one owned FlipCompletion, consumed only after
publication through real retained VideoOut subscriptions. Clock refusal retains
the observation for bounded retry. Other delivery refusal leaves it retained
and returns explicit NeedsEventDelivery backpressure. Queue delivery failure
cannot repeat acknowledgement or presentation.

These events use filter -13, flip ident0 and vblank ident1; they never signal an
AGC interrupt. The bounded default-coalescing profile keeps the latest payload,
saturates both source counters at15, preserves an active event's queue position
and resets it only after successful guest output. Subscription identity includes
the live port generation. Closing the port or deleting its queue removes the
subscription; generation replacement cannot revive it. Public KytyPS5 revision
7b7f9a70c652afd6ec8ea8599884d9b4b3abc9f3 supplies VideoOut identities,
registration and packed event fields. The separately retained authorized
209dffe source supplies the default-coalescing queue policy already used by
PentaX. Public7b uses queued copies; its unbounded legacy trigger profile is not
claimed or enabled. Source provenance remains distinct.
Sharing one queue's same event kind between different live ports is an explicit
typed refusal; this initial profile retains one exact port-generation owner per
subscription instead of approximating upstream cross-port registration races.

No operation fabricates a kernel event, display observation or presentation.
Changed dimensions or port generation require session replacement; in-place
reconfiguration is refused. This source slice establishes no rendered game
frame, measured FPS or compatibility with a particular title.

Original source fixtures cover fragmented Ready, send completion, independent
tick metadata, premature/foreign control refusal, EOF/cancel/deadline behavior,
staging limits and independent scheduler wake budgeting. The existing actual
owned-memory CPU-flip fixture also checks metadata peeking, foreign-owner
refusal and queue-to-external accounting. These descriptions state intended
proof scope; execution results must be recorded separately.
