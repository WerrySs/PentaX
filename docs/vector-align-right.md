# Bounded packed align-right profile

The checked Rust mediator supports exactly legacy PALIGNR XMM and VEX
VPALIGNR XMM/YMM. MMX and EVEX forms remain unsupported. Translation policy,
host feature admission and the closed native YMM register whitelist are
unchanged; declaring SSSE3/AVX/AVX2 does not grant new native execution.

Each 128-bit lane concatenates source1 as the high half and source2 as the low
half, shifts right by the unsigned immediate byte count and extracts 16 bytes.
Legacy source1 is the old destination. Immediate 0 selects source2, 16 selects
source1, 31 selects source1's last byte followed by zeros, and 32..255 produce
zeros. Counts are not masked. YMM halves operate independently and never draw
bytes across the middle lane. All aliases are copied before publication.
Legacy upper YMM survives; VEX128 zeroes it and VEX256 replaces both halves.
RFLAGS, MXCSR and unrelated state remain unchanged.

The complete memory operand is checked and read even for a zero-result
immediate: 16 bytes for XMM and 32 for YMM. Intel Type4 requires 16-byte alignment
for legacy m128; VEX permits unaligned ordinary-memory operands. Existing
owner/source/continuation, prefix/address-size and whole-span permission rules
remain enforced. Fixed vector work and complete operation/byte-counter
admission precede the single state publication. Failure preserves CPU state,
guest memory and completed counters; attempted work remains cumulative.

Primary contracts are Intel SDM
[Volume2B PALIGNR](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2b-manual.pdf),
[Volume2A exception Type4](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2a-manual.pdf)
and Intel's official
[_mm256_alignr_epi8 reference](https://www.intel.com/content/www/us/en/docs/cpp-compiler/developer-guide-reference/2021-8/mm256-alignr-epi8.html).
VEX.W is ignored; vvvv encodes the actual first source. Both vector lengths
have explicit ISA forms. No reserved-vvvv or VEX.L0-only rule is invented.

Original fixtures cover all 256 immediates with independent literal byte
windows and reused bounded paused-backing setups, source/destination aliases,
lane boundaries, upper-state rules, full reads for high counts and atomic
refusal. The native consumer uses six register/memory cases and independent
literal results, preserving flags, MXCSR, stack and unrelated registers.
These source fixtures make no execution, game-boot or frame claim.
