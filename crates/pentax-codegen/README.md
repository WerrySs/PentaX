# PentaX code generation

An explicit, default-off `with_affine_loop_trace()` request permits a separate
optional two-iteration template on a materialized self-target conditional loop.
The ordinary block remains unchanged and contains no guest branch. The template
requires ordinary memory and exactly one nonzero 32/64-bit INC or ADD-immediate
of an address register after all accesses, excluding other address writes,
stack use, atomic/streaming operations and division. Its sole internal branch
is forward: an inverted first condition skips the second body. Both bodies keep
their original instruction order and RIP-relative targets. A checked native
runtime must independently bind the private exits, validate both iterations'
live source/owner/operands and distinguish one versus two retired iterations.

Template preparation is a separate, bounded cold attempt after an exclusive
work reservation. Optional work and actual allocation capacities are accounted
without changing ordinary-plan usage; instruction, output, allocation and cache
ceilings remain finite. An ineligible shape, wrapping induction or insufficient
optional allowance leaves ordinary execution available. Retained cache hits
reuse the source/placement-specific template without encoding. This is neither
an execution capability nor evidence of a title performance improvement.

`pentax-codegen` builds immutable same-ISA x86-64 basic-block translation plans.
It emits bytes; it does not allocate executable memory, run native code, expose
host pointers, install exception handlers or authorize execution. The crate's
own Rust code forbids `unsafe`. Its decoder/encoder is the pinned, pure-Rust
`iced-x86` 1.21.0 dependency, rather than a second byte parser.

VPMADDWD remains mediated in both VEX widths, retaining its exact register or
complete memory source and original RIP target. Declared AVX/AVX2 features do
not broaden native policy. Legacy XMM PMADDWD keeps its existing SSE2 native
admission; the pure mediator tests exercise its checked fallback through an
explicit missing-feature declaration. MMX and EVEX remain excluded.

Under the default policy, legacy PCMPGTQ and VEX.128/256 VPCMPGTQ retain their
exact operands, complete source widths and original RIP targets in mediated
exits. Declaring SSE4.2, AVX or AVX2 alone never emits these instructions.
The existing legacy byte/word/dword comparison admission remains unchanged;
EVEX mask-destination comparisons require separate semantics and stay outside
the runtime's three-form qword comparison profile.

Saturating PACKSSWB, PACKSSDW and PACKUSWB retain their existing SSE2 legacy
native admission; this change does not force those instructions through a
software exit. Under the default policy, legacy PACKUSDW and all VEX.128/256
versions remain mediated, including when SSE4.1/AVX/AVX2 features are declared.
Their exact source operands,
16/32-byte read widths and RIP targets survive planning. The runtime implements
the twelve non-MMX, non-EVEX forms with signed source values, saturation to the
selected signed/unsigned destination range, and independent 128-bit halves for
YMM packing. Checked mediation requires 16-byte alignment for legacy memory
sources and permits unaligned VEX sources. Legacy upper state is preserved;
VEX.128 clears the upper half. Flags and MXCSR are unchanged. MMX/EVEX state is
not admitted by sharing a mnemonic; see the
[Intel instruction reference](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2b-manual.pdf)
entries for PACKSSWB/PACKSSDW, PACKUSDW and PACKUSWB.

## Source and ownership

`plan_block(&SymbolicLinkPlan, module_index, guest_pc, CachePlacement,
&TranslationPolicy, &BlockLimits)` retains the exact immutable linker plan and
selected module identity. It copies a bounded initialized executable window,
applies only that retained plan's ordinary eight-byte source-overlapping writes,
and decodes the resulting bytes. Original and relocated snapshots, instruction
guest/cache offsets and the copied immutable policy remain inspectable.

`plan_block` refuses any deferred relocation, even outside the selected block.
The separate caller-declared path below produces a distinct pure planning type.
Cross-segment instruction fetch and executable BSS synthesis are unsupported.
At a configured window boundary, a truncated instruction fails rather than
guessing bytes. A completed initialized window ends at a fallthrough exit.

The requested cache interval must be within the lower 47-bit address space and
disjoint, at 16 KiB page granularity, from every retained module mapping,
including page padding. It is a numeric request, not evidence of an allocation.
The native mapping owner must reserve the full checked interval and separately
establish actual host page geometry, ownership and permissions.

## Caller-declared materialized source

`declare_materialized_source(&SymbolicLinkPlan, &[DeclaredAppliedWrite],
&BlockLimits)` retains the exact source and owns bounded declaration records.
Every deferred record must have exactly one declaration, with the exact original
index, consumer, relocation record and checked biased target address. Only the
fixed eight-byte relocation encodings 16/17/18 are admitted. Targets must be
wholly within their consumer's logical mappings, ordered, nonoverlapping and
disjoint from all ordinary patches. The original deferred list remains visible.

`plan_materialized_block(&MaterializedSource, module_index, guest_pc,
staged_window, CachePlacement, &TranslationPolicy, &BlockLimits)` requires every
staged byte to equal the exact initialized source with ordinary and declared
writes overlaid, including partial intersections. It uses the same bounded
decoder, encoder, positive policy and exit construction as `plan_block`.
Declaration and per-block work/allocation limits are enforced independently;
ordinary and declared writes both count against the aggregate patch bound.

The returned `MaterializedBlockPlan` has read-only byte/provenance/exit getters
and no conversion or implicit dereference to `BlockPlan`. Its assurance is always
`CallerDeclared`: numeric values are supplied by the caller and are not proof of
TLS resolution, actual write application, prepared storage, live permissions or
execution admission. An unsafe native consumer must separately bind every
declaration to the exact prepared owner and verified typed-TLS plan, establish
owner liveness, and compare the actual paused initialized executable bytes. This
API alone cannot authorize execution or resolve unknown deferred records.

## Emission and exits

`TranslationPolicy::with_isolated_memory` splits before and after a potential
ordinary memory instruction, retaining batches of register-only work. The next
memory instruction is retried at its original PC after the preceding CPU state
has actually been observed. This is a planning option, not permission to run.

`with_independent_memory` batches accesses whose base/index GPRs are unchanged
by preceding instructions. Partial, implicit and conditional writes invalidate
the full aliased GPR; a dependent access starts the next block. Atomics remain
isolated. The checked dispatcher independently repeats this analysis and checks
every live span before entry, including later accesses in the same block.
Memory aliasing is allowed because native execution preserves instruction order.
Dependency masks come from the pinned
[Iced instruction-info API](https://docs.rs/iced-x86/1.21.0/iced_x86/struct.InstructionInfo.html).

`MemoryAccessAnalyzer` derives numeric read/write footprints from an emitted
record and supplied PRE-instruction GPR values. Its fixed two-entry result covers
explicit operands and implicit PUSH/POP/LEAVE memory, including POP destinations
relative to incremented RSP. RIP-relative operands retain their original target;
LEA/NOP address expressions cause no dereference. Conditional accesses are
conservative, and a read-modify-write requires both rights. Unknown/non-64-bit
addresses for actual accesses and variable or oversized footprints return
explicit errors. Exact ordinary BT/BTC/BTR/BTS memory forms use the complete
selected16/32/64-bit element. Register bit offsets are signed at operand width;
floor division selects the element before or after the decoded address.
Immediate offsets stay within that element, including high imm8 values; an
assembler's displacement adjustment is already part of the decoded address.
Bit-index GPRs join base/index dependencies in both block planning and live
execution checks, including preceding partial or conditional register writes.
The selected full element must pass ordinary live read or read/write checks;
no single-byte permission shortcut or allocation is introduced. Existing LOCK,
HLE, segment and address32 restrictions remain in force. Numeric footprints establish no live
backing or permissions;
the native consumer recomputes and checks those on the actual exclusive owner
immediately before execution. Iced's reused instruction-info scratch uses its
dependency allocator, outside the caller's fixed output value; recoverable
process-wide OOM is not promised.

Ordinary instructions pass a positive integer/SSE family policy, operand and
register checks, encoding checks, privilege/save-state checks and declared
CPUID feature gates. This covers decoder-defined forms of integer arithmetic,
logic, multiply/divide, shifts, bit operations, MOV/extensions, LEA, CMOV/SETcc,
GPR PUSH/POP/LEAVE and legacy XMM arithmetic, conversions, shuffle and loads/stores.
The thirteen VEX-encoded BMI1/BMI2 GPR families are admitted only with their
declared feature bits and 32/64-bit GPR operands. Their memory reads retain
isolation and relocation checks. Other VEX/EVEX families remain mediated by default; adding
BMI support does not authorize vector state. Native consumers must check actual
host BMI support before entry; no software BMI fallback is provided here.

`with_native_ymm_registers()` explicitly requests a separate closed profile of
70 exact register-only VEX forms: VMOVDQA/VMOVDQU in both opcode directions,
VPAND/VPANDN/VPOR/VPXOR, VPCMPGTQ, and VPACKSSWB/SSDW/USWB/USDW, each with
128/256-bit vectors, plus VADDPS/PD, VSUBPS/PD and VMULPS/PD at both widths,
VMOVAPS/UPS/APD/UPD in both opcode directions and VAND/ANDN/OR/XOR PS/PD
at both widths.
Every operand must be a same-width XMM0–15 or YMM0–15. This request alone excludes
memory encodings, GPR transfers, scalar or other floating-point operations,
vector-state control and EVEX. Existing legacy SSE2 admission is unchanged.
The request requires declared AVX plus each decoded instruction's exact feature
requirements, including AVX2 for the 256-bit integer operations. Decoder
reserved-field checks and existing prefix gates still apply.

This policy flag is not an execution capability. The platform must independently
bind it to the exact machine's admitted full-YMM entry, trailer and cache mode,
including actual CPU support and OS-enabled vector state. The worker obtains
AVX/AVX2 declarations only from that retained engine's `native_ymm_features()`;
without it, the default mediated path remains in use. Its closed aggregate
`native_ymm_mode=sse2|avx|avx2` field reports that actual engine mode, not host
identity or the guest CPUID profile. No guest feature advertisement is changed,
and no performance gain is claimed without execution measurements.

The thirty-two additional copy/logical forms require only AVX, including their
256-bit forms. They transfer raw bits: signaling NaNs are neither classified
nor quieted, and MXCSR status and RFLAGS remain unchanged. VEX.128 replaces the
low half and clears the destination's upper half, even for a self-copy; VEX.256
replaces all 256 bits. Store-opcode register encodings still write the decoded
destination, and ANDN preserves `(~source1) & source2` order. This extension
does not admit any copy/logical memory encoding, including when the separate
arithmetic-memory request is enabled. Host control and full-YMM preservation
reuse the existing bridge; no bridge code or guest CPUID profile changes.

Original codegen and actual-platform fixtures enumerate all thirty-two forms,
directions and aliases using arbitrary bits and unchanged status. The native
worker consumer uses two finite original images with exact mode-aware counter
deltas. Newly native operations count as `native_instructions`, rather than
mediated `vector_ops`. Without the separate temporal-memory request below,
memory setup and readback retain their exact mediated bytes; enabling that
request changes those actual-path counts too. The ignored `native_vector_register_bits_original_baseline_measurement`
requires explicit independently verified `PENTAX_VECTOR_REGISTER_BITS_BASELINE_WORKER`
and `PENTAX_TEST_WORKER` artifacts. It compares the same 50,000-iteration original
inputs with intermediate/final snapshot layouts and bounded diagnostic drains. It asserts
results and counter differences, not a timing inequality or a title speedup.

Packed native ADD/SUB/MUL uses the existing checked host-compatible MXCSR
profile: all six numeric exceptions must be masked, reserved or unsupported
control bits are refused, and only admitted DAZ/FTZ and rounding settings can
reach the bridge. Logical source order is preserved, including NaN payload
priority; instructions are neither commuted nor fused. The bridge captures
sticky guest status and the full active vector state before restoring host
control. These twelve forms need AVX, including the 256-bit forms; AVX2 is not
required. Without the separate memory request below, memory arithmetic retains
exact live 16/32-byte software mediation.
Default policies and hosts without the admitted full-YMM mode retain that
fallback for register arithmetic too. No guest CPUID or control authority grows.

`with_native_packed_float_memory()` separately requests only the twelve VEX
ADDPS/PD, SUBPS/PD and MULPS/PD reads at 128/256 bits. It requires the full-YMM
request as well as declared and independently admitted actual AVX. Exactly three
operands are accepted: same-width XMM0–15/YMM0–15 destination and logical source,
then one ordinary 64-bit memory operand of exactly 16/32 bytes. Source order and
the existing masked MXCSR profile remain unchanged. Segment overrides, addr32
(including displacement-only absolute SIB), VSIB, writes and other vector
families remain excluded. The worker enables this policy only from its retained
engine's admitted vector mode. It adds no guest CPUID or control authority and
requires no new host ISA feature.

The immutable memory template still holds only decoded address expressions.
Every execution evaluates current GPRs, checks the complete live read and retains
owner, source, stack, cache and permission admission. The 32-byte footprint
exception applies only to one exact unconditional read from these twelve forms;
it is not a general increase of the ordinary memory-size ceiling. Existing
isolation and independent-address dependency splits remain unchanged. Native
memory arithmetic contributes to `native_instructions` and ordinary checked
memory bytes; mediated `vector_ops` and `vector_bytes` intentionally decrease.
The ignored `native_packed_float_memory_original_baseline_measurement` uses
explicit `PENTAX_PACKED_FLOAT_MEMORY_BASELINE_WORKER` (the independently verified
register-only reference artifact) and `PENTAX_TEST_WORKER`. Its two snapshot
layouts use 50,000-iteration original inputs with literal results and exact
counter checks. Concurrent bounded diagnostic drains are shared with the prior
measurement helper. Timings include process startup/teardown; no title speedup
or whole-run instruction-frequency inference is made.

`with_native_scalar_float()` separately requests exactly six decoded VEX
ADDSS/SD, SUBSS/SD and MULSS/SD instructions with either an XMM or memory right
source: twelve operand shapes. It requires the full-YMM request and declared
AVX; actual CPU/OS vector-state admission remains the native runtime's separate
obligation. No AVX2 feature is needed. All three registers, where present, are
XMM0..15. A memory right source must be one ordinary 64-bit address with an
exact 4/8-byte read; unaligned reads are allowed. Address32, segment overrides,
VSIB, writes, EVEX and adjacent scalar DIV/MIN/MAX/SQRT/FMA/conversion or merge
families gain no admission. The old register-only and packed-memory requests
continue to mediate scalar arithmetic when this separate request is absent.

Only the low scalar element performs arithmetic. The rest of the low128 bits
comes from logical source1, including when destination aliases either source;
the destination's upper128 bits are zeroed. Inactive source elements are copied
as raw bits and cannot raise numeric exceptions. The same admitted all-masked
MXCSR, ordered NaN operands, rounding, DAZ/FTZ and sticky status as the scalar
mediator apply. RFLAGS and unrelated registers are preserved. Guest unmasked
numeric traps, signal delivery and new control instructions remain unsupported.
The pinned encoder emits canonical scalar VEX.L=0, including a decoded LIG
input with L=1; original planning fixtures verify the actual emitted prefix.

`BlockPlan::emits_scalar_float_memory()` and its materialized counterpart are
immutable classifications of actual emitted reads, not requests or numeric
permission grants. They require full span/dependency preflight even on the
manual native path. The ordinary footprint evaluator already supports these
4/8-byte reads, so no generic size ceiling changes. Source identity, current
GPRs, live backing/permissions, owner, stack, continuation and cache attestation
still apply on every entry. Whole-block preflight can reject a later bad access
before earlier newly coalesced arithmetic executes; identical partial-fault
state across former mediated boundaries and precise guest fault recovery are
not claimed. Newly native scalar arithmetic contributes to native instruction
counts instead of mediated vector counters. The six-code source audit finds no
eligible instruction in existing worker fixtures with exact counters; their
literal results and counter assertions remain unchanged. New original native
proofs must establish actual execution separately from planning/source review.
Sampled planned exits alone do not establish retired frequency or a speedup.

`with_native_float_min_max()` independently requests twelve exact VEX MIN/MAX
Codes: packed PS/PD at 128/256 bits and scalar SS/SD. Each permits three correctly
sized vector registers or two vector registers plus one exact ordinary read
of 4/8/16/32 bytes. The request defaults off and does not imply full-YMM or host
authority. Declared and independently admitted actual AVX are required; FMA and
AVX2 are not. Existing legacy and other profile APIs retain their meanings.
Address32, segments, VSIB, writes, EVEX and other arithmetic remain excluded.
Scalar LIG source encodings are re-encoded with L=0 before native execution.

The memory analyzer's 32-byte exception covers only the four exact packed-YMM
MIN/MAX reads with one unconditional Read descriptor. An immutable flag records
only actual emitted MIN/MAX memory operands, so manual and diagnostic consumers
must perform the same full live-span and address-dependency preflight as checked
execution. The flag grants no permission or fence authority. Original planning
proofs cover all twelve Codes and twenty-four shapes, high-register aliases,
SIB/RIP addressing, source-bound re-encoding and unchanged finite limits. They
make no numerical hardware-equivalence or title performance claim.

`with_native_scalar_fma()` is an independent, default-disabled request for
exactly VEX VFMADD213SS and VFNMADD213SS with three XMM0..15 operands. It
requires the full-YMM request and declared AVX plus FMA. Existing prefix,
source and placement checks remain mandatory. Decoded LIG variants are
canonicalized to L=0. The old destination remains an input; its inactive
low128 bits are retained and its upper128 bits are cleared. Packed, memory,
other operand orders, scalar double and EVEX forms gain no authority from
this request. Both execution publishers independently require the actual
engine's admitted FMA capability, including supported OS vector state.
FMA and AVX2 admission are independent cache identities. Planning neither
proves execution nor changes the guest CPU identification profile.

`with_native_fma_registers()` is a separate default-disabled request for all
ninety-six VEX FMA3 Codes supported by the software mediator: the 132/213/231
orders of FMADD/FMSUB/FNMADD/FNMSUB SS/SD and packed PS/PD, plus packed
FMADDSUB/FMSUBADD PS/PD. Exactly three XMM0..15 or YMM0..15 operands of the
declared width are admitted. The full-YMM request and declared AVX plus FMA
remain required; actual host FMA and vector-state authority must be checked
independently by the execution publishers. The existing two-form scalar request
retains its narrower meaning. Memory, FMA4, EVEX and other families gain no
authority. Scalar LIG encodings are re-encoded with L=0, and instruction-info
dependencies retain the destructive destination input and source aliases.
Original planning fixtures enumerate every Code and its aliases on both source
paths. Planning proves neither numerical execution nor a title speedup; actual
full-state hardware comparisons remain separate runtime proofs.

`with_native_streaming_stores()` separately requests exactly VEX.128/256
VMOVNTDQ memory destinations, with XMM0..15/YMM0..15 sources and ordinary 64-bit
addresses. The full-YMM request and independently admitted actual AVX are also
mandatory; neither width requires AVX2. Source registers, including upper bits
of a VEX.128 store's source, flags and MXCSR remain unchanged. Other vector
writes, streaming families, EVEX, VSIB, segment overrides and addr32 stay closed.

Every execution checks the complete owned writable 16/32-byte span and its
architectural 16/32-byte alignment. The size exception permits only the exact
unconditional VMOVNTDQ Write32 footprint. Existing legacy MOVNTDQ/MOVNTPS/MOVNTPD
stores now receive explicit alignment checks too. Immutable plans retain one
emitted-body flag; a merely decoded, nonemitted store does not set it. Both native
return paths conditionally execute a host MFENCE before Rust reads or service
publication, covering the admitted legacy stores as well. Its inline assembly
retains compiler memory effects. This does not make a vector store atomic.

The original `native_streaming_stores_publish_before_rust_reads_and_preserve_all_source_bits`
consumer checks both widths and all sixteen source vectors, immediately reads
the result through a mediated memory operation without a guest fence, and
checks canaries, flags, status and service continuity. Newly native stores move
from mediated vector counters to native instruction and checked memory counters.
The ignored 50,000-iteration comparison requires independently verified
`PENTAX_STREAMING_STORES_BASELINE_WORKER` and `PENTAX_TEST_WORKER` artifacts. Its
elapsed times also include the executable-source interval memo and process
startup/teardown; no isolated NT-store or game speedup is inferred.

`with_native_vector_moves()` separately requests twenty-eight exact temporal
VEX memory forms: MOVDQA/DQU and MOVAPS/UPS/APD/UPD load/store at128/256 bits,
plus MOVSS/SD memory load/store at4/8 bytes. It requires the full-YMM request,
declared AVX and independent actual AVX CPU/OS admission; AVX2 is not required.
One XMM0..15/YMM0..15 register and one ordinary64-bit memory operand are allowed.
The direction-specific single Read32/Write32 exceptions are confined to these
exact transfers. Addr32, VSIB, segment/repeat/lock overrides, EVEX, register
merge/scalar arithmetic and additional streaming forms remain outside this request.

MOVDQA/APS/APD require16/32-byte alignment; DQU/UPS/UPD and scalar accesses may
be unaligned. Scalar memory loads zero all destination bits above bit31/63,
including YMM upper bits; scalar stores preserve their whole source and touch
only4/8 bytes. All transfers are raw bits with unchanged MXCSR/flags and no
NaN classification. These semantics follow the corresponding entries in
[Intel Volume2B](https://cdrdv2-public.intel.com/835752/253667-sdm-vol-2b.pdf).

The six already-native legacy MOVDQA/APS/APD memory directions also receive
explicit16-byte alignment admission. A separate immutable emitted-body flag
covers them and the twenty-eight new VEX forms, forcing complete span/dependency
preflight in manual/diagnostic consumers. This deliberately strengthens those
legacy/manual checks and may add their existing conservative analysis charges;
it grants no unrelated legacy memory authority. Temporal transfers do not set
the NT flag or introduce an MFENCE. The checked worker retains all live owner,
source-byte, stack, operand, cache and permission checks. Native moves contribute
to native/ordinary-memory counters, rather than mediated vector counters; no
budget, host control, scheduler or guest CPUID profile changes are implied.

Consecutive admitted register operations can share the existing finite native
block. Every live source/cache/owner/operand/stack check remains in place, with
the existing 256-instruction/4096-byte default block limits. The complete joined
body's ordinary memory accesses are admitted before entry. Consequently a later
denied native read can refuse the body before an earlier register operation;
identical partial-fault state across former mediation boundaries is not claimed.
This remains the existing fail-closed block model, without precise guest fault
recovery or unchecked native chaining. Native instructions contribute to the
actual `native_instructions` counter; `vector_ops` counts mediated operations
and intentionally decreases for newly emitted forms. Actual-path work and
memory checks remain charged; old aggregate counts are not fabricated.

The ignored original worker test `native_packed_float_original_baseline_measurement`
requires explicit `PENTAX_PACKED_FLOAT_BASELINE_WORKER` and `PENTAX_TEST_WORKER`
artifacts, independently verified by the coordinating runner. It uses the same
finite 50,000-iteration inputs with two arithmetic snapshot layouts and the
existing extended-startup profile, without changing any production limit.
It checks results and intentional native/mediated counter differences, then
records end-to-end process elapsed time, including startup/teardown, cache and
work counters. There is no timing assertion or commercial-title extrapolation.
Alignment NOPs retain ignored segment/address prefixes without computing or
checking a memory operand that the instruction never accesses.
LEA likewise ignores segment overrides without reading a segment base. Its
16/32/64-bit destination semantics and wrapping 32-bit address arithmetic,
including EIP-relative forms, are retained. It is not isolated as a memory
access, but its destination write still invalidates a later dependent address.
Exact families are listed in `policy.rs`; adding a CPUID bit never admits another
family.
AMD decoding mode preserves long-mode 16-bit branch/return distinctions, which
are mediated. `NO_INVALID_CHECK` is never enabled.

RIP-relative ordinary memory accesses are re-encoded at their new cache IP while
retaining the original guest target. Unrepresentable signed displacements fail
explicitly. IP-relative LEA computes a numeric constant: when its original target
fails the relocation range/address checks or normal re-encoding overflows after
prefix normalization, a width-matched MOV immediate retains the result without
a scratch register or a memory access. A 16-bit
destination preserves its upper bits, a 32-bit destination clears its upper bits,
and a 64-bit destination retains the complete value. The original LEA instruction,
source PC/length and guest target remain in the provenance record; actual emitted
bytes still obey the existing output/allocation/work limits and attestation.
EIP-relative LEA is relocated modulo 2^32. Actual memory accesses with 32-bit
bases or EIP-relative operands remain mediated.

CALL, JMP, Jcc, indirect transfers, RET, SYSCALL, interrupts and system/exception
instructions never appear as native guest transfers in emitted blocks. Each
block ends in its privately constructed two-byte UD2. Typed exit metadata retains
the original instruction, guest PC/continuation, direct or register/memory target,
condition and near-RET adjustment where supported. Segment overrides outside
LEA and alignment NOPs, REP/HLE prefixes, unsupported LOCK forms, strings,
far/non-64-bit transfers, loop-counter branches,
privileged/state-control operations, x87/MMX, vector forms outside the explicitly
requested register-only profile and other
unported families become explicit mediated exits. Invalid input is an error.

CPUID (`0F A2`) always remains a no-operand mediated exit, even when additional
instruction feature bits are declared. The Windows native resolver admits only
the exact two-byte form; segment, repeat, LOCK and redundant operand/address/REX
prefixes do not gain execution authority. It checks finite work, the live
executable continuation and the successful-operation counter before observing
the time backend or performing serialization. The four software results replace
RAX/RBX/RCX/RDX with zero-extended 32-bit words; flags and other CPU state remain
unchanged. Register ordering and instruction serialization follow the
[Intel SDM Volume 2A, CPUID](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2a-manual.pdf).

The guest sees the deliberate `PentaXEmuCPU` software profile, with maximum basic
leaf 7 and maximum extended leaf `80000001h`. Leaf 1 has zero signature/topology,
ECX `00800000h` (POPCNT), and EDX `06008100h` (CX8, CMOV, SSE, SSE2), plus TSC
bit 4 only when the existing timestamp backend succeeds. Extended leaf 1 has
ECX `00000020h` (LZCNT) and EDX `20000000h` (long mode), plus RDTSCP bit 27 under
the same condition. The first feature query may initialize the same cached
frequency calibration used by timestamp instructions and kernel services; a
failed backend hides both time features. No new clock domain is introduced.

Known scalar leaves ignore ECX. Leaf 7 returns zero for all subleaves, including
its maximum-subleaf word. Unimplemented in-range leaves return zero; out-of-range
leaves normalize to basic leaf 7 while retaining the original ECX subleaf. There
is no processor brand, serial number, APIC identity, host feature passthrough or
FPU/MMX/FXSR/XSAVE/AVX/AVX2 claim. This conservative profile is not a complete
hardware model or proof that every instruction form in an advertised family has
been ported; the positive form policy and explicit runtime refusals still apply.
After admission, one fixed host CPUID leaf 0 provides the actual serializing
instruction, surrounded by compiler barriers. Its outputs are discarded and
never become guest identification data.

Decoder-validated memory-destination LOCK arithmetic, unary operations, XADD,
XCHG and compare-exchange preserve their actual prefix in native emission.
These and implicit-lock memory XCHG always split into a single emitted
instruction, even without optional memory isolation. Numeric footprint admission
requires exactly one read/write access and natural alignment, a PentaX policy
that avoids split locks rather than emulating a guest alignment exception.
The platform checks actual owned backing and both permissions on its manual
and checked paths. Ordinary CPUID gates remain: CMPXCHG8B is admitted under CX8,
while CMPXCHG16B remains unavailable in the default profile. Bit-string offsets,
HLE hints and segment-prefixed atomics remain explicitly unsupported. Real CPU
ordering and flags are retained; Rust read/write sequences do not replace them.

The instruction and output bounds can split a block before its next un-emitted
instruction. The split exit retries the same guest PC, so no instruction is
silently skipped. `attest_emission` compares a complete buffer with the retained
emission; `attest_exit` checks only its exact final instruction-boundary PC and
UD2 bytes. Incidental UD2/SYSCALL byte pairs inside immediates have no authority.
The native consumer must supply actual owned paused backing for these comparisons.

## Limits and remaining execution work

Aggregate module/segment/patch counts are checked before source copies. Sorted
ordinary and declared relocation tables use bounded binary searches for each
source window, including partial eight-byte intersections. Search visits and
overlapping writes are charged without rescanning unrelated patches. Complete
declaration validation and exact staged-byte comparison remain required. All
per-plan byte, record and encoder-scratch buffers are fallibly reserved with
reported-capacity accounting. Record/output buffers start with small capacities
and grow only when the decoded block requires more space. Growth precharges
the full replacement and movement; reported allocation is cumulative, retaining
both old and replacement capacities to cover their overlap. Short isolated
blocks therefore avoid reserving a worst-case record for every fetched byte.
A single work budget charges metadata visits,
reservations, copies, decoding/classification and encoding before the work.
Usage reports window bytes, actual decode attempts, emitted bytes, recorded
buffer capacities and work units. Bounds are per call, not wall-clock timing.
Iced's fixed dependency-global lazy tables and its internal encoder diagnostic
strings use that dependency's ordinary allocator; recoverable process-wide OOM
is not promised. Allocator bookkeeping is outside reported buffer capacities.

`CpuFeatures` is an immutable **declared policy**, not a host probe. The native
consumer must verify actual host CPUID and OS/context support, and preserve every
admitted GPR, RFLAGS (including DF), XMM/MXCSR and exception effect. Conditional
exits must evaluate guest flags. CALL/RET/indirect exits must resolve against
owned guest memory, validate canonical executable targets and permissions, and
commit guest stack/context effects transactionally. Target metadata alone does
not assert that a target is mapped, executable or translated. No guest instruction
may be resumed by incrementing the host IP across UD2 without these operations.

Ordinary native memory operands remain guest-address operands. The separate
native-memory owner now supplies checked physically NX guest mappings and an
exclusive paused adapter sharing actual backing and service-write semantics
with HLE. The reusable platform stepper binds an exact materialized source to a
stable actual-owner identity, checks live initialized immutable executable bytes
and host features, and separately attests its private native tail before entry.
The dedicated worker obtains typed materialization provenance from the composed
core session, pauses that owner for one block, then releases it before checked
PRE-SYSCALL services. The sparse `GuestMemory` Vec backend cannot be paired with
a detached native snapshot. The composed path executes an original kernel/TLS
program in debug and release; this establishes no supplied-binary compatibility.

The stepper executes checked blocks with source-bound publication and reuse of
an immutable native cache. Writable guest code, state outside the admitted
GPR/SIMD/control profile and interruption within a live native block remain
outside this interface. Native
identity addressing is not a sandbox: ordinary runtime memory operands are not
instrumented here. The manual native entry requires caller-established bounds; the checked
entry derives live spans from address-independent instructions and validates permissions.
Typed TLS provenance, byte equality and worker isolation alone do not establish
ordinary operand confinement. No game, PS5 startup ABI, firmware
TCB/DTV or GPU compatibility is established by this crate or its encoding tests.

## Primary evidence and dependency provenance

- [Iced 1.21.0 release](https://github.com/icedland/iced/releases/tag/v1.21.0)
  and [Rust crate documentation](https://docs.rs/iced-x86/1.21.0/iced_x86/).
- [Decoder options](https://docs.rs/iced-x86/1.21.0/iced_x86/struct.DecoderOptions.html):
  AMD mode and validation controls.
- [Encoder](https://docs.rs/iced-x86/1.21.0/iced_x86/struct.Encoder.html):
  encoding an original decode at a chosen IP and supplying its buffer.
- [Flow metadata](https://docs.rs/iced-x86/1.21.0/iced_x86/enum.FlowControl.html)
  and [opcode metadata](https://docs.rs/iced-x86/1.21.0/iced_x86/struct.OpCodeInfo.html):
  these are classification inputs, not a proof that arbitrary fallthrough
  instructions are safe to emit.
- [Intel architecture manuals](https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html):
  instruction semantics, addressing, state and exception requirements.
  The bit-string extension follows Volume2A, Bit() and Table3-2, and the
  BT/BTC/BTR/BTS entries in the [primary instruction manual](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf).

PentaX's original code is GPL-2.0-only. Iced is MIT-licensed, copyright
2018-present iced project and contributors; its
[pinned license](https://github.com/icedland/iced/blob/c50f29b7bc305696895c075f3fc7719751426b12/LICENSE.txt)
is retained verbatim in `third-party/iced-x86-LICENSE-MIT.txt` because the
published crate archive does not include that license file. The archive's
`.cargo_vcs_info.json` pins this exact upstream commit. Its transitive
`lazy_static` 1.5.1 dependency offers
MIT OR Apache-2.0 licensing; see its packaged license files. No upstream
implementation or guest instruction fixture is copied into this crate.
