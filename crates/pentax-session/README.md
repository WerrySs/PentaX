# Private frame-session codec

`Configure` (kind 5) and `Ready` (kind 6) carry actual surface dimensions and
alpha/color flags with sequence zero and no payload. Both validate the same
frame-size bounds before any allocation. The consumer opens the real window
from Configure, then echoes that exact configuration with Ready. The bridge
checks `matches_configuration` and its own phase before accepting the echo.
Neither message consumes a frame sequence, observes a refresh or acknowledges
presentation. Vblank remains an independent real observation. Configuration
cannot be accepted while a frame is outstanding. This ordering permits the
first real tick before core frame handoff, without guessed dimensions.

This portable crate defines a separate binary session protocol. It does not
change the existing worker probe/diagnostic protocol. It performs no I/O,
creates no endpoints and supplies no guest-memory, vblank or presentation
authority. It contains only original code and bounded original fixtures.

Version 1 uses this exact 64-byte little-endian header:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 8 | `PNTXSES` followed by NUL |
| 8 | 2 | Version, exactly 1 |
| 10 | 2 | Closed message kind |
| 12 | 2 | Header size, exactly 64 |
| 14 | 2 | Closed frame flags; also retained by Configure/Ready; zero on other controls |
| 16 | 16 | Nonzero session nonce |
| 32 | 8 | Frame sequence, starting at 1; zero for Vblank/Close/Configure/Ready |
| 40 | 8 | Nonzero window generation |
| 48 | 4 | Width; also set on Configure/Ready; zero on other controls |
| 52 | 4 | Height; also set on Configure/Ready; zero on other controls |
| 56 | 4 | Exact payload length |
| 60 | 4 | Reserved, exactly zero |

Kinds are FrameRgba8=1, Presented=2, Vblank=3, Close=4, Configure=5 and Ready=6.
FrameRgba8 contains
exactly `width * height * 4` tightly packed RGBA8 bytes, with stride `width * 4`.
Presented contains no payload and echoes the exact outstanding frame identity.
Vblank contains exactly one nonzero little-endian u64 observation count. Close
has no payload. Unknown kinds, versions, flags and reserved fields refuse.

Flag bit 0 preserves straight (0) or premultiplied (1) source alpha. Bit 1
describes unspecified source encoding (0) or explicitly proven sRGB-encoded
bytes (1). Other bits refuse. This codec never changes alpha, normalizes color
channels, unpremultiplies or performs a transfer/gamma conversion. A sender maps
the real core Frame's metadata to these flags; an unproven source encoding must
remain Unspecified. An opaque presenter must explicitly choose whether ignoring
alpha is supported; the transport does not silently relabel pixels as opaque.

Default hard policy is at most 4096 per dimension, 16 MiB per frame and 64 MiB
simultaneous dynamic CPU staging. Limits may be tightened, never raised past
those maxima. These are PentaX transport limits, not firmware limits. Presenter
GPU storage needs its own independent bound.

`Decoder::new(nonce, generation, limits)` binds the expected private session.
`feed(input, held)` accepts arbitrary fragments and returns the exact consumed
count and at most one complete Decoded message. It leaves following input for
the next call. The whole header, including expected nonce/generation and exact
pixel byte product, is validated before fallible payload allocation. Requested
and actual Vec capacity plus `held` are checked; `held` includes the input's
actual backing capacity, earlier returned messages, the retained original core
Frame and consumer conversion buffers. It excludes only the decoder's separately
reported retained bytes. Fixed header metadata is inline. The endpoint owner
must keep this accounting accurate and bound each pump's message/work count.
Endpoint pumps use `feed_checked(input, held, |h| session.check_incoming_header(h))`
so known stale sequences, duplicate receipts and occupied-slot frames refuse
before allocation as well. That check is read-only. Final Session admission
still occurs after the complete validated payload arrives; a header alone does
not reserve a received frame or consume a receipt.

Every decoder error, EOF or Close is terminal. Partial payload capacity is
released. EOF distinguishes a truncated message from a clean closed stream;
neither case acknowledges a frame. Do not resume a failed decoder against a
new stream. Window generation may advance only at a clean message boundary,
in coordination with a Session that has no outstanding frame.

`Session` is allocation-free and not Clone. `begin_frame` reserves the sender's
one outstanding identity; `accept_frame` checks the consumer's exact next
sequence and one-slot ownership. Sequence increments are checked and never reset
across a generation change. Reordered, duplicate, stale or foreign incoming
messages terminate the session while retaining pending metadata. A frame cannot
be replaced or a generation advanced while that frame remains outstanding.

After independently verified actual presentation, the consumer calls
`receipt_for_presented`, transmits the returned empty-payload header on its
dedicated control endpoint, then releases its local slot with `complete_frame`.
An I/O failure terminates instead. The worker's `receive_control` validates the
same identity and marks receipt-observed without releasing its original opaque
core Frame. The worker calls the real core acknowledgement at a safe native
boundary and calls `complete_frame` only after that succeeds. A clock failure
leaves the same receipt-observed slot and original Frame available for a local
acknowledgement retry. It must not present or transmit that frame a second time.

Vblank observation remains independent: receiving it updates only the checked
monotonic observation count, never receipt-observed or frame completion. Decoded
numbers do not prove either presentation or a genuine tick. Only the separate
authenticated endpoint/presenter owner can establish their physical provenance;
SwapAccepted, texture upload and repaint requests cannot become Presented here.

On Close, EOF, timeout, window loss or cancellation, terminate and stop/reap the
supervised worker without acknowledgement. Retain the outstanding original Frame
until that owner shuts down. There is no cancel/requeue capability in this crate.
Do not send frame bytes on diagnostic stdout/stderr, dump them to files, retain
unbounded queues or reconstruct core authority from FrameId numeric fields.

The source fixtures cover literal wire geometry, all split boundaries, exact
message consumption, malformed/preallocation refusal, simultaneous capacities,
EOF, receipt retry, one-slot backpressure, stale/reordered identities, generation
changes, overflow, separate vblank and terminal ownership. They are not a live
window, game-frame or FPS proof.
