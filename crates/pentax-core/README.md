# PentaX core

The common AGC command-buffer builders retain the pinned public allocator and
packet contracts: NOP, direct dispatch, index type/base/count, instance count,
direct indexed/automatic/offset draws, DCB/ACB writes and waits, release-memory,
parser drain, their selected size helpers and five packet patch helpers.
The register family adds SH ranges, grouped SH/UC runs, single SH/CX/UC
registers, indirect CX/SH/UC pair arrays, their four size helpers and nine
address/count patch helpers. Fifty-two builder identities use only the exact
`Graphics5/Graphics5` and `libSceAgc/libSceAgc` module 1.1/library 1 pairs.
No hybrid pair or NID-only lookup is added. The original 178 markers remain in
place; new metadata fits the same
16 KiB immutable provider mapping and its existing allocation limit.

Insufficient capacity can suspend each allocation for one genuine guest allocator
callback, using `(buffer, requested_dwords + reserved_dwords, user_data)` with
the source's 32-bit addition. The owner/source/thread/executor-bound ticket is
completed only by that exact nested call's actual return. The callback may
change the descriptor and payload; completion rereads current cursor/capacity
and reads payload only after publishing the cursor and fixed header words in
source order. Reentrant calls receive independent admitted tickets within the
existing 32-frame bound. Callback false or insufficient returned capacity follows
the individual source NULL/fatal branch, without retrying that allocation's callback.
Grouped calls may continue to another allocation and genuine callback. Each
completed receipt is consumed before any fallible continuation; a subsequent
grow gets a fresh serial in the same retained slot and must establish a new
actual nested call. The whole call retains its original work/output admissions.

Grouped SH snapshots every offset/value pair before its first allocation;
allocation failure returns the first previously emitted command. Grouped UC
stages only the current run's values, rereads the next boundary pair after
allocation/callback, and continues past failed allocations. Neither operation
is replaced by universal presnapshotting or fit-only publication. Range values
are read after cursor/header publication; a null range source leaves the value
words untouched. Single registers are eight-byte by-value arguments. Indirect
builders encode the source pointer/count without reading its pair array; its
eventual execution still requires independent current GPU-read authority.

Finite work, output bytes and any payload capacity are admitted before callback
entry. Each WriteData body's pending payload capacity is bounded by the existing
per-call command host allowance and the source maximum of 65,524 bytes. Thirty-two
pending tickets containing only those bodies therefore retain at most 2,096,768
payload bytes plus fixed metadata.
Ranges and grouped register arrays instead retain their actual admitted
capacity under the unchanged per-call host allowance; grouped calls also
include their fixed pending metadata in that host bound. At most 32 tickets
are live, including reentrant calls; sequential grows reuse one slot. Across
all retained continuations, actual payload and register-staging capacities
share the unchanged 2,096,768-byte aggregate payload cap. A new continuation
is refused before retention if its capacity would exceed that cap; a consumed
slot's prior body is already released before replacement. Fixed ticket metadata
remains separately bounded by the 32 slots. A later guest
read/write fault preserves already committed cursor/header
prefixes, consumes the callback ticket and poisons further driver submissions;
neither guest execution nor a partial builder is replayed. Payload `memcpy`
overlap with its output is a typed refusal before copying; prior source-ordered
prefixes remain visible. Packet patches retain their individual source error
domains and masks, including low-DWORD-only wait-reference patching. The older
flip encoder keeps its separate atomic fit-only publication profile.

Constructing bytes does not claim GPU acceptance. The checked graphics executor
retains source order for supported Write/Wait/Release/Drain actions, invalidates
any previous current-draw receipt at every such boundary, and uses actual live
GPU permissions and command-source physical disjointness for writes. An
unsatisfied wait stops before later actions. Empty writes access no target;
the drain accepts only already fenced prior work without inventing a Vulkan
job, frame, interrupt or display acknowledgement. Unsupported packet modes
continue to refuse explicitly.

Guest memory, checked image staging, the bounded reference interpreter and original bring-up diagnostics. Guest addresses never become host pointers. General PS5 execution remains unsupported.

`with_native_image_session` scopes explicit immutable position-independent inputs,
real libc/kernel/Graphics5Driver factories, a classified unavailable-function image, retained
plans and the actual native owner through one callback. It derives image biases
while preserving admitted load alignment, budgets the reservation, computes typed
TLS placement with owned guards, and consumes the existing main-entry staging
API. Source payloads stay immutable. The callback result is not execution evidence;
the separate reviewed worker supplies that evidence. Fixed-address images,
unsupported payloads/relocations and unavailable data/weak/TLS imports refuse.
Default formatting reports stable error categories, not source identifiers.

The opt-in `plan_kernel_startup_with_unavailable` recomputes exact missing-function
classification with the actual real providers before adding an immutable factory.
It checks every identity/reference and final target binding, then attests marker
bytes and live permissions on the same native owner. `service_request` rejects an
unavailable function or the exit callback with a typed error after owner,
continuation, full ID and exact live-site admission. It dispatches no kernel/libc
implementation and emits no successful result. Missing implementations stay
visible even though their relocation addresses are known. The original strict
constructor is unchanged. Automatic module discovery, complete parameter interpretation and firmware
TLS, asset capabilities and module shutdown remain unfinished.

The explicit `NativeImageLimits::startup_libc` provider index prepares a unique
nonzero `DT_INIT` under the observed three-zero-argument preload convention.
`PreparedNativeModuleStart` binds the actual source, native owner, selected shared
module, initialized immutable target and opaque process-entry history. Duplicate
tags, overflow, padding, another module's target and primary/executable roles
refuse. Absent/zero `DT_INIT` supplies no call, without inventing success. This
does not run guest code or sweep constructor arrays; the worker retains the thread
and cumulative execution state while making the observed call before main entry.

The private kernel adapter implements `GetProcParam` from its actual retained
primary source plan. It returns the checked biased `PT_SCE_PROCPARAM` address,
with whole-extent live read admission and bounded metadata/page work before
dispatch. No payload is allocated/copied or schema inferred. Absence is a real
null result; empty/shared-primary shapes and unbound integration remain errors.
`PT_SCE_MODULE_PARAM` and provider metadata never substitute for primary process
parameters. Typed backing faults remain faults; no guessed errno/zero is emitted.

The loaded-symbol query uses process-local supplied-image handles from one,
separate from SCE selectors; generated HLE/unavailable images are excluded.
`supplied_module_handle` exposes only that identity, never an execution capability.
The private adapter binds the actual source, module count and cumulative provider
ledger to the optional linker query. It checks complete live readable exports;
functions additionally require initialized immutable source and denied write
permission. Successful pointer outputs and explicit source miss/null error
outputs use the existing atomic kernel transaction. Unknown integration, work
limits, unsupported metadata and backing faults do not zero the output or resume
with invented status. This does not establish module initialization or loading.

`process::tls::plan_static_tls` retains an immutable borrow of the exact symbolic link plan. Its explicit `PentaXVariantIIInputOrderV1` policy assigns separate runtime TLS IDs and places static blocks below a guest thread pointer in input order, preserving each template's virtual-address alignment bias. Initialized PT_TLS payloads must agree with readable logical initialized image bytes, including discontiguous backing; BSS, gaps and page padding cannot supply them.

`load_symbolic_with_static_tls` accepts that private plan, bounded guest TP positions and memory limits. It requires complete supported relocations, stages the retained images internally and copies the relocated templates into independent guarded RW/NX storage with zero-filled tails. It preflights aggregate storage, resident memory, mapping and work bounds. The resulting `PreparedTlsSession` exposes immutable memory and diagnostics only. Generic `load_symbolic` keeps its existing behavior.

Ordinary image relocations may share the numeric range of a declared TLS zero tail. That tail belongs to independent per-thread storage, not the ordinary image. Preparation applies checked image writes, copies only the relocated `p_filesz` prefix into each instance and leaves the remaining `p_memsz` bytes zero, including writes crossing the initialized boundary. Image coverage, permissions and relocation overlap checks remain enforced. Instance count follows the retained configurable limit (default two), alongside the shared storage, memory and work bounds.

The work limit applies separately to each planning or materialization call and covers the declared scans and comparisons. Template copying is bounded by the initialized-byte, instance, storage and memory limits.

`plan_relocated_static_tls` is a separate opt-in path over a linker `TlsBindingPlan`. It refuses any remaining unbound deferred record, derives the same layout from the binding plan's exact base and privately computes typed eight-byte values: DTPMOD64 writes the provider's layout runtime TLS ID, DTPOFF64 writes S+A and TPOFF64 writes S+A-O as two's-complement data, where S is the module-relative TLS offset and O the provider's positive distance below TP. Under this initial profile S+A must lie in the provider's `[0, p_memsz)` and DTPMOD64 addends must be zero. Runtime IDs never derive from module positions, SCE selectors or a fixed value, and no load bias is added to these scalars. The bound provider's module, PT_TLS header and memory size must match the layout. Symbol-zero DTPMOD64 selects the consumer's own nonempty PT_TLS template and receives its derived runtime ID without inventing a symbol provider. Extra writes join the resident page union, including otherwise untouched BSS pages; only initialized template bytes reach TLS instances. `load_relocated_static_tls` applies every payload, ordinary write and TLS write before final original/RELRO permissions, then copies the relocated templates. No public patch, memory or session pairing exists; `load_symbolic` and `load_symbolic_with_static_tls` keep their deferred behavior.

The original closed startup fixture consumes this same preparation path and interprets planned negative TLS offsets. This verifies the shared component without defining a PS5 TCB, DTV, real thread scheduler, constructor ownership or service ABI. No host FS/GS state is changed. Empty and zero-sized templates remain explicit. The default `ThreadControlBlockPolicy::None` preserves this layout without adding storage at TP.

The explicit `KytyObservedSelfPointerV1` profile adds a 64-byte zeroed header whose first word is its guest self-pointer. It occupies an owned RW/NX 16 KiB page at TP; the upper guard moves to the following page. Static blocks remain below TP. Sparse and native preparation share header initialization, geometry and aggregate storage/mapping/resident/work checks, including header-only instances with no static payload. This profile records the pinned Kyty allocation behavior, not a complete firmware TCB/DTV or module-TLS ABI.

`load_native_relocated_static_tls` is an explicit opt-in over that same private relocated TLS plan. `PreparedNativeTlsSession` retains the exact immutable link source and owns the actual native address space, template diagnostics and independent instances. Sparse and native materialization share source validation, instance layout, guard separation, staging and the template-copy loop. Original and typed relocation writes precede final image/RELRO permissions; template copies then read those relocated native pages. No sparse snapshot supplies the native copies.

The native policy eagerly commits every mapped image, TLS storage and opted-in header page, so their complete sum is charged against both mapped and resident limits before allocation. Resident bytes mean logical committed guest bytes, not physical working-set or exact heap measurements. A separate bounded reservation-union preflight accounts for the supported 64 KiB Windows allocation geometry. Both guards must fit inside the reservation union created by image/storage mappings while remaining logically unmapped. Unsupported boundary placements are refused; no unrelated host page is treated as a guard. Actual owned reservation coverage is checked again after materialization. The platform owner keeps these holes reserved and OS NOACCESS and keeps all committed source pages physically NX, including logical-X provider pages. This placement restriction is PentaX policy, not a PS5 ABI rule.

`prepare_native_hle_startup` admits the existing `HleStartupPlan`, recomputes typed bindings/layout under its retained limits and owns the resulting `PreparedNativeTlsSession` throughout use. It attests all fifteen generated provider sites on the same native backing. Its service-stop adapter remains crate-private, checks exact site/continuation/full service ID and retains one cumulative budget. Public access provides immutable diagnostics and the existing exclusive paused-memory borrow, with no raw pointer, host FS change, complete firmware TCB/DTV, native entry or general executor. Ordinary native memory/HLE preparation continues to refuse TLS and deferred relocations. Any preparation error drops the whole unpublished owner and leaves immutable source images unchanged.

The composed `PreparedNativeKernelStartup` additionally retains generated kernel/libc providers and bounded kernel state over that actual native TLS owner. Its public checked `pause` exposes the same backing, and `owner_id` is stable across Rust moves. `prepare_native_source` recomputes typed TLS bindings/layout under retained limits and constructs a private `PreparedNativeSource` bound to the exact immutable base and actual owner. Its materialized declarations cover every supported deferred record with the derived consumer, relocation provenance, target and value. Unknown deferred records still fail. Ordinary image writes never populate independent TLS zero tails. This proves preparation history; it does not prove continuous byte identity, owner liveness or ordinary native operand bounds.

`service_request` accepts PRE-SYSCALL observations, rechecks owner identity, exact continuation, live immutable provider bytes and full 64-bit service ID, and preserves the retained service budgets. Kernel calls receive RDI, RSI, RDX, original RCX, R8 and R9; the legacy libc continuation is synthesized only after capture and admission. Typed errors remain explicit and do not authorize a fake zero result or guessed errno conversion. The core still forbids unsafe code and supplies no executor or host pointer.

The native kernel adapter now connects direct physical allocation/map/release
and subrange protect/unmap to that same paused owner. Shared pagefile sections
provide real aliases at 16 KiB offsets; view removal preserves their bytes until
physical release. Whole or partial release withdraws every affected alias page,
preserves surviving section offsets and permits zeroed physical-hole reuse.
Prospective output and alias-overlap checks precede
effects. Partial irreversible unmap or failed rollback poisons the backing and
stops execution. Dynamic private pages and physical sections share the unique
commitment limit, while every alias consumes virtual mapped capacity. The original
linked worker consumer verifies actual native writes through both views,
protection changes, persistent remap and fresh zeroes after release/reallocation.
Physical release stages rollback data and metadata before effects; a recoverable
failure restores the original views. Failed restoration or irreversible partial
section closure poisons the owner. Partially retained sections stay fully charged
to the commitment cap until their last piece is freed. Pools and GPU/cache modes
remain unavailable; the unchecked source wrapper's unowned no-op remains an
explicit kernel admission stop.

The generated kernel provider has exactly three load segments. The established
four-page RX extent contains only the ELF header and initialized service markers;
every marker RVA and service ID is retained. The fixed RW/NX object page starts
at RVA `0x10000`, with the guard at `0x10000`, program-name pointer at `0x10008`
and its BSS buffer at `0x10010`. A separate R--/NX segment starts at `0x14000`
and holds the program headers, dynamic descriptors, names, symbols and relocation
record. Moving the program-header table also avoids overwriting the first marker
at `0x100` when a fourth ELF program header is needed.

Metadata uses its actual initialized byte count and page-rounded mapped extent,
with an independent ceiling of eight 16 KiB pages. Checked layout arithmetic
accommodates the declared 2,048-registration limit, all admitted namespaces and
three-digit selectors without moving objects or introducing another provider.
The complete file is capped at 147,728 bytes and the mapped extent at `0x34000`;
ordinary source, mapping, resident and work budgets still charge actual admitted
sizes. Provider placement follows the actual mapped extent. Larger images,
catalogues, names and arithmetic failures refuse before image allocation.

Source execute rights remain logical; physical guest backing stays NX. Exact
factory identity and all three segment permissions are validated during planning
and preparation. Initialized metadata bytes are compared on the actual native
owner at preparation. RX and metadata permission checks run again before service
admission, with work charged for actual pages. Metadata is never callable source.
Namespace/version checks, exact marker bytes and source lineage remain required.
Initialized file bytes and zero-filled memory extents stay distinct; the
program-name buffer uses the existing RW BSS. Additional capacity grants no new
service ABI, alias, firmware behavior or implementation success.

The fixed kernel provider also exports the source-observed stack-check guard and
program-name pointer as genuine GLOBAL/PROTECTED OBJECT symbols. Their separate
R-W/NX page belongs to each prepared owner; the guard retains the pinned guest
initial value, and a checked RELATIVE relocation points the name object at its
owned 512-byte buffer. Main-entry preparation publishes that buffer and argc/argv
in one transaction before any selected preload runs. These objects are neither
callable service markers nor host pointers. The image runtime explicitly selects
the linker's optional weak-address null profile; strict standalone linking and
TLS retain their default refusals. No commercial boot is implied.

`gpu::execute_compute` bridges one self-contained PM4 compute stream to Vulkan
while retaining an exclusive pause on the actual native guest owner. It copies
the bounded command span, derives explicit known compute registers and shader /
resource bindings, compiles the bounded acyclic shader, checks live permissions
and rejects resource self-aliasing before GPU use. The driver receives only the
opaque prepared compiler product and owned upload. Host/device geometry, memory,
objects and work remain bounded; barriers and fence completion precede same-owner
transactional writeback. No guest CPU can race the submission through this API.
General asynchronous queues, draws and presentation remain unfinished.
Original isolated-worker diagnostics verify two dynamic submissions,
their full output and untouched input/tail bytes; no game frame is claimed.

The image runtime also installs a fixed `Graphics5Driver` library 1 / module 1.1
provider before classifying missing functions. Four source-observed submit
imports (`SubmitCommandBuffer`, `SubmitMultiCommandBuffers`, `SubmitAcb` and
`SubmitMultiAcbs`) route through exact owner/site/full-ID/live-byte/permission
checks. Their private service numbers never reach the host kernel. The original
strict kernel-only constructor stays available; `plan_kernel_startup_with_graphics`
is the explicit composition API. Normal exact linking retains namespace, version
and ambiguity checks; no graphics NID is placed in libkernel.

The same immutable provider exports `AgcSuspendPoint` in the exact paired
`Graphics5` and `libSceAgc` module/library 1.1/1 scopes. Its zero-argument ABI
records the source frame boundary and next-graphics reset flag without clearing
compute registers. Every supported submission already completes its Vulkan fence
and same-owner writeback synchronously, so there is no outstanding prefix to wait
for at a successful boundary. A poisoned partial submission remains terminal;
call budgets and boundary-counter overflow refuse before boundary publication.
`graphics_frame_state()` exposes this bookkeeping independently of presented
frames. Draw submissions and presentation remain unfinished.

`gpu::driver::Limits`, retained in `NativeImageLimits::graphics`, bounds calls,
batch metadata, dispatch attempts, command bytes, admitted invocations and
preparation work cumulatively. Each compute dispatch has independent compiler /
host/device bounds. Batch address/size metadata is captured and admitted before
the first GPU call; command/resource data is prepared in sequence under the same
exclusive pause. The pinned null and zero-count branches return their actual
source domains without reading irrelevant pointers. Packet flags/reserved bytes
are ignored by this source path. Nonempty work accepts only observed ACB queues
`0x20..0x58`; draw queues fail with a typed unsupported stop. Each of these 56
queues retains independent known compute registers across successful buffers.
Register-only updates and multiple dispatches are admitted by a complete bounded
command plan. Ordinary indirect calls return to the parent; chains discard the
current tail. The explicit stack and active chain lineage have independent depth,
hop, cumulative byte, packet, record, capacity and work limits. Conditional /
predicated commands and other indirect control modes remain unsupported.

Each dispatch still uses the acyclic wave32 raw-buffer compiler profile. Its
resource bytes are read after the previous dispatch has fenced and committed.
The final queue state publishes only after the complete buffer succeeds. Once a
GPU effect could have happened, any later failure leaves the driver terminal;
no stale-state retry or false whole-buffer rollback is offered. Captured command
spans are retained in the plan, and `preflight_disjoint_backing` rejects output
bytes aliasing any of them, including different virtual views of one physical
section. This is a bounded synchronous compute profile; asynchronous scheduling,
command-memory coherence, draws and presentation remain unfinished.

An original x64 file consumer resolves all four imports, builds command addresses
from its actual relocated guest base, invokes four GPU jobs with different SGPR
seeds, and branches on results plus untouched tail bytes before returning 42.
Three additional calls exercise null/zero-count return ordering. This is actual
native CPU/import/compiler/Vulkan/writeback evidence, not a commercial-game boot.

A second native consumer first supplies register-only state to two queues, then
invokes all four imports with indirect call/chain streams containing two
dispatches each. Its eight GPU jobs verify separately inherited seeds and new
seeds in distinct buffers, queue isolation and preserved tails. Chained-away
unsupported commands remain unexecuted; the nine imported calls return 42.

Ordered memory commands now share the same exclusively paused owner: supported
WRITE_DATA packets and immediate 32/64-bit release labels use checked
transactional writes, cumulative byte/work limits and command-backing alias
refusals. Fulfilled 32/64-bit waits compare actual current bytes after preceding
fenced work. Unsatisfied waits return a distinct `Waiting` stop; asynchronous
queue suspension, polling and resumption remain unimplemented. Interrupts,
timestamps and unported destination/cache/event profiles remain typed stops.

Indirect dispatch arguments are read at execution time, using either an absolute
address or the retained compute SET_BASE plus an offset. Zero dimensions charge
their bounded read work and execute no shader, resource read or Vulkan job. A
third original native consumer verifies GPU-produced zero dimensions, a skipped
output, WRITE_DATA-fed nonzero dimensions, a based indirect dispatch, repeated
address writes, 32/64-bit labels and both fulfilled wait widths. It returns 42
after one real imported call and preserves its input file. These are synchronous
compute observations, not an Astro Bot boot or frame.

`prepare_main_thread` requires the explicit observed header profile, exact retained source and first TLS instance. It checks live RW/NX permissions, header geometry, self-pointer and all remaining initial zero bytes within the retained work bound. The opaque `PreparedNativeThread` records owner/source/TP preparation history; it is not ongoing liveness or execution authority. Other instances remain independent storage rather than invented scheduled threads. The worker uses this proof to initialize its modeled FS base and refuses a conflicting initial base.

`prepare_process_entry` accepts only the exact prepared source/owner and explicit `NativeEntryPolicy::KytyObservedV1`. It derives a guarded page-aligned stack, 32-byte argc=1/argv object, owned terminated name, two zero root words and an outer return marker. Initial guest RDI/RSI/RBP/RSP values are exposed through an opaque `PreparedNativeEntry` history object. Both the primary entry and supplied exit callback must belong to initialized immutable source ranges and remain live executable/nonwritable. Execute-only entry, callback and DT_INIT segments use instruction-source access without granting guest data-read permission, keeping the independent [ELF segment flags](https://docs.oracle.com/cd/E19120-01/open.solaris/819-0690/chapter6-83432/index.html) intact. Host pointers, executable padding and missing implementations are not accepted as callbacks. The callback's behavior remains the caller's lifecycle responsibility.

Name, committed stack, metadata work and transaction limits precede effects; one checked transaction maps the fresh RW/NX stack and writes arguments/frame. Stack and adjacent guards must already be wholly reserved and unmapped on that owner. Recoverable failures leave this storage untouched; rollback poisoning requires discarding the owner. The fixed argument area admits at most 223 non-NUL name bytes. The original native consumer verifies actual argument/name/frame reads alongside the existing services and TLS. Module initializers, process-parameter decoding, exit services and full firmware entry semantics remain unfinished.

The separate platform/worker path consumes this provenance through an explicit unsafe one-block boundary. It attests actual live source, owns the cache/context and releases its paused borrow before service work. The manual native entry requires caller-established ordinary operand bounds; the checked worker entry instead isolates memory instructions and validates their complete live footprints before execution. The original prepared kernel/TLS consumer returns 42 after four services in debug and release; its first TLS instance becomes 42 while its second and the original template remain 37. Three modeled FS operations check the header self-pointer and a real write/read of the first header's field, leaving the second unchanged. These observations do not establish complete firmware TCB/DTV, host FS/GS TLS or commercial-game startup.

The NpWebApi2 provider exposes nineteen source-backed operations through four
finite module/library combinations of `NpWebApi2` and `libSceNpWebApi2`, with
module1.1/library1 and FUNC imports. Local contexts and requests use independent
monotonic IDs and bounded owner state. Initialization records a rounded pool
size without allocating a network pool or validating the logged HTTP context.
Requests retain copied opaque strings; sending an existing request clears its
empty response and returns NOT_SIGNED_IN. Reads then return zero, and response
header helpers expose only the pinned local metadata. Statistics publish exactly
28 defined bytes, preserving trailing padding. Live handles, actual retained
text capacities, scratch and work share the kernel limits; failed checked
publication leaves retained state unchanged. User, push, filter and callback
receipts record data only. This profile supplies no PSN authentication,
transport, queued events or guest callback execution.

`bind_loaded_module_file` owns an explicit file capability after comparing its
complete original input with one actual supplied shared module. Canonical guest
paths, roles, uniqueness, handles and cumulative file-read/compare work are
bounded before registration. A file binding supplies no initialization evidence.
On Windows, `observe_module_initializer` accepts only an opaque actual first-call
return bound to this prepared source, live owner, initializer entry, exact stack,
zero arguments and admitted thread bases. A repeated observation refuses.
The pinned existing-program `LoadStartModule` branch then commits its positive
handle, optional int32 zero output and checked count increment without another
initializer. Unbound files, missing observations and invalid output remain explicit
stops; recoverable failures leave count and output unchanged. This registry does
not implement unloading.

`register_dynamic_module_file` retains a canonical explicit read-only capability
inside a privately configured reservation window. `service_action` checks the
current opaque materialization proof and existing marker/owner/full-ID admission
once, returning either an actual retained handle or an opaque new-file ticket.
Reading is bounded and deferred until requested. Publication requires that exact
admitted input, the active parent epoch, unchanged old ordinary/TLS bindings and
a unique initialized immutable shared-module `DT_INIT`. Only new pages are
staged, relocated and protected; failed staging rolls them back, poisoning the
owner if cleanup fails. Old writable memory is never replayed from source.
The source-index/handle map excludes generated providers. An opaque nested-call stamp and actual
return admit the int32 result and Ready state; mapping alone supplies no return
evidence. Failed results never permit automatic caller continuation. Legacy
initial-source service/proof APIs refuse after an append; epoch APIs preserve
the original actual owner and thread/TLS materialization history.

A late-loaded guest may introduce new missing function imports. The extension
classifier uses the same source-bound retirement policy as final linking, and
publishes the guest plus an optional generated stop provider in one transaction
with the live source as its sole parent. Core independently reclassifies the
exact selected input and checks every factory identity and reference before
mapping. Additional providers retain bounded owned manifests and static public
diagnostic hints; their immutable image storage stays borrowed by worker-owned
source epochs. Source identity, live bytes and permissions authorize each stop,
with the actual PC range selecting its provider before interpreting local IDs.
Only the guest receives a handle or initializer. All new pages and TLS are
staged and sealed before retained-slot replacement; failure restores the whole
suffix. Later real definitions can replace authenticated markers from any
retained factory. Functions remain explicitly unavailable until implemented;
data, TLS and incompatible imports are never replaced by callable placeholders.

Dynamic-file sessions admit bounded spare static TLS storage below the original
thread pointer up front. A later template may append under the explicit PentaX
Variant II policy only if all old IDs, offsets, declarations and block locations
remain unchanged and every new block fits that capacity. Typed DTPMOD64/DTPOFF64/
TPOFF64 writes in the new image precede copying its relocated native template.
Only the new block is initialized; old TLS and TCB bytes remain live. Failed
publication restores prior spare bytes as well as removing new image pages.
Cleanup failure revokes the owner. Prepared instances remain independent.

The attested `tls_get_addr` marker reads a bounded two-word descriptor and resolves
its private runtime TLS key/offset against the actual prepared main thread. It
returns only a live owned RW/NX block address; unknown identities, invalid spans,
missing integration and exhausted limits remain typed stops. The first word is
produced by the typed DTPMOD64 policy, never interpreted as a host `Program*`.
This does not infer a firmware DTV, lazy host-thread allocation or a scheduler.


`LibcHeapGetTraceInfo` has two exact exports: library `LibcInternalExt` version 1
inside module `LibcInternal` 1.1, and the matching `libSce`-prefixed pair. The
pinned source normalizes that prefix; PentaX registers these identities explicitly
without changing ordinary name/version/type resolution. The libc factory now maps separate immutable
code and RW/NX trace-data pages. Its zero-initialized mask and 64-entry state table
belong to each prepared owner and persist across queries and module appends.
The size-32 record retains its size and unknown words; a checked single write
publishes the two guest addresses. Unused argument registers are ignored.
Wrong sizes, unavailable or non-writable backing, partial output spans and
budget exhaustion stop without a fabricated result. Querying never resets the
guest's table and exposes no host address. The existing 13 memory/string handlers
retain their canonical identities and service IDs. Each also has three explicit
scope pairs: prefixed module/prefixed library, prefixed module/canonical library,
and canonical module/prefixed library. These 39 additional GLOBAL/PROTECTED FUNC
symbols share the original markers without adding marker sites.
All four module/library combinations require module 1.1 and library version 1;
unreviewed spellings, versions and types retain ordinary exact-linker failures.
No resolver normalization or additional trace-query alias is introduced.

`__cxa_atexit` additionally registers an actual process-owned callback, argument
and DSO tuple under canonical `LibcInternal` 1.1/library1 and the same four finite
scope pairs. Its SysV integer arguments are three opaque 64-bit words: DSO is
not a byte count. Null values and duplicate registrations are retained in order;
registration neither dereferences nor invokes them and returns int32 zero only
after successful storage. A dedicated callback bound and checked allocation/work
admission refuse without a partial entry or fabricated guest errno. Registries
persist across module loads and stay isolated between prepared owners. The
provider has17 attested sites and67 symbol rows including null. Finalization,
unloading and process-exit callback execution remain separate unsupported work;
registration does not silently run host callbacks or claim completed teardown.

`snprintf` adds the pinned public `eLdDw6l0-bU` FUNC identity in the same four
finite `LibcInternal`/`libSceLibcInternal` scope pairs, module1.1/library1.
Its appended index 16/service 0x50584811 uses the original eight-byte marker at
RVA 0x200; the dynamic table moves to 0x300. Existing marker IDs/RVAs, trace data
and dependency-object addresses remain stable. The 17 canonical functions,
45 finite function aliases, four object rows and null row fit the same image.

The formatting engine reads destination/capacity/format from RDI/RSI/RDX and
consumes the captured RCX/R8/R9 and low64(XMM0..7) through independent cursors.
Both exhausted register classes share the actual callee overflow cursor at
RSP+8, read only as conversions need it. The source wrapper captures all eight
XMM slots without an AL gate. Rust implements the pinned flags, widths,
precisions, length modifiers, integer/binary/pointer and floating converters,
characters, opaque narrow strings and UTF-16-to-UTF-8 strings. Format/string
reads, conversion work, staging capacities and output use finite shared limits;
one complete checked guest write publishes the final NUL-terminated prefix.

This intermediate capability preserves source quirks; complete libc and
graceful C99 truncation remain unsupported. Unknown conversions print their
specifier; `%n` neither consumes a pointer nor writes through one. Source
floating rounding and negative-zero behavior are retained. The source's final
`%s` copy stops at an embedded NUL while its return counts all formatted bytes.
Zero capacity, truncation, incomplete formats, invalid UTF-16 and source-undefined
numeric/high-precision cases produce typed pre-write `UnsupportedFormat`
refusals. Missing required argument capture and guest faults remain explicit.
The engine uses no host `snprintf`, host `va_list` or title-specific patterns.
The [provenance](../../docs/upstream-provenance.md) and [NOTICE](NOTICE) retain
the pinned source identities, Marco Paland/PALANDesign and Martijn Jasperse
attribution, and the complete original MIT grant alongside the GPL terms.

The same provider exposes the canonical `Need_sceLibcInternal` dependency object
under library `LibcInternal`1/module `LibcInternal`1.1 and the four exact
canonical/`libSce` scope pairs supported by pinned Kyty's prefix normalization.
Pinned public Kyty declares
it as mutable `uint32_t` initialized to one. Each guest owner receives its own
four-byte aligned RW/NX storage adjacent to, and separate from, the trace record.
Ordinary object relocation/type checks apply; a function call cannot bind to it.
Lookups and marker attestation preserve guest writes. This object adds no service
or libc function, and no unreviewed name/version alias is inferred. All four
export symbols share one storage object. Its source
contract and generated ELF metadata policy are distinguished in
`docs/upstream-provenance.md`.

Terminal worker diagnostics classify software interrupts without executing them
or resuming a stopped CPU. Source-observed debug breaks remain distinct from
other software interrupts. Only the primary/non-primary source category and
closed keyword hints from at most 256 retained bytes per standard stream enter
the protocol. These hints do not establish the cause of an assertion. Output is
borrowed without consumption; text, immediate trap bytes, PCs and identifiers
are not reflected into diagnostics. The unsupported-stop result remains explicit.

Exact public memory-status APIs additionally retain aggregate call/error counts,
requested byte counts and closed refusal provenance. Actual ENOMEM branches
distinguish physical coverage, virtual-window and fixed-collision failures;
provenance resets for each invocation. No pointer-return API or guest address is
included, and a previous error does not prove an assertion's cause.

The explicit runtime memory profile allows nonfixed mapping searches beyond the
initial 32GiB arena, up to the native user-address ceiling, as required by the
pinned source's hint-as-lower-bound search. The native owner reserves each
missing exact placeholder before publishing a mapping; OS collisions, resource
refusal and failed rollback remain explicit. Fixed mappings retain their initial
window policy. Additional reservation headroom is bounded by the selected host
memory allowance; no pages are
committed merely by selecting this policy. Compact defaults stay unchanged.

The kernel's configured flexible pool is an explicit immutable policy value:
64 MiB in the compact profile and 1 GiB in either runtime backing profile,
alongside the runtime 12800 MiB direct pool. The exact
`KernelConfiguredFlexibleMemorySize` binding writes that value as eight bytes
through the current owned backing; null returns the source kernel int32 EINVAL.
Flexible mappings share the dedicated capacity while retaining independent
aggregate and host admission. Image protection imports, direct aliases and
internal runtime mappings do not consume it. Mapping and unmapping change live
occupancy, never the configured value. Actual backing can still refuse and this
query does not report free or available memory.

Runtime backing/mapped-alias admission derives from a volatile host snapshot.
It retains half of scarce physical availability or a 6 GiB initial margin,
and independently half of scarce commitment or a 2 GiB margin. The result rounds
down to MiB and caps at 14 GiB: pinned 13.5 GiB guest pools plus 512 MiB startup
allowance. Budgets below 1 GiB refuse; image/compiler/GPU costs remain separate.
Actual allocation can still fail as host usage changes. The explicit runtime
profile defers physical-section commitment until mapping; whole section capacity
still consumes its backing allowance and each alias consumes mapped capacity.

`with_file_backed_runtime_memory` explicitly selects private sparse-file direct
sections instead. Logical native backing and mapped capacity use the 14 GiB
ceiling; private image, TLS and anonymous commitment keep the independent
host-derived allowance. The static TLS storage cap also retains that smaller
allowance. The 12800 MiB physical pool and all alias/ownership checks remain
bounded. Preparation validates the existing selected directory and disk capacity
before execution; worker intake rejects overlap with read-only data, saves or
explicit source files. Data-file sections are reported separately from pagefile
commitment. Default runtime and compact policies do not create backing files.

The retained graphics driver owns three video-output ports. Reviewed open/close,
master/slave, flip-rate and output-options/mode APIs retain source bus handles,
busy/error ordering, checked generations and reference counts. Closing a slave
releases its actual master reference; closing a master with live slaves remains
a typed source-aborting operation. Exact64-byte initialization and sequential
four-byte option validation preserve output and source early-error ordering.
Mode119.88 support follows the explicit emulated vblank-frequency policy, not a
host monitor query. Opening/configuring does not imply a vblank tick or image.
Each port retains four attribute groups and sixteen registered video buffers.
RegisterBuffers2 captures exact80-byte attributes,32-byte descriptors and both
SysV stack arguments before publishing any group or buffer. All seven reviewed
pixel formats, source64KiB render-target footprints and two DCC control profiles
retain checked40-bit source ranges, alignment, options and metadata footprints.
ChangeBufferAttribute2 validates every affected buffer before replacement;
unregister and close release the actual group's surfaces. Reserved descriptor
words remain ignored. Registered geometry grants no memory-access authority;
future pixel consumers must revalidate live backing. Flip/event delivery,
drawing and presentation remain work in the same retained port lifetime.
Fifty-two exact VideoOut/libSceVideoOut entries share the generated
106-entry graphics provider with submit/boundary and AGC startup APIs. Original marker IDs/RVAs and one immutable RX
page stay unchanged; metadata fits an8KiB file under checked layout
bounds. Mixed module/library spellings remain explicit; versions stay exact.

VideoOutGetOutputStatus writes the exact48-byte source record from the retained
port mode and explicit display policy. Caller-admitted ATTRIBUTE3 gates resolution
detection against the configured Resolution; absent metadata uses the pinned
zero fallback. Default guest refresh is59.94, with119.88 reported for the selected
mode or the explicit follow-vblank policy at119Hz or above. The complete record,
including zero flags/reserved words, publishes atomically after a null-output
check before handle validation. Reporting a display mode is not a presented-frame
or timing measurement. No host monitor inference or implicit metadata read occurs.

VideoOutSetBufferAttribute2 captures both actual SysV stack arguments before
publishing its complete80-byte record, including zeroed reserved/padding fields.
Register dimensions/control use their source32-bit widths; format/option/clear
color retain all64bits. Output may alias the captured argument words. Source
null, missing/invalid stack data, output/backing and work refusal stay typed.
VideoOutAddBufferAttributeOption reads/updates only its8-byte option field,
preserving every neighboring byte and allowing that exact field at a mapping
boundary. Both functions are source void calls; the unspecified transport
register value is never treated as a guest success/status code.

Dispatch stops expose closed failure classes and the planned exit mnemonic.
The latter describes the translated block exit, not an inferred faulting body
instruction. Addresses, error payloads, unknown labels and Debug text are never
reflected. Diagnostic observation does not resume or modify a stopped CPU.
Terminal memory denial additionally reports closed CPU/GPU access categories
and a bounded source-role classification, using permission/metadata checks only.
The planned encoding/destination category and whether a native body returned
remain observations, not a guessed faulting instruction or an access grant.

Kernel protection can track already loaded input-image pages, including RELRO
and zero-fill pages, after bounded full-span source-plan and committed-owner
checks. It never discovers arbitrary host regions or grants provider/runtime
storage authority. Logical CPU/GPU rights retain the actual request; host
backing remains NX and native copied-source guards still apply. Kernel metadata
and resource accounting publish only after the native transaction succeeds.
Closed protection failure labels distinguish untracked ranges, overflow and
runtime-owned mappings without exposing call arguments or addresses.

AgcInit follows the pinned library contract: it returns zero for every version
without reading or writing the state pointer. It consumes the shared call/work
budget and retains poison refusal. This API does not initialize the renderer,
change queues or signal that a frame can be drawn. Exact Graphics5/Graphics5 and
libSceAgc/libSceAgc module1.1/library1 registrations share the graphics provider;
all prior marker IDs/RVAs and the single immutable guest page stay unchanged.

AgcGetRegisterDefaults2 and its Internal variant return real per-owner guest
storage for all source versions0..13; larger uint32 versions use13 and argument
high bits are discarded. The exact64-byte descriptors retain distinct version
identities and pointer tables while sharing underlying register/type arrays as
the pinned source does. The121 unique arrays, three-word type records and
reviewed derived version patches are Rust data. Guest edits persist across
repeat calls and shared arrays; separate owners remain isolated. No host
pointer is returned. One89,880-byte RW/NX blob occupies six guest pages under
kernel handle/byte/work/scratch and shared graphics call/work admission. Failed
initialization/backing publication creates no cached pointer or mapping record.
Four exact scoped registrations share the106-entry provider; checked metadata
tables retain one immutable source page and all prior IDs/RVAs.
These register defaults provide data for startup, not a rendered-frame claim.

AgcGetIsTrinityMode writes exactly one zero byte, following the pinned base
console profile. Source void transport values are not status codes. Neighbor
bytes are preserved; invalid/unwritable output, shared limits and poison remain
typed refusals. Two exact namespace registrations extend the source file to8KiB
while retaining the same logical16KiB RX guest page and existing marker slots.

The four AGC owner/resource registration and unregister APIs return the pinned
source int32 error0x8a6c9018 without guest access or resource creation. They retain
shared admission and poison refusal. Exact Graphics5/libSceAgc and the separately
reviewed AgcDriver/libSceAgcDriver registrations share the provider;
these driver spellings are distinct from Graphics5Driver. This is source failure
behavior, not an implemented resource registry. Metadata fits the8KiB source file;
existing marker IDs/RVAs and the single logical RX page remain unchanged.

AgcCreateShader performs actual source-ordered in-place relative-pointer
relocation, including negative displacements, and patches the appropriate
program-address register pair. Exact field reads preserve mapping-boundary
headers without requiring unused padding. Same-address overlapping outputs see
earlier staged effects. The exact source program-error result retains relocated
header/user-data and shader-map effects while leaving the destination untouched.
Memory, work, allocation and source-aborting failures publish no partial output
or retained mapping. Distinct virtual views aliasing the same edited physical
bytes are explicit typed refusals, never guessed overlay reads.

Shader metadata is retained per owner with bounded capacity and generations;
replacement preserves one code-address identity. User-data/semantic pointers,
counts, scratch size, type and declared code size remain guest data rather than
memory authority. Existing compute dispatch consumes matching registered types
and nonzero code extents before compilation and GPU execution; unspecified zero
size retains the original bounded decoder policy. Later GPU consumers still
revalidate actual live backing and access. Read-only metadata queries expose no
host pointer. Creation does not claim general graphics translation or rendering.
Two exact Graphics5/libSceAgc shader-creation registrations share the106-entry provider in
the same8KiB file and one immutable guest page, preserving previous IDs/RVAs.

Four graphics event APIs register/delete actual retained kernel subscriptions
and read exact event type/context fields. Source Graphics5Driver and explicitly
reviewed AgcDriver/libSceAgcDriver scopes retain exact versions. Kernel GRAPHICS
filter-14 records keep source flags0, saturating uint32 interrupt counts and the
newest context; delivery always resets them. Repeated registration retains source
pending state and updates userdata. Closing a queue cannot leave a live producer.

Admitted RELEASE_MEM packets retire after preceding Vulkan fences/writeback.
Optional labels publish before their interrupts; pure interrupts also require a
real admitted command. An exclusive prepared kernel delivery admits all targets,
allocation/work and sequence capacity before the label store. Failed stores
discard it without a notification. Reference-clock labels use the source's
checked host-TSC scaling to100MHz. Successful prefixes across multiple buffers
remain terminal after a later refusal, preventing replay or duplicate events.
General GDS releases and full graphics/presentation compatibility remain work.

The synchronous nonindexed draw path additionally accepts one declared32bpp
RenderTarget64KB target: exact2D swizzle0x1b, zero metadata/compression flags,
64KiB base alignment and128-pixel padded pitch/height. The current backend still
requires visible widths divisible by64. Core reuses VideoOut's pixel mapping
to detile the actual full LOAD snapshot, executes the real guest VS/PS in Vulkan,
waits its fence and overlays active output pixels back into the original padded
snapshot. Invisible bytes remain unchanged. Whole padded permissions and physical
disjointness from command/shader sources are checked on the same paused owner;
host storage budgets and reverse work are admitted before GPU effects. Atomic
writeback then produces a private receipt with actual layout/geometry/range.
This RGBA8_UNORM byte receipt does not perform VideoOut sRGB transfer conversion,
acknowledge a flip, emit GPU-EOP or establish a rendered game frame.

The draw adapter additionally admits the GPU preparation layer's optional
read-only direct buffer for each vertex and pixel stage. These are immutable
copies from the actual current paused owner, discovered from live shader
requirements rather than inferred from arbitrary register contents. Complete
resource extents and GPU-read permissions precede backend admission. The target
must have distinct physical backing from both resources, including aliases at
different guest addresses; read/read aliases between stages, shaders, indices
and command sources remain legal. Each stage upload is charged independently
under the existing host and pixel transport ceilings, even when its backing is
shared. Preparation runs at the draw's current ordered command point, so earlier
retired writes are visible. No writable buffer, ambient host pointer, replacement
shader result or commercial frame is authorized by this resource metadata.

The ordered graphics driver separately admits the exact six-dword R_FLIP packet
`0xc004105c`. A fresh fenced tiled draw retains its exclusive native pause until
the next flip consumes it. Matching the live registered surface, owner, port
generation and full geometry precedes owned capture and GPU-ready publication.
Multiple Draw/Flip cycles retain order; display count, vblank and events do not
advance. Old reports and numeric receipt fields cannot authorize this path.
Queue and capture quotas are admitted before GPU work. A failed new capture
removes its reservation; effects already retired remain terminal and cannot
replay. Full queues stop explicitly until real consumer wake is implemented.

`AgcDcbSetFlip` appends two exact exports: canonical Graphics5 and demonstrated
libSceAgc module/library pairs, each module1.1/library1. The five-GP source ABI
writes six packet words and advances the source56-byte command buffer's cursor.
The finite fit path reads only cursor/reservation fields and atomically publishes
the24-byte packet plus eight-byte cursor. Growth/callback, malformed spans and
descriptor/packet physical aliases refuse before effects. All106 prior marker
IDs/RVAs remain fixed; the provider now has108 exports within its existing8KiB
file bound. This is command encoding and real GPU-ready capture, not desktop
transport or an actual presentation acknowledgement.
The provider keeps its8KiB file and one RX guest page, with dynamic metadata at
0x7c0 and string/symbol tables at0x900/0x1100. These positions derive from the
fixed export count and exact name bytes under the same image bound; appending
reviewed exports never relocates existing marker IDs/RVAs.

Fused shader sizing and both shader-half fusion exports additionally retain
source GS/HS pairing, uint8 register counts and exact16-byte SizeAlign output.
Fusion copies the back header, sets the combined stage, validates the source
stage bit and copies registers to optional caller scratch. It transfers both
checksum occurrences, merges the exact source resource fields and patches the
optional front-program address pair while preserving high-word flags. The newer
export recomputes shared VGPRs and clears userdata; the legacy export takes the
maximum shared field and retains front userdata. The source stage-bit error
preserves the copied header/type; hard memory/work/alias refusals publish no
partial state. Same-address overlays retain source order, including in-place
back headers. Distinct physical aliases remain explicit typed refusals.
Fixed staging covers96 header bytes plus255 eight-byte registers. Adjacent
final byte edits publish as contiguous runs, preserving all checks and avoiding
quadratic transaction work over thousands of one-byte outputs. Six exact
Graphics5/libSceAgc registrations share the same provider and resource limits.
This prepares retained shader data; it does not execute GS/HS or present frames.

Both interpolant-mapping variants match PS input semantic tags against the first
matching GS output and emit exact SPI_PS_INPUT_CNTL records. Missing/empty PS
uses32 identity records without accessing unused GS data. Matching and missing
outputs retain each variant's distinct F16 modes, low/high defaults, flat/custom
flags and five-bit hardware mapping, then append the source identity tail.
Counts above32 retain source behavior under a finite256-record staging bound;
larger or subsequently rewritten counts refuse explicitly, without clamping.
Guest counts/pointers and semantic records are reread through ordered overlays,
so overlapping output/source data observes previous writes. Hard late input,
output, alias or work failures publish nothing. Four exact registrations use the
same source namespaces. Synthetic primary-source C++ oracles cover32768 scalar
cases per variant; original native consumers cover real imported calls. The
reference oracle is development evidence; production and fixtures remain Rust.

The separate JSON provider implements the observed allocator-constructor
first-word write. Five exact module/library identities resolve through the
ordinary linker. Its generated image is bound to one guest placement: a code
page contains constructor and private terminal markers, and a separate read-only,
non-executable page contains two zero words followed by six guest terminal
addresses. The terminal marker has no public export. Dispatch rechecks current
source/factory identity, full service ID, owner, continuation, immutable
permissions and live table bytes before a bounded whole-span eight-byte write.
Rejected pointers, stale/tampered providers and exhausted work leave the object
unchanged. Calling a virtual slot stops with a typed unavailable result; no
allocator, complete C++ layout or destructor ABI is inferred. Legacy explicit
startup builders preserve their provider set; `KernelStartupExtensions` admits
JSON explicitly and the supplied-image runtime includes it by default.

The same provider separately implements Kyty's observed Null-value constructor:
a 32-byte object with parent/root-parameter words, an eight-byte value union,
four padding bytes and a uint32 type at offset28 is initialized entirely to zero.
The constructor returns self and explicitly accepts null. Both public constructor
NIDs have four finite canonical/prefixed scope pairs; only the independently
observed `qBMjqyBn3OM` adds the distinct `libSceJson` module / `libSceJson2`
library pair. Each export binds its own exact marker through the private symbol
manifest. Whole-span admission precedes the 32-byte write, including unaligned
destinations; failed boundary, permission or budget checks preserve all bytes.
This representation follows Kyty's explicit size/offset assertions, without
using AnyPS5's incompatible host-owned Node representation. Strings and dynamic
Array/Object values use the process-owned storage described below.

The JSON primitive batch adds boolean/integer/type construction, scalar setters,
reference getters, copy/assignment, cleanup, count and allocator destruction.
The provider has61 private markers and224 exact export rows. Existing rows and
marker addresses remain stable; metadata moves within its original code page.
Three separate read-only zero words back the boolean, integer and real default
references. GetBoolean/GetInteger/GetReal return actual guest addresses; GetReal
does not return floating-point bits in RAX. Double construction and assignment
consume the low64 raw bits of captured XMM0. The native stop token binds a full
XMM0..7 snapshot to its existing completion identity, and the worker forwards
that snapshot. Missing vector capture is an explicit typed failure; NaNs,
infinities and signed zero keep their original bits without host arithmetic.

Fixed staging follows source order for overlapping copies, including both
source-type reads and initialization before a copy-constructor's self check.
Clear/set/assignment preserve four padding bytes, while constructors initialize
all32bytes; the explicit Clear method also preserves parent/root fields. Queries
read only their needed fields. Whole transactions admit all writes, ranges,
physical-alias checks and finite work before effects. Owned Strings use one
bounded process owner across their constructor, assignment, CStr/length queries,
destruction and Value copy/set/clear paths. CStr returns actual guest bytes;
String defaults have a separate immutable empty wrapper and NUL byte. Scalar
and String formatting preserves the reviewed source profile. Real formatting
uses six fractional digits, including source nonfinite spellings. Arrays and
objects serialize their actual owned children with the source's raw string/key
quote wrapping. Foreign or stale container storage refuses explicitly.
Calls keep constant-work source validation and
live marker/default attestation; full binding validation occurs at plan/epoch
publication instead of scanning unrelated relocations for every scalar call.

The appended Initializer constructor returns the complete opaque `self` word
unchanged, including null, exactly as the pinned public body specifies. It reads
or writes no guest object bytes and does not initialize an allocator. Only the
source `Json2` module/library pair and the independently observed `libSceJson`
module / `libSceJson2` library pair are exported for this NID, at module1.1/lib1.
The constructor does not stand in for the separate supported default-engine
initialization profile described below.

String growth publishes its new allocation and all caller fields atomically.
Impl identities remain stable during assignment, while Value copies own
independent storage. Retired addresses are quarantined and remain accounted;
stale tokens cannot become live objects again. Unsupported address-dependent
partial-wrapper overlap refuses before effects, rather than inventing pointer
bytes. Existing ordered Value overlap semantics remain unchanged.

Twenty-two appended functions provide owning Object/Array construction,
deep copy, destruction, indexed access, Array push-back and iterators, Value
container conversions and the sized parser. Each NID adds only its source
`Json2` pair and its separately observed `libSceJson` / `libSceJson2` pair at
module1.1/library1. Every earlier marker, service ID, export ordinal and immutable
default address remains stable. Iterator positions refer to real guest pointer
cells and become invalid after the underlying Array table grows or is destroyed.
Copies own independent children; SetObject snapshots its source before clearing
the destination, including copies from its own descendants.

The parser uses serde_json's mature lexer with fallible custom deserialize
seeds rather than an unchecked library DOM. It preserves integer/unsigned/real
types, lexical integer `-0`, signed real zero, Unicode strings, last duplicate
members and bytewise sorted object keys. Malformed input returns the source
parse error without changing the destination. Explicit policies bound input
and individual text to256 KiB, depth to64, structural nodes and retained
container records to4096, container metadata to1 MiB and simultaneous retained
plus staged host storage to64 MiB. Actual custom-buffer capacities are counted;
a conservative `4 * input + 4096` allowance covers the pinned library's internal
scratch before parsing. Its internal allocator still has Rust's infallible OOM
boundary; the envelope is not a recoverable host-OOM guarantee. All variable
work, including failed staging attempts, remains charged.

Children, keys and iterator cells live in bounded guest mappings, with a shared
64 MiB JSON guest-storage cap and the kernel's mapping/handle quotas. Mutations
publish the new blob and caller fields in one transaction, then commit private
metadata without further fallible work. Failed reads, writes, ownership checks,
quotas or staging leave previous bytes and ownership intact. Retired mappings
remain quarantined and accounted until process teardown. This container/parser
support does not initialize an external allocator or invoke user callbacks.

The separately bound InitializeV1 admits actual process-owned Null, empty Array
and empty Object storage before returning zero. It does not dereference its
opaque self/parameter arguments or infer an allocator configuration. When the
defaults already exist, including every healthy repeat, initialization checks
their actual current64-byte backing, RW/NX permissions, exact live empty
container identities, implementation words and Null bytes without allocating
another mapping. Changed bytes, permissions or container contents refuse;
initialization does not silently repair them. Readiness commits only after the
first atomic publication or a complete successful live validation. Work/quota
and late publication failures leave prior ownership and bytes intact.

The complete pinned Terminate and Initializer destructor bodies have no object
effects, so they preserve process-owned defaults and live Values. Null-access
callback setup retains one owner-bound inline receipt containing the complete
opaque64-bit callback/context words; null clears it and later setup replaces it.
Fixed work admission precedes mutation. Setup ignores self and neither reads nor
executes callback/context, allocates guest storage or consumes a handle. Low
work and foreign-owner refusal preserve the previous receipt. Terminate and
Dtor preserve this process-owned registration, whose lifetime ends with the
process owner. Event delivery and callback invocation remain unimplemented.
These four roles append
only their canonical and independently observed exact scope pairs; the distinct
Initialize ABI remains unavailable. Neither a complete allocator lifecycle nor
official firmware callback behavior is claimed.

Seven additional Pad runtime markers append only the exact libScePad module1.1 /
libScePad library1 profile. Existing sites, ordinals and object RVAs are retained.
The kernel owns checked input snapshots and desired device requests; its
disconnected default does not imply a physical controller. Actual native
backing/current-rights checks cover full2/3/8/28/120-byte spans, and a failed
PadRead output preserves queued input. The kernel's explicit owner API is the
future device boundary; this provider adds no desktop/game-input bridge,
motion fusion or hardware acknowledgement. See pentax-kernel/NOTICE.


The keyboard-type service appends one libSceConvertKeycode module1.0/library1
marker and its exact module/library descriptors. Existing function markers,
object RVAs, 2048-function ceiling and image/mapped metadata bounds remain
unchanged. Original consumers cover qualified linking, current whole-span
write rights, cross-page refusal, and forged-site rejection.

### Prepared fiber continuations

Fiber actions bind the current source epoch and memory owner to actual opaque
executor contexts. Fresh entries require initialized immutable RX source, exact
RW/NX stack bounds and a checked native return marker. Saved guest context bytes
cannot select host state. Run roots belong to the currently selected pthread;
idle fibers can migrate without replacing that thread's TLS or errno identity.
Actual root-entry RET is distinguished from nested HLE callback return by a
sealed entry/call/stack origin. Only the actual entry return terminates a fiber.

Provided contexts keep their exact bounds. A contextless fiber lazily receives a
256 KiB RW/NX stack with unmapped guards in the explicitly configured reserved
thread-storage capability. One bounded stack slot is retained per initialized
object address and reused by later generations. This avoids the reference
implementation's environment-dependent small-stack host workaround. Current
stack/object and pthread/fiber physical-backing separation is rechecked before
selection; read/write permissions alone cannot authorize a remapped alias.

The worker exchanges CPU contexts without executing code, then commits atomic
ABI/results before destination entry. A failure after exchange stops execution
terminally; it is not a completed or unchanged service return. Suspended HLE
callbacks retain their existing thread/TLS bindings: their cross-thread migration
is explicitly unsupported. Ordinary idle fiber migration remains supported.
These source and original-fixture contracts do not establish a game frame.

### Cooperative pthread join

The prepared owner services the existing raw-kernel `PthreadJoin` role through
its normal cooperative wait queue. A sole serial-bound claim identifies the
published created-thread handle and current waiter. Readiness requires the
actual authenticated thread return after exit cleanup. An already completed
thread is immediately ready; an active child lets other runnable threads make
progress. The optional result receives the complete 64-bit exit value only when
ready. A failed output write leaves the target unjoined and its claim intact;
source errno results do not modify the output.

Null, self/deadlocked, non-owned/reaped, detached and competing-claim cases keep
the source kernel error domain. Creation attributes provide detached state; no
new Detach or POSIX alias is inferred. Successful join consumes the claim once.
Stacks, TLS, context slots and identity records retain the existing bounded
owner-lifetime allocation policy until teardown. This deliberate policy does
not claim the reference implementation's immediate `FreeGuestStack` reclamation
or authorize reuse of a joined context. Original native fixtures independently
exercise migrated fibers, a real sleeping-child join and failure after RET.

### Prepared pthread detachment

Detach resolves the selected owner's actual registered handle, including the
initial thread, before changing retained lifecycle state. Created-thread
publication, completed Join state and sole Join claims remain authoritative;
detachment cannot cancel an accepted waiter or manufacture a return. Source
validation and work admission precede infallible kernel/created-record updates.
No guest output, errno cell, host pthread or backing mapping is modified by
detach. Real entry execution and cleanup continue under the existing scheduler,
with bounded stack/TLS/context retention until owner teardown.

VideoOut's canonical vblank status and color-setting exports retain owned
40-byte status and four-byte gamma semantics. The captured SysV XMM0 argument
is independent of GPR values. A per-open display epoch shares the validated
kernel process clock and explicit display cadence. Paused scheduling can deliver
coalesced vblank events before the first pixels, while mode changes rebase the
same grid atomically. Numeric consumer ticks do not create additional periods.
This display count remains separate from real draw, frame and acknowledgement
evidence. Detailed source/error/ownership scope is in src/gpu/video/README.md.

The exact `AgcAcbResetQueue` and `AgcDcbResetQueue` encoders reserve two real
command DWORDs through the existing checked descriptor and grow continuation.
Their returned address identifies the emitted packet; encoding does not reset
host, display or other guest queue state. The canonical Graphics5 pair and the
separately verified paired libSceAgc identities are explicit, with old export
ordinals retained. Invalid arguments/allocation failures remain typed refusals.
On submission, reset effects occur in that processor's captured command order.
ACB reset discards prior modeled register authority and restores only mapped
public defaults; DCB clears context state while preserving SH, uconfig and
independent index/instance state. Reset invalidates an earlier draw's flip
eligibility. These contracts do not establish commercial rendering or speed.

The four AGC AcquireMem/EventWrite builders preserve the pinned209 producer ABI,
including DCB's seventh stack argument, byte narrowing, truncated encoded cache
coordinates and actual grow callbacks. Exact canonical and observed paired AGC
bindings are appended after the existing graphics markers. Emission alone has no
GPU effect. Submitted tagged AcquireMem and supported partial/cache-flush events
become explicit, bounded ordered synchronization actions. Each previous actual
draw/dispatch has already completed its fence and owned-memory writeback before
this receiver retires the action. The receiver does not dereference encoded cache
coordinates, fabricate cache state, signal a fence/event or create draw readiness.
Unsupported query/ignored events remain explicit refusals. An original ignored
Vulkan/window fixture covers one encoded draw, synchronization, flip, real
presentation and owned acknowledgement; it requires separate hardware execution.

The four exact AGC PushMarker/PopMarker bindings encode source diagnostic
annotations through the existing owner-bound CommandBuffer/grow path. Push
captures the first terminating byte's length before allocation, zeroes the full
rounded destination after the header, then copies that captured length from the
current post-callback source. It neither snapshots text before growth nor scans
again afterward. Null buffers and source-defined allocation failures return zero;
late memory faults retain the actual prefix and stop further submission. Text
length, staging, work and output remain bounded by existing limits. Color is
ignored as in the pinned producer. SetMarker aliases remain unavailable.

Submitted exact Push/Pop packets are explicit bounded annotations in static,
captured and live ordered plans. Their binary payload is never interpreted as a
host C string or logged. They do not change modeled GPU state, synchronize work,
signal events or create readiness; they retain only an already current draw's
eligibility. This source support makes no claim about a commercial game frame.
The DCB ContextStateOp builder preserves each source reservation and fragmented
allocation, including Pop's first packet before ReserveDW(22). A real grow
callback can move the current descriptor; the first actual packet address is
retained independently. Each allocated segment publishes its cursor, a checked
whole-segment zero, then its header/operation. Allocation failure returns null
with earlier prefixes retained; late memory failure makes the driver
nonreplayable. Encoding alone does not change the owned graphics context.
Submitted context history saves only the current CONTEXT bank and uses existing
owner, packet, host/work and target preparation checks.

CreatePrimState computes two optional CONTEXT pairs and three optional UCONFIG
pairs from current owned shader metadata. It preserves the source's optional
hull shader, original geometry-stage output choice before the hull-stage merge,
full primitive argument and default primitive mapping. Both-null outputs perform
the defined empty operation before shader access. The helper adds no shader-map,
header/version, resource or primitive-domain restriction absent from the source.

Field access follows source order through the existing bounded shader Editor,
including fresh specials-pointer reads after same-address output aliases. This
checked profile publishes outputs atomically, so late memory, tag or work refusal
leaves staged prefixes unpublished. Distinct virtual physical aliases are refused
explicitly. Successful output arrays remain CPU data until actual register
encoding/submission; original consumers carry them through real register builders
and original shader/target preparation without claiming hardware execution.

`AgcGetDataPacketPayloadAddress` computes the source-defined packet address as
plain guest data. Its nonzero low32 type branch does not access the command;
type zero reads exactly the current four-byte header before one checked eight-
byte output, including overlapping input/output. Unknown packet tags are not
filtered by this metadata helper. Unmapped returned pointers confer no access
or GPU authority. Null output, arithmetic overflow, permissions and existing
work/write limits remain typed refusals with atomic output. Only its canonical
identity and observed paired alias are registered; the unobserved Range helper
remains unavailable. Original fixtures cover current headers, same-address and
physical aliases, exact page edges and output/budget refusal boundaries.

The exact `AgcDcbSetIndexSize_0600` four-argument variant is registered under its
canonical and observed paired identities. It reuses the existing real three-
DWORD allocator/grow path, retains the low2/low2/low1 argument masks, and emits
the source bit14 alongside the established index-size packet. Null buffers and
allocation failure return zero; actual cursor/header publication and late
memory failures retain the existing ordered failure boundary. Earlier export
rows and the original index-size builder remain unchanged. Original consumers
take the actual packet through owned indexed shader/resource preparation;
no physical GPU result or commercial image is claimed by this helper.

CopyData/DmaData builders preserve exact four-role argument narrowing, packet
words, actual grow calls and captured outgoing stack arguments. Null buffers and
source-defined allocation failures return null before transfer memory access.
The submitted subset supports aligned disjoint owned-memory copies, four-byte
COPY immediates and whole-word DMA fills. Unsupported nonzero selectors, partial
words, clock, GDS, register, query and discard paths refuse explicitly.

Execution uses one preadmitted staging vector sized to the largest transfer;
aggregate write bytes remain separately bounded. Every action rechecks current
GPU-read/write permissions, full destination backing uniqueness, source/output
physical disjointness and every captured command-source exclusion. Source bytes
are read at their ordered turn after earlier actual fenced writeback. One
checked whole-output transaction follows; a late failure retains earlier effects
and cannot replay them. Zero-byte DMA validates the encoded engine then touches
no address and creates no bytes, fence, event or completion. Transfer actions
consume prior graphics draw eligibility, including this conservative zero case.
Original ordinary consumers and one separately ignored Vulkan consumer are
provided; their presence alone establishes no physical execution or game image.
Three additional observed producers encode ACB absolute dispatch arguments,
DCB base-relative dispatch offsets, and the DCB indirect-argument base. Their
source masks and real allocator/grow results are preserved; emission neither
reads dimensions nor grants memory authority. The existing compute receiver
reads the current owned twelve-byte dimensions at the ordered dispatch turn,
after earlier completed writes, and retains its wave32, permission, alias and
work limits. Graphics indirect drawing and unobserved size exports remain
outside the admitted profile. Original consumers cover actual emitted streams,
zero-group retirement, changed permissions, fresh dimensions and grow migration.
The observed single indexed-indirect draw builder now connects to the existing
owned graphics path through source-ordered live twenty-byte argument reads.
Actual count/clamp, instance count, first index and ordered user-SGPR patches
are retained, including zero-work state changes. Native shaders consume signed
base-vertex and first-instance values in those patches; Vulkan offsets remain
zero by the proven native-fetch convention. This does not enable embedded-fetch
rewriting, unsupported SGPR destinations, index8, or DrawParameters-dependent
relative-instance shaders. Original ordinary and separately ignored physical
GPU consumers are source fixtures; their presence is not a rendering result.
