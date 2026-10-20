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

`complete_flip` currently completes only CPU preparation. A GPU-EOP tag never
provides fence evidence and cannot advance a request to ready. The separate
linear draw completion receipt does not attest a RenderTarget64KB buffer;
fenced execution, explicit validated retile/writeback and same-surface receipt
integration are required before enabling GPU-EOP completion.

`take_frame` requires a nonzero caller-provided real vblank count, observes the
configured flip-rate interval and permits at most one acknowledged frame per
port in a tick. It transfers the owned frame buffer to a separately bounded
worker/UI mailbox. Hand-off alone advances neither flip count nor event state.
After actual consumer presentation, `acknowledge_frame` accepts the opaque frame
once, records real caller process clocks and returns a noncopyable completion
receipt containing the exact owner, generation and event argument. The source
128-byte flip-status layout is encoded with zero reserved fields.

The queue does not create a vblank clock, deliver kernel equeue events, present a
desktop texture or implement IPC. Those integrations must retain actual event
owners, bound outstanding frame capacities and acknowledge only real consumer
presentation. Master/slave compositing and explicit black/blank flip indices
remain typed refusals. No window, real game image, frame rate or GPU-EOP
completion is established by the original unit fixtures.
