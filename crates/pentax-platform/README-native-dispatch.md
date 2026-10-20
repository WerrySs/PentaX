# Native block dispatch

PCMPGTQ and VEX.128/256 VPCMPGTQ compare corresponding signed 64-bit elements
into complete zero/all-one qword masks. Sources are captured before destination
aliases change. Legacy preserves upper YMM; VEX.128 zeros it and VEX.256 replaces
all four lanes. Memory reads cover the complete 16/32-byte source; legacy m128
requires 16-byte alignment, while VEX permits unaligned memory. RFLAGS, MXCSR
and unrelated state remain unchanged. Exact-code, prefix, next-PC, work and
counter checks precede staged CPU publication; failures never expose a partial
destination. W is ignored, L selects the VEX width and vvvv supplies SRC1.
EVEX mask forms are excluded and no host AVX instruction is needed. Contracts
follow the [Intel PCMPGTQ entry, Volume 2B pages 4-267–269](https://cdrdv2-public.intel.com/835752/253667-sdm-vol-2b.pdf)
and [Type 4 exception table](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf).

Stack admission combines CPU read/write and nonexecute requirements into one
live complete-span page walk. A failed combined check repeats the original
read, write and nonexecute checks to preserve their error and address precedence.
No permission result is cached; guard validation, layout/RSP checks and existing
conservative work charges remain unchanged. This removes two redundant walks
from the normal valid-stack path without a measured runtime speedup claim.

RDTSC and RDTSCP are mediated timestamp observations in the same admitted
invariant TSC domain as kernel time-counter services. Both split ticks into
zero-extended EDX:EAX; RDTSC preserves RCX, while RDTSCP writes ECX=0 for the
cooperative executor's single modeled virtual processor. This guest TSC_AUX
policy exposes no host processor or thread identifier. Flags, MXCSR, vectors
and unrelated state remain unchanged. No guest timestamp opcode is emitted
natively. Exact opcode/prefix, next-PC, finite work and integer-counter admission
precede the observation; unavailable counters and failed calibration refuse
without publishing a CPU result. The existing fixed LFENCE/RDTSC/LFENCE adapter
orders the host observation and retains its cached frequency and kernel epochs.
It adds no store-draining guarantee. Contracts follow the
[Intel RDTSC/RDTSCP entries](https://cdrdv2-public.intel.com/835752/253667-sdm-vol-2b.pdf).

PHMINPOSUW and VPHMINPOSUW find the minimum of eight unsigned words and its
lowest index on ties. The low two destination words contain the value and index;
the remaining XMM bytes are zero. VEX clears upper YMM, while legacy preserves
it. Both snapshot the complete source before an aliased write and read all 16
memory bytes, even when the first word is zero. Legacy memory requires 16-byte
alignment; VEX permits unaligned memory. RFLAGS, MXCSR and unrelated state remain
unchanged, with existing finite work/counter admission and staged publication.
VEX requires L0 and reserved vvvv; W is ignored. Both forms remain mediated,
without host AVX or native-AVX policy changes. Contracts follow the
[Intel PHMINPOSUW entry](https://cdrdv2-public.intel.com/835752/253667-sdm-vol-2b.pdf)
and [Type 4 exception table](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf).

Eight exact VEX VPERMILPS/PD forms select elements within each 128-bit lane.
Immediate PS repeats four two-bit selectors per lane; immediate PD uses
successive bits for successive destination qwords (two for XMM, four for YMM).
Variable PS uses each control dword's bits 1:0; variable PD uses each control
qword's bit 1, ignoring bit 0 and higher bits. Immediate memory operands supply
data; variable memory operands supply controls. Every source reads its complete
16/32 bytes even for repeated selections. Both inputs are captured before any
data/control alias changes, XMM results clear upper YMM and YMM results replace
both halves. RFLAGS, MXCSR, GPRs and unrelated vectors remain unchanged. The
existing exact register, next-PC, finite work/counter and staged publication
checks remain. These are original integer bit transfers with no host AVX/FP or
MXCSR changes. W0 is required; both lengths are legal. Only immediate forms
reserve vvvv, while variable forms use it as the data source. Native-AVX and
EVEX policies are unchanged. Contracts follow the
[Intel VPERMILPD/PS entries and diagrams](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2c-manual.pdf).

Four exact VEX.256 immediate permutations operate on complete source snapshots.
VPERM2F128/I128 select either 128-bit half of either source using immediate
bits 1:0/5:4; bits 3/7 override the respective half to zero, and bits 2/6 are
ignored. VPERMPD/Q use four two-bit selectors to choose any source quadword,
including repetitions across halves. Every memory source reads the full32
bytes even when zero overrides or repeated selections leave bytes unused.
Unaligned reads are allowed. Both YMM halves are replaced only after source,
next-PC and finite work/counter checks; aliases, unrelated state, RFLAGS and
MXCSR are preserved. No host AVX/FP operation is needed. Decoder admission
requires L1 for all four, W0 with a live vvvv source for PERM2, and W1 with
reserved vvvv for PD/Q. Native-AVX emission and EVEX runtime support are unchanged.
Contracts follow the [Intel VPERM2F128/I128 and VPERMPD/Q entries](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2c-manual.pdf).

The exact VEX VEXTRACTF128/I128 forms copy the 16-byte half of YMM selected by
immediate bit zero; bits 7:1 are ignored. A register destination replaces XMM
and clears its upper YMM half; an unaligned memory destination writes exactly
16 bytes and preserves every vector. The source is captured before an alias
changes. RFLAGS and MXCSR stay unchanged, including for NaN bit patterns.
Whole-span writable-memory and finite work/counter checks precede publication;
refusals publish no partial store or CPU result. These operations use copied
bits in Rust with no host AVX requirement. Exact decoder admission requires
L=1, W=0 and reserved vvvv; no EVEX support or native-AVX emission is added.
Contracts follow the [Intel extraction entries](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2c-manual.pdf)
and [Volume 2A Tables 2-16 and 2-23](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf).

Twelve exact VEX packed PS/PD-to-signed-DQ and signed-DQ-to-PS/PD conversions
reuse fixed integer arithmetic. CVT uses guest rounding; CVTT always truncates.
Invalid conversions produce the signed32 indefinite value and sticky invalid;
inexact valid lanes contribute precision even when another lane is invalid.
DAZ acts on floating inputs; these conversions do not raise denormal exceptions.
DQ-to-PD is exact, and FTZ has no effect on these integer or normal float results.
Memory inputs use exact 8/16/32-byte spans. PD.256-to-DQ writes XMM, while
DQ.128-to-PD reads only m64 or the low64 of XMM. All unused destination bytes
are zeroed; full source snapshots preserve aliases, RFLAGS and unrelated state.
The existing masked-exception profile, finite work/counter checks and staged
publication remain required. Production requires no host AVX or host MXCSR
changes; legacy/EVEX forms and native-AVX emission are unchanged. Contracts
follow the [Intel SDM conversion entries](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf)
and [packed exception rules, sections 11.5.2.2 and 11.5.3.1](https://cdrdv2-public.intel.com/843827/253665-sdm-vol-1-dec-24.pdf).

The eight exact VEX.128/256 VMOVMSKPS/PD forms extract each single/double
element's sign bit into a 32/64-bit GPR, zeroing every remaining destination
bit. Register-only sources, all vector bits, RFLAGS and MXCSR are preserved,
including signaling NaNs, subnormals and inactive upper lanes. The existing
VPMOVMSKB path retains its byte-sign mask behavior. These operations use bounded
integer Rust with staged CPU publication and the existing stack, work and
counter checks; they require no host AVX. Decoder admission remains exact and
does not add legacy or EVEX forms. Semantics follow the
[Intel SDM Volume 2B MOVMSKPS/PD entries](https://cdrdv2-public.intel.com/835752/253667-sdm-vol-2b.pdf).

The six exact VEX CMPPS/PD/SS/SD mask comparisons run on copied integer bits.
They implement all 32 predicates, signed zeros, infinities, quiet/signaling
NaNs, DAZ and sticky masked invalid/denormal flags. Packed lanes contribute
their own status flags; a NaN suppresses denormal only within its comparison.
Scalar results retain SRC1's remaining low128 bits, and every XMM destination
clears its YMM upper half. RFLAGS remain unchanged. These rules follow
[Intel SDM Volume 2A, Table 3-1 and the comparison entries](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf)
and [Volume 1, sections 4.9.2, 11.5.3.1 and 14.12](https://cdrdv2-public.intel.com/843827/253665-sdm-vol-1-dec-24.pdf).

The runtime snapshots both inputs before overlapping destination writes and
reads exactly 4/8 scalar or 16/32 packed memory bytes. Complete backing, next-PC,
work and counter checks precede publication of the staged CPU state. The helper
uses fixed stack storage and performs no host floating operation or MXCSR write.
The existing masked-exception profile remains required. Codegen keeps these
forms as mediated exits even when AVX is declared; no native AVX emission,
EVEX comparison support or unmasked exception delivery is added. Independent
original copied-value hardware references belong only to the test code.

Exact VEX.128/256 VMOVNTDQA forms perform complete aligned 16/32-byte guest
loads through the checked vector move engine. VEX.128 clears the destination's
upper YMM half; VEX.256 replaces both halves. Other registers, flags, MXCSR and
source bytes stay unchanged. Alignment, full readable coverage, finite work and
success counters are validated before CPU state publication. The streaming
cache hint is ignored as permitted by Intel; the data transfer is required.
No host AVX, hardware WC-buffer model, legacy SSE form or EVEX support is added.
See [Intel SDM Volume 2B, MOVNTDQA](https://cdrdv2-public.intel.com/835752/253667-sdm-vol-2b.pdf).

The four legacy PREFETCHNTA/T0/T1/T2 locality hints run as ignored hints in
Rust. Only exact memory-form encodings are recognized; no suggested effective
address is calculated, checked as a data access or passed to a host prefetch.
Unmapped, denied and noncanonical operands consequently do not become data
faults. FS/GS, RIP-relative and address32 encodings retain the same rule.
The handler checks declared SSE-family policy, refuses repeat/HLE/LOCK prefixes,
charges finite work and validates the next executable guest PC before changing
only that PC. Declared-feature/repeat refusals are PentaX policy, not invented
architectural exceptions. Source/owner/cache/stack checks remain in force;
the native-emission whitelist is unchanged. PREFETCHW/WT1, instruction-prefetch
extensions and cache flush/writeback remain unsupported. This follows
[Intel SDM Volume 2, PREFETCHh](https://cdrdv2-public.intel.com/835757/325383-sdm-vol-2abcd.pdf).

VEX.128/256 VTESTPS/PD reduce only the packed single/double element sign bits
to ZF/CF; VPTEST continues to examine all bits. OF/SF/AF/PF clear, other flags,
every source vector bit and MXCSR remain unchanged. Complete memory operands
are read even though only sign bits determine the result. Work, byte counters
and failed reads publish no flag result. Exact VEX forms require no host AVX
and exclude EVEX, following the [Intel packed bit-test reference](https://cdrdv2-public.intel.com/671110/325383-sdm-vol-2abcd.pdf).

All 24 exact VEX.128/256 VPMOVSX/ZX widening forms expand signed/unsigned
bytes to words/dwords/qwords, words to dwords/qwords and dwords to qwords.
They consume the low XMM source elements across the complete result; memory
reads cover only the required narrow tuple, including at a page boundary.
Source snapshots preserve aliases. VEX.128 clears the upper destination half,
while flags, MXCSR and GPRs remain unchanged. Failed reads or exhausted work/
counters publish no CPU result. The bounded Rust path requires no host AVX,
excludes EVEX and follows the [AMD media instruction reference](https://docs.amd.com/api/khub/documents/ioQiNhSqxlMkRaU~IyGQYQ/content).

Exact VEX.128 VPEXTRB/W/D/Q forms extract one byte, word, dword or qword from
the retained XMM source. Both word opcode families and the decoder's WIG
register-width spellings are admitted. Only the low element-selector bits
of imm8 select the source; register destinations zero-extend to the full GPR,
and memory destinations write only the scalar width without an alignment
requirement. The complete source vector, including its upper half, flags and
MXCSR remain intact. Memory, work/counter and stack-pointer validation precede
publication. VEXTRACTPS retains its existing four-byte bit-transfer semantics
through the same path. These forms require no host AVX and exclude EVEX,
following the [Intel extraction reference](https://cdrdv2-public.intel.com/835757/325383-sdm-vol-2abcd.pdf).

VEX.128/256 VMOVNTDQ stores exactly 16/32 source bytes to guest memory.
The destination must be aligned to the complete operand width. All source
vector bits, including the upper half of a 128-bit store, GPRs, flags and
MXCSR remain unchanged. Permissions, complete spans, work and counter capacity
are checked before publication. The serialized runtime completes ordinary
checked memory writes before continuing guest execution or synchronous GPU
submission; it does not model cache bypass or a write-combining queue. Host AVX
is unnecessary, and EVEX forms remain excluded. This follows the
[Intel VMOVNTDQ reference](https://cdrdv2-public.intel.com/835757/325383-sdm-vol-2abcd.pdf).

Guest x87 control uses retained control/status/tag words, instruction/data
pointers, opcode and eight physical 80-bit register bit patterns. WAIT checks
pending unmasked exceptions and returns a typed stop before continuation or
memory publication. Exact FLDCW, F(N)STCW, F(N)STSW (memory/AX), F(N)CLEX and
F(N)INIT forms operate on this owned state. No-wait forms remain available for
examining or clearing pending state; waiting forms check it first. Word loads
and stores validate the full two-byte span, AX stores retain upper RAX bits,
and FLDCW normalizes reserved bits and recomputes ES/B. Clear preserves stack
condition/TOP bits; init resets the environment and empties tags while retaining
the physical register bits. MXCSR, vectors and integer flags are independent.

The full x87 snapshot survives thread exchange and nested initializer returns;
newly prepared threads inherit the parent's floating environment. Guest x87
instructions never borrow host FPU state. Existing `vector_ops`/`vector_bytes`
diagnostic counters also include these modeled floating-control operations.
x87 arithmetic and environment save/restore instructions remain unfinished.
Semantics follow the [Intel x87 control reference](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf)
and [floating-point exception rules](https://cdrdv2-public.intel.com/835781/325462-sdm-vol-1-2abcd-3abcd-4.pdf).

STMXCSR/LDMXCSR and their exact VEX forms transfer four checked bytes between
guest memory and the retained guest MXCSR. Stores preserve surrounding bytes;
loads validate before CPU publication. Flags, GPRs and vector data are unchanged.
The same masked-exception profile now uses the actual host `MXCSR_MASK`, read
once from an aligned owned FXSAVE area; a zero report uses Intel's `0xffbf`
fallback. DAZ is admitted only when its mask bit is supported, and closed SSE2
arithmetic retains and restores host control. Reserved bits and unmasked guest
exceptions remain typed refusals. Guest transfers do not execute host LDMXCSR
or require host AVX. This follows the [Intel MXCSR reference](https://cdrdv2-public.intel.com/843823/252046-sdm-change-document-1.pdf).

VEX.128/256 packed logical shifts support word/dword/qword elements and
arithmetic right shifts support word/dword elements, with immediate or shared
counts. Shared counts read the low unsigned qword of a complete XMM/m128
operand for both vector sizes. Variable dword/qword logical shifts and variable
dword arithmetic right shifts use independent unsigned element counts. Counts
are never masked: excessive logical shifts yield zero and arithmetic shifts
fill with the sign bit. VPSLLDQ/VPSRLDQ shift bytes within each independent
128-bit lane and zero a lane for counts above 15. Complete counter spans and
source snapshots are checked before publication, including destination aliases.
All 46 exact VEX forms preserve flags, GPRs and MXCSR, clear the upper half for
128-bit results, and run in checked Rust without host AVX. EVEX remains excluded.

VEX.128/256 VPBLENDD/W and VBLENDPS/PD select elements from full source
snapshots using an immediate mask. The word mask repeats independently per
128-bit lane; dword/single/double masks advance through the complete vector.
VPBLENDVB and VBLENDVPS/PD use only each element's mask sign bit, with a
separately snapshotted explicit mask register. Destination aliases retain all
original inputs. Memory operands are read completely even if no elements are
selected. Results clear the upper half for 128-bit forms and preserve GPRs,
flags and MXCSR, including when transferred bits encode NaNs. Exact VEX forms
run in checked Rust without host AVX; EVEX remains excluded. Semantics follow
the [Intel instruction reference](https://cdrdv2-public.intel.com/835757/325383-sdm-vol-2abcd.pdf).

VEX.128/256 VPMULUDQ/VPMULDQ widen the even unsigned/signed dwords to
quadword products; VPMULLD/VPMULLW retain the low element product and
VPMULHW/VPMULHUW select signed/unsigned high-word products. Complete source
vectors are snapshotted, including memory bytes belonging to ignored odd
dwords. Results clear the upper half for 128-bit forms and preserve flags,
MXCSR and GPRs. The same bounded Rust path works without host AVX.

VEX.128/256 VPMINSB/SW/SD, VPMINUB/UW/UD and their VPMAX counterparts select
the minimum or maximum per signed/unsigned byte, word or dword. Exact decoder
forms admit register and full-width memory inputs. Sources are captured before
destination publication, including aliases. VEX.128 clears the upper YMM half;
all forms preserve flags, MXCSR and other CPU state. Read failures and vector-byte
budget refusal publish no CPU result. The bounded Rust implementation requires
no host AVX and admits neither EVEX nor packed quadword variants. Semantics follow
the [Intel instruction reference](https://cdrdv2-public.intel.com/789581/325383-sdm-vol-2abcd.pdf).

VEX.128 VPINSRB/VPINSRW/VPINSRD/VPINSRQ insert one scalar element from an
exact-width memory span or the low GPR bits into a snapshot of source1. Only
the element-index bits of imm8 select the lane. The result clears the upper
YMM half and preserves GPRs, flags and MXCSR. Both reviewed register-width
spellings of byte/word insertion are supported; EVEX is excluded. Failed reads
or exhausted byte counters publish no vector state. These operations run in
checked Rust without host AVX, following the [Intel instruction reference](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2b-manual.pdf).

This note describes the bridge and original diagnostic in
`src/windows/dispatch.rs`, plus the reusable explicitly unsafe one-block API in
`src/windows/dispatch/prepared.rs`, exported as `native_dispatch`.

## Verified scope

The isolated Windows worker executes three original context-probe blocks and
the original multi-block arithmetic program, returning 42. Checks cover actual
guest register changes and reloads, a fixed 128-byte red zone, and normal-return
restoration of the Windows host state. This is evidence for the closed original
diagnostics; it does not establish game compatibility. The reusable prepared
path also executes an original three-module kernel/TLS consumer in dedicated
debug and release workers: four services, result 42, TLS0=42/TLS1=37
and fourteen actual mapped/resident pages. The checked path validates twenty ordinary
accesses covering 156 bytes from observed state before entry, plus three modeled
FS operations covering 24 bytes on the same owner. The original
diagnostic remains a closed input; this establishes no game compatibility.

The composed original program also reads the staged argc/argv object, actual
program-name bytes and zero root frame through native instructions. Its observed
initial RDI/RSI/RBP are checked after return. This exercises the core's reusable
entry staging without establishing module lifecycle or callback implementation.

Portable crates retain `forbid(unsafe_code)`. The dedicated worker uses
`deny(unsafe_code)` with a narrow exception for its reviewed native-session
module. Both that worker boundary and the platform retain
`undocumented_unsafe_blocks = deny`.

The native thread-exchange check runs two original guest contexts on independent
guarded stacks in a supervised worker test. It alternates native GPR/SSE work
and checked stores, observes both outer returns, and verifies distinct results,
MXCSR/YMM preservation and code-cache reuse under one executor.

The application runner now admits direct `PthreadCreate`, POSIX `pthread_create`
and `pthread_create_name_np`, including the explicit `libScePosix` spelling.
Creation snapshots attributes, allocates fresh TLS and either allocates a guarded
stack or admits the caller's exact live RW/NX stack. Caller stacks are not rounded,
enlarged, cleared, unmapped on rollback or given implicit guards; their guard-size
attribute is ignored, matching pinned `CreateGuestStack`. The entry RSP is aligned
from the supplied top. The runtime registers a stable prefix and admits the CPU
and queue capacity before atomically publishing the return frame and handle.
A native original consumer verifies byte-granular caller bounds and retained data
inside the stack and in mapped neighbors. A bounded cooperative queue alternates contexts every 2048 blocks and
yields after creation or a child entry return. Kernel/TLS selection and external
nested-call frames follow the selected CPU. A native original consumer proves
parent/child progress through mutually dependent loops, independent identity and
errno, and restoration of its parent across all three creation entry points.

Condition waits retain the exact pending syscall while parking the guest CPU,
TLS and nested frames. Shared kernel wait queues release the full recursive mutex
ownership on enrollment. Signal/broadcast notifications make a waiter eligible
only after its mutex is available; the scheduler restores ownership and recursion
before completing that saved syscall. Original worker execution verifies repeated
direct and scoped POSIX waits, signal/broadcast and a full quantum with a notified
waiter still blocked behind its mutex. No runnable notifier yields the typed
`thread-deadlock` result, including when the final notifier returns.

Kernel semaphore waits share the same exact continuation/TLS/context machinery.
Their ordered queue reserves counts when signaled and returns the actual success,
cancellation, deletion or timeout result to the original caller. Full-width finite
deadlines do not block another runnable context. When all contexts are parked,
the scheduler polls deadlines using requested host-idle slices of at most1ms
within the explicit cumulative requested-sleep budget, retaining the same pending
tickets. Indefinite waits
with no runnable notifier report deadlock. Original worker consumers exercise
signal/cancel/delete, a long deadline signaled immediately, exact remaining-time
outputs, actual idle timeout, sleep-budget refusal and indefinite deadlock.

Join, contended general mutex locks, other blocking waits, explicit exit/detach, scheduling-policy enforcement
and exit callbacks remain incomplete. Entry returns that
require registered cleanup stop explicitly; callbacks are not skipped. Returned
thread records and storage remain owned until session destruction, with bounded
admission rather than slot reuse. This is partial threading support and does not
establish a commercial game boot.

`prepare_thread` registers a parked context; `exchange_thread` swaps it with the
selected context only after checking declared guards, exact live bounds, permissions, return
slot and target. The opaque value retains pending syscall/nested-call state,
REP continuation, return observations and all guest registers. Source lineage,
cache, call-sequence allocation and resource counters remain executor-wide.
Registered stacks cannot overlap. Registration is bounded by
`max_thread_contexts` (default 256, maximum 4096); slots are not retired or reused
yet. The runtime must select the matching core TLS/kernel context before stepping.

## What was built

- A private, reusable `GuestContext` (16 GPRs, RFLAGS incl. DF, MXCSR, XMM0-15)
  and a single process-wide bridge context referenced by the assembly.
- `pentax_block_enter` / `pentax_block_trailer` assembly: a full host/guest
  context round trip. The enter path saves Windows nonvolatile registers, the
  x87 control word, MXCSR, XMM6-15 and the TEB `StackBase`/`StackLimit`,
  installs the guest TEB bounds, loads the complete guest context and jumps to
  the cache body via a RIP-relative memory-indirect jump (no scratch GPR, so all
  sixteen guest GPRs are correct at entry). The trailer saves guest GPRs (plain
  stores do not perturb EFLAGS), switches to the saved host stack and only then
  captures RFLAGS (so the guest red zone is never written), saves MXCSR/XMM,
  restores host nonvolatile/control state and TEB bounds, and returns to Rust.
- An owned, re-armable code cache: reserved and committed `PAGE_READWRITE`
  for staging, sealed `PAGE_EXECUTE_READ` with an instruction
  cache flush after each new publication. Never RWX. Privately owned; no detached
  cache pointer, exit table or arbitrary overwrite authority is exposed.
- Per-block publication: ordinary re-encoded instructions are copied verbatim to
  the exact planner placement base (preserving every re-encoded RIP-relative
  operand and its original guest target), the planner's terminal UD2 site is
  replaced by a **controlled native tail** (an indirect jump through an in-cache
  cell holding the trailer address). The tail has its **own** emission
  attestation; the planner's UD2 attestation is never reused for the mutated
  bytes.
- Pre-execution attestation: the host must support the declared CPUID features
  for every emitted instruction; the paused guest source bytes must equal the
  retained relocated window; the published cache bytes and the engine tail must
  match byte-for-byte. Unknown features are refused, not assumed.
- A Rust dispatcher that mediates every control transfer against owned guest
  memory: `Jcc` evaluates captured guest RFLAGS; direct/register/memory targets
  are resolved and checked to be mapped, executable and canonical; `CALL`/`RET`
  commit checked guest-stack effects. The closed diagnostic refuses `SYSCALL`
  before execution with precise stop metadata. The reusable path returns an
  opaque PRE-SYSCALL token preserving original PC, continuation, full RAX and
  RDI/RSI/RDX/original RCX/R8/R9. Completion requires the current executor and
  sequence; the worker resumes only after the core's checked service succeeds.
  Interrupts/exceptions/unported instructions stop explicitly.
- The closed original program: an ELF image assembled in-crate whose blocks
  exercise ordinary arithmetic, a RIP-relative data operand, a conditional
  branch, a direct jump, an indirect call and `RET`, with a live red-zone
  sentinel re-checked across every exit. It runs across five native blocks and
  returns 42.
- A separate three-block context probe preserves initially distinct values,
  changes all guest GPR/XMM values and RSP in hardware, raises a masked MXCSR
  status bit, then requires those changes to survive another host round trip.
  A native caller also checks all Windows nonvolatile GPRs, XMM6-15, MXCSR,
  x87 control word, RSP, TEB bounds and cleared host DF. Guest DF survives in
  the saved context. The full fixed red zone is checked after each round.
- `NativeBlockStepper` retains the exact `MaterializedSource`, stable native
  owner identity, guest-only state, private cache and cumulative bounds. The
  worker obtains source provenance from the actual `PreparedNativeKernelStartup`
  rather than trusting caller-supplied numeric TLS values, then borrows that
  owner's pause for one block and releases it before kernel/libc service work.
- Optional modeled FS/GS bases live in guest CPU state. Selected GPR MOV loads
  and stores, MOVSX/MOVSXD and MOVZX loads use checked Rust memory operations.
  Decoded effective addresses retain original RIP, SIB and address-size behavior;
  partial registers, 32-bit zero extension and flags follow the admitted forms.
  A store checks its continuation before committing. Unknown forms, unavailable
  bases, permissions and budgets fail explicitly. No host FS/GS state changes.
  The worker binds its main FS base to the core-prepared thread header; other
  prepared TLS instances are storage, not running threads.

The prepared dispatcher models 64-bit-address MOVS/STOS/LODS/CMPS/SCAS over its
actual owned backing, in byte/word/dword/qword widths. Documented REP forms use
RCX and DF with at most 256 elements per step, retaining the original PC for
unfinished work. Per-element transactions preserve sequential overlap and
completed indices on a later failure. Conditional-repeat faults restore their
original flags across quanta and machine reattachment. Zero count avoids data
access, including unavailable source segment bases. FS/GS source bases remain
guest values; destination ES has its long-mode zero base. Unsupported address
sizes, I/O strings and undefined repeat forms refuse. Separate string counters
do not increase `native_instructions`. The original hardware matrix covers 48
encoding variants across 384 states and eight further destructive-overlap cases;
real worker consumers verify checked/native transitions and multiple quanta.

## Invariants

`time_counter` supplies the bound kernel adapter with native invariant TSC ticks
and one cached frequency. It checks CPUID support, uses the crystal-clock ratio
when enumerated, or calibrates once against `Instant` over 100 ms with the pinned
Kyty reference's 100 kHz rounding. Fixed LFENCE/RDTSC/LFENCE observations accept
no guest pointers or instructions. Unsupported hosts and failed calibration
refuse; no substitute frequency is invented. Process epochs remain in the kernel.
Feature and frequency decoding follow the [Intel CPUID contract](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf).

VEX.128/256 `VMOVUPS`, `VMOVUPD`, `VMOVDQU`, `VMOVAPS`, `VMOVAPD` and
`VMOVDQA` are modeled in Rust at checked exits. YMM0..15 upper halves live in
explicit guest state across native SSE blocks and service round trips. VEX.128
register destinations clear their upper halves; legacy SSE preserves them.
`VPCMPEQB/W/D/Q` compare complete elements into all-zero/all-one masks, reading
both sources before an aliased destination changes. `VPMOVMSKB` extracts byte
sign bits into a zero-extended GPR while preserving vectors and flags. Both
128/256-bit forms use the same checked continuation and bounded work; memory
comparisons require a complete readable span, and an RSP destination remains
confined to the owned stack.
`VPTEST` reduces all selected bits into CF/ZF, clears the other arithmetic
flags and preserves both vector operands, including unused upper halves.
`PCMPISTRM/I` and `PCMPESTRM/I`, including exact VEX forms and 32/64-bit explicit
lengths, use bounded Rust string comparisons. Byte/word signedness, all four
aggregation modes, validity overrides, polarity, mask/index selection and flags
are modeled without host SSE4.2/AVX requirements. Full-width memory access is
checked even for empty strings. Mask forms snapshot both inputs before XMM0
changes; VEX clears its upper half while legacy preserves it. Index forms
zero-extend ECX and preserve vectors. Original SSE4.2 hardware reference tests
cover all 128 controls and signed/saturated lengths; integration checks all
encodings, aliases, complete-span refusal, budgets and unchanged failure state.
`VZEROUPPER` and `VZEROALL` update all sixteen guest registers. The decoder's
exact VEX forms are selected; EVEX, masking and wider vector state stay refused.
Memory transfers check the complete owned span and required alignment before
any write, with bounded work and a valid continuation. No host AVX capability
or extended-context modification is required. `vector_ops`/`vector_bytes` report
modeled completion separately from native execution. Semantics follow the
[Intel instruction reference](https://cdrdv2-public.intel.com/825757/253667-sdm-vol-2b.pdf).

VEX.128 `VMOVLPS/LPD/HPS/HPD` merge an exact eight-byte memory input with
the other quadword of the first vector source, or store the selected quadword.
`VMOVHLPS/LHPS` perform the corresponding register merges. Both sources are
captured before an aliased destination changes. Register results clear YMM.HI;
stores preserve all vector state. These are bit transfers without floating-point
evaluation or flag changes, following the same Intel MOV instruction contracts.
Complete-span permissions and success-counter admission precede publication.

VEX scalar and packed precision conversions now support binary32/binary64
narrowing and widening, including both packed vector lengths. They reuse the
closed SSE2 copied-bit boundary, with actual guest rounding, DAZ/FTZ, NaN payloads
and sticky exception status, restoring host MXCSR before returning to Rust.
Scalar forms merge the first source and clear upper YMM state; packed forms
capture the full source before any destination alias is written and zero unused
destination lanes. Exact 4/8/16/32-byte memory input and cumulative admission
precede publication of any CPU result. No host AVX, EVEX or guest pointer enters
the conversion primitive. Behavior follows the
[Intel instruction reference](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf).

The VEX scalar float-to-signed conversions support binary32/binary64 input and
32/64-bit GPR output, with explicit truncation or all four guest rounding modes.
An original integer-only IEEE decoder handles signed zero, subnormals/DAZ,
ties-to-even, signed limits and masked invalid/precision status without host
floating-point operations or host AVX. Conversion does not signal denormal;
invalid results use the architectural indefinite integer and suppress precision.
Only exact 4/8-byte memory sources are read. A 32-bit result zero-extends its GPR;
all vector/upper-vector state and integer flags remain unchanged. Results directed
to RSP must remain within the current owned stack before publication. Full input,
byte/work and result-counter admission precedes state publication. Contracts
follow the [Intel instruction reference](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf)
and [SIMD exception rules](https://cdrdv2-public.intel.com/671436/253665-sdm-vol-1.pdf).

LZCNT, TZCNT and POPCNT have exact 16/32/64-bit Rust-modeled forms, independent
of host support. Register and exact-width memory input precede publication;
16-bit results preserve upper GPR bits and 32-bit results zero-extend. LZCNT and
TZCNT set CF/ZF from the actual input/result and retain otherwise undefined
arithmetic flags. POPCNT clears the arithmetic flags except ZF for zero input.
Stack-register results must fit the owned stack. Separate `integer_ops` and
`integer_bytes` counters exclude these operations from native/vector counts;
full byte/work/result admission precedes state publication. No host intrinsic,
guest pointer or unsafe boundary is needed. Contracts follow the
[Intel LZCNT reference](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2a-manual.pdf)
and [TZCNT/POPCNT reference](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2b-manual.pdf).

Scalar signed integer conversions use integer IEEE encoding with all guest
rounding modes and sticky precision. VEX floating add/subtract/multiply/divide,
minimum/maximum and square root support scalar and 128/256-bit packed forms.
The latter use fixed SSE2 scalar instructions on copied bits inside a closed
assembly block that saves/restores host MXCSR. Guest controls must retain every
exception mask and exclude reserved bits and DAZ, matching the native bridge.
Full source reads and counter admission precede arithmetic; all result lanes
and accumulated status publish together. Guest addresses and instruction bytes
never enter this arithmetic helper. Scalar source merges and VEX upper-zeroing
are applied in Rust. These operations follow the [Intel scalar/packed arithmetic
contracts](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf).

`VUCOMISS`/`VUCOMISD` and `VCOMISS`/`VCOMISD` compare scalar register or exact
4/8-byte memory inputs using the same closed SSE2 control-state boundary. They
update CF/PF/ZF, clear OF/SF/AF, preserve other flags and every vector bit, and
retain sticky guest exception status. Ordered comparisons signal invalid for
quiet and signaling NaNs; unordered comparisons do so only for signaling NaNs.
Complete reads and counter admission precede any CPU-state publication. The
[Intel COMISS/UCOMISS contracts](https://cdrdv2-public.intel.com/835757/325383-sdm-vol-2abcd.pdf)
define these flag and exception effects; unmasked guest exceptions remain refused.

LOCK memory RMW and implicit-lock memory XCHG always execute in an isolated
native instruction. Both `step` and `step_checked` require the actual owned
read/write span and natural alignment before cache publication. This avoids
split locks under the current policy without changing system lock-detection
settings. The emitted hardware instruction retains its atomic operation,
ordering, register and flag effects; no Rust load/store imitation is used.
`MFENCE`, `LFENCE` and `SFENCE` retain their actual hardware instructions in
native blocks under the existing SSE/SSE2 feature checks, preserving memory
ordering and unchanged flags. They are never replaced with empty operations.
HLE hints, segment overrides, memory bit-string offsets and unsupported feature
profiles still stop. This provides CPU instruction support, while additional
guest threads and a complete scheduler remain unfinished.

- Platform safe executable entrypoints accept closed original factories. The
  separate worker also composes a supplied-image entry probe from core-owned
  preparation and checked stepping. The reusable platform API accepts
  source/context metadata but executes only through an
  explicit unsafe contract. The caller must bind exact prepared/TLS provenance.
  Manual stepping additionally requires caller-established ordinary operand
  bounds; checked stepping derives and validates them before entry. There is
  no raw-pointer execution API.
- `native_instructions` counts emitted guest instructions only after their native
  body returned. Attempted blocks and the host bridge do not count. The worker's
  `primary_native_instructions` additionally excludes generated provider code;
  partial observations survive typed stops without inventing a return value.
- Guest source pages stay OS NX (logical fetch only). The CPU executes only from
  the owned cache.
- The original diagnostic uses an OS-selected arena for its image/stack bias.
  Prepared stepping checks the supplied stack and guards against the actual
  owner; guards must remain reserved and logically unmapped. A bounded nearby
  cache search never replaces another allocation. Unrepresentable RIP-relative
  encodings fail instead of changing the original target.
- One dispatch at a time, process-wide (`DISPATCH_ACTIVE`); the context is zeroed
  on drop so no guest or host register contents linger.
- Guest stack/data backing is owned, non-growing and kept alive under the same
  non-`Send`/`Sync` borrow as execution. Real unexpected exceptions stay fatal to
  the dedicated worker; nothing is resumed under incomplete recovery.
- Codegen's `CallerDeclared` assurance is not preparation authority. Core-derived
  provenance binds typed writes to the exact retained source and actual owner;
  each publication independently checks current immutable initialized source.
- Post-execution failures are terminal. Service errors remain typed and do not
  authorize guessed errno conversion or a synthetic zero result.
- `step_checked` independently derives each emitted instruction's memory use.
  A memory instruction must stand alone; every complete explicit/implicit span
  is checked on the same exclusively paused owner before publication. This
  prevents using stale PRE-block registers for a later memory instruction.
  Register-only spans remain batched; unknown footprints fail before entry.
  The separate manual `step` keeps its explicit caller confinement obligation.

## Remaining integration (not done here)

`begin_call` is available only after an exact outer return, distinguished from
terminal failures and pending services. It retains the source, owner, cache,
executor identity and cumulative usage. Before accepting another context it
rechecks the live guarded RW/NX stack, exact return slot/marker, initialized
immutable target and unchanged segment bases. It charges the additional work
without resetting prior block/source/emission/access budgets. This operation
executes nothing and does not choose or authorize module initialization.

- Broader title startup beyond the verified original prepared kernel/TLS program.
- Cache invalidation/retranslation for guest-writable code and broader code
  mutation. Prepared execution now retains fixed native slots with exact live
  source/emission checks; direct block chaining and inline access checks remain.
- Native AVX context, vector arithmetic outside the modeled subset, x87 arithmetic,
  cancellation, general guest fault recovery and broader
  services/imports/startup. A complete firmware TCB/DTV and scheduler remain
  unfinished. Segment-prefixed forms outside the checked scalar/string subset refuse;
  the observed thread-header profile does not establish a complete console ABI.
- Broader memory footprints, including bit-string offsets, and inline checks for
  higher performance. The checked path supplies access preflight for its admitted
  subset; source checks and a worker process do not form a security sandbox.

`NativeCallReturn` is minted after checked native/model execution admits an exact
outer return. It retains source, owner, executor/call identity, initial guest
context, stack geometry and returned value. Data-only exit resolution and public
`Returned` integers cannot construct it. Starting the next call clears the
stepper's retained observation without resetting cumulative budgets. The core
uses this history to admit its explicit first-call initializer completion; it
still separately checks current owner, source, arguments, TLS and prepared slot.

`NativeBlockMachine` owns CPU/context, cache, cumulative counters and bounded
suspended frames independently of a borrowed source view. `from_machine` accepts
the same source identity, or one preserving append while an actual syscall is
pending, only after checking the live owner/context and unchanged original typed
declarations. Failed reattachment retains the owned machine and attempted work.
Metadata lineage alone proves no mapping; the worker separately requires the
core's active materialization proof before unsafe entry. `begin_syscall_call`
prepares a callee below the saved caller's red zone using the exact pending
token. `complete_syscall_call` requires the innermost opaque actual native return,
restores the original context and completes only that saved syscall. Calls share
one executor, monotonic call sequence, thread bases and cumulative budgets.

All96 exact VEX FMA3 forms execute in the checked Rust model: scalar SS/SD,
packed128/256 PS/PD, three operand orders, positive/negative add/subtract and
alternating add/subtract. A fixed66-limb integer accumulator retains the full
finite product and sum before one rounding. There is no host FMA, AVX context
requirement, floating intermediate or external arithmetic dependency.
The existing masked-MXCSR profile retains rounding, sticky exceptions, DAZ/FTZ,
signed zero, NaN priority/payloads, invalid operations and directed overflow.
Tiny results are classified before rounding, including minimum-normal boundary
and FTZ behavior. Scalar forms retain the old destination's other low128 bits;
VEX128 clears its upper128. Full source snapshots preserve aliases and exact
memory widths. Arithmetic work, memory and counters admit before CPU publication.
Semantics follow [Intel SDM Volume1 section14.5](https://cdrdv2-public.intel.com/843827/253665-sdm-vol-1-dec-24.pdf)
and the [Volume2C FMA entries](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2c-manual.pdf).
Original hardware references are test-only and capability-guarded; analytic
software checks run without host FMA. EVEX/FMA4 and unmasked SIMD exceptions
remain outside this admission.
