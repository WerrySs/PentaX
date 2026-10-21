# Architecture and milestones

PentaX targets PS5 execution, rather than wrapping another emulator. All first-party implementation is Rust. Native OS APIs and independently licensed libraries may be used at narrowly defined boundaries. A source translation of another project is still an adaptation and must retain its licensing obligations; research references are not permission to relicense code.

## Boundaries

| Component | Responsibility | Current state |
|---|---|---|
| `pentax-loader` | Validate executable metadata and locate guest bytes | ELF/SELF inspection, strict parsing, lazy payloads, runtime load plans and bounded dynamic tables |
| `pentax-linker` | Inspect linking records and plan relocations | Bounded ELF64/classic and observed SCE descriptors, NID selectors, exact scoped providers and image-bound session relocation plans; no memory writes |
| `pentax-core` | Guest address space, image mapping, CPU execution contracts | Shared sparse/native staging, typed TLS, owner-bound source provenance, checked PRE-SYSCALL services and reference interpreter |
| `pentax-kernel` | Bounded stateful kernel services | Qualified registrations, checked guest access, typed handles and transactional outputs; explicit unsupported operations |
| `pentax-gpu` | Validate bounded command input independently of rendering | Owned flat packet inspection, encoded register-write intents and terminal indirect-buffer observations; no GPU state or execution |
| `pentax-shader` | Decode guest shader syntax before compilation | Complete observed linear-span frontend, including vector/memory/image metadata; no CFG, IR, SPIR-V or execution |
| `pentax-shader-compiler` | Validate supported programs/resources and emit SPIR-V | Checked control flow and defined-register use, bounded IR, resource/feature manifests and original validated outputs |
| `pentax-codegen` | Plan same-ISA CPU blocks and mediated exits | Source-bound integer/legacy-SSE encoding, checked RIP-relative targets and typed transfer exits; no executable allocation or dispatcher |
| `pentax-config` | Declarative preferences and profile inheritance | Versioned TOML, validation and atomic persistence |
| `pentax-cli` | Reproducible inspection, finite diagnostics and explicit interactive sessions | Bounded progress streaming and owned native worker supervision |
| `pentax-desktop` | Library, preferences and sessions for mouse, keyboard and controller | Supervised Play/Stop launch path; commercial boot and playability unverified |
| `pentax-platform` | Audited native memory, ABI and Vulkan boundary | Windows x86-64 NX guest owners, reusable unsafe one-block context/cache bridge and closed compiler-to-Vulkan diagnostics |
| `pentax-worker` | Separate native fault-isolation process | Named original diagnostics; reusable prepared-owner loop verified with an original kernel/TLS program in debug and release |
| `pentax-xtask` | Build-time distribution tools | Strict third-party notice rendering without machine paths |

The portable core never casts guest addresses into host pointers. Memory ranges, resident allocation and instruction work have explicit bounds. The platform crate owns its host allocations behind a reviewed boundary. Portable crates forbid unsafe code; the dedicated worker denies it except in its separately reviewed native-session module. That module propagates the platform's explicit unsafe ordinary-memory obligation. Unsupported formats, instructions and services must produce actionable failures; they must not silently report success.

## CPU and operating system

PS5 software contains x86-64 machine code. The current native backend targets Windows x86-64, using an audited context bridge, owned code cache and mediated transfers. Linux native execution and a complete console startup/TLS ABI remain future work. The safe interpreter is a reference for small fixtures, not a proposed way to reach game performance. Native guest execution is not a sandbox; a separate worker limits fault impact but does not confine arbitrary memory operands.

The closed `process::startup::run_startup_fixture()` takes no caller images, paths, plans or entry points. It generates two original modules, uses the same symbolic planner and owned staging, and interprets a relocated provider call with its own guarded RW/NX stack, argument object, constructor and synthetic service. Modeled TLS bytes come from the bound staged `PT_TLS` template, preserve its alignment bias and seed two independent instances. The modeled FS base is a guest integer; host FS/GS is untouched. Complete argument, provider, service, constructor, stack, guard and TLS postconditions must pass within one shared instruction budget before diagnostics escape. `UnstartedSession` gains no conversion or mutable accessor. This is PentaX fixture policy, not a PS5 TCB/DTV or a complete title-entry ABI.

Runtime planning retains `PT_SCE_MODULE_PARAM` separately from process parameters and requires initialized logical backing without a separate copy or mapping. The observed tag has public producer evidence; contents, revisions, magic and platform-wide sizes remain uninterpreted. Unknown program headers and strict payload policies retain their explicit rejection behavior.

The original leaf native diagnostic passes arguments through the SysV ABI, publishes owned executable pages, and contains an illegal-instruction fault in its worker process. Its syscall case is rejected by a closed fixture policy before allocation; this does not implement general syscall interception.

The separate `guest-stack-entry42` worker fixture ports the pinned Kyty entry bridge to an owned fully committed RW/NX stack with inaccessible guards, a fixed original argument object and private RX code. It checks entry alignment, arguments, root frame, a guest red-zone access and one SysV callback. The bridge preserves Windows nonvolatile GPRs, XMM6-15, MXCSR/x87 control state and the host TEB stack bounds on normal return. Recovery relies on this closed fixture preserving its SysV callee-saved registers; it supplies no arbitrary entry capability, general stack-switch unwinding, guest TLS or complete Windows red-zone guarantee. Source and modifications are recorded in [upstream provenance](upstream-provenance.md).

A separate original fixture attests its complete immutable bytes, replaces its declared `SYSCALL` with `UD2` before RX publication, and registers an exact thread/site/service exception handler. The handler records one synthetic sum service and resumes the leaf with RAX=42. It has no allocation, locking or panic path; TLS is cleared before unregistering. This establishes a closed diagnostic only, with no PS5 syscall, errno, startup or red-zone contract.

Windows vectored exception handlers observe exceptions; a valid `SYSCALL` does not have to raise one. General Windows guest execution therefore needs validated same-ISA basic-block translation with non-executable source pages and mediated guest transfers. A list of patched sites does not cover arbitrary indirect entry or newly generated code.

`pentax-codegen::plan_block` retains the exact immutable symbolic plan and selected module, copies initialized executable bytes, applies retained ordinary source writes and refuses every deferred record. A positive integer/legacy-SSE policy gates forms and declared CPUID features. Re-encoding preserves original RIP-relative guest targets or fails; all CALL/JMP/Jcc/RET/SYSCALL and unported semantics terminate at a privately constructed UD2 with typed original metadata. Cache intervals are checked against all guest mapping pages, and source, instruction, output, allocation-capacity and work bounds are explicit. These remain pure immutable plans with no executable allocation or execution authority. The distinct materialized path requires a full source-bound declaration of supported deferred writes and exact staged-byte agreement; its `CallerDeclared` assurance proves neither typed TLS values nor actual application. See the [code generation contract](../crates/pentax-codegen/README.md).

`pentax_platform::native_dispatch::NativeBlockStepper` retains that exact materialized source, a stable native owner identity, guest-only context, private cache and cumulative limits. Its explicit unsafe `step` borrows actual paused memory for one block. It checks live initialized immutable executable bytes, host features, stack/guard geometry and separately attested native-tail emission before entering the existing bridge. Transfers resolve against checked owned memory; CALL/RET commit their mediated stack effects only after validation. PRE-SYSCALL stops preserve full RAX, the callee stack pointer and all six original SysV register arguments, including RCX. Pending completion is bound to an opaque executor/sequence token. Post-execution failures are terminal. The manual `step` retains the caller's ordinary-memory obligation. `step_checked`, used by the prepared worker loop, isolates each memory instruction, derives its full explicit/implicit footprints from observed state and checks live spans and permissions before entry. Unmodeled footprints fail explicitly. Source/TLS provenance remains independently required, and this is not a security sandbox.

The separate native-memory owner reserves exact guest virtual addresses without replacing existing host allocations. Windows reservations follow host allocation granularity, while committed logical pages remain 16 KiB; reservation holes stay inaccessible. Prepared RX source maps to physical read-only NX memory, and logical execute permission never grants host execution. Mapping/protection failures roll back unpublished OS changes or poison the owner if rollback fails. Shared internal staging applies the same checked copies, relocations and final RELRO policy as sparse memory. Native eager commitment charges every mapped page against the resident limit. An exclusive non-Send/non-Sync paused borrow supplies checked whole-span operations; HLE and kernel adapters use this actual backing rather than detached sparse copies. Ordinary native/HLE preparation still refuses TLS and deferred records. The opt-in kernel-startup path separately composes typed static TLS and checked service requests. Only the unsafe platform stepper publishes admitted source into its separate executable cache; the storage owner exposes no raw pointer or execution conversion.

An image-bound relative relocation plan checks logical target coverage, composition, overlap and bias arithmetic. The core applies its writes after payload staging and before final protections. The resulting `UnlinkedProcess` exposes read-only diagnostics and keeps other relocations deferred; it has no conversion to an executable process. General symbol semantics and native guest startup remain unfinished; the narrower session linking policy is described below.

SCE inspection decodes classic and observed PS5 `_1` module/library formats per descriptor role. IDs remain scoped by image and role, malformed names retain structured diagnostics, and same-role encoding conflicts retain raw records and suppress every completed import association. Opaque filename markers, attributes and ELF ABI versions do not select an encoding. Inspection itself does not resolve providers.

`plan_symbolic` accepts a closed set of immutable executable views with explicit retained biases. Consumer import selectors and provider export selectors first identify their own image's declarations; only then do exact name bytes, semantic version fields, NIDs and supported types match across images. Numeric selector IDs are never compared across images. Missing or ambiguous providers and unsupported binding policies fail without inventing addresses. Exact version matching is an explicit PentaX restriction, not a complete SCE compatibility model. Aggregate input metadata, copied names, indexed exports, lookup work and patches are bounded.

Provider eligibility admits DEFAULT/PROTECTED definitions under their own export declarations; protected self-references remain local. A unique GLOBAL outranks WEAK definitions, otherwise a unique compatible WEAK may satisfy an import; multiple candidates in the winning tier refuse. A qualified definition whose selectors match only the two import roles is retained as a bounded exclusion diagnostic, without copying its name or publishing its address. Every examined candidate and role comparison still consumes the existing session budgets. Partial or absent declaration pairs fail; both complete role pairs always use the owned export identity. This restriction supplies no thunk classification, NID fallback or new execution capability.

`load_symbolic` stages the private plan in one owned `GuestMemory`, copying all modules, applying checked relative and symbolic patches, then installing final permissions and RELRO. Biased mappings must remain within the core address space and separate modules cannot share a guest page. The first RELRO policy accepts empty ranges or complete 16 KiB pages and explicitly rejects partial-page protection semantics. Every failure drops the unpublished session. `UnstartedSession` exposes only read-only diagnostics; complete relocation status does not establish stack, TLS, services or permission to execute.

`process::tls::plan_static_tls` borrows that exact symbolic plan into a privately constructed layout. `load_symbolic_with_static_tls` accepts only the retained plan, bounded guest TP positions and memory limits, stages the same images internally and copies their relocated initialized TLS bytes. Callers cannot pair templates with a detached session or arbitrary memory. The explicit `PentaXVariantIIInputOrderV1` policy preserves virtual alignment bias and places blocks below page-aligned TP boundaries in supplied-module order; runtime TLS IDs have a separate namespace. Full readable logical initialized backing and original byte agreement are required; page padding, BSS and gaps cannot supply initialized bytes. Materialization rejects every deferred relocation. Ordinary image writes may overlap the numeric TLS zero-tail range; only `p_filesz` bytes are copied to independent instances, whose remaining bytes stay zero. Aggregate image/storage/resident/work budgets and guard separation precede allocation. Prepared sessions expose immutable diagnostics; only the closed original fixture consumes their private state for interpretation. TCB/DTV, firmware startup and general execution remain unfinished.

`plan_tls_bindings` retains the exact symbolic plan and partitions supported DTPMOD64, DTPOFF64 and TPOFF64 records from unknown deferred records. `plan_relocated_static_tls` binds those typed records to its owned static layout, using runtime TLS IDs and checked signed addend arithmetic; SCE selectors are never runtime IDs. `load_relocated_static_tls` applies ordinary and typed writes before final protections and template copying. Symbol-zero DTPMOD64 uses the consumer's own nonempty PT_TLS template and derived runtime ID. Remaining unknown relocations remain explicit refusals; image writes do not initialize independent TLS zero tails. These read-only prepared sessions do not initialize host FS/GS or authorize execution.

Commercial-image startup still requires broader imports/relocations, module lifecycle, process-parameter interpretation, firmware-compatible stack/TLS and sufficient kernel services. Existing exact linking, owned preparation and service subsets do not establish that boot contract. Keep unknown imports and unavailable semantics explicit. The out-of-process worker isolates fatal guest faults from the desktop, and supported marker SYSCALLs must be mediated before hardware execution; neither mechanism supplies a sandbox or general guest fault recovery.

The first `process::hle` slice builds an original immutable ET_DYN provider for one exact LibcInternal profile. Ordinary module/library/version/NID/type resolution applies; duplicate providers remain ambiguous and there is no NID-only fallback. Its private manifest binds generated bytes, symbol positions and RX stub sites to the exact prepared session. The reference interpreter stops at a marker; no native SYSCALL runs. Dispatch requires the retained RIP/RCX continuation and full service ID, and all thirteen memory/string operations share monotonic attempt/byte/work/scratch limits. The closed original consumers exercise GOT calls, tail jumps and repeated calls. Sparse prepared sessions expose immutable diagnostics; native paths provide checked exclusive pauses without raw pointers. Ordinary HLE planning still refuses TLS-bearing images.

The appended libc `snprintf` service uses a private context captured at the same
PRE-SYSCALL stop: three remaining GP words, all eight raw XMM slots and the
actual callee RSP. GP and FP consumption are independent until either register
class exhausts; both then advance one shared checked overflow cursor from RSP+8.
Only consumed arguments are read. Bounded Rust parsing and source integer/float
converters, narrow-byte strings and UTF-16 conversion stage the complete result
before one whole checked destination write. Admission covers format/input bytes,
conversion work, simultaneous scratch capacities and final output.

The pinned wrapper's final `%s` copy preserves only the prefix before an embedded
NUL, while its return retains the full formatted byte count. Zero capacity,
truncation, incomplete conversions, invalid UTF-16 and source-undefined numeric
or high-precision cases are typed pre-write `UnsupportedFormat` refusals.
This is an intermediate pinned-source profile, without graceful C99 truncation
or complete libc coverage. No host formatter, host variadic object or
title-specific format substitutes for the guest arguments. The exact four
LibcInternal canonical/prefixed scope pairs remain module1.1/library1/FUNC.
The factory has 17 markers and 67 symbols; the appended marker occupies 0x200,
the dynamic table moves to 0x300, and old sites/data addresses remain unchanged.

The opt-in `plan_hle_startup` additionally retains typed TLS binding/layout limits and an explicit generic AMD64/static-layout policy. `prepare_hle_startup` derives its borrowing plans locally from that exact retained module set, materializes ordinary and typed writes before template copies, and attests the HLE provider in the actual owned `PreparedTlsSession`. Unknown deferred records retain their explicit refusal, and each instance retains a separate zero-filled tail. A closed original consumer keeps that owner throughout interpretation and checks two independent instances, runtime IDs, signed offsets and two real GOT libc calls. This composes reusable preparation; it does not define firmware TCB/DTV, constructors or a general native execution capability.

`PreparedNativeKernelStartup` retains the real prepared native TLS owner plus generated kernel/libc providers and bounded kernel state. Its `prepare_native_source` recomputes typed bindings/layout under retained limits and produces opaque owner-bound preparation provenance, not continuous liveness or operand confinement. The explicit `KytyObservedSelfPointerV1` profile adds an owned RW/NX header page at TP with a zeroed 64-byte self-pointer header, moving the upper guard above that page. `prepare_main_thread` checks its initial bytes, permissions and owner/source identity; the worker binds its modeled FS base to that proof. Selected FS/GS MOV operations and sign/zero-extending loads use checked Rust memory semantics, preserving flags and partial-register/address-size rules. Unsupported forms stop explicitly and host FS/GS is untouched.

The same modeled path supports 8/16/32/64-bit ADD, ADC, SUB, SBB, AND, OR, XOR, CMP, TEST, INC, DEC, NEG and NOT with one explicit FS/GS memory operand. Register destinations retain partial/high-byte rules and zero-extend 32-bit results. Arithmetic derives all six defined status flags; INC/DEC preserve carry, NOT preserves flags, and logical operations clear carry/overflow while retaining the architecturally undefined auxiliary flag as a deterministic policy. Semantics follow the [Intel instruction reference, A-L](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2a-manual.pdf) and [M-U](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2b-manual.pdf). Effective addresses use current guest registers and original guest RIP, including signed displacement, SIB and 32-bit address truncation before segment addition. Complete read/write spans, continuation, stack results, work and successful counters are admitted before any output. Rejected operations retain CPU/flags/output and never increment successful counters. Read-modify-write byte accounting includes both transfers. Atomic/repeat forms remain unsupported.

An original supervised worker-test process compares 229 encoding pairs and 4,580 state pairs against actually executed ordinary native instructions, including carry inputs, signed overflow, immediate sign extension, high-byte and extended registers, vector/control preservation and memory canaries. Undefined logical AF is excluded from the hardware comparison and its retained policy is checked separately. A real file-based new-module initializer also uses FS comparisons, TEST and ADD to observe zero TLS, change existing/new storage and drive checked branches before a later exported call returns 42. Backend-only GS/address/permission/refusal tests enter no native body. The prepared worker's observed startup profile still binds FS only; these checks establish neither a complete thread ABI nor commercial-game boot.

`prepare_process_entry` binds the exact prepared source and owner to an observed main-call context. Its single transaction stages a fresh guarded RW/NX stack, argc/argv object and terminated name, zero root frame and return marker. Primary entry and guest callback targets require initialized immutable source ranges and current executable/nonwritable permissions. Execute-only instruction targets do not require or gain guest data-read access. Numeric metadata cannot forge the opaque history object, which does not establish ongoing liveness or implement module lifecycle. The original consumer records its actual initial argument registers and reads the staged data before its service sequence.

The worker pauses only for source planning/one checked block and releases the pause before `service_request`. The core attests actual immutable marker bytes, exact continuation and full service ID; kernel calls receive all six original SysV register arguments. The exact named-direct mapping registration also reads its seventh argument at captured callee RSP + 8, through checked guest memory and shared byte/work limits. Other calls do not read the optional stack pointer. A typed service error cannot be replaced by zero or an invented errno mapping. An original consumer executes 38 blocks and four services with twenty ordinary-memory checks and three modeled FS operations, returns 42 and checks actual entry arguments/root frame, real thread/semaphore state, TLS/header isolation and eleven eagerly committed pages in debug and release. Persistent code caching, writable-code support, cancellation, extended state, complete firmware TCB/DTV, scheduling, module initialization/exit and title startup remain unfinished.

The 19-operation NpWebApi2 profile owns bounded local library contexts and
requests, independently of HTTP2 state. Contexts retain the source-rounded pool
size; request IDs retain copied opaque input strings and an empty response.
Send resets response state and returns NOT_SIGNED_IN, while ReadData returns
zero for a valid empty response. Checked output transactions publish exactly
28 defined statistics bytes and preserve response-descriptor padding. Shared live-handle,
retained-text, scratch and work limits precede publication. User, push-handle,
filter and callback receipts are data-only owner records; they establish no
PSN session, subscriptions, event delivery or callback execution. The exact
NpWebApi2/libSceNpWebApi2 module/library pairs form four finite combinations at
module1.1/library1, without changing normal name/version/type resolution.

The supplied-image runtime also admits a separate placement-bound JSON constructor
provider through `KernelStartupExtensions`. Its allocator constructor has five
finite qualified exports sharing one marker, while a private unexported marker guards six
unknown virtual slots. The observed eight-word table lives in a separate
read-only, non-executable guest page; its entries contain guest addresses.
Current source/factory provenance and live CPU/GPU immutability are attested at
preparation, epoch publication and each constructor/virtual service stop.
Bounded work precedes a whole-span eight-byte constructor write. Unknown
virtual behavior remains a typed terminal stop. This models one observed
constructor effect, without inferring the complete class or allocator ABI.
Its separately source-backed Null-value constructor initializes exactly32 guest
bytes, including padding and the type at offset28, and returns the supplied
object address. Null is explicitly accepted by that source contract. A private
per-symbol manifest binds each qualified constructor export to its own marker;
it never grants the unexported virtual guard a callable import identity.
Unknown dynamic Value, parser and lifecycle methods remain explicit stops.

The primitive JSON path stages each destination in a fixed32-byte overlay.
Ordered source reads observe earlier staged writes, including unaligned virtual
overlap; one final bounded transaction publishes only the source's field spans.
Distinct unresolved physical aliases refuse explicitly. Padding and parent/root
preservation follow each individual operation. Reference getters return guest
union addresses or separate immutable defaults. Dynamically owned payloads
require a real lifecycle owner and cannot be replaced by empty scalar success.
Full export/relocation validation runs for each immutable plan publication;
every service stop rechecks its exact current factory/module and all live
markers/defaults in constant bounded work, independent of unrelated imports.
Double construction and assignment consume the low64 raw bits of XMM0 from
the same PRE-SYSCALL CPU snapshot as the integer arguments. The private stop
token retains all128 bits of XMM0..7 and binds them to its completion identity;
the production worker forwards that snapshot. Direct requests without vector
capture refuse double operations explicitly. No host floating-point conversion
or exception changes the guest payload, including NaNs and signed zero.

String operations share one process-owned registry with the Value lifecycle.
Guest wrappers contain checked guest impl identities; CStr returns real owned
NUL-backed guest bytes. Current guest contents remain authoritative. Assignment
retains its impl identity, while deep copies have independent storage. Retired
identities are rejected and their mappings remain charged while quarantined.
Source-ordered Value staging also handles String cleanup and copy, so a scalar
setter cannot bypass the String owner. Mutable and const empty-wrapper helpers
retain their different initialization rules. Array/Object ownership and parsing
remain separate work; unported formats refuse explicitly.

Kernel runtime-data publication can combine one new RW/NX blob with three small
caller outputs in one backing transaction. Scratch includes simultaneously held
staging capacity; every allocation, metadata slot and output is admitted before
publication. Only infallible owner/accounting updates follow successful backing
publication. This covers lazy source-wrapper creation together with destination
field writes, including the two spans that preserve Value padding. No operation
may publish an allocation and then attempt a separate fallible wrapper write.

ARM64 support requires dynamic binary translation or a suitable virtualization strategy. Merely compiling the launcher for ARM64 does not make x86-64 guest code executable.

The supplied-image entry probe composes these same components rather than a
second execution engine. Read-only bounded intake precedes core image placement,
exact provider linking, typed TLS and owned entry preparation. Missing callable
imports are classified with the existing resolver; the opt-in constructor
independently repeats classification against actual real providers before adding
explicit unavailable exports. Those markers require live owner/site/ID/byte
attestation and always stop without a returned guest value. Weak/data/TLS and
version/ambiguity errors cannot become fallback function implementations.

`pentax-cli probe-entry` owns a hidden child with bounded pipes, a thirty-second
deadline and guaranteed kill/reap on early failure. Instruction observations count
returned native bodies, separately for the primary source and generated providers;
attempts or decoded syntax alone do not establish execution. The route reports
stable categories/counts without paths, NIDs, PCs or bytes. It probes the observed
main-call context; broader module loading and process startup, complete file-system profiles,
termination lifecycle, graphics and a complete console boot remain separate work.

POSIX once initialization separates a pure kernel request from native call
preparation and actual return evidence. A private core ticket binds owner,
active source, control, callback and parent service. Target/permission/frame
admission precedes IN_PROGRESS publication. The worker suspends the same
caller below its red zone and runs the initializer under the existing checked
engine, TLS and cumulative limits. Core compares opaque return owner/executor/
sequence/target/stack/guards and initial arguments before publishing DONE;
then the original service completes with zero, ignoring the void callback's
numeric result. The continuing guest MXCSR environment, including exceptions
raised by the callback, is preserved when restoring caller registers. Successful
entry-probe results expose only its six status bits as `fp_status`. Mixed
module/once frames survive admitted source append. A
failed completion write poisons that record rather than allowing replay.
Waiting states and concurrent guest-thread execution remain unsupported;
scalar admission or a copied ticket cannot prove an initializer returned.

Pthread key storage is owned by the process and the actual admitted initial
guest thread. The source's 256-key table uses odd/even allocation generations;
specific values retain their generation to reject stale data after reuse.
Creation commits its exact four-byte guest output before infallible key state
publication. Values and retained destructor registrations are opaque guest
words, never host pointers. A lazy fixed-size value table, shared resource/work
bounds, positive POSIX errors and unchanged errno preserve the source contract.
Additional guest-thread values and destructor execution require future real
thread lifecycle integration; no successful exit path is fabricated.

The mutex subset binds only the actual paused initial guest thread. Checked
eight-byte slots carry opaque owned mutex/attribute identities; creation and
destruction publish their guest outputs before infallible retained-state
changes. Zero/adaptive initializers, copied type attributes, recursive counts,
trylock, self-deadlock, busy destroy and relative/absolute timeouts follow the
pinned source. Direct errors are int32 kernel words; POSIX pthread wrappers
return positive errors without errno writes. Resource/work/byte limits remain
shared. Bounded self-contention waits in the existing no-signals profile;
foreign-thread blocking, priority protocols and additional guest threads stop
explicitly. This does not establish a complete pthread or firmware TCB ABI.

`with_native_image_session_with_roots` consumes explicit kernel read-only capabilities: either a finite file inventory or an owned directory root. The original API supplies no roots. Worker `--data-file` bindings retain individual regular files; `--data-directory` bindings retain selected directory handles without reading/enumerating their contents at admission. Requested paths resolve inside those retained roots through pinned cap-std, never by concatenating a guest name with an ambient host path. Canonical names, duplicate/overlapping intake bindings, root/data counts and cumulative admission work are checked. Path lookup charges bounded work before OS operations; escaping symlinks/junctions and device/alias names are refused. Opened regular-file handles feed the same kernel descriptors, source errno/POSIX wrappers, metadata, positional reads, transactional guest outputs and shared I/O/byte/scratch/work/resource bounds. They do not become images, select a preload index or authorize module loading. Windows files exclude writers/replacement while retained, and root identity remains protected; separate opens can observe host changes. Root/subdirectory reachability, read-only directory descriptors, ordinary directory reads, seek, getdirentries/getdents and directory status are implemented over actual bounded packed entries. A descriptor retains its captured bytes and independent cursor; fstat uses that length while stat captures a fresh list. The exact source packing uses 512-byte blocks, source name hashes (including reversed tail bytes), four-byte record alignment and final-record block padding. Logical dot entries do not read the host parent. Each Windows entry retains its own attributes, including junction type, without opening its target. Payload/base outputs are one transaction and cursor changes follow success; EOF avoids an unused nonnull buffer. Buffer capacity has a separate aggregate retained-storage budget and fallible geometric growth. Directory pread and unsupported name/device/writable profiles still refuse. Original native and CLI cases read actual file data and retain unchanged inputs; real Windows junction tests reject outside access. No game-directory scan or commercial-game execution was used for validation.

An explicit preload role can select one supplied shared module's unique nonzero
`DT_INIT` under the pinned Kyty three-zero-argument convention. Core preparation
binds it to the exact source, owner and process-entry history. The worker admits
the initial thread once and keeps one executor/cache and cumulative budget set
across initializer/main calls. Only an exact completed outer return admits the
next call, with live stack/guard/marker/target and unchanged segment-base checks.
Actual TCB writes and observed guest control/vector state survive this transition.
Initializer failures prevent main entry; absent/zero targets and other providers
are not counted as completed initialization. This does not supply dynamic module
discovery/start/unload, finalizers or a completed firmware lifecycle.

`KernelGetProcParam` now uses the actual primary source retained by the native
backing. The private adapter returns a checked biased `PT_SCE_PROCPARAM` address
after whole-extent live read admission; it neither allocates/copies payloads nor
guesses a parameter ABI. Known primary absence returns null. Unbound integration,
empty/shared-primary shapes and unreadable ranges remain typed failures; provider
and module-parameter metadata cannot substitute. Metadata/page work is admitted
before the query and service attempts use the same retained kernel budget.

Loaded-module symbol lookup is scoped by process-local supplied-image handles,
not SCE selectors. Generated provider images are excluded. A separate pure linker
query reuses owned provider-role validation and address rules, with a bounded
pinned name transform/raw-NID fallback only inside the selected module. Normal
import resolution is unchanged. The actual core adapter charges cumulative query
work even on refusals and checks live target extent/permissions before output.
Only known Dlsym miss/null cases receive its explicit source int32 status and
transactional output; other typed failures stay failures. Returning an export
does not initialize or dynamically load its module.

Explicit `--module-file` capabilities can supply a real application module when
an attested load request suspends the current caller. Ordinary source append
preserves predecessor bindings. A separate source-bound fallback policy permits
only exact GLOBAL FUNC definitions from the retained generated unavailable
factory to yield to a unique compatible function in that selected module. Core
authenticates the actual factory identity, owner, active source and immutable live
marker bytes; linker policy metadata alone grants no native publication rights.
Retirements persist across later source extensions, TLS planning and module
lookup. Ambiguous winning tiers, unrelated binding changes and incompatible definitions
still refuse publication.

An extension can add one selected guest and one optional pure-RX unavailable
provider with a single parent-to-final source transition. Missing-function
classification and final relocation planning share the same aggregate policy
and target only the guest for real replacements. Core independently rechecks
the selected input's exact missing identities and reference provenance before
accepting the immutable generated image. Per-process owned manifests retain
copied sites and static public diagnostic hints while worker arenas retain the
actual borrowed image storage; numeric source identities are never dereferenced.
Provider function, image and generation-work budgets include initial and later
retained factories, even after their markers retire. Local marker service IDs
may repeat across images; a unique authenticated PC range chooses the origin.
Generated deltas never receive guest handles or initializer calls.

Native publication compares every affected old relocation word before changing
payload, replaces only the listed bytes and restores each page's current CPU/GPU
permissions. Unlisted physical aliases and guest-modified words refuse. New
mapping, relocation, TLS initialization and sealing finish before this final
fallible replacement stage. Recoverable failures restore bytes and permissions;
failed rollback poisons the owner. Existing thread TLS data is preserved, while
affected saved initializer bytes change for future threads. This relocation-slot
policy does not rewrite function pointers copied elsewhere by the guest, provide
missing object storage or establish an arbitrary C++ object ABI.

The prepared kernel dispatch uses `Kernel::invoke_library` to return supported
source errno through the audited library ABI instead of terminating the worker.
Read/pread/lseek error ints are sign-extended to 64 bits; audited file-status,
memory, semaphore/event-flag and clock/sleep int results use a zero-extended
32-bit kernel code. The underlying raw invoke API retains typed errno for
non-ABI consumers. Each attempt and its output/state effects occur once under
the original budgets: an unsuccessful semaphore poll consumes nothing, while
an event-flag poll retains the source's current-pattern output even on EBUSY.
Guest code can branch on these actual words and continue. Unsupported domains,
blocking waits, output-overlap policy, allocation, budgets and backing faults
remain typed stops, without raw-syscall carry synthesis.
Original native consumers verify full return words, then perform valid file
reads or synchronization operations; unavailable service tests never resume.

The generated kernel provider declares both exact `libkernel` and `Posix`
libraries under module `libkernel` 1.1. Local selector IDs are translated
through owned declarations; the same NID in two libraries has distinct
markers and qualified resolution. Wrong scopes never fall back to a matching
NID, and wrong versions refuse admission before execution.

Selected POSIX wrappers have their own source-specific -1 result widths and
set a shared initial-thread errno cell only for recognized source errors.
`get_error_addr` returns a stable address in an internal guest RW/NX page,
initially zero and separately budgeted from the pthread prefix. The actual
prepared source/thread binding and whole-cell backing permissions are checked;
no host TLS pointer or firmware TCB offset is inferred. Successful calls leave
errno intact. Backing, unported semantics and resource failures still stop.
Original native consumers write/read that cell through real guest instructions,
check both library scopes and full return words, then read actual selected file
data. Additional guest threads and scheduling remain unimplemented.

Atomic memory instructions retain their actual LOCK prefix, or memory XCHG's
implicit lock. They always split into a single native instruction independent
of optional memory isolation, so their operands use the current observed
registers. Both platform stepping paths check the exact owned read/write span
and natural alignment before publication. Misalignment is an explicit PentaX
policy stop to avoid split locks; no guest alignment exception is synthesized.
Hardware ordering, flags and register results survive the existing context
bridge. HLE, segment-prefixed atomics and LOCK-prefixed bit strings remain
unsupported. Ordinary BT/BTC/BTR/BTS memory accesses separately admit the full
selected16/32/64-bit element. Signed register bit offsets participate in both
planning and execution address dependencies; immediate offsets do not extend
the decoded element. Live owner/range/permission checks precede native entry,
including cross-page spans. No new instruction/prefix authority is added.
Additional guest-thread execution and scheduling remain separate
unfinished work.

String instructions execute as checked Rust memory transitions on the same
paused native owner. MOVS/STOS/LODS/CMPS/SCAS support byte through qword elements
with 64-bit indices, direction-flag traversal and their documented repeat forms.
At most 256 elements execute per dispatcher step; an unfinished repeat resumes
at its original instruction with updated indices/count and retained cumulative
budgets. Each element checks actual backing and commits its write before its
register changes. Sequential source reads preserve destructive overlap rather
than substituting memmove. Zero count skips data access and segment resolution.
Conditional-repeat faults retain completed indices/count and restore the flags
from the instruction's original start, including across source-view reattachment.
No host segment register or host string routine is involved. Address-size32,
I/O strings, LOCK and undefined repeat combinations remain explicit refusals.
String element/byte counters remain distinct from actual native instructions.

VEX integer vector operations execute through the same checked Rust path as
packed moves, using explicit guest XMM/YMM state rather than host AVX execution.
VMOVD and VMOVQ support register and exact-width memory transfers, including
32-bit GPR zero extension and full destination upper-bit clearing. Writes to RSP
must remain in the admitted stack. VPAND, VPANDN, VPOR and VPXOR support two
independent 128/256-bit inputs; VPSHUFB uses separate 128-bit lookup lanes and
per-byte zero masks. Aliased inputs are read before destination publication.
VPSHUFD/HW/LW select immediate-indexed elements independently in each 128-bit
lane, retaining the unselected half for word shuffles. Full vector memory reads
remain mandatory even when the immediate repeats just one element.
VEX.256 VPERMPS and VPERMD select each dword across the entire 256-bit source
using only the low three bits of its corresponding index dword. Their independent
index/data snapshots preserve destination aliases; selections copy raw bits,
including NaNs and signed zero, without floating-point exceptions or conversion.
The complete 32-byte memory source remains mandatory and need not be aligned.
Flags and MXCSR are preserved. Complete memory spans, continuation, work and
success counters are checked before output; rejected instructions leave the
CPU state and successful operation count unchanged. Unlisted VEX/EVEX forms
retain explicit unsupported stops. Semantics follow Intel's
[MOVD/MOVQ, packed logical and PSHUFB instruction reference](https://cdrdv2-public.intel.com/868141/253667-089-sdm-vol-2b.pdf).

## GPU and timing

`pentax-gpu::inspect` copies one bounded little-endian input into an immutable report. Packet views borrow that owner; payloads remain ranges in the same input. Fixed-size records are reserved once, and input capacity, record capacity and visited dwords have explicit limits. The admitted subset covers exact type-2 padding, zero-flag NOPs (including the header-only form) and context/SH/uconfig relative register-write grammars. Header flags and opaque payloads are retained without inventing hardware state. Indirect-buffer references always stop at their header, even with zero size; targets are never followed. Every stopped prefix is diagnostic only. `self-test-gpu` checks fresh original words through this API. The [crate documentation](../crates/pentax-gpu/README.md) records the exact admission policy and pinned primary sources.

Command inspection, shader decoding, shader compilation and Vulkan submission are separate modules. `pentax-shader-compiler` constructs checked control flow and supported program/resource profiles, lowers its documented subset through bounded IR and emits SPIR-V with resource and required-feature manifests. Unknown semantics/features remain explicit errors. The Vulkan backend uses `ash`; desktop rendering is independent of this guest-oriented backend.

`pentax-shader` owns one caller-delimited linear word span and ports every family recognized by the pinned classifier: scalar, VOP1/2/C/3/3P, EXP/VINTRP, SMEM, MUBUF/MTBUF, FLAT, DS and MIMG. It retains exact opcode maps, modifiers, memory/image fields and bounded NSA/address metadata. Literals are consumed as data, branch targets are signed metadata and trailing words after ENDPGM remain visible. This preserves the observed source's accepted fields and explicit unsupported cases; it is not complete ISA validation. Program/stage legality and supported compilation belong to the separate compiler, not this syntax API. See the [frontend scope](../crates/pentax-shader/README.md) and [compiler contract](../crates/pentax-shader-compiler/README.md).

The Windows Vulkan backend owns device/queue, memory, buffers/images, descriptor layouts, pipelines, commands, synchronization and readback resources. It accepts closed original factories and opaque prepared compute products, checks output manifests and host features, and exposes no raw SPIR-V submission. The separate `pentax-gpu::compute` path admits one self-contained flat wave32 dispatch, explicit compute register state, a bounded acyclic shader and one raw buffer. `pentax-core::gpu::execute_compute` holds its actual native owner paused through capture, permission/self-alias checks, Vulkan execution, fence completion and transactional writeback. Original worker submissions verify different local sizes/SGPR seeds, 112 output bytes including an untouched suffix, and unchanged command/shader bytes. The earlier fixed compute/triangle diagnostics remain available.

The native image runtime now includes an exact Graphics5Driver 1 / module 1.1 provider for four single/batch submit calls. Its separate fixed ELF factory and immutable private manifest undergo ordinary qualified linking before unavailable-function classification. A PRE-SYSCALL request must match the actual owner, exact full marker ID/site/continuation, immutable logical permissions and current source bytes. Checked reads interpret the observed context queue at offset 4 and 16-byte packet address/dword-count fields, never host pointers. Source null/zero-count branches retain their actual result ordering. Selected flat draw queues use the separate graphics preparation and owned presentation paths. Bounded batch metadata snapshots precede GPU work, then the retained driver charges cumulative call, dispatch, command, invocation and preparation budgets before each job's effects. Synchronous completion retains the same pause through fence and transactional writeback. An original native x64 consumer calls all four actual imports, verifies independently changing GPU results and preserved tails, and exercises null/zero-count branches. Known compute registers persist independently across the 56 observed ACB queues. Complete bounded command plans support state-only updates, multiple dispatch snapshots and ordinary indirect call/chain frames, with active-lineage cycle detection and byte/depth/hop/packet/record/work limits. GPU resources are read in dispatch order after previous writeback. Final register state publishes after full buffer success; any failure after possible GPU work makes the driver terminal. Same-owner physical/virtual backing checks refuse writes aliasing captured command spans. A second original native consumer verifies two isolated queues and eight GPU jobs through call/chain streams. Ordered memory actions now include admitted WRITE_DATA, immediate 32/64-bit release labels and fulfilled memory waits on the same paused owner. Transactional writes reject command-backing aliases and retain byte/work limits; failures after possible writes are terminal. Absolute and based indirect dispatch dimensions are read after prior GPU/command effects; zero dimensions produce no Vulkan job. Unsatisfied waits are distinct stops, with asynchronous suspension/resumption still unimplemented. A third original native consumer verifies GPU-produced zero dimensions, a skipped output, CPU command-fed nonzero dimensions, real indirect execution, labels and waits. Asynchronous GPU scheduling, conditional/predicated commands, broader draw/resource modes and general coherence tracking remain unfinished. The selected draw, tiled writeback and owned presentation profiles have separate original fixtures; these observations do not establish a commercial game frame or title compatibility.

The selected triangle-list profile supports explicit `INDEX_TYPE` and
`DRAW_INDEX_2` packets with unsigned 16-bit or 32-bit indices, alongside ordinary
nonindexed draws. The packet's maximum count bounds the number of submitted
elements; numeric index values do not have to be smaller than that count.
Base-vertex offsets, primitive restart and indirect indexed draws remain explicit
unsupported cases. Preparation captures the complete GPU-readable index span
into an owned immutable product while the guest owner is exclusively paused.
The writable target must be physically disjoint from index, command and shader
sources. Read-only sources may overlap without granting write authority.

Vulkan uploads the actual captured bytes to a dedicated index buffer, makes host
writes visible to index fetch, binds the correct index type and submits
`vkCmdDrawIndexed`. The buffer remains owned through fence completion. Only
supported `fullDrawIndexUint32` is enabled; numeric values must fit the selected
device's actual `maxDrawIndexedIndexValue`. Index snapshots, uploads, command
work and the simultaneous target readback retain finite admission limits. The
existing tiled LOAD/writeback and current-draw flip receipt remain responsible
for presentation; packet admission alone creates no frame or acknowledgement.

Use the AMD RDNA 2 ISA as an instruction reference. Do not transplant PS4 GCN register layouts into a PS5 implementation without evidence. Begin with bounded command buffers, synthetic shader fixtures and deterministic traces. Then compare decoded commands and outputs against an established implementation using local, privately held test material.

Parallel command preparation must consume immutable snapshots and produce dependency-ordered work. Keep resource mutation, cache publication, synchronization and final submission explicitly ordered. Design CPU/GPU memory coherency before adding worker threads; Rust's ownership checks do not establish guest cache or Vulkan synchronization correctness.

Frame presentation, guest vblank and engine time are separate clocks. Performance work must preserve real-time guest speed, avoid frame generation and keep the requested internal resolution at or above 1080p. A reference-game frame-time target is not evidence of general compatibility.

## Configuration and input

Global UI, graphics, audio, input and runtime preferences occupy separate TOML sections. Per-title files contain overrides; absent values inherit globally, and explicit `false` and zero retain their meanings. Unknown schemas and fields fail visibly rather than being overwritten. Game paths belong to local library data, never repository fixtures.

The UI must distinguish applied frontend behavior from preferences awaiting an emulation backend. Keyboard and controller navigation require visible focus, reconnection and equivalent core actions. A connected-controller count does not by itself prove navigation works with real hardware.

## Delivery sequence and evidence

1. **Foundations:** adversarial executable parsing, memory invariants, deterministic fixture execution, persisted preferences and native desktop interaction. Evidence: tests, local CLI runs and rendered desktop inspection.
2. **Guest boot:** map SELF payloads, relocate modules, resolve imports, construct stack/TLS and run original native homebrew in a guest worker. Evidence: guest exit status, syscall trace and fault isolation on the supported host.
3. **Services and display:** kernel/libc subset, controller and audio services, command decoding, shaders, resources and a presented original guest frame. Evidence: output comparisons and invalid-input tests.
4. **Reference game:** iterative boot and rendering against a locally owned game, without modifying its files. Evidence: concrete boot stage, missing-service reports, screenshots, correct engine time and reproducible frametimes. Do not distribute game data or private paths.
5. **Compatibility and performance:** broaden instruction/service coverage, parallelize measured hot paths and verify correctness across games and hosts. Evidence: per-title compatibility reports, controlled A/B captures and regression checks.
6. **Distribution:** reviewed PRs, protected branches where available, dependency surveillance and versioned native packages. Describe each platform's tested capabilities; do not advertise untested devices.

These milestones preserve the full emulator scope. A working launcher or synthetic program is not completion of PS5 emulation.

## Research references

- [KytyPS5](https://github.com/KytyPS5/KytyPS5): PS5 translation layer using native x86-64 and Vulkan; [ELF loader](https://github.com/KytyPS5/KytyPS5/blob/main/src/loader/elf.cpp) and [format definitions](https://github.com/KytyPS5/KytyPS5/blob/main/src/loader/elf.h) inform format research. GPL-licensed implementation; do not copy or translate without retaining applicable terms.
- [SharpEmu](https://github.com/sharpemu/sharpemu): independent C# PS5 implementation useful for comparing subsystem boundaries and observed behavior. Coverage is incomplete.
- [SharpProspero](https://github.com/SvenGDK/SharpProspero/tree/5dfdb4b0b166e83dc9ae275001afb695f12c7174): public PS5 module producer and editor used to corroborate observed `_1` descriptor words independently of KytyPS5. This establishes observed encoding, not firmware acceptance or version compatibility.
- [shadPS4](https://github.com/shadps4-emu/shadPS4): PS4 CPU/HLE/linker and shader architecture research. PS4 compatibility is not PS5 compatibility.
- [fpPS4](https://github.com/red-prig/fpPS4): a Pascal PS4 compatibility layer; an additional architectural reference.
- [Obliteration](https://github.com/obhq/obliteration): Rust PS4 kernel/VMM work with platform virtualization backends. Its architecture is useful research, not a shortcut to a working commercial-game runtime.
- [AMD RDNA 2 ISA, document 70648](https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture): primary shader instruction reference.
- [System V gABI program headers](https://gabi.xinuos.com/elf/07-pheader.html): primary segment layout and alignment reference. Console extensions need separate evidence.

Performance gains in an existing fork are evidence about that fork and workload. They do not predict the performance of a Rust rewrite.


### Runtime memory admission

The explicit worker/CLI `--memory-profile runtime` uses
`NativeImageLimits::with_runtime_memory` after read-only image intake. Guest direct
capacity follows the pinned Kyty default (13824 MiB total minus 1024 MiB flexible).
Native commitment and mapped alias bytes remain independently counted under one
host-derived cap: retain half of scarce physical availability, or a 6 GiB initial
margin on larger hosts, independently bounded by process commitment after
retaining half when scarce or a 2 GiB margin otherwise. The cap is rounded down
to MiB, at most 14 GiB (13.5 GiB guest pools plus 512 MiB startup allowance),
and refused below 1 GiB. Input/compiler/GPU
allocations are outside that cap. Kernel mapping headroom, cumulative memory-effect
work and native mapped/resident/reservation limits are selected together; copied
bytes, I/O and attempt limits are unchanged. Compact fixture defaults stay intact.

The 32 GiB kernel virtual window stays an owned NX placeholder until pages are
needed. Reservation descriptors and OS splitting scale with used regions rather
than all reserved pages. Splits retain exact ownership, perform fallible metadata
admission before OS mutation, and publish each successful boundary immediately.
Mapping rollback can retain harmless placeholder splits without exposing pages;
failed backing rollback still poisons the owner. Destruction releases every exact
recorded region. Dynamic code is placed before the data window and must be disjoint
from it. Cache placement alternates above/below code within the same attempt cap.
No arbitrary guest instruction or memory operand gains admission from this policy.

Host availability follows Microsoft's
[GlobalMemoryStatusEx](https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-globalmemorystatusex)
and [MEMORYSTATUSEX](https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/ns-sysinfoapi-memorystatusex)
contracts. Snapshots are volatile and do not guarantee later allocations. Owned
placeholder splitting follows
[VirtualFree](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualfree).


### CPU/GPU memory visibility

Native mappings retain independent CPU R/W/X and GPU R/W bits. Windows data
protection admits their necessary union and never execute permission. Ordinary
CPU reads/writes/instruction-source checks use CPU bits; PM4, shader, indirect
argument and wait reads use GPU read permission. Submit ABI structures remain
CPU data and require CPU reads. Kernel mapping outputs still require CPU write
permission even when the new mapping itself is GPU-writable.

Compute buffer staging uses the compiled shader manifest's actual read/written
requirements. Write-only storage can preserve its untouched bytes in the host
upload without granting shader reads. Fenced results and ordered command writes
use a separate GPU-write transaction on the same paused owner with unchanged
complete-span, overlap, alias and resource checks. CPU stepping stays paused until
writeback completes. Physical aliases observe those actual bytes immediately on
resume. Unsupported cache flags, asynchronous execution and broader draw/resource modes
remain separate limitations; GPU permission is no substitute for those features.

Original native graphics consumers explicitly request a CPU/GPU shared mapping,
copy their original command/shader/data templates into it, submit actual Vulkan
jobs, and branch on the results. The original PM4 fixture additionally runs with
GPU-only source pages and GPU-write-only output storage, verifying CPU denial,
GPU read denial and preservation of untouched output bytes.
