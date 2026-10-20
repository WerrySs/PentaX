# pentax-shader

A dependency-free safe Rust port of the shader decoding frontend from
[KytyPS5-Performance at 209dffe9032d72749257ce18680d506b891284de](https://github.com/WerrySs/KytyPS5-Performance/tree/209dffe9032d72749257ce18680d506b891284de).
This is a modified GPL-2.0-only adaptation, with retained notices in [NOTICE](NOTICE),
[LICENSE](LICENSE) and [Kyty-MIT.txt](Kyty-MIT.txt). The pinned fork is private;
source provenance identifies the authorized working source rather than implying
that its link is publicly accessible.

## Coverage

Every family recognized by the pinned classifier has a ported decoder:

| Family | Source map entries | Words |
| --- | --- | --- |
| SOP1 / SOP2 / SOPK / SOPC / SOPP | 47 / 50 / 23 / 18 / 21 | 1, +1 shared literal |
| VOP1 / VOP2 / VOPC | 63 / 50 / 135 | 1, +1 SDWA/DPP/DPP8 word, +1 shared literal |
| VOP3 | 82 native + VOPC, VOP2 (seven excluded) and 58-entry VOP1 aliases | 2, +1 shared literal |
| VOP3P | 22 | 2, +1 shared literal |
| VINTRP / EXP | 3 / all targets | 1 / 2 |
| SMEM / MUBUF / MTBUF / FLAT / DS | 13 / 52 / 8 / 15 / 62 | 2 |
| MIMG | 56 sample, 8 gather, 15 atomic, 8 other | 2 + NSA dwords (0-3) |

Ported rules include the per-opcode vector destinations (VCC/EXEC compares,
scalar lane-read destinations, VOP3B carry destinations, VOP2 implicit
VCC/literal operands), source counts, literal sharing across all four source
slots (including literals after SDWA/DPP modifier words and VOP3 bodies), the
VOP2/VOP1 SDWA translator profiles, DPP16/DPP8 fields, VOP3 native modifier
classes, VOP3P op_sel/neg_hi/mix-abs handling, memory offsets/flags/formats,
DS two-address scaling and ordered-count fields, D16 partial halves, MIMG
dmask/D16/NSA addresses, sample/gather flags, address component counts, BVH
address dwords and the source `ImageAddressComponentLayout` helpers.

`decode` accepts a caller-delimited dword span and stage, owns the source, and
returns immutable instruction views tied to that owner. Views expose exact
encoded words, family/opcode IDs (aliases keep their encoded ID), operands with
SDWA/DPP/VOP3 modifiers, the secondary destination, `MemoryAccess` and `Image`
metadata (fixed 12-entry NSA array), export flags and signed branch metadata.

## Errors and unsupported source cases

Source aborts are `UnknownFamily` or `UnsupportedOperand`; "opcode is not
implemented" is `UnsupportedOpcode`; every other source `SetUnsupported` case
is `Unsupported` with a typed `UnsupportedReason` carrying the source's static
message. These include FP64 SDWA/DPP, SDWA selector/profile/modifier limits,
VOPC SDWA/DPP opcode limits, unsupported VOP3 modifiers and perm-lane op_sel,
VOP3P integer clamp, FLAT LDS/segment 3, DS ordered-count/swizzle-FFT/GDS
lane/addtid restrictions, BVH dmask/R128/TFE-LWE/NSA requirements, gather and
atomic dmask, 64-bit atomics and D16. Checks run in the source's order: VOP3
reads its second word and rejects modifiers before an unmapped opcode, VOPC
DPP/SDWA read the modifier word first, and where the source keeps decoding
after `SetUnsupported` (SMEM, MUBUF, MTBUF, MIMG) an operand abort wins and
otherwise the last recorded MIMG reason is returned. `Truncated` reports the
total length known when a needed word is missing (for MIMG, 2 + NSA). No
length is guessed and no partial span is returned. `UnsupportedFamily` remains
in the API but is no longer produced.

## Retained source behaviour that is not ISA validation

Reserved bits and register selectors accepted by the pinned decoder are
accepted here; syntax success does not establish ISA or firmware validity.
Notably: a MUBUF/MTBUF SOFFSET of 255 stays an unresolved `LiteralConstant`
with value 0 (those families never read a literal word); a FLAT VGPR pair
continuation keeps the unchecked `addr + 1` (VGPR 256 is possible); VOP3P
modifiers are written to all three slots even when two sources are used; MIMG
DIM encodings 3 and 5 both map to `Dim2DArray` (raw bits are retained); MIMG
data dwords are not updated for 64-bit atomics. BVH intersections are decoded
unconditionally; the program walker's disable-and-stop policy is not ported.

## Intentional differences from the original walker

This API decodes the entire linear span, including words after ENDPGM. It does
not port `DecodeProgram`/`DecodeFrontProgram`, infer code boundaries, follow
branches or validate a shader CFG. Signed branch targets may be negative,
outside the span or inside literal data; a future IR/CFG builder must validate
them. Stage is metadata and does not assert stage legality. Register pair
constraints, hardware semantics, resource/descriptor numbering, SPIR-V/Vulkan
translation, execution and debug-branch policy are outside this crate.

## Bounds

Limits cover logical input and actual Rust-reported source capacity, instruction
count, actual record-vector capacity bytes (`capacity * size_of` of the larger
record, which still fits the default instruction limit) and work. Work charges
source copying, every consumed instruction word (each charged after it is
proven present and before it is read) and existing records potentially moved
during bounded geometric record growth. Opcode lookups are fixed match tables
with constant cost. Records contain no nested allocations. Every reservation
uses `try_reserve_exact`; allocation errors return no partial span. Diagnostics
are `Copy` values with static strings. An allocator that aborts the process is
outside recoverable-error guarantees.

All tests use newly authored synthetic words, with no game fixtures, upstream
fixtures or binaries.

Source mapping at the pinned revision:

- [Shared family/operand decoder](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/graphics/shader/recompiler/frontend/decode/ShaderDecoder.cpp)
- [Scalar decoder and opcode maps](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/graphics/shader/recompiler/frontend/decode/ScalarAluOps.cpp)
- [Vector decoders and opcode maps](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/graphics/shader/recompiler/frontend/decode/VectorAluOps.cpp)
- [Memory decoders and opcode maps](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/graphics/shader/recompiler/frontend/decode/MemoryOps.cpp)
- [Image decoder and opcode maps](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/graphics/shader/recompiler/frontend/decode/ImageOps.cpp)
- [Export decoder](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/graphics/shader/recompiler/frontend/decode/ExportOps.cpp)
- [Stage vocabulary](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/graphics/shader/shader.h)
