# Bounded ROUND instruction profile

The checked Rust vector mediator supports exactly the four legacy SSE4.1
ROUNDPS/ROUNDPD/ROUNDSS/ROUNDSD forms and their six VEX forms (packed128/256
and scalar128). Translation policy and native instruction authority do not
change. EVEX VRNDSCALE, XOP and unrelated instructions are not aliases.

Integer operations on copied IEEE32/64 bits implement nearest-even, downward,
upward and toward-zero rounding. Immediate bit2 selects guest MXCSR.RC instead
of immediate bits1:0; bit3 suppresses newly raised precision only. Existing
sticky precision and other status bits survive. Signaling NaNs retain sign
and payload while becoming quiet and raising invalid; quiet NaNs, infinities,
signed zeros and finite values already integral retain their bits. DAZ turns
subnormals into signed zero before rounding; without DAZ, inexact subnormals
raise precision but never denormal status. FTZ cannot change integral results.
The instruction pseudocode uses only immediate low4; high4 do not become a
VRNDSCALE scale selector. No Rust floating-point rounding, optional host SSE4.1
or AVX instruction, or changed host floating-point environment is required.

The existing live host-compatible MXCSR mask and all-masked exception policy
remain mandatory. Unsupported or unmasked control values refuse before state
publication; this profile does not emulate guest SIMD exception delivery.
Legacy packed memory reads all16 bytes with16-byte alignment. Legacy scalar
reads exactly4/8 bytes. VEX packed reads all16/32 bytes and scalar reads4/8;
these VEX forms permit unaligned ordinary-memory inputs. Existing segment,
address-size, prefix, source/continuation and whole-span permissions remain.

All inputs are captured before results, including aliases. Legacy scalar high
low128 lanes come from the old destination, VEX scalar lanes from source1.
Legacy upperYMM survives; VEX128 zeroes it and VEX256 replaces all32 bytes.
RFLAGS and unrelated state survive. Fixed per-lane work, operation and byte
counter admission precedes the single machine-state publication. Refused
memory, control or quota leaves vectors, MXCSR, PC and completed counters
unchanged; attempted work remains cumulative.

Primary contracts are Intel SDM
[Volume2B ROUND entries](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2b-manual.pdf),
[Volume2A exception types2/3](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2a-manual.pdf)
and [Volume1 sections4.8.3.5/12.8.4](https://cdrdv2-public.intel.com/671436/253665-sdm-vol-1.pdf).
VEX scalar LIG/WIG and packed WIG are not replaced with invented reserved-bit
faults; packed reserved vvvv and LOCK retain decoder rejection.

Original source proofs cover literal endpoint/control/NaN oracles, all ten
forms and merge/alias rules on actual paused backing, exact scalar end-of-page
reads, full-span/permission/prefix/work/counter refusal, and native guest
continuations consuming results and sticky status. Adjacent VMOVAPS, VMULPS,
VPSUBD and FMA213/231 packed forms already have closed implementations; this
slice adds no unrelated operations. No execution or game-frame result follows
from the existence of these source fixtures.
