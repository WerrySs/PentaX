# Bounded surface presentation

The port and flip lifecycle adapts `graphics/presentation/videoOut.cpp` at
KytyPS5-Performance revision `209dffe9032d72749257ce18680d506b891284de`.
The checked RenderTarget64KB address transformation adapts the same revision's
`graphics/guest_gpu/tile.cpp`, blob
`16d86b8cb6b2ac6248fe81ddbab3091c0763788e`. Original copyright is
Copyright (c) 2021 InoriRus, GPL-2.0-only; the root LICENSE and core NOTICE retain
the license and source attribution.

Registered surfaces use explicit RenderTarget64KB geometry. `detile_32` and
`retile_32` accept an explicit `ImageLayout`; they never infer linear layout from
an address or a matching byte count. The admitted tiled profile is one level,
one layer, one sample and 32 bits per pixel, with 128 by 128 pixel macroblocks.
Offsets include the pinned source's in-block XOR equation and block XOR.
Pure conversions require an exact full source extent and bounded dimensions,
actual output capacity, combined held bytes and cumulative work. Retiling
initializes padding to zero. A later guest LOAD/writeback path must overlay
active pixels onto its retained original full tiled snapshot so unknown padding
survives; the zeroed conversion product alone is not that writeback operation.
These byte conversions confer no guest authority.

Scanout snapshots separately validate the actual exclusively paused native
owner and the entire registered data extent with GPU read permissions. CPU read
rights do not grant scanout access; GPU-only read backing is supported. Actual
guest pixels are copied and detiled before the request becomes prepared.
Uncompressed RGBA8/BGRA8 and RGB10A2/BGR10A2 are converted to tight RGBA8;
10-bit channels use rounded integer UNORM conversion. The original pixel format
and premultiplied-alpha option remain attached to the frame. PQ, half-float,
DCC, other tiling profiles, multisampling and color-management guarantees are
not provided by this profile and refuse explicitly.

Each port admits at most 16 requests; the three-port queue has 48 fixed slots.
Opaque globally unique tickets bind the actual native owner, handle and port
generation. The lifecycle is reserved, recording after a successful pixel
snapshot, ready after supported completion, and presenting after FIFO hand-off.
The oldest incomplete request blocks later hand-offs. Buffer retirement waits
while requests remain. Closing a port cancels retained requests and invalidates
old frame acknowledgements before its next generation opens.

`complete_flip` completes only CPU preparation. A GPU-EOP tag alone never
provides fence evidence. The ordered graphics path separately consumes a private
`CurrentDraw` holding the actual owner's exclusive pause and a genuine fenced,
retiled writeback. It compares the current registered surface, port generation,
full padded range, dimensions and pitch before capture. The admitted bridge is
uncompressed four-byte RGBA8 storage, with no metadata/options/DCC or compositing.
It captures owned pixels and marks only GPU readiness. The RGBA8_UNORM draw
receipt does not establish an sRGB transfer conversion or color-management
equivalence merely because the registered byte layout matches.

All packet/state admission and conservative queue/capture quotas precede GPU
effects. Each draw prepares live resources in order, so later LOADs see earlier
writebacks. A flip consumes its fresh current draw; a later draw may create new
authority for another flip. No old public report, address, source tag or resumed
owner can recreate that proof. A failed capture rolls back only its new
reservation/frame/status; earlier queued frames and irreversible draw effects
remain. Any failure after possible effects leaves the driver terminal, preventing
prefix replay. A full queue returns a controlled `Waiting` stop before execution;
consumer-driven suspension/wakeup is not implemented by this path.

`take_frame` requires a nonzero caller-provided real vblank count, observes the
configured flip-rate interval and permits at most one acknowledged frame per
port in a tick. It transfers the owned frame buffer to a separately bounded
worker/UI mailbox. Hand-off alone advances neither flip count nor event state.
After actual consumer presentation, `acknowledge_frame` accepts the opaque frame
once, records real caller process clocks and returns a noncopyable completion
receipt containing the exact owner, generation and event argument. The source
128-byte flip-status layout is encoded with zero reserved fields.

The flip queue does not advance the display clock or acknowledge a frame from
a numeric tick. The driver separately materializes an owned monotonic display
grid and delivers bounded kernel equeue events. The window/IPC integration must
still bound outstanding frame capacities and acknowledge real presentation. Master/slave compositing and explicit black/blank flip indices
remain typed refusals. No window, real game image, frame rate, display
acknowledgement or kernel event delivery is established by GPU readiness.

The exact R_FLIP route follows public KytyPS5/KytyPS5 revision
`7b7f9a70c652afd6ec8ea8599884d9b4b3abc9f3`, `src/libs/agc.cpp`,
`src/graphics/guest_gpu/command_processor/pm4Handlers.cpp`,
`src/graphics/presentation/videoOut.cpp` and
`src/graphics/host_gpu/renderer/sync.cpp`. Exact blobs and retained licensing
are recorded in the core NOTICE and upstream provenance document.

The canonical GetVblankStatus, ColorSettingsSetGamma and AdjustColor exports
retain the pinned 209dffe source layouts and error order. Gamma is the actual
captured XMM0 float argument. Range checks accept unordered NaNs exactly as the
source comparisons do; only four settings bytes change. AdjustColor reads only
that field after validating the owned open handle. The retained gamma survives
close/reopen as in the source; it does not attest host color correction.

Each opened port has its own display epoch using the existing kernel process
counter and validated native frequency. The configured display policy selects
the source 60/120 Hz cadence or the explicitly configured follow-vblank rate.
Queries and paused scheduler readiness boundaries materialize elapsed periods
with fixed three-port arithmetic. Same-time queries cannot create periods. Mode
changes and main-port closure first materialize the old cadence, then rebase
at the same clock sample without retroactively applying the new rate.

The exact 40-byte status records the latest elapsed boundary, including process
microseconds, raw counter and process counter; flags/phase/padding stay zero.
Missed periods coalesce into existing bounded event counters without a per-period
loop. Event admission and optional guest output precede metadata/event commit;
failed output leaves that batch unconsumed. A parked WaitEqueue can wake before
any registered surface, prepared pixels or window exists. The scheduler uses
its existing bounded idle slices and one display-clock authority. Only an
authenticated waiter on the exact generation-matched subscribed equeue gains a
display wake deadline; unrelated waits retain their deadlock/error behavior.
Consumer tick
numbers retain generation/replay checks but cannot double-count periods. These
counts describe emulated display boundaries, not rendered frames or measured FPS.
