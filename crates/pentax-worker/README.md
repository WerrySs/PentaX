# Native diagnostic worker

`probe-entry --execution-profile interactive` selects an explicit interactive
lifetime policy. It has no fixed session deadline or diagnostic block/work
allowance. Checked u64 lifetime totals remain monotonic; they are never reset
to renew permission, executor identity or callback tickets. The desktop selects
this profile for Play. The existing `probe`, `startup`, `extended-startup` and
`long-startup` profiles retain their finite execution and supervision rules.

Interactive execution keeps the existing bounded scheduling turns, per-block
planning, source windows, caches, live source/stack/TLS/operand checks, per-call
I/O/scratch/work limits and resident resource capacities. Its initial owner
allowance uses the same 4,096-object startup profile. Cumulative kernel,
provider, service, graphics and dispatch accounting has checked lifetime
ceilings independently of those per-operation and resident limits. Output
still has its retained 8 MiB ceiling; this does not provide unlimited logging.
Unsupported operations, overflow, exhausted resident limits and real host
failures still stop explicitly. This profile enables continued execution; it
does not establish title compatibility, startup completion or a rendered frame.

For interactive launches, the CLI forwards complete validated public protocol
records while the worker runs. Each record is at most 16 KiB, at most two wait
in the forwarding queue, and no record history grows with session length. Raw
stderr, incomplete/oversize records and free-form error payloads are refused;
they are never copied into the public stream. Final completion still requires
the actual child exit and exactly one outer-return result. Desktop supervision
retains its owned process job, cancellation, finite shutdown/reap grace and
quarantine rules. Window handshake and individual frame deadlines stay finite even though
the interactive window session has no fixed lifetime deadline.

`probe-entry --private-console <fresh-file>` explicitly creates a new private
binary file for guest-console output already retained by the kernel. It is
written only after the execution runner returns, including a typed guest stop,
and never consumes output or reads guest memory. The existing file cannot be
replaced. Keep this potentially title-specific payload private; it is never
copied to the worker's public stdout/stderr protocol.

The file contains `PXCNSL01`, followed by three little-endian u32 byte lengths
and the raw suffixes in stdout, stderr, stdin-compatibility order. Each suffix
is at most 4,096 bytes; the complete file is at most 12,308 bytes. Bytes are not
decoded, escaped or padded. An explicit request adds only final
`private_console_status=written|unavailable|write-failed`. Preparation can leave
the file unavailable, and a failed write can leave a partial file; neither
replaces the guest result or grants execution/presentation authority.

`probe-entry --private-samples <fresh-file>` explicitly creates a new host-only
diagnostic file. It never replaces an existing file. After execution stops,
it can contain at most 64 recent periodic samples as module-relative offsets,
main/child roles and counts; it contains no paths, absolute PCs, input bytes or
operands. Keep this title-specific artifact private. Default protocol output
remains closed; only an explicit request adds `private_samples_status` with
`written`, `unavailable` or `write-failed`. An output failure does not replace
the actual guest result. Samples describe recent activity, not whole-run
hotspots, branch outcomes, a wait condition, deadlock or a rendered frame.

The little-endian binary starts with `PXSAMP01`, a u32 row count and u32 flags
(bit 0: observer disabled; bit 1: observer overflow). Each 24-byte row contains
a u32 module slot, u32 role (0 main, 1 child), u64 module-relative code offset
and u64 recent count. The complete file is at most 1,552 bytes. Module slots
refer to the exact captured source epoch, not filenames or execution authority.

`probe-entry --libc-memory-builtins checked` explicitly selects the standard
memory ABI of the module supplied with `--preload-libc` or `--preload-libc-as`.
The default is `off`. Exact qualified memcpy, memmove and memset exports can use
the existing checked Rust memory helpers instead of executing their native
bodies. Module and library versions, source epoch, owner and function entries
must match the finite profile; missing exports keep their original native path.
This neither changes ordinary symbol resolution nor recognizes instruction
patterns as library functions.

The route retains normal CALL/tail-JMP/RET handling, preserves RCX/R11 and SIMD
state, and keeps the original guarded stack and live source checks. Physical
output aliases of the real return slot or protected source spans fall back
before a helper attempt. Entry REP strings retain their original bounded
continuation. Optional helpers have one persistent process-owned service ledger
with the same finite memory-service limits. Required provider calls retain their
original independent ledger, so optional work cannot consume their allowance.
Both routes use the same checked memory implementation and actual paused owner.
The optional ledger survives source-epoch changes and thread switches.
Before selecting a helper, a read-only admission check includes its retained
attempt, lookup-work, span, scratch, aggregate-byte and work allowances. A call
that does not fit keeps its original checked native implementation without
consuming a helper attempt or changing either ledger. This is selection before
effects, not recovery from an error, a refund or a reset of consumed work.
After a helper attempt, failure is terminal; a late return fault preserves
completed writes and cannot replay the operation through the native path.

Final-only `libc_builtin_attempted_memcpy`, `libc_builtin_attempted_memmove` and
`libc_builtin_attempted_memset` count this profile's helper attempts. The matching
`libc_builtin_completed_*` fields count completed memory helpers, including one
followed by a terminal RET fault. `libc_builtin_bound_entries` describes the last
prepared epoch's matching exports, not calls or live execution authority. These
fields contain only fixed counts and do not add native instruction counts.

Guest scheduling keeps runnable contexts in round-robin order and varies the
bounded turn by their effective priority class: high 4096, normal 2048 and low
1024 blocks. A changed priority is observed without another service attempt,
allocation or context-table scan. Blocking, creation and actual child return
still force the existing switch path. Exact requested priority/policy remain
kernel metadata; POSIX setschedparam does not change an already running class.
These cooperative quanta are PentaX policy and never change host priorities.

The optional frame-window bridge shares a64MiB CPU pixel/transport staging
ceiling with core capture and draw pixels. Before each non-guest pump, core
returns a metadata-only census of its still-queued Frame capacities. Decoder
and presenter growth includes that census, the original handed-off Frame,
actual endpoint capacities and inline bridge storage; requested and actual
growth are checked at that phase. A future16MiB decode or24MiB backend cap is
not treated as already allocated. The presenter still admits its backend cap
before opening, then reports actual mapped upload and metadata capacities.
Every successful pump return refreshes core's actual external-capacity count
before guest entry, including short-write and early-control paths. Handoff
moves one existing allocation from core to transport without counting it twice.
Pressure may still refuse/cancel explicitly while retaining original owners;
this does not promise arbitrary queued-frame closure or a rendered title frame.
Shader/command compiler workspace and native execution caches retain their
independent finite bounds and are not claimed to fit inside this pixel ceiling.

The source-window buffer is reused within one source epoch. Every attempted
block still freshly checks and reads its complete executable window, including
plan-cache hits. Allocation ceilings, full live-window comparisons and dispatch
attestation remain enforced; previously fetched guest bytes never authorize a
later execution. This removes repeated scratch allocations without claiming a
measured game speed or frame rate.

The decoded-plan cache has 4,096 positions in paired sets, matching the runtime
native cache's position count and PC hash. Lookup checks both the primary hash
position and its adjacent partner. A changed variant of the same logical key
is replaced before an empty way or least recently validated way is selected.
Slot metadata and fixed recency storage are charged before allocation;
retained plan storage keeps its independent 8 MiB ceiling.
Source epoch, module, placement and the full freshly read live window still
participate in reuse. This retention change makes no unmeasured speed claim.

An optional4,096-entry memo retains only successful initialized-source window
metadata for an exact PC and exact requested maximum within that same source
epoch. A complete original module/segment scan fills each miss; errors and
segment-wide authority are never cached. Epoch replacement drops the memo.
Every attempt retains its original metadata-walk charge, complete live fetch,
source-byte comparison, permission/owner/continuation checks and ordinary-memory
mediation. The memo contains no guest bytes, live permission proof or evaluated
operand address.

Its one-time fallible construction is separately admitted and charged by the
actual table capacity. Insufficient construction work/space, reserve failure or
an excessive actual capacity leaves the original scanner available; attempted
allocation work may remain consumed. Table capacity shares the existing8 MiB
retained-plan ceiling. It yields its entire allocation before prior plan-cap
decisions, preserving the previous full plan capacity without reducing the
independent planner limit. Once disabled it stays disabled for the epoch. Total
admission work can therefore differ by the known one-time construction cost;
unchanged exact-budget reachability is not claimed when caching is enabled.
An ignored original16-module lookup measurement reports scanner/memo timings
and actual capacity without a speed assertion. It does not measure title
startup or frame rate.

Prepared NetCtl checks suspend their original service call while real guest
callbacks execute and return in order. Owner/source/thread/target/frame identity
checks apply to each callback; the original caller resumes only after the pending
iteration completes. Callback return values do not substitute for actual-return
evidence or establish network connectivity.

VEX.128/256 VMOVNTDQA performs a real aligned 16/32-byte read through the
checked Rust vector move engine. Destination upper-lane rules and complete
source permissions remain enforced; only the optional cache hint is ignored.
This path requires no host AVX and does not add legacy SSE or EVEX forms.

Legacy PREFETCHNTA/T0/T1/T2 execute as ignored locality hints in the Rust
mediator. They never dereference the suggested target or issue a host prefetch,
including FS/GS and address32 forms. Exact encodings, prefix/declared-feature
policy, finite work and the next executable PC remain checked. Registers,
flags, vector/control state and data bytes stay unchanged. These mediated hints
do not contribute to native instruction or data-transfer counters. Other
prefetch extensions and cache flush/writeback remain unsupported.

`probe-entry` accepts one optional `--config <path>` containing a global PentaX
TOML document, read-only and bounded to 64 KiB. Its validated system preferences
are copied into the prepared process before guest execution. Invalid files stop
intake without exposing configuration contents or paths in diagnostics.

Standard-stream writes retain checked binary bytes in the kernel's bounded
output buffers. Native progress reports only cumulative `output_bytes`, never
the payload. The diagnostic worker does not drain these buffers; reaching the
pending output limit remains an explicit `kernel-output-bytes` budget stop.

Prepared native blocks declare BMI1/BMI2 GPR features in addition to the SSE2
baseline. Actual processor support is checked before entry; unsupported hosts
stop without attempting these instructions. There is no BMI software fallback.

LEA retains 16/32/64-bit destination semantics, ignores segment overrides and
supports wrapping 32-bit address expressions, including relocated EIP-relative
forms. An IP-relative constant that normal relocation cannot represent uses a
width-matched MOV immediate, including overflow after prefix normalization.
The original LEA source record remains attested. LEA requires no guest data
access, while its destination write still forces a fresh observation before a
later dependent memory access. Actual loads/stores retain their address-width,
segment, backing and permission checks.

VEX packed PS/PD logical operations run through bounded Rust bitwise semantics,
including full memory reads, aliased sources and upper-half zeroing for 128-bit
destinations. They do not evaluate floating-point values or change MXCSR/flags.
The same checked Rust route supports VEX scalar/128-bit tuple broadcasts and
packed wrapping integer add/subtract. Broadcast memory reads use the source tuple
width; arithmetic wraps independently within each byte/word/dword/qword. Both
preserve aliases, flags and MXCSR, and clear the upper half of 128-bit results.
VEX `VMOVSS`/`VMOVSD` also use checked bit transfers: memory forms touch exactly
four/eight bytes; register forms merge two source snapshots and clear the
destination's YMM upper half. No host floating-point operation or AVX execution
is involved. `VINSERTF128`/`VINSERTI128` replace the selected 128-bit lane of a
snapshotted YMM source, preserving the other lane and reading exactly 16 bytes
for memory sources. Semantics follow the corresponding instruction entries in the
[Intel instruction reference](https://cdrdv2-public.intel.com/671110/325383-sdm-vol-2abcd.pdf).
VEX unpack/interleave instructions cover low/high PS/PD and byte/word/dword/qword
integer forms at 128 and 256 bits. Each 128-bit lane interleaves independently;
full memory sources are read even when only one half contributes to the result.
Both inputs are snapshotted before publication, preserving aliases and the usual
128-bit destination upper-half zeroing without changing flags or MXCSR.
`VSHUFPS`/`VSHUFPD` use the same source/permission rules. PS repeats its four
two-bit selectors in each lane; PD uses two distinct one-bit selectors per lane.
`VINSERTPS` snapshots its sources, inserts one selected dword, then applies its
zero mask. Memory supplies exactly four bytes regardless of the source selector
or zero mask. `VEXTRACTPS` copies one selected dword to memory or zero-extends it
into a GPR; the source vector is unchanged and stack-register writes are checked.

`VCVTSI2SS`/`VCVTSI2SD` convert signed 32/64-bit GPR or exact-width memory inputs
to IEEE binary32/binary64 through integer arithmetic. All four guest MXCSR
rounding modes, halfway ties and exponent carry are modeled. Inexact results set
sticky precision under the existing masked-exception CPU profile. Other status
bits and RFLAGS remain intact; remaining XMM bits come from the first source and
the destination YMM upper half clears. No host rounding-mode change is needed.
Semantics follow the [Intel SDM CVTSI2SS/CVTSI2SD entries](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf).

VEX scalar and 128/256-bit packed add, subtract, multiply, divide, minimum,
maximum and square root cover 42 exact decoder forms. Checked Rust code snapshots
all source operands and mediates each element through fixed SSE2 instructions on
copied bits. Each closed assembly block installs the admitted masked guest MXCSR,
captures its sticky status and restores host MXCSR before returning to Rust.
There are no guest pointers, dynamic instructions or calls inside that block.
Scalar merges, aliasing, upper-half clearing and full memory-width checks remain
explicit; packed elements accumulate status before publishing the complete result.
This fallback requires no host AVX state or new dependency. When the actual
executor admits its full-YMM bridge, the worker separately requests native
packed ADD/SUB/MUL PS/PD register forms and exactly twelve memory-source forms.
The latter retain complete read-only 16/32-byte live spans, ordinary 64-bit
addresses, dependency splits, owner/source checks and masked guest MXCSR.
The default register-only policy acquires no memory authority. Other arithmetic
families retain mediation. Native instructions increase and mediated vector
operation/byte counts decrease only for operations that actually move to the
checked native body; these counters are not comparable as identical work.

Startup progress also reports bounded aggregate timing diagnostics. One block
in 4093 samples source-window preparation, plan lookup/translation and checked
dispatch separately; `profile_samples` identifies that sampling scope. Service
time covers all observed calls. Times are nanoseconds, not guest time or FPS,
and sampled stage totals must not be read as whole-run durations. Successful
steps also count blocks with native bodies separately from exit-only blocks.
No guest address, service identifier or source bytes enter these observations.
The final `RESULT` alone adds an optional, allocation-free observer at the same
4093-block cadence; existing `PROGRESS` records do not grow. `sample_completed`
counts successful sampled dispatches, separately from attempted `profile_samples`,
and `sample_records` counts their planned instruction records. A fixed
`sample_capacity=64` rolling window reports `sample_recent`, exact
`sample_distinct` private block locations and `sample_max_repeat`; the associated
`sample_planned_exit` is a public Code label, not a faulting body instruction.
Keys use the exact immutable linker `SourceIdentity`, module and PC and are
discarded on source-plan changes. This identity is not a live-permission or
hardware execution epoch. No keys, hashes, addresses or thread identities are
serialized or exposed by Debug. The window observes locations, not semantic
progress: repeated samples cannot establish an infinite loop or deadlock.
`sample_fp_forms` gives all twelve exact VEX ADD/SUB/MUL PS/PD128/256 public
Codes as `Code:register_occurrences:memory_occurrences`, across successfully
sampled plans. Register occurrences require three same-width vector registers
0..15; memory occurrences have the matching complete operand2 width. These
sampled planned counts are not whole-run frequencies or native admission.
`sample_epoch_changes` counts linker-source transitions after the initial bind.
The independent `sample_observer_work_limit` is 64 Mi units; fixed ring work
and actual plan-record visits are admitted before dispatch. The observer has
no per-sample allocation. Quota exhaustion disables it with
`sample_observer_disabled=1`; checked counter exhaustion also sets
`sample_observer_overflow=1`. Previously completed counts remain available.
Guest limits, dispatch permissions, existing attempt/work counters and clocks
are unchanged. Original pure fixtures cover eviction, source identity, all
twelve forms, failed-sample publication, optional limits, Debug redaction and
a conservative 3 KiB bound for the observer's final fields with maximum counters.
The same observer also retains the first 64 distinct planned-exit Codes across
successful periodic samples and source-plan changes. `sample_exit_top` lists
the four most frequent among these retained Codes as `Code:count`, or `none`;
ties use static Code order. `sample_exit_codes_retained` and
`sample_exit_capacity=64` describe the fixed table. `sample_exit_overflow` counts
observations of unretained Codes, not distinct unretained Codes, while
`sample_exit_no_instruction` counts plans without an exit instruction. These
are sampled planned exits, not proof that those exit instructions executed or
the most frequent instructions across all code. `sample_main_context` and
`sample_child_context` count successful samples in the scheduler's main/child
contexts; they do not measure fairness, elapsed time or semantic progress.
The additional 64 lookup visits are preadmitted within the same observer work
cap before dispatch. Histogram and context updates commit only after a
successful step. Existing guest budgets and `PROGRESS` records remain unchanged.
`service_top` lists the four most frequent public kernel catalogue labels among
the first 64 distinct admitted labels; their counts are exact. `service_overflow`
counts successful actions for additional labels and `service_other` counts other
providers. `service_last` is the latest successful action's public kernel label,
or `other-provider`. Labels are selected only after live provider admission; no
guest-supplied name is emitted. These counts diagnose repeated startup activity,
not completion or time spent in each function.
The final result also snapshots the bounded cooperative queue after an execution
epoch returns, including before propagating a typed stop. `scheduling_threads`,
`scheduling_pending_waits` and `scheduling_returned` count retained queue entries;
`scheduling_main_state` is `unparked`, `pending-wait`, `returned` or `unobserved`.
`scheduling_main_wait` retains only the admitted public service label, or `none`,
and `scheduling_active_role` is `main`, `child` or `unobserved`. A pending ticket
may already be ready; observing it neither polls readiness nor proves deadlock.
These fields expose no thread identity or guest data, allocate no queue storage,
visit at most 4096 retained entries once per epoch stop and leave `PROGRESS`
unchanged. They do not establish game boot or change scheduler behavior.
`service_nonzero_top` counts nonzero returns for the same four labels. Nonzero
does not generally mean failure: identity and pointer results can be nonzero.

Scalar `VUCOMISS`/`VUCOMISD` and `VCOMISS`/`VCOMISD` use fixed SSE2 comparisons
inside the same host-MXCSR restoration boundary. They preserve all vector bits,
publish comparison flags and sticky status together, and read exactly 4/8 bytes
from a memory operand. Ordered and unordered forms retain their distinct quiet
NaN exception behavior. Original native consumers check branch-observable flags,
unchanged source lanes and guest status after returning through the dispatcher.

`VMOVDDUP`, `VMOVSLDUP` and `VMOVSHDUP` now duplicate the selected elements in
128/256-bit sources. The Rust bit-copy path preserves flags and MXCSR, snapshots
aliased registers and clears the upper half for VEX.128. `VMOVDDUP` reads exactly
8 bytes for its 128-bit memory form; the other forms validate their complete
16/32-byte source, including unselected elements. Semantics follow the
[Intel instruction reference](https://cdrdv2.intel.com/v1/dl/getContent/671241).

The supervised `probe-entry` path also accepts explicit future capabilities:
`--module-file <guest-path>=<host-path>`. Files are retained read-only and their
bodies remain unread until an admitted guest load selects them. A source append
maps/relocates the new shared module and runs its actual initializer below the
saved caller's red zone. One non-borrowing native machine retains CPU, stack,
thread/TLS, cache and cumulative limits across temporary source views. A fixed
bounded input arena preserves original bytes without copying old guest memory.
Nested initializers complete in order; failed reads, admission or execution do
not resume callers. Only actual returned initializer observations count. This
profile rejects unknown deferred relocations and missing initializers; it does
not provide host filesystem discovery, unloading, presentation or game boot.

Sysmodule loads for the thirteen public application-module IDs select the same
explicit capabilities at their fixed `/app0/sce_module/*.prx` paths. Their
initializer receives three zero arguments; only its actual return permits
reference/output publication and a zero Sysmodule result. Retaining a mapped
module does not restart it. KernelLoadStartModule retains its distinct handle
and initializer-result contract. An absent capability grants no file discovery.

Future-file sessions preadmit static TLS capacity (4 MiB under the default
profile) before choosing the original thread pointer. Later TLS templates and
supported typed AMD64 relocations use only that capacity, preserving every old
block/ID/offset and actual thread write. Original native cases check relocated
template pointers, zero tails, TPOFF accesses, real descriptor service calls
inside/after initialization and invalid descriptor stops. This remains explicit
PentaX TLS policy; a complete PS5 thread/DTV ABI and scheduler are unfinished.

`pentax-worker` is a separate process for original diagnostics and a bounded
supplied-image entry probe. Windows x86-64
executes an original leaf summing arguments 19 and 23 through SysV64 RDI/RSI after
owned memory transitions from reserved to RW staging to RX and instruction-cache
publication. The illegal-instruction fixture deliberately terminates only this
worker. Native execution is **not a sandbox**.

```text
pentax-worker fixture return42
pentax-worker fixture trapped-sum42
pentax-worker fixture guest-stack-entry42
pentax-worker fixture native-block-dispatch
pentax-worker fixture prepared-kernel-startup
pentax-worker fixture invalid-instruction
pentax-worker fixture unsupported-syscall
```

Each protocol record is one flushed line, with tab-separated fields beginning
`PENTAX_WORKER<TAB>1`. Records are `READY`, `RUNNING`, `PROGRESS`, and `RESULT`. A normal result
is `RESULT<TAB>exit<TAB>value=42`. No success result is emitted after a guest fault.
Supervisors must inspect both the protocol and the process exit status.

`trapped-sum42` is a closed original leaf containing synthetic service number
`0x50580001`, which is **not a PS5 syscall ABI**. Before RX publication, the
implementation attests the exact original bytes and replaces its sole known
two-byte `SYSCALL` with `UD2`. A private Windows exception scope accepts only
that registered thread, instruction address, exception code, service number,
and arguments 19/23. It returns 42 after one observed trap, reported as
`RESULT<TAB>exit<TAB>value=42<TAB>traps=1`. No native `SYSCALL` executes.

The callback does no allocation, locking, formatting, I/O, or host-service
dispatch. It changes only saved RAX, RIP and the architectural RCX/R11 clobbers.
Its copied thread-local scope is cleared before the handler is unregistered;
even a failed unregister retains no borrowed state or executable-memory pointer.
That failure reports an error. The static callback then continues the exception
search whenever this thread has no active scope. Unrelated UD2 instructions
retain the fatal worker behavior.

| Process status | Meaning |
|---|---|
| 0 | Complete return fixture result, help, or version |
| 1 | Platform or protocol I/O failure |
| 2 | Unsupported syscall rejected before allocation/execution |
| 3 | Native backend unavailable on this host |
| 64 | Invalid command; executable paths are not accepted |
| `0xC000001D` on Windows | Intentional illegal-instruction child fault |

The closed `guest-stack-entry42` fixture ports the observed fixed argument and
entry convention from Kyty's working runtime. It uses its own guarded RW/NX
stack and checks arguments, entry alignment, a root frame, red-zone accesses
and one original callback. On normal return, the bridge restores the host stack,
Windows nonvolatile registers/control state and TEB stack bounds. It provides
no unwind guarantee or firmware ABI validation. Its copied code and callback
are fixed original diagnostics; no caller executable or pointer is accepted.

The closed `native-block-dispatch` fixture runs an original multi-block program
through the private translated-block engine in `pentax-platform`. It plans each
basic block with `pentax-codegen`, attests the paused guest source bytes and the
published cache bytes, publishes ordinary instructions into an owned
writable->RX cache, and runs them under a fully owned guest context. Every
`CALL`/`JMP`/`Jcc`/`RET`/`SYSCALL` is mediated in Rust against owned guest pages;
none is copied as a raw guest transfer. It preserves a live red-zone sentinel and
the full guest register/flag/SIMD state across its exits and returns 42. Its
entry first checks three actual context round trips: distinct and hardware-modified
GPR/XMM values, a masked SIMD exception status bit, DF/CF, a fixed 128-byte red
zone, and Windows nonvolatile registers, control state, RSP and TEB bounds. See
`crates/pentax-platform/README-native-dispatch.md` for exact scope and the
remaining integration work. This is not a sandbox, PS5 ABI or game runtime.

The `prepared-kernel-startup` route connects the reusable unsafe one-block
executor to one original prepared kernel/TLS session. Its safe wrapper accepts
no external image, bytes, context or paths. The core owns the generated consumer,
exact qualified libc/libkernel providers and OS-selected arena throughout the
run. Each block checks the opaque typed-materialization provenance, actual owner,
live initialized source and private cache publication. Memory instructions are
batched only while preceding instructions leave their address registers unchanged.
Partial, conditional and implicit writes count as changes; atomics stay isolated.
Each explicit/implicit access is derived from observed registers and checked over
the complete actual span with the required read/write permissions before entry.
The exclusive memory
pause ends before dispatching an attested PRE-SYSCALL request; original RCX is
preserved as argument four. Service errors stop execution without a guessed
return value.

Prepared execution fetches at most 64 live source bytes per block, within the
existing cumulative 16 MiB source and 64 MiB admission-work limits. A fetch ending
inside an instruction may split after its complete prefix; the next step retries
the unconsumed instruction from its exact start. A truncated first instruction or
an invalid encoding still stops execution. The whole fetched window remains
attested against the active materialized source and is rechecked before entry.
No source-byte cache is used. Entry-probe results
include charged `fetched_source_bytes` and `admission_work` counters, plus a closed
`budget` category on a session or dispatch budget stop. The default execution
profile `probe` retains its 10,000-block limit. Explicit `--execution-profile
startup` permits 100,000 blocks, independently of the memory profile; all other
source, work, cache, service and memory budgets remain unchanged. CLI supervision
retains its thirty-second deadline and output bound. Reaching a budget is not an
unsupported opcode and does not establish initializer completion.

Entry probes emit aggregate `PROGRESS` records after every 2,048 blocks, at most
48 records in the startup profile. These contain only observed counters and the
current phase, with no guest data or success value. The CLI retains these records
even when it kills a timed-out worker; only a successful `RESULT` and process
status establish the observed outer return. Named original fixtures do not emit
progress records.

Unsupported-instruction diagnostics include `stopped_form`, a closed Iced
instruction-code variant, and `stopped_reason` when a closed policy refusal is
available. These fields distinguish an operand form or admission rule without
printing operand values, register contents, addresses, source bytes or private
identifiers. Reporting a stopped form never resumes the instruction.

An unavailable-function result may include `public_function`, obtained solely
from an exact module/library/version match in the pinned public reference
catalogue. Unknown identities have no label. This field never echoes an input
identifier, address or data, and does not provide or substitute an implementation.
An unambiguous catalogue candidate with different scope or versions is reported
separately as `reference_function` plus `reference_match`; it is not an exact
binding. Ambiguous or absent candidates emit only a closed match category.
An additional `library-reference` may identify a public SharpEmu export in its
declared library. That source establishes neither module nor descriptor
versions, so these hints never become `public_function` or callable providers.
An unavailable call may also include `reference_library`, a category selected
only from static public library names. It classifies the library independently
of the function identity. Unknown libraries remain unnamed; the category never
establishes an export, descriptor version or callable binding.
`public-name-hash-reference` denotes a unique name candidate derived from the
pinned public SharpEmu corpus. That names-only source establishes no library,
module, version, symbol type or ABI. Conflicting labels refuse ambiguity and
the call remains unavailable, without a success value or an inferred binding.

The kernel provider declares both `Posix` and `libScePosix` library spellings for
its existing 46 POSIX registrations, following the pinned Kyty prefix rule.
Both reach the same owned Rust state and retain exact descriptor versions and
their original result domains. Other module/library identities remain distinct.
Runtime service failures also include a closed `runtime_failure` category to
distinguish allocation, guest access, unsupported semantics and provider checks
without printing input addresses or raw internal error details.
An admitted kernel call that fails reports its static catalogue label through
`public_function`, including deferred thread preparation. This identifies the
failed call separately from `service_last`, which counts completed admissions.
Custom thread stacks, required exit callbacks and missing lifecycle operations
have distinct closed failure categories.

The original sequence calls PthreadSelf, creates and polls a semaphore with
initial count 17, reads its thread self-pointer through FS:[0], writes 73 through
FS:[0x28] and reads it back. It derives its TLS symbol from the applied TPOFF64
value, computes 37 + 73 - 68 and copies 42 into the first TLS instance through libc. Success requires
actual result 42, independent second-instance value37, prefix thread ID 73, all
four observed services, the outer return slot and restored R12/stack state.
Dedicated debug and release child runs both exit 0 with these observations,
38 executed blocks, twenty ordinary access checks covering 156 bytes, three modeled
FS operations covering 24 bytes, and eleven actual mapped/resident pages. The
second thread header retains its zero field. The supervisor checks
both the completed protocol result and process status within a 30-second limit.

The same program first records the initial RDI/RSI/RBP, reads argc, follows argv[0]
to the actual program-name bytes and reads both zero root-frame words. Core
postconditions require these observations. Entry storage comes from reusable
`prepare_process_entry` under its explicit observed policy, with owner/source,
initialized immutable targets, guard geometry and transaction limits checked
before mutation. The original no-op callback is not a module-shutdown service.

Named fixtures remain original-only. The separate `probe-entry` route accepts
explicit supplied images through core preparation. The platform also
exposes an explicitly unsafe reusable block API. Its manual `step` retains the
caller's ordinary-memory obligation; `step_checked`, used by the prepared loop,
derives and validates address-independent accesses before native entry. Source/TLS
provenance and exclusive context ownership are still required. The worker obtains
its modeled FS base from the core's checked main-thread provenance under the
explicit Kyty-observed self-pointer profile. Selected segment MOV operations
execute through Rust memory semantics, leaving host FS/GS untouched. Complete
firmware TCB/DTV, scheduling, PS5 startup, arbitrary-title support and performance
remain unfinished.
The unsupported syscall fixture is rejected before execution. Other hosts
compile this worker and report the unavailable backend explicitly.

Use `pentax-cli probe-entry --image program.elf [--memory-profile compact|runtime] [--preload-libc libc.prx] [--image provider.prx ...]` for
the supervised image route. Read-only intake holds stable handles while reading,
requires regular files and enforces a limit of 125 files/512 MiB. Core
admission rebases position-independent images into an owned bounded arena, retains
exact linking/TLS provenance, stages the observed main-call context and supplies
an explicit unavailable-function/exit provider. No native SYSCALL executes.
Unavailable calls stop with typed errors; no implementation or returned zero is
invented. Fixed-address and unsupported payload/relocation cases refuse.

The CLI supervisor drains bounded output, enforces thirty seconds and kills/reaps
the child on timeout or early errors. A fatal native fault remains a failed worker.
Output omits paths, NIDs, PCs and raw bytes; stopped results have no guest return
value. Native instruction counts require a returned native body. The primary
counter excludes generated services, while modeled operations remain separate.
Original process tests cover actual supplied-file return, partial progress at an
unsupported instruction, unavailable function calls and exit-callback refusal.
This is an entry diagnostic, with complete module lifecycle, process parameters,
asset mounts, presentation and complete game startup still unavailable.

An explicit preload selects one supplied shared library's unique initialized
immutable `DT_INIT`, called with RDI/RSI/RDX zero before main entry. It uses the
same admitted thread/TLS and executor/cache; dispatch, source-window, admission
and service budgets stay cumulative. Only an exact outer return admits another
call. Live stack/guard/marker/target checks and unchanged segment bases are
required for that transition. Guest TCB writes and observed control/vector state
survive; no fresh initial-zero-header admission runs after the initializer.
Typed failures stop before main. A missing/zero initializer is not counted, and
other provider modules are not swept. Phase and completed-call counters describe
actual execution observations. Original file-process tests verify the three zero
arguments, a nonzero returned initializer int, real TCB write/read persistence,
explicit-only selection, missing metadata and stops before main entry.

An exact `KernelGetProcParam` import now reaches the source-bound query through
the same attested marker/owner adapter. Original native process tests dereference
the returned actual primary data and distinguish absent process metadata from a
provider or module-parameter block. The full live range must remain readable;
the getter supplies no schema/version interpretation or copied parameter object.

Original supplied-image programs also exercise `KernelDlsym`: an actual loaded
module handle resolves a function pointer that guest code calls, or an object
pointer that guest code reads. Missing/private generated-image handles and
missing exports produce the observed int32 ESRCH plus a real cleared output.
That is a completed error return, not compatibility or module-loading success.
No host function address or arbitrary host library is supplied.

`--preload-libc-as <guest-path>=<host-path>` names the optional preload in the
guest namespace and retains its explicit read-only input handle throughout the
probe. Full original-file comparison precedes core binding; input replacement
and writers are denied by the worker's open-sharing policy. The actual native
outer return supplies opaque source/owner/initial-context/stack evidence before
the core can record initialization. Original programs increment a real TCB field
once, request the same loaded module twice, check handle2 and its optional int32
zero result, then verify the TCB was not initialized again. Uninitialized,
unbound and unsupported-option cases stop explicitly. Dynamic new-file loading
and nested initialization remain unfinished.

This synthetic trap does not establish instruction coverage for arbitrary
executables. It uses the existing Windows stack and no red zone or guest TLS.
It does not solve Windows/SysV red-zone incompatibility, stack switching, full
guest register/XState preservation, process startup, imports, or game execution.

Primary contracts: [Windows VEH callback restrictions](https://learn.microsoft.com/en-us/windows/win32/api/winnt/nc-winnt-pvectored_exception_handler),
[Windows handler removal](https://learn.microsoft.com/en-us/windows/win32/api/errhandlingapi/nf-errhandlingapi-removevectoredexceptionhandler),
[Rust constant thread-local storage](https://doc.rust-lang.org/std/macro.thread_local.html),
and [Windows stack usage](https://learn.microsoft.com/en-us/cpp/build/stack-usage?view=msvc-170).


`--memory-profile runtime` selects the pinned 12800 MiB direct-memory capacity
and a separate native backing/mapped-alias cap. From the initial snapshot it
retains half of scarce physical availability, or 6 GiB when more is available;
process commitment independently retains half when scarce, or 2 GiB otherwise.
The smaller resulting budget is rounded down to MiB and capped at 14 GiB
(the pinned 13.5 GiB guest memory profile plus 512 MiB native startup allowance).
It refuses when that budget is below 1 GiB. The snapshot is taken after
image intake. Other host allocations can still cause later OS failures. Input,
compiler and graphics allocations are outside the native cap. Kernel mappings
leave 512 MiB of headroom for image/TLS/entry backing. This is a bounded startup
profile, not a promise to fit every title or emulate full console memory.

With runtime memory, `--physical-backing-directory EXISTING_DIRECTORY` explicitly
backs direct physical sections with private sparse temporary files instead of
pagefile sections. The selected fixed-drive directory must be disjoint from all
read-only data roots, save roots and explicit input files. It needs free space
for the 12800 MiB direct pool plus 2 GiB; this volatile check cannot reserve disk
space against other applications. Files use fresh exclusive names, owner/SYSTEM
access, explicit zeroed extents, NX shared mappings and delete-on-close. Existing
files are never opened or truncated. The logical native backing/mapped cap is
14 GiB, while private pagefile commitment retains the smaller host-derived
allowance described above. File capacity is charged until the final referring
section closes. This trades pagefile pressure for possible disk I/O and does not
change host settings. Omitted mode keeps the original pagefile policy.

The 32 GiB kernel address window uses lazily split owned Windows placeholders;
reservation does not commit that amount of RAM. Dynamic executable modules stay
before the data window, and the translated cache searches both sides of code
under one finite attempt budget. Real physical allocations still create bounded
sections using the explicitly selected backend; aliases share them. Kernel memory-effect work capacity scales
with the selected budget, while attempts, copied bytes and file I/O retain their
independent limits. Omitted/explicit `compact` preserves the small fixture policy.

Execution profiles are independent of memory profiles. `probe` allows 10,000
blocks and `startup` 100,000, both supervised by the CLI for 30 seconds. Explicit
`extended-startup` allows 64,000,000 blocks with a 120-second CLI deadline, 4 GiB
source/emission/window totals, 16 Gi work units for admission and 256 Gi dispatch
work units. These aggregate counters cover repeated work rather than retained
guest backing. Progress is emitted every 2,097,152 blocks (at most 30 records), keeping
the same 64 KiB CLI output cap. The 64-byte per-block window, native memory,
source ownership, permission and cancellation rules still apply. This explicit
profile admits 1,000,000 kernel stops and 1 Gi provider-registration work units;
default profiles retain 10,000 kernel stops. Native backing, handles, I/O,
libc service bounds and per-call memory checks retain their independent limits.
Closed budget categories distinguish kernel stops/attempts, provider work and
other runtime resources without printing internal error strings.
These are diagnostic execution bounds; reaching one is not a completed boot.

Explicit `long-startup` permits up to 320,000,000 blocks under a 300-second CLI
deadline. Its cumulative source/emission/window, admission, dispatch and provider
work limits are five times the extended profile, except provider work at four
times. It admits 4,000,000 kernel calls
and four times the usual cumulative kernel byte/work accounting. Progress every
8,388,608 blocks retains a 38-record bound. This explicit supervised
profile raises the shared kernel live-resource metadata allowance from 256 to
4096; `probe`, `startup` and `extended-startup` retain 256, independently of the
memory profile. Native backing, mapped bytes, thread counts, per-call I/O, libc
limits and memory permissions remain independently bounded. This finite
allowance is a PentaX resource policy, not a firmware limit or proof that no
resources leak. The longer deadline is intended to distinguish slow
initialization from the next concrete runtime stop.

After prepared execution completes or refuses, the worker takes one
allocation-free census of the retained kernel owners outside the per-block
progress path. Final diagnostics report `kernel_handles_live`,
`kernel_handles_limit`, the independently summed `kernel_handles_accounted`,
`kernel_pthread_objects` and closed `kernel_handles_by_type` numeric counts.
The categories are memory, files, modules, sync, events, network, AMPR, keys,
semaphores and fibers. Pthread objects are already included in memory and their
subset is never summed twice. Free slots in the fixed key and reusable NP
tables are excluded. Matching totals establish accounting consistency, not
that retained objects are needed or that cleanup occurred. No guest identifiers,
addresses, names or payloads enter these diagnostics.

The explicit long profile also admits up to4,096 cumulative graphics library
calls for shader initialization. Other finite profiles keep the compact256-call cap.
Graphics work, retained shader slots, GPU limits and every backing/access check
remain independently bounded; this does not admit general rendering or boot.

Each source epoch retains at most 4096 pure block plans with an 8 MiB conservative
sum of their reported allocation capacities, plus a fixed table and one bounded
plan being prepared. Lookup and table initialization are precharged. A full
paired set replaces its least recently validated entry; exceeding retained
capacity clears the table and its recency hints.
The cache borrows one immutable source declaration and policy; exact native placement
is part of each lookup key along with PC, module and the full live source window.
module publication drops it before a new source view is bound. Hits still fetch
and compare every live source-window byte and call the same checked stepper for
owner/source/stack/operand validation and native publication. Existing dispatch
work charges remain conservative on hits. Aggregate hit/miss counters reveal
reuse without exposing guest addresses or instructions.

Within one borrowed source declaration, the stepper retains one successfully
checked initialized executable segment interval. Targets inside that immutable
interval skip repeated module/segment scanning, but still enforce numeric limits
and fetch-check the exact target byte against the current owner's executable
backing on every use. Misses preserve the original lazy scanner and its error
order. The memo carries no live permission or operand value, starts empty after
construction or machine reattachment, and does not change conservative work
charges, source-byte admission or guest execution limits.

The worker requests the separate two-form native streaming-store profile only
from its engine's admitted full-YMM mode. Exact owned write bounds and alignment
are checked before entry. Actual emitted legacy and VEX non-temporal stores are
ordered with a host MFENCE before Rust reads or subsequent service effects; this
also retains a compiler memory barrier. There is no guest-limit increase or
claim of atomic vector writes. The default software fallback remains available
when the admitted native vector mode is absent.

The separate native temporal-move request covers exactly 28 memory forms:
128/256-bit DQA/DQU and APS/UPS/APD/UPD loads/stores, plus SS/SD scalar memory
loads/stores. The worker enables it only through its already admitted full-YMM
mode; both widths require actual AVX, not AVX2. Scalar memory loads zero all
destination bits above the low 32/64 bits, packed VEX128 loads clear the upper
half, and memory stores retain the complete source vector. These transfers do
not evaluate floating-point bits or modify flags/MXCSR.

Every execution retains exact current-owner read/write spans and address
dependency checks. DQA/APS/APD alignment is checked for the new forms and six
existing legacy load/store forms; the closed emitted-move flag also requires
these checks in manual and diagnostic entry paths. Other memory families,
addr32, segment overrides, VSIB and EVEX remain excluded. Temporal moves do not
acquire a non-temporal fence requirement.

Original consumers preserve their literal results and meaningful native-to-Rust
boundaries through still-modeled operations. A modeled streaming load after NT
stores remains a software operation, including when its guest encoding names
AVX2; it does not demonstrate native host AVX2 execution. Historical comparison
artifacts pass their arithmetic and temporal-copy profiles explicitly rather
than deriving a release's capabilities from the shared observed vector mode.
Mediated vector counters intentionally decrease as real copies become native;
the complete memory accesses remain checked. No game boot or frame is implied.

The final epoch-stop snapshot also reports `scheduling_pending_equeue_waits`,
`video_open_ports`, `video_registered_buffers` and `video_vblank_subscriptions`.
These count actual pending event-queue tickets and retained registrations for
current live port generations. Subscriptions are included before their first
signal. The bounded snapshot adds no per-block polling, ticks or presentation
acknowledgements, and reveals no queue, port or buffer identity. Registration
counts do not prove accessible pixel backing, readiness or a completed frame.
Together with the real window state they can investigate a possible dependency
between a guest's first vblank wait and the first submitted flip.

The separate native scalar arithmetic request covers VEX ADD/SUB/MUL SS/SD
with an XMM register or an exact 4/8-byte read as the final operand. It requires
the existing full-YMM request and actual AVX admission. Only the active scalar
element is evaluated; the remaining low 128 bits come from source1 and the
upper 128 bits are cleared. Current masked MXCSR controls, ordered NaN behavior,
flags and all other vector state remain enforced. The emitted scalar-memory
flag independently requires live span/dependency checks in manual and diagnostic
entry paths. Divide, square root, fused arithmetic and EVEX gain no authority.
Original consumers retain a real modeled VMOVQ boundary and verify inactive
signaling-NaN payloads without introducing spurious exceptions. A finite
comparison selects its preceding release artifact explicitly; the observed
native vector mode alone does not identify an artifact's scalar policy.

A separate request admits only register VFMADD213SS and VFNMADD213SS with
the actual full-YMM engine's AVX and FMA support. It preserves the old
destination's inactive low128 bits, clears the upper128 bits, and retains
masked MXCSR rounding, DAZ/FTZ, sticky status, ordered NaNs and integer flags.
Other supported VEX fused forms and memory operands retain their checked
software route; EVEX remains explicitly unsupported.
`native_scalar_fma=true|false` reports this engine capability after
admission; it does not count retired operations or establish a speedup.
Original callers compare real native and mediated counter deltas while
checking exact fused cancellation and state across a kernel service call.
Software underflow uses precision rounding with an unbounded exponent to
determine tininess before the final subnormal rounding; merely rounding a
tiny value to the minimum normal value does not by itself suppress FTZ.

Final-only cache observations distinguish successful executable replacements
from `cache_evicted_key_revisits`: returns to a slot's immediately previous
evicted publication key, qualified by source epoch, guest PC and full native
vector mode. No keys are emitted. Byte-identical reuse leaves its historical
producer unchanged. A revisit does not prove identical emitted bytes, complete
reuse distance, a dominant cost or the cause of every cache miss. Publication
metadata has fixed storage bounded by the existing slot count; its allocation
and fixed miss work are admitted before executable writes. Saturating counters
set `cache_history_saturated` without making diagnostic overflow fatal.

The final `plan_cache_occupied_collisions` and
`plan_cache_same_key_replacements` count committed retention changes.
`plan_cache_byte_purges` and `plan_cache_purged_plans` report actual byte-pressure
purges and their victim counts. A collision and a purge may describe the same
operation. Retained and peak byte gauges describe the existing conservative
allocation-charge ledger, not simultaneous or whole-process heap consumption.
Counters accumulate across source epochs; the retained gauge is the last
successful snapshot, cleared before preparing a new epoch's cache.
`plan_cache_observations_overflowed` marks saturated observations.

Final `pthread_mutex_waits`, `pthread_condition_waits` and
`pthread_notified_condition_waits` describe actual retained queues.
`pthread_eligible_mutex_releases` counts final releases with eligible queued
waiters, including the full recursive release performed by an admitted
condition wait. `pthread_barging_mutex_acquisitions` counts fresh acquisitions
while an eligible waiter is still queued; completion of a queued wait is not
a fresh acquisition. Recursive unlock decrements and unnotified conditions do
not create eligible-release observations. Kernel observation lookup work has
an independent fixed allowance; saturation or exhaustion sets
`pthread_wait_observations_overflowed`, making history potentially partial.
These observations do not change ownership, notifications, readiness or
scheduling, and cannot establish starvation or a title's blocking cause alone.

Final-only graphics fields expose existing committed accounting:
`graphics_calls` includes all implemented graphics-provider calls, including
VideoOut and shader helpers; `graphics_command_buffers` counts nonempty admitted
command ranges before parsing. `graphics_draw_attempts` and
`graphics_dispatch_attempts` count charged attempts, not completed execution.
`graphics_written_bytes` includes committed PM4 memory/label writes as well as
actual draw/compute writeback, so it cannot prove that a draw completed.

The fixed retained presentation snapshot reports `video_flip_reserved`,
`video_flip_recording_with_frame`, `video_flip_ready_with_frame`,
`video_flip_presenting` and `video_flip_acknowledged`. It visits at most 48
request slots and three current live ports without guest reads or readiness,
clock or permission probes. Ready requests include those blocked behind an
older incomplete FIFO request. Presenting means core Frame handoff, not display
completion. Acknowledgements aggregate current live-port counts with saturation
and reset on close. An empty snapshot cannot distinguish no request from a
rejected/rolled-back request or one removed by closing its port. These fields
add no progress polling or scheduling, budget, window or rendering behavior.

`graphics_function_top` and `graphics_function_last` retain a fixed history of
44 closed graphics Function variants. Every existing call-admission path records
once after its current quota debit, including register defaults and the bound
VideoOut flip route. Normal source-defined rejected returns and subsequent
preparation failures still count as admitted attempts. Quota refusal before
admission retains the prior history. The top four use fixed catalogue order for
ties; the last label is historical admission, not an in-flight function, fault
attribution or successful GPU execution. No arguments, registry identities or
guest text enter this profile.

`graphics_shader_create_ok`, `graphics_shader_create_rejected` and
`graphics_shader_create_failed` separately count admitted CreateShader calls
that return zero, return a nonzero guest status, or stop with a typed failure.
They are final-only saturating counters, recorded after the helper returns.
Calls refused before driver admission do not count. A zero helper status is
neither GPU compilation nor a rendered frame; other graphics APIs can return
valid nonzero sizes or pointers and are not classified by this rule.

The periodic completed-block observer also reports
`sample_self_target_branches`, `sample_self_target_native_register_blocks` and
`sample_self_target_native_register_instructions`. These describe sampled plans
whose direct conditional exit targets their exact block start, including when
the condition is untaken. The strict register-body subset requires a nonempty
actually emitted body, known immutable memory summaries, no explicit/implicit
memory or RSP writes, and no stack, divide, fence or control instructions.
Unknown summaries are excluded. The instruction total excludes the terminating
branch. Counts commit only after successful sampled dispatch, within the existing
1-in-4093 sampling and unchanged observer quota; they add no guest reads or new
planning. They are candidate-selection observations, not taken-loop counts,
whole-run frequencies or execution authority for a native loop.

Service profiles count admission actions. A parked wait is not an immediate
nonzero return, and its later resumption/timeout is not recorded by that profile.
Zero nonzero-return counts therefore do not establish immediate mutex success
or useful game progress.

Prepared image execution uses a bounded 2 MiB native code cache with 4096 fixed
512-byte slots, each including a private trailer cell. Guest PC selects a pair;
an exact source-epoch/PC/vector-mode residency hint is preferred, followed by
an empty or stale-domain way, then the least recently validated way. The
single-slot API profile retains its one-slot fallback. The pure planner
re-encodes for that exact selected address; an old plan is never retargeted.
A reuse requires previously
successful sealing plus exact live code, tail and cell comparisons. Collisions
replace one slot; publication revokes execute on the affected 16 KiB page groups
before writing, reads back the native body and tail, then seals RX and flushes
those groups. Neighboring page rights remain unchanged. A page becomes eligible
for entry only after a successful flush; every write checks its complete writable
page span. The source remains logically
immutable and is checked against the current declaration on every step. Stack,
CPU, host features and current operand rights are also rechecked on every reuse.
Counters distinguish successful publications, exact reuses and hardware entries.
Resident hints commit only after validated cache reuse or complete publication
and sealing. A later hardware-entry admission refusal can follow that genuine
cache event without executing the body. Historical publication keys remain
separate, including when another PC reuses identical bytes. Every cache hit
still performs the same checked execution admission. Native residency and pure
plan recency use the existing admitted block count, not a new clock or guest
value. This retention policy does not itself prove a title speedup or startup.

Each immutable block also retains a fixed native-admission summary, computed
from its final instruction records within the planning allocation and work
limits. Reuse reads cached instruction counts and required feature bits instead
of walking the same records again. A conservative proof of an emitted body with
no memory accesses can also skip repeated footprint analysis. Unknown templates,
atomics and potential memory forms keep the full analysis path. These facts do
not cache runtime addresses or permission checks. The existing per-step work
charges are preserved separately from the added construction work; source,
owner, stack, host mode and actual memory operands retain their live checks.

An attested plan containing only the exit sentinel resolves directly through
the existing modeled control/vector/string/segment path without publishing code
or entering the hardware bridge. It retains source/owner/state/stack/operand and
budget checks and the same call/return observations. This is an execution
optimization, not a bypass for unsupported behavior or missing source backing.

Page transitions follow the Windows [VirtualProtect](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualprotect)
and [FlushInstructionCache](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-flushinstructioncache)
contracts within one committed reservation, without granting RWX.

Typed guest memory stops distinguish finite native-memory admission refusal
(`guest-memory-budget`) from allocator failure (`guest-allocation`). A failed
exact public allocate/map service reports only its static operation label,
requested byte count, fixed-mode bit and aggregate mapped-byte/handle counters.
Addresses, physical offsets, output pointers and raw messages remain withheld;
these observations neither retry the call nor relax its memory policy.
The same failure records actual native-owner mapped/committed/reserved/backing byte
counts and its retained mapped/reserved ceilings. Kernel logical accounting
remains separate. These aggregate figures expose no address or handle, and
committed backing is distinct from the host process working set/RSS.
`memory_native_file_backed_bytes` reports retained data-file capacity, while
`memory_native_private_commit_limit_bytes` reports the independent private cap.
Data-file views are excluded from pagefile-backed committed-byte accounting.
The explicit runtime profile reserves physical sections and commits each offset
when first mapped. Whole section capacity still consumes the backing allowance;
aliases and remaps reuse existing commitment. Compact fixtures retain eager
sections. A typed memory stop can also report the latest transaction's closed
host operation and numeric Win32 code, excluding addresses and raw error text.
