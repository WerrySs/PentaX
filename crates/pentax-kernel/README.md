# pentax-kernel

A stateful, safe Rust port of the startup-relevant libkernel subset in the
licensed Kyty fork pinned in [NOTICE](NOTICE). Pinned `cap-std` provides
directory-handle-relative lookup. There is no execution entry point,
host-pointer cast or implicit host filesystem root.
The core binds this crate to its exact source and actual native backing through
the private `GuestAccess` adapter.

`TlsGetAddr` requires that adapter's admitted image/thread context before
charging or reading its sixteen-byte descriptor. The bounded runtime lookup
returns an address in an actual prepared TLS block; unbound owners retain
Unsupported without guest I/O. The module word is an opaque typed-relocation
identity, never a host pointer. No generic success/null result or firmware DTV
is inferred. Guest thread creation and cooperative scheduling require the
retained core executor; this crate alone never executes guest threads.

## Calling and backing contract

The seven SystemService notice, identity and lifecycle roles follow the pinned
reference's silent profile. The notice getter writes exactly one byte from
`system.skip_notice_screen` (default false). SetNoticeScreenSkipFlag and
DisableNoticeScreenSkipFlagAutoSet retain their source no-op behavior, so the
configured preference remains authoritative. HideSplashScreen, PowerTick and
the opaque abnormal-termination report do not operate host UI, power or files.
ParamGetString supports system-name ID 6, reporting the explicit `PentaX\0`
profile in seven bytes, retaining the remaining output bytes. Null/zero-size
precedence, full-width capacity and source parameter errors remain exact;
unknown IDs and invalid backing retain typed refusals. Configuration reaches
the worker through the existing `--config` path.

Thread priority services preserve signed guest priority/policy metadata and
four-byte parameter/output layouts. Direct Getprio/Setprio use kernel errors;
POSIX wrappers return positive errors without changing errno. Scalar priority
queries return 256/767 for any policy. POSIX setschedparam changes attributes,
while Setprio changes the running execution class. The native worker grants
fair round-robin turns of 4096/2048/1024 blocks to high/normal/low classes;
these are explicit PentaX quanta, not Windows scheduling promises. Created
threads latch the source's single-core class demotion at first activation.
Later affinity changes do not relatch it, and the main thread is not demoted.
Host thread priorities, affinity and scheduling policy remain untouched.

Created-thread exit cleanup follows the pinned `CleanupThread` order for the
admitted profile: one current `ThreadDtors()` callback, then live pthread-key
destructors. The unavailable `CxaThreadAtexit` registration service admits no
thread-local records; process-wide `cxa_atexit` registrations remain separate.
Module-unload Count/Report hooks are retained and never called on thread exit.
The key scan exchanges each slot before testing its live generation/destructor,
walks 256 slots in ascending order and performs at most four passes. Actual
callback returns, including callbacks that set/delete/recreate keys, determine
the next live scan. Remaining values are discarded after the fourth pass.
Private owner/thread/context/serial tickets prevent replay or cross-thread
completion; resource refusal retains pending cleanup. The native owner admits
each executable target and executes it with the child's real TLS/owned stack.
It preserves the original entry return independently of callback RAX and marks
the thread returned only after every admitted cleanup callback actually returns.
Callback faults do not become clean completion. Created-thread storage retains
its existing bounded owner lifetime; this adds no join/detach reclamation,
cancellation unwinding or unimplemented thread-local Cxa registration support.

PlayGo implements eighteen pinned module 1.0/library 1 functions over actual
selected `/app0` metadata. Binary `sce_sys/playgo-chunk.dat`, literal
`playgo-chunkdefs.xml` tags and contiguous `Data/chunkmanifest` records are
tried in source order. The binary reader requires a complete twelve-byte prefix;
fallback inputs are capped at 16 MiB with independent work/scratch admission.
No ambient file search or fabricated 1000-chunk hack is enabled. Missing valid
metadata returns the source NOT_SUPPORT error on Open. Handle 1 is a source
constant; Initialize, Close and Terminate do not own a lifecycle resource.
Initialize reads only its pointer/size fields and leaves its opaque work buffer
and logging-only reserved field untouched. All source scalar widths, validation
order, masks, speed 2 and count-based progress are retained. Locus queries observe
prior staged stores through real backing aliases; invalid chunk IDs publish only
the admitted valid locus prefix. Typed backing/resource refusals publish nothing.
Chunk-list output preserves the final count store even on overlap. Prefetch and
todo setters validate inputs but retain no request and perform no download.
Only the seven independently observed matching libScePlayGo aliases are bound.
This local source profile does not establish installation activity or game boot.

Raw Mouse/Keyboard implements the pinned stateless disconnected-device
profile: ten canonical module 1.1/library 1 FUNC identities plus only the
observed libSceMouse Init/Open/Read aliases. Open validates type/index and
returns 1; user IDs and opaque parameters are not dereferenced. Any positive
integer handle remains accepted after Close. Read clears exactly 40 bytes per
Mouse record or 96 per Keyboard record and returns zero samples. Keyboard
Read accepts 1..16 records, ReadState clears 96 bytes, and six-GP GetKey2Char
clears 20 bytes at R9 without translating the ignored LED/modifier/key values.
Guest padding is included and output tails remain intact. Mouse has no source
record-count cap; PentaX uses the existing I/O output budget (1 MiB by default)
and returns a typed limit before writes for excessive requests.
These calls create no retained handles, devices, callbacks or host input and
remain separate from IME's editor/event pipeline.

IME binds thirteen canonical Ime module 1.1/library 1 FUNC identities and only
the observed libSceIme Update/KeyboardOpen identities. All arguments use GP
registers; float coordinates live in guest structures. The source profile
owns bounded UTF16 text, a 20480-byte caller work buffer, separate keyboard
state and real queued 72-byte events. ParamInit clears exactly 96 bytes and
sets user_id=-1; keyboard resource queries clear exactly 24 bytes and return
the source disconnected/not-opened error. GetInfo leaves its output intact.
Update requires the captured handler and actual native callback returns;
detached text events are generation-guarded while detached keyboard events
survive reentrant text close/reopen. These local lifecycle operations do not
provide an interactive host keyboard, connected device, UI or filter bridge.
Exact authorized-fork pins and the finite identity boundary are recorded in
[provenance](../../docs/upstream-provenance.md).

Font/FontFt binds seventy-two exact canonical NIDs plus fifty-four independently
observed matching libSce-prefixed identities. The catalogue preserves actual GP
arity independently of raw XMM arguments: FontMemoryInit has seven GP values
with destroy_object at captured callee RSP+8; horizontal rendering has five GP
values plus two floats in XMM0/1. MemoryInit writes the declared64-byte guest
record, retaining callback/interface/arena addresses as opaque data. A binding
or source no-op is no proof of allocator callback execution, installed system
fonts, actual glyph pixels or presentation. Checked owned guest records and
explicit bounded font-data capabilities replace source host pointers and
ambient source-directory/CWD font searches; no font asset is supplied by the
catalogue. [Provenance](../../docs/upstream-provenance.md) records exact pins
and the separate rasterizer/asset boundary.

The bounded Rust font backend uses skrifa0.44.0 outlines and an original4x4
grayscale rasterizer. Its bundled Hack-Regular fallback from
epaint_default_fonts0.37.0 deliberately differs from donor Roboto/VGA pixels
and metrics. Images are at most128x128, font inputs8 MiB and draw scratch2 MiB;
composite/charstring/edge work is independently bounded. No ambient font search
or full Unicode shaping is inferred. Writing/line functions return actual
owned guest structures and stable step views; lines have at most4096 steps in
a17-page arena. Reinitialization, cursor changes, ordered outputs and line
destruction publish atomically, retaining old state on a recoverable failure.

Entitlement Access exposes five source-profile operations over an explicit
local virtual full-package profile: SKU3, an empty default addon catalogue and
bounded configured local records. The empty catalogue deliberately replaces
the donor's three hardcoded GTA-V addons. This profile is no evidence of PSN
authentication, account ownership, purchases or network/backend success.
Initialize validates two nonnull pointers and clears exactly32 boot bytes;
its input remains opaque and no initialization state is invented. List/info
queries retain exact count,28-byte record and source-order overlap behavior.
The no-key result preserves every key byte. Staging/output/work limits apply
before atomic guest publication. Canonical module/library NpEntitlementAccess
version1.1/1 covers all five roles; only Initialize and List additionally have
their independently observed matching libSce-prefixed identities.

Trophy2 contexts register against real bounded UCP packages from the selected
`/app0` mount and unlock records from the selected per-title save directory.
Package localization, trophy/group metadata, saved timestamps and weighted
progress supply exact query records; no empty successful registration is
invented. The fifteen source contracts include both seven-argument arrays,
whose final count pointer comes from captured callee RSP+8. Recoverable output
failures leave records/counts and owned state unchanged. Contexts, cached
packages and per-user records share the handle quota and have a separate
trophy resource census. Source callback/handle no-ops and missing icon results
remain explicit; PSN and callback delivery are separate work. Source pins,
parser limits and save namespace are documented in
[upstream provenance](../../docs/upstream-provenance.md).

Universal Data System implements the pinned forty-one-role family. Contexts
register independently and posted events use the same package/user records as
Trophy2. Typed int32/uint32/uint64 properties, event deep copies, supported UDS
extraction rules, explicit unlock events, timestamps and linked platinum awards
feed real per-title persistence. Each opaque object has actual owned RW/NX guest
backing and an authenticated mapping identity; destroying an event also retires
its embedded property view. Logical UDS records and actual mappings both enter
the shared resource census. Save replacement is atomic and failures retain the
previous RAM/file state. Source no-op arrays, scalar key erasure, literal `{}`
serialization, zero statistics and unused floating values retain their source
effects. These fallbacks do not measure storage use or serialize event content.

Sixteen exact libkernel event-queue exports retain positive int64 handles,
byte-string names, user level/edge registrations and periodic/high-resolution
timers in one bounded owner. Repeated user triggers replace pending data while
preserving active order. Each wait scans a pending event at most once; levels
requeue behind other events, edges clear after delivery and one-shot timers are
removed. Re-add updates source deadline/userdata fields without changing the
original flags or pending data. Periodic timers count elapsed periods.

Event records use the source's exact 32-byte layout. Record/count publication
is transactional; a fault consumes no event. Overlapping count output retains
the source's final four-byte store. Getters read only their actual field and
preserve source return widths and null behavior. Creation observes its staged
handle store when the name overlaps it; different virtual aliases of edited
physical bytes are explicitly refused by the native adapter. Name limits and
shared handle/work/scratch/byte budgets apply before state publication.

`begin_event_queue_wait` returns an owner-retained continuation when no event
is ready. Core parks its actual source-bound caller and schedules other guest
threads. Ready checks and completion recheck live state, including competing
consumers, queue closure and timer deadlines. Closure wakes pending calls with
EBADF; timeout writes count zero and preserves its input. Empty indefinite
waits are not fabricated successes; no notifier/deadline remains a deadlock.
Direct synchronous invocation admits ready events or a zero-time poll, while
blocking callers require the continuation API. The source GRAPHICS filter is
registered by the checked AGC bridge and receives only actual retired release
interrupts. An exclusive prepared delivery admits target storage/work/counters
before the GPU label write; abandonment publishes nothing. Graphics flags remain
zero and the filter always resets after delivery, with saturated counts and the
newest context. Video and AMPR completion producers remain separate integration
work; these queues do not claim graphics presentation.

The reviewed AMPR lifecycle binds command buffers without executing asynchronous
file or memory requests. Base construction zeros exactly24bytes and erases owned
metadata; APR construction initializes only its two reserved words, normalizing
duplicate/overlapping/adjacent zero spans before one atomic guest transaction.
SetBuffer validates the source alignment/size domain and retains the actual
binding without allocating or dereferencing the command backing. Reset uses the
retained snapshot or backing-address alias even after direct guest header edits;
queries read the exact live scalar spans. An empty snapshot remains retained when
Reset returns EPERM, as in the source. Clear returns the actual old buffer and
clears only size/base while releasing metadata; destruction does not free guest
backing, and the APR destructor preserves the source ignored-input no-op.
Alias replacement/removal never revives a superseded owner. Bindings share the
kernel's simultaneous resource limit, with bounded fallible metadata admission
before outputs and infallible publication afterward. Failed output preserves
headers, snapshots and aliases. Normal/prefixed module/library spellings retain
exact versions. Other command kinds and asynchronous event delivery remain
unfinished and must share this owner. Lifecycle calls alone perform no IO.

APR read-record construction retains its seven-argument ABI, including the file
offset at captured callee RSP+8. It writes the source's complete 20/24-byte record
(opcode `0x17`, remaining bytes zero) while retaining the actual ID, destination,
size and offset in the owning command buffer. It increments the live count,
advances the cached offset and retains gather continuation/type flags. Ordered
staging handles record/header virtual overlaps before one checked transaction;
distinct physical aliases remain explicit refusals. Numeric destination zero
and unresolved IDs remain valid at append time: queuing performs no file read or
destination write. The matching size helper returns 20/24 or zero-extended source
kernel error bits, independently of POSIX errno.

Exact objects with queued commands precede backing aliases during reset/lookup;
append itself uses the exact object. Source first-use header caching survives a
later space refusal. Queue capacity is admitted before output, and failed output
cannot advance retained commands, offset or gather state. Reset/SetBuffer clear
queued commands and gather state; Clear/destruct release their accounting. Old
record bytes and reset's existing type bits remain. An independent default limit
allows 65,536 retained commands across all buffers, with fallible capacity and
shared work/byte/scratch budgets. Submission must execute the retained arguments;
the mostly zero record bytes alone cannot reconstruct a read or certify its
completion.

The four exact libkernel APR submit/wait exports execute retained read records
against the same file-capability owner. Reads use current file sizes and bounded
positional chunks; short reads and EOF leave the remaining destination untouched.
An unknown ID or source IO error stops subsequent commands and reports an inner
kernel error plus record offset while preserving completed reads. Guest and
resource-policy failures stay typed. Queue metadata persists for repeated submits.
Result-bearing submits retain bounded completion receipts before execution,
publish ID then result separately after IO, and use the active-thread POSIX
return/errno domain. Wait consumes its receipt before rewriting the saved result;
reset/destruction do not consume receipts. Plain submit allocates no receipt.
Other record kinds and asynchronous completion events remain unsupported.

`Apr::ResolveFilepathsToIdsAndFileSizes` resolves each live guest path through
the existing read-only mount capability and returns a unique stable process-owned
ID and actual file size. Directory resolution returns `0x10000`; its retained
ordinary size is zero. Exact guest spellings and missing-path results are cached.
IDs are independent of file descriptors, use no path hash and retain no OS handle
per identity. Separate default limits admit 262,144 cached paths and 64 MiB of
retained name capacity, with fallible table admission and shared work/byte limits.
Each item reads its inputs after prior item outputs, so aliases that change later
inputs retain source ordering. Current-item virtual output overlaps are normalized
in ID/size/error-index order before one checked transaction. A first semantic
failure writes the source sentinels and index, preserves the successful prefix
and untouched suffix, and returns int32 `-1` with active-thread POSIX errno.
Successful calls preserve errno and the error-index output. Typed guest/resource
failures do not become fabricated errno results; a refused current transaction
retains no new cache entry. Distinct physical aliases remain explicit refusals.
Resolving metadata does not read file contents or execute APR command records.

The reviewed SaveData/SaveData_native base profile shares process-owned state.
Initialization ignores its optional input and records a receipt without clearing
events or resetting transaction identifiers. Creation issues monotonic positive
int32 identifiers without allocating storage from the caller's size; the pinned
delete-transaction function ignores its identifier and performs no deletion.
Prepare validates its fixed mount-point/header inputs and performs no mount or
persistent write. Commit reads the resource/mode header and, when the source
async-backup flag is set, queues the source completion event. This event is not
evidence of a durable backup or a saved game.

The preallocated queue retains the newest twenty 104-byte events. Reception ignores
the source diagnostic parameter and atomically writes the full front event before
consuming it; faults preserve the event, and an empty queue leaves output unchanged
with the source not-found code. With no live directory mounts, termination clears this queue without resetting
the transaction counter. Save-memory Setup2/Get2/Set2/Sync retain actual data and
metadata with source range/error ordering, four live entries and a 32MiB aggregate
logical limit including double buffering. Setup loads existing data or initializes
zeros; optional results report the actual prior size. Get publishes its data,
1328-byte metadata and icon length together after complete output preflight.
Set validates all five possible descriptors and stages updates before persisting
and publishing RAM. Sync reports actual storage errors through its return value
or a bounded type-three completion event. Termination releases retained memory.
The directory mount family shares this owner's busy and cleanup state.

TransferringMount opens real existing saves, and Mount3 creates or opens saves
according to the reviewed source mode and block limits. Sixteen process-owned
slots expose `/savedata0` through `/savedata15`; duplicate saves return busy before
the full-table error. Mount results preserve the source's initial zero store and
error ordering. New metadata is staged and flushed before publication; a failed
final guest output or filesystem admission rolls back a newly created save.
Umount2 removes its resolver entry while already-open file descriptors retain
their owned files. Live mounts block termination and conflicting save-memory
setup/sync/delete operations. Mount information reports the source's stored
block count in both block fields, not measured host free space.

Only mounted save capabilities allow file creation and writes. Read-only package
roots retain their restrictions. Save writes, positional writes, directory creation,
rename, removal and descriptor flush operate within the selected save capability;
Fsync performs an actual host flush, and reading a write-only descriptor fails.
Parameter/search operations retain the reviewed layouts, wildcard language and
partial-output ordering. Icons use original bounded, real sidecar persistence
where the upstream implementations are stubs. Backup completion events retain
their source behavior and do not prove that a backup was made.

Persistence requires an explicit existing dedicated per-title directory capability
(`probe-entry --save-directory <host-directory>`), separate from read-only game
data roots. An explicit `--save-title-id` binds launcher-supplied title metadata;
the older capability constructor uses `UNKNOWN`. An exclusive OS file lock
prevents concurrent sessions. PentaX writes
one bounded checksummed bundle per numeric user/slot using an exclusively created
temporary, file flush and same-directory replacement before RAM publication.
Failed preparation/write/replacement preserves the old RAM and bundle; corrupted
bundles return the source broken-data code. This is an original PentaX format,
not an automatic import of upstream saves; file flush/replacement does not promise
recovery from every storage-device or power failure. Directory saves use a
separate `directories/t<hex-title>/d<hex-name>` namespace, with a numeric user
child only for the four source save-memory names. Lowercase hexadecimal encoding
preserves case and trailing-dot distinctions without Windows name aliases.
Lookup rejects symlinks and reparse points; Unix also checks occupied-directory
device/inode identity. Search and recursive metadata/delete traversal are bounded.
The checked bundle format is not exposed as upstream `rawmemory.dat`; existing
upstream saves are never discovered, imported or migrated automatically.
No ambient host path or write to the game directory is granted. Independent
retained-memory, persistent-I/O, scratch, work and guest-transfer policies apply.
The adapter explicitly admits the full 32MiB output plus metadata while preserving
complete native permission and physical-alias checks before any copy.
Normal and native namespaces have separate reviewed spellings at exact module
1.1/library 1 versions, without cross-family fallback. Existing marker/descriptor
indices remain stable. The provider retains a 1024-entry admission ceiling;
its independent 64KiB image limit and four logical RX pages
plus one RW/NX data page still bound the actual generated layout.

System display queries expose the pinned emulated profile: HDR tone-map
luminance is three little-endian float fields (80, 1000, 0), written as one
checked 12-byte transaction. The safe-area query writes only ratio 1.0 in four
bytes, preserving its 128 reserved bytes. Null outputs return the source
parameter error; invalid spans and exhausted budgets refuse before output.
Both queries require exact module1.1/library1 scopes and the four reviewed
SystemService/libSceSystemService spellings. Their appended registrations keep
all earlier marker IDs stable. They do not query host monitor capabilities or
establish HDR rendering support.

Controller entry services retain the PadInit/PadClose normal zero results,
configured user/port/index rules for PadOpen/PadGetHandle, exact int32 error
words and the one-byte PadIsRemoteController output. Normal and reviewed
libSce-prefixed identities have exact module1.1/library1 scopes. User-dependent
calls retain the observed configured profile; work refusal occurs before that
observation. These entries do not imply a connected device or delivered input.
PadInit initializes process-owned input once without observing the user.
The seven runtime roles bind only exact libScePad module1.1 / libScePad library1.
Handle1 is checked before null/count errors; normal SCE int32 errors remain
zero-extended, while backing and quota failures are typed.

PadRead serializes actual owned120-byte snapshots. The default is explicitly
disconnected: neutral sticks, identity orientation, gravity along +Y and zero
input/connection counts. An empty queue yields one current record; a fixed64
queue retains real owner-supplied changes and returns only actual records.
Successful whole-span output precedes dequeue. Shared I/O, scratch, byte and
work bounds apply. All padding is zero; unchanged polls never invent input
events or timestamps. First PadInit captures microseconds from the kernel's
owned monotonic epoch, deliberately distinct from the optional native-counter
ProcessTime origin. Repeated initialization preserves input, queue and time.
Snapshot publication validates finite motion, bounded fields and monotonic time.

ControllerInformation serializes28 bytes from an explicit static profile plus
the actual snapshot's connection fields. Desired vibration2B, light-bar3B and
trigger120B requests persist only after checked inputs and full validation.
Trigger modes0..6 decode into the reference effect request; both modes are
validated even if their mask is clear. TriggerEffectState retains the source's
zero8B status rather than inventing a hardware receipt. Tilt stores the setting
and advances a checked reset generation on every accepted setter, including
repeated settings, so the supplying motion owner can reset its own filter.
This resolved-input/device boundary provides no host controller discovery,
desktop input bridge, motion fusion or physical rumble/light/trigger delivery.
Accepted requests do not claim those effects.

Hardware-model queries retain the pinned base-console profile: Trinity is false
and operation mode/submode are2/0. They never infer capabilities from the host.
Two exact four-byte outputs publish atomically. Overlapping/adjacent outputs
retain the source's mode-then-submode write order; disjoint gaps stay untouched.
Null source-aborting and overflow/backing/budget failures remain typed stops.

Guest mapping/protection calls decode CPU-write requests as readable/writable,
following x86 page semantics and the pinned CPU-write implication. Original
protection words remain unchanged in range queries, including non-access bits.
GPU-only mappings continue through their independent GPU access path. Explicit
native-memory capability declarations retain their own stricter checks; no
execution permission is added. See the
[Intel paging reference](https://cdrdv2-public.intel.com/825758/253668-sdm-vol-3a.pdf).

NP account services retain the pinned offline behavior independently of local
user login. Country/account/online identifiers are cleared to their exact source
spans before the signed-out result; state reports signed out, reachability is
unavailable and signup is false. No country, account identity, authorization or
network connection is invented. Source-fatal null outputs remain typed refusals;
reviewed parameter errors keep their distinct int32 codes.

NP requests have process-owned Ready/Complete/Aborted/Free states. A preallocated
table retains the source ceiling of 128 requests and the configured shared handle
budget; deletion releases that budget and creation reuses the lowest free ordinal
as in the source. Sync completion returns signed out directly, while async
completion returns success and retains the signed-out result for polling. Polls
do not consume it. Repeated completion records invalid-argument, abortion retains
the actual aborted result, and deletion makes the ID unavailable until reuse.
Source state/error ordering is preserved for availability, reachability, language
and premium queries. Age checks positivity without requiring an owned request,
matching its distinct pinned branch.

Availability clears its optional four-byte result before a missing-ID error;
premium clears exactly 33 bytes only for a ready request. Output faults prevent
completion, and policy admission precedes state mutation. Async parameter sizes,
actual field reads, fixed output spans and source error widths are checked without
allocating from caller-provided sizes. Requests and account state do not change
host scheduling or access online accounts. Callback execution, title-secret setup
and unreviewed network APIs remain unavailable. Exact NpManager/libSceNpManager
spellings retain module 1.1/library 1 checks and prior marker indices.

Identity and accessibility user services share one configured local guest profile. Global `[user]` settings
select its nonnegative 32-bit signed ID and 1 through 63-byte UTF-8 name; defaults
are ID 1 and `PentaX`. Fixed validated name storage keeps startup limits copyable
without allocating on runtime limit reads. Profile changes are refused after an
observable result from these services, while repeated application of the same profile
is harmless. No host account lookup or online entitlement is involved.

Both pinned initializers ignore caller arguments and retain an idempotent
receipt. Queries also work before initialization, matching the source. Initial
user, the four-ID login list and the once-per-process login event all read the
same profile. The event's eight-byte output commits before it is consumed;
faults or budget refusals keep it pending. Empty reception returns the source
no-event code without touching the output, and repeated initialization never
replays it. Name output preserves UTF-8 and writes only its bytes plus a NUL;
source-fatal truncation is refused before publishing a partial string.

The reviewed number aliases, age level and accessibility getters retain their
explicit source local-profile values with exact four-byte outputs. These report
preferences, not active host accessibility or parental enforcement. Null/user
checks retain each source function's order and specific int32 error code; fatal
source null cases remain typed refusals. The 40-byte game-preset getter honors
the requested partial clear while always writing its full eight-byte size header,
preserving every other byte. Read/output/scratch/work admission is checked before
publication. Reviewed UserService/libSceUserService scopes use exact module 1.1/library 1 versions and
preserve existing service markers.

`sceUserServiceGetPlatformPrivacyWs1` implements the public AnyPS5 two-argument
getter: the signed int32 user ID is ignored, null output returns the normal
`0x80960005` source error, and nonnull output receives exactly four zero bytes.
Checked full-span output admission precedes the store; invalid backing and
resource limits remain typed refusals. The getter works independently of
initialization and login delivery and does not freeze the configured profile.
It reports the reference's local zero value without consulting host privacy or
online accounts. Only `libSceUserService` module 1.1 with
`libSceUserServicePlatformPrivacyWs1` library 1 and NID `D-CzAxQL0XI` is bound.
This explicit PentaX compatibility identity retains exact namespace/version/type
checks. Existing markers remain fixed and no general NID fallback is enabled.

`AppContentInitialize` ignores its source-reserved input, clears an optional
40-byte boot object and queues one entitlement-update notification on every
successful call. The process retains an initialization receipt. Optional title
metadata is unavailable in this selected-input profile; initialization neither
reads extra metadata files nor grants DLC ownership. Content mount operations
remain unfinished.

`AppContentAppParamGetInt` writes exactly one int32 without requiring content
initialization or changing its event queue. Parameter0 returns the pinned FULL
SKU profile constant3. With no supplied SFO metadata, parameters1..4 preserve
the source's missing-value zero; loaded title-metadata integration remains
unfinished. Null outputs and unknown IDs retain explicit typed refusals for
source abort branches; an unknown ID preserves the source's initial zero store.
Checked output and resource failures remain typed. This profile value neither
verifies an entitlement nor grants content access.

`SystemServiceReceiveEvent` and `SystemServiceGetStatus` share that process-owned
FIFO, preallocated and bounded by `Limits::max_handles` independently of live
handle usage. Queue/work admission precedes boot output and notification
publication. Reception writes the complete 8196-byte event object before removing
the front notification; a failed output or budget refusal leaves it pending.
Empty reception still writes type `-1` and8192 zero payload bytes, then returns the
source no-event code. Null event/status outputs return the source parameter code.
Status writes the136-byte source span with the actual pending count and current
dialog active state. Reserved bytes and tail padding are cleared; no background
or VR dialog is admitted by this profile. All outputs honor scratch/byte/work
limits. Reviewed module/library spellings retain exact1.1/1 version checks;
existing provider marker IDs remain unchanged.

The ten reviewed NetCtl exports preserve the pinned fork's forced-disconnected
network-control profile. Queries work before initialization; Init preserves
registered callbacks and their delivered events, while Term clears the eight
slots. State/V6 results, the 16-byte NAT object, 256-byte info union and result
queries retain exact spans, source errors and null/size ordering. No host adapter
or socket is queried, and no connection is invented. Registration uses the first
free slot and checked ID publication; unregistration preserves invalid/missing-ID
distinctions. CheckCallback invokes each pending disconnected-event callback in
slot order through the prepared native owner, with the original event/argument.
Only a verified actual return marks delivery; repeated checks do not duplicate
completed events. Later slots are observed live, and completed prefixes survive
a later failure. Recursive checks remain explicit typed refusals. Raw kernel
dispatch cannot claim a pending callback executed without the prepared owner.

Thirteen network startup exports adapt pinned `network.cpp/h`, `libNet.cpp`
and `errno.h`: `NetInit`, `GetNetErrorAddr`, pool create/destroy, resolver
create/destroy/abort, SSL init/term and CA get/free, and HTTP init/term. Exact
Net/Ssl/Http and reviewed `libSce` spellings retain library version1 and module
version1.1. `NetTerm` and the internal `FinishNetCall` have no export in this
reviewed registration subset.

PentaX additionally binds only `SslInit` and `SslTerm` with both module and
library named `libSceSsl`, module version2.1 and library version1. These two
exact requested scopes were observed by the closed metadata inspector; the
pinned source declares SSL module1.1 only. Reusing the reviewed one-argument
`SslInit(uint64 size)` and `SslTerm(int32 id)` contracts and the same owned
lifecycle state is an explicit PentaX compatibility inference, not an upstream
2.1 declaration or complete SSL SDK support. It does not admit CA operations,
unprefixed names or other versions under the 2.1 binding.

`NetInit` requests actual platform socket-backend initialization, including
Windows `WSAStartup(2.2)`, and preserves the pinned zero result even if backend
initialization fails. Successful backend initialization is retained; a failure
permits a later retry. It does not clear the owner tables. `GetNetErrorAddr`
returns a separate initially zero four-byte cell for the current guest thread,
independent of POSIX errno. Net wrappers preserve successful-call errno and
convert negative results with the source `result - NET_ERROR_EPERM + 1` rule,
returning the original error. SSL and HTTP calls do not change net errno.

Pools copy a checked bounded name and retain their signed size as metadata,
without allocating pool backing. Lowest-free pool IDs1..31 and resolver IDs0..31
are reused; resolver zero is valid. Negative pool sizes follow the source's
accepted signed metadata contract. Nonzero pool flags, zero pool size and an
unsafe null pool name remain typed refusals. Resolver creation rejects null
names/nonzero flags with EINVAL, then requires a live pool; invalid pools and
resolver exhaustion both return RESOLVER_ENOSPACE. Resolver abort is the pinned
explicit no-op: it ignores both arguments and performs no DNS or cancellation.

SSL and HTTP each reuse 32 lowest-free typed slots, encoded as `0x20000 | slot`
and `0x10000 | slot` respectively. Their nonzero uint64 sizes are metadata;
zero sizes remain typed source-abort refusals. HTTP creation requires both a
live pool and SSL context; invalid dependencies and exhaustion return its source
OUT_OF_MEMORY error. Destroy/term releases only the selected object: pool and
SSL destruction neither checks nor cascades into dependent resolver/HTTP state.
The source reuses numeric identities without generations.

CA get/free check null output before SSL identity, then transactionally clear
exactly 24 bytes: two null pointers and a zero uint64 count. Get returns the
source NOT_FOUND error after writing; Free returns zero without reading or
freeing prior pointer values. No certificates are loaded. These bounded
lifecycles do not provide pool backing, connectivity, DNS execution, socket or
packet operations; checked backing, permissions and resource policies still
apply to every actual guest-memory effect.

The twenty-one HTTP2 metadata exports adapt `LibHttp2` in pinned `libNet.cpp`
at `209dffe9032d72749257ce18680d506b891284de`, with exact Http2/libSceHttp2
library1/module1.1 scopes. They cover context/template/request create/delete,
headers and content length, three boolean options, SSL option bits, two callback
registrations and six timeout setters. This subset provides no send/read,
response or async operations, pool backing, transport or TLS guarantee.

HTTP2 initialization retains all four arguments without validating Net/SSL
dependencies, pool size or signed concurrency. Each of its three object families
has an independent monotonic ID starting1; deletion never reuses or resets an
ordinal. Context termination cascades through its templates and requests;
template deletion cascades through its requests. Missing objects preserve the
source INVALID_ID result. PentaX bounds the shared live-object quota and refuses
counter overflow rather than reproducing unbounded maps or signed overflow.

Template user agents accept null as empty; other text is opaque bytes. Required
null method/URL/header inputs preserve the source NULL_POINTER-before-ID order.
All retained text capacities share a checked per-process `max_io_bytes` limit;
header records share an aggregate `max_handles` limit. These are PentaX resource
policies in addition to the context/template/request records sharing the kernel
handle budget. Strings and record capacity are admitted before publication.
Header mode is ignored and duplicates append. Options select a request before a
template when their IDs collide; a context alone owns no options. New requests
start with independent false/zero/null options rather than copying their
template. Callback words are retained as opaque data, including null and
noncanonical values, and are never invoked. Metadata calls write no guest
outputs and leave both Net and POSIX errno unchanged. The original production
worker consumer exercises all twenty-one exports, real ID collisions, exact
errors and cascading cleanup, then compares the final owner census with the
running handle count; it executes no HTTP transport operation.

The three reviewed RUDP bootstrap exports follow pinned `libRudp.cpp`.
`RudpInit` retains an owned initialization receipt, ignores both memory-pool
arguments and preserves the registered handler and argument. `RudpSetEventHandler`
retains both guest words, including a null handler with a nonnull argument;
neither is dereferenced or invoked. `RudpEnableInternalIOThread` is the source's
explicit no-op: stack size and priority are ignored, and no I/O thread is created.
All three calls retain the source zero result, and the latter two require no
prior initialization. This bootstrap subset provides no RUDP transport, packets
or event delivery.

Kernel semaphore waits use a bounded shared queue and the exact retained guest
thread priority. FIFO and stable priority order follow the pinned source; grant
scanning can satisfy a smaller request after an unsatisfied larger one. Signal
reserves each granted count once, before its parked thread resumes. Poll cannot
steal a reserved grant. Cancel publishes the actual unready count before resetting
the semaphore and completing those waits as ECANCELED; delete completes unready
waits as EACCES. Previously granted waits retain their success across either
operation. Opaque globally unique tickets reject foreign owners or consumed waits.

Finite waits accept the complete uint32 microsecond range and use a monotonic
deadline. Completion writes exactly four remaining-time bytes before consuming
the ticket; a fault or work refusal retains its result/reserved grant for retry.
The runtime captures the original syscall, parks its CPU/TLS/nested frames and
completes that exact token only when ready. All-parked finite waits use short idle
slices within an explicit cumulative requested-sleep budget; indefinite waits without
a notifier produce a real deadlock result. A large finite deadline is not an
unconditional host sleep. Raw scalar calls without continuation capture still
refuse a wait that would block. POSIX/direct pthread semaphores retain their
separate profile. Event flags use the cooperative continuation profile below.

`CommonDialogInitialize` and `CommonDialogIsUsed` implement the pinned common
dialog base profile, with exact CommonDialog/libSceCommonDialog spellings at
module1.1/library1. Initialization is idempotent and retains a process-owned
receipt. No dialog-open service is admitted yet, so the shared active state is
false; future dialog-open/close integration must update that same state. Neither
zero-argument call reads caller registers or guest storage. They do not display
a modal panel, complete user interaction or establish general dialog support.

The pinned `FiberInitialize`, `FiberInitializeInternal`, option initialization,
finalize, information and rename APIs retain real caller-owned guest objects.
`Fiber` and `libSceFiber` module/library spellings are explicit registrations at
module1.1/library1; ordinary exact version matching is unchanged. Initialize's
eighth argument and Internal's ninth argument come from the captured callee stack,
with checked16/24-byte reads after its return address. Build versions and internal
flags retain their uint32 semantics independently.

The208-byte source object has explicit little-endian offsets, magic and idle state,
entry/argument/context bounds, a truncated31-byte byte-string name, version flags,
and initially empty saved context. Option initialization writes exactly128bytes;
information writes128bytes with its defined zero margin/padding. The object and
context's eight-byte stack marker publish atomically before retained lifecycle
state changes. The rest of caller context storage is not cleared or enlarged.
No host pointers, native CPU context or guest execution authority are invented.
Null/alignment/range/option errors preserve the source int32 fiber codes; backing
and policy failures remain typed. Reinitialization replaces an existing record;
finalization transitions idle to terminated only after its checked state write.
Initialized/finalized records remain bounded and share the handle budget until
session destruction, so copied magic cannot manufacture an owned fiber.

Context-less initialization is admitted as metadata; actual backing must be
allocated/admitted before any later run. PentaX does not copy the pinned host
exception/HLE stack workaround: its checked memory operations and HLE already
use separate host storage. Fiber run/switch/return, migration, per-thread fiber
selection and actual saved CPU execution are not implemented by these APIs.

`KernelSetVirtualRangeName` and `KernelClearVirtualRangeName` retain semantic
labels on the actual paused native owner, including mapped source/stack pages
outside kernel allocation records. Names are bounded byte strings up to 31
bytes; null returns EFAULT and a 32-byte unterminated prefix returns ENAMETOOLONG.
Endpoints saturate, holes and empty ranges have no effect, and clear assigns
an empty name. Naming never changes payload, permissions or physical aliases.

Byte-granular labels are separate from page-aligned backing records and bind to
fresh mapping lifetimes. Unmap/remap cannot resurrect an old name, even through
physical aliases; later edits collect stale metadata. `QueryProtection` refines
its original range bounds at semantic name changes, preserving full protection
bits and the mapping's initial default label. Name metadata has independent
bounded owner work/scratch/record budgets. Unbound `GuestAccess` refuses naming
instead of discarding it while returning success.

POSIX semaphores use the pinned 16-byte guest prefix: `magic:u16`, `nameid:u16`,
`has_waiters:u32`, `count:u32`, `flags:u32`. Owned counts are keyed by guest
address, independent of guest-edited count bytes and tagged kernel semaphore
handles. Init/reinit publishes the complete prefix; wait/try/post synchronize
the first12 bytes while preserving flags. Destroy clears all fields except
nameid. Getvalue writes exactly int32. Live records share the handle budget;
reinit reuses its record. Counter range is0 through `i32::MAX`.

All eight `sem_*` operations (including relative and absolute timed waits) have
explicit `Posix`/`libScePosix` exports. Seven `PthreadSem*` direct exports share
the same state and return kernel int32 errors without changing errno. POSIX
errors return int32 -1 and update the active thread errno; success preserves it.
Prefix updates and errno writes form one checked transaction before count or
lifecycle publication. Absolute timespec validation precedes lookup/acquisition,
as in the source. Available counts acquire immediately; an empty try returns
EAGAIN and an already expired wait returns ETIMEDOUT. A wait requiring another
thread remains Unsupported until the scheduler is implemented. `pshared` flags
are retained metadata, not host interprocess synchronization; direct init
requires flag zero and its diagnostic name does not create a named semaphore.

The pinned `SysmoduleLoadModule`, `SysmoduleUnloadModule`, `SysmoduleIsLoaded`
and zero-argument `SysmoduleLoadModuleInternalWithArg` profile maintains a
process-owned HLE activation table keyed by the low unsigned 16-bit module ID.
Loads retain references; unload removes only the final reference. Queries and
unloads of absent records return the source int32 `0x805a1001`. Every live
record shares the kernel handle budget, while repeated retains use the same
record. Lookup work, reference overflow and allocation are bounded. Internal
load writes exactly int32 zero through its required guest output before
publishing the reference; failed output or admission changes neither state nor
backing. Nonzero low32 argument values and null output remain typed stops,
replacing the source's fatal assertions. Module/library spellings `Sysmodule`
and `libSceSysmodule` are exact explicit exports at library1/module1.1.

For the thirteen publicly listed application-module IDs, the native runtime
can suspend a load request and select its exact `/app0/sce_module/*.prx` file
from explicit `--module-file` capabilities. It reads and appends that module,
rebinds imports and executes its actual initializer with three zero arguments
before publishing the Sysmodule reference/output and resuming the caller with
zero. An already mapped image is retained without restarting its initializer;
a reentrant self-load does not mark the outer initializer complete. Reference
unload does not unmap the image. Failed initialization never resumes its caller.
Unlisted IDs and absent explicit capabilities retain the reference-only profile.
No directory sweep, firmware search or replacement exports are introduced.
`KernelLoadStartModule` keeps its distinct actual handle and initializer result.

`SystemServiceParamGetInt` reads validated, process-owned `SystemConfig`
preferences. Both `SystemService` and `libSceSystemService` spellings are
explicit exports (library 1, module 1.1), including their reviewed combinations.
The generated image declares each module identity; ordinary exact linker
matching, versions, marker attestation and service budgets remain in use.

Language IDs 1 and 400 share the configured language; date/time formats,
base UTC offset, daylight-saving, parental level, captions, screen-reader
preference and confirm button use the corresponding settings. Output is an
atomic four-byte guest write. Null returns the source int32 parameter error
`0x80a10003`, without changing POSIX errno. Unsupported IDs remain typed stops.
PentaX defaults to UTC instead of the pinned source's fixed +180-minute offset.
Reporting preferences does not implement host accessibility or parental
enforcement. Launcher UI localization is independent of guest language.

`KernelGettimezone`, `KernelConvertUtcToLocaltime` and
`KernelConvertLocaltimeToUtc` use the same configured base UTC offset and active
daylight-saving state. `daylight_saving_offset_minutes` defaults to 60 and admits
-180 through 180 minutes; it contributes only when daylight saving is enabled.
The configuration is a fixed process snapshot, with no automatic seasonal or
historical time-zone rules and no ambient host-zone lookup. This explicitly
replaces the pinned source's current host-zone sample.

The timezone record contains two int32 words (minutes west and source DST type).
The UTC conversion's optional 16-byte record contains the original int64 UTC
seconds followed by uint32 base-east and DST seconds; its separate optional DST
output is uint64. The inverse ignores its reserved argument, requires the timezone
record and writes optional int64 UTC plus int32 DST seconds. All outputs commit
together. Null errors retain the int32 kernel domain and leave POSIX errno alone.
Overlapping outputs remain unsupported; timestamp overflow returns EINVAL before
any output is published. Optional unrequested timestamps do not require arithmetic.

`Service::registrations()` contains exact pinned source registrations,
including aliases and explicit unsupported services. The profile
is libraries `libkernel` and `Posix` version 1, module `libkernel` version 1.1. `resolve`
requires every scope/version component and a matching NID; it has no NID-only
fallback. Registration presence does not imply full implementation.

`Kernel::invoke(service, CallArgs([u64; 6]), &mut GuestAccess)` retains all
AMD64 SysV library register arguments: RDI, RSI, RDX, original RCX, R8 and R9.
A mediated SYSCALL marker must capture arguments before synthesizing continuation RCX/R11.
Original RCX is not the marker continuation and is not raw syscall argument
four in R10. `invoke_with_stack` and `invoke_library_with_stack` additionally
accept captured callee RSP. Only `KernelMapNamedDirectMemory` reads the seventh
argument at RSP + 8, through checked and budgeted guest memory. Missing stack
capture is Unsupported; invalid backing remains a typed error. Source kernel
errno is `0x80020000 + errno`; host errno and POSIX `-1`/thread-local errno are separate domains. `Failure::Errno` preserves
the source kernel code, while policy limits, unavailable behavior and backing
failures remain typed failures for the trusted caller to handle explicitly.

`Kernel::invoke_library` applies the pinned direct-library return domains after
exactly one normal invocation. Supported source errno from read/write/pread/lseek is
sign-extended from the source negative int into an int64 result. Audited int
returns from file metadata/open/close/reachability, memory, mutex, semaphore/event-flag
and supported clock/sleep functions use the low 32-bit kernel code, represented
by zero extension in PentaX's RAX result. Success values are unchanged. Pointer,
unsigned, thread and loader services have no blanket conversion; Dlsym retains
its separate explicit null/miss transactions. Unsupported behavior, limits,
allocation and backing failures remain typed, as do errors from unbound return
domains. Overlapping output transactions are an Unsupported admission policy,
not a guest EINVAL. Direct kernel results do not set POSIX errno; raw-syscall
carry conventions remain outside this library-call profile.

`KernelWrite` accepts stdout/stderr; the pinned `LibKernel::write` wrapper also
accepts descriptor zero. PentaX copies the complete checked binary range into
owned per-stream buffers instead of the pinned wrapper's discarded output.
`take_output` transfers those bytes to the application and releases their
retained budget. `output_tail` borrows a bounded suffix without consuming bytes
or changing accounting. `output_usage` distinguishes cumulative accepted bytes from
currently retained bytes. No payload is printed into the worker protocol or
written to a host console or game file. The default aggregate pending limit is
8 MiB; each transfer also uses the existing scratch, I/O and guest-byte budgets,
and is limited to `i32::MAX`. Invalid guest backing or budget exhaustion leaves
pending output unchanged and remains a typed stop. Applications must drain the
buffers to continue beyond the pending limit; the diagnostic worker does not
yet expose a console viewer or drain them.

Direct writes check null buffers before descriptor validity, including empty
writes, and return sign-extended int64 kernel errors. The POSIX standard-stream
wrapper permits a null empty write, preserves errno on success, and returns
int64 -1 with the active thread's errno for admitted source errors. All ordinary
descriptors currently represent read-only files, so writing them returns EBADF.
Writable files and sockets remain unimplemented. This bounded output device is
an explicit PentaX behavior beyond the pinned standard-stream no-op wrapper;
it does not reproduce its unchecked signed-size truncation or fatal branches.

The caller retains exclusive access to the same actual guest memory through
`GuestAccess`. Reads validate the whole range. `apply` validates an entire
memory effect and every output. Recoverable errors leave backing unchanged;
partial irreversible decommit or failed rollback must poison the owner and stop
execution. Overlapping outputs are refused before the adapter is called.
Fresh physical/flexible allocations are zero-initialized; direct views alias
the retained physical backing and preserve bytes across unmap/remap. Every
host view stays NX. Mapping effects unsupported by an adapter must return
`GuestError::UnsupportedMapping`; accepting a request without performing it is
invalid. A new-map transaction may write within its prospective writable map,
as used to materialize the initial pthread prefix. An adapter must not pair
detached bytes with a live session or race guest execution.

The configured dynamic virtual window must be selected by the trusted process
owner, outside its existing images/stacks/TLS. Actual backing collision checks
remain mandatory. Kernel records cover only effects successfully admitted by
this owner. Process teardown drops the backing owner as well as the kernel;
dropping this service object alone does not unmap caller-owned guest storage.

`KernelIsAddressSanitizerEnabled` reports disabled, matching the pinned profile
and the absence of guest ASan instrumentation. This query does not describe host
build sanitizers or claim that guest sanitizer services are implemented.

## Real state and explicit boundaries

The exact POSIX `pthread_once` export supports actual initial-thread
initializers through the prepared native owner's suspended-call mechanism.
Null control or initializer pointers and unknown states return positive
EINVAL; DONE returns zero without fetching a nonnull initializer. State zero
admits a callback request. Core checks initialized immutable executable source
and live permissions, and the worker prepares a bounded frame below the
caller's red zone before IN_PROGRESS is written. Only an opaque actual return
matching owner, executor, call sequence, target, stack/guards and arguments
allows DONE to be written. The callback has void result: its numeric return
does not become a module error or the once result. Guest errno is unchanged.

The same stack, TLS, floating-point environment, service state and cumulative
execution limits remain in use. Different once controls can nest, including a callback that loads and
initializes another explicitly bound module before returning in an appended
source epoch. The existing initializer counter includes completed once calls.
Pending callback records and nested frames are bounded at 32. Invalid targets,
failed output, stale/replayed admission, unreturned callbacks and unavailable
execution remain typed stops; none may publish a fabricated successful return.
Contended states 2/3, same-control recursive waits, concurrent guest threads
and signal/cancellation behavior remain unsupported. Waiting does not become
a guessed errno. The plain kernel invocation API cannot execute callbacks;
its admission and backing-transition methods do not prove native execution.

Pthread keys retain the source's 256-entry process table and lowest-free-index
allocation. Key creation writes exactly four guest bytes before committing
allocation; deletion advances its generation and releases the shared handle
resource. Each initial-thread value records that generation, so deleting and
reusing a key never exposes its previous value. Values are full opaque u64
guest words and are never dereferenced or called as host pointers. Invalid or
deleted getspecific returns null, while create/delete/setspecific retain their
audited int32 kernel or positive POSIX errors. Pthread calls preserve errno.

The checked source/thread binding is required before key payload access.
The fixed key table owns 4 KiB; the initial thread lazily allocates a separately
bounded 4 KiB value table on its first nonnull setspecific. A null set before
that needs no allocation. Per-key allocation uses the shared handle budget;
source exhaustion of all 256 keys returns EAGAIN, independently of an earlier
configured resource-limit stop. Failed guest output or allocation publishes
no partial key/value. Generation wrap is refused rather than aliasing stale
values. Key deletion does not run destructors. Callback registration retains
the guest callback word; executing thread-exit destructors and additional
guest threads remain unsupported. Outer diagnostic return is not a successful
thread-exit operation or a complete pthread lifecycle.

Mutex and mutex-attribute services publish owned guest addresses into checked
eight-byte slots. Each has a zeroed RW/NX page containing the pinned 256-byte
mutex or 64-byte attribute prefix; page padding contains no host-private state.
Rust retains the locking metadata separately. Explicit initialization, copied attributes,
types 1 (errorcheck), 2 (recursive), 3 (normal) and 4 (adaptive), ownership,
recursive counts, trylock, unlock, busy destroy and zeroed destruction outputs
retain the pinned behavior. A zero slot lazily creates the default mutex;
exactly 1 lazily creates an adaptive mutex. Failed slot publication leaves the
object, address cursor, mapping and shared handle count unchanged. Stale or cross-kind identities
and overwriting a live object are refused explicitly.
Creation maps and publishes the slot in one transaction; destruction unmaps
the whole page and clears the slot together. Failed destruction preserves both.
Guest mapping calls cannot unmap or protect these internal pages. Each live
object and its mapping consume one shared handle plus 16 KiB of mapped backing.
Destroyed addresses are never reissued to pthread objects, preventing stale
copied slots from naming replacements. The monotonic cursor is bounded by the
configured virtual window; exhausting it returns ENOMEM even if earlier pages
were released. This implements the reserved prefixes, not the host-private tail
or a complete firmware pthread layout.

An actual paused guest thread with matching source-bound TLS is required.
Unbound adapters refuse before guest I/O. Self-lock on a nonrecursive mutex returns EDEADLK,
trylock returns EBUSY and an unowned unlock returns EPERM. A source relative
timeout uses its low 32-bit microseconds; a POSIX timedlock validates the full
absolute realtime timespec and converts it to the source's clamped relative
microseconds. Free locks acquire immediately. Native blocking and timed-lock
calls enroll the active thread in a bounded cooperative wait and leave the
original syscall pending. Another guest thread can release the mutex; only
then may the matching waiter acquire it and complete that continuation. Timed
waits use real monotonic deadlines and return ETIMEDOUT in the correct result
domain. A zero timeout tests availability immediately. Direct synchronous
invocations without a continuation still refuse foreign contention; bounded
self-timeouts retain their existing synchronous behavior. Signals and
priority inheritance/protection scheduling remain unsupported. Protocols 0/1/2
retain the pinned Windows metadata contract: a setter stores NONE/INHERIT/PROTECT
and initialization copies it independently of later attribute changes/destruction.
The source does not apply winpthreads priority scheduling. All three stored
protocols use actual exclusion and cooperative waiting under that profile.
Other protocol values retain the source-abort
boundary as a typed refusal. Only the low int32 argument is consumed, with no
fabricated EINVAL for source-aborting inputs or changes to guest errno.

Wait tickets bind a globally unique serial, resolved live mutex and admitted
thread. Readiness does not grant ownership; completion requires that exact
selected thread and consumes the ticket once. Explicit waiters and condition
reacquisition claims prevent premature destruction. A native executor phase
check validates the original pending token before any wait can transfer
ownership. Polling does not create another service attempt or reread a replaced
caller slot. Exhausted queues, work limits, stale tickets and wrong contexts
leave ownership unchanged. An all-parked indefinite cycle remains a typed
deadlock; no successful lock or unearned unlock is fabricated.

Direct mutex calls return zero-extended int32 kernel errors. POSIX pthread
wrappers return positive errno values, independently of the file wrappers'
`-1` convention, and never allocate or change the guest errno cell. Their exact
library scopes and signatures are retained, including the two-argument POSIX
init and libkernel-only absolute timedlock export. This does not establish a
complete pthread, firmware TCB/DTV, thread scheduler or static-unload ABI.

Condition variables and their attributes use the same transactional owned-page
lifecycle, with zeroed 256/64-byte reserved prefixes and copied clock selection.
Signal and broadcast lazily initialize zero slots; a nonzero slot must identify
a live owned condition. Direct `PthreadCondWait` and scoped POSIX `pthread_cond_wait`
enroll the active thread in a bounded shared FIFO queue and release its complete
recursive mutex ownership. Signal selects the first unnotified waiter; broadcast
notifies all current waiters. Empty notifications are not retained for future waits.
Readiness also requires the mutex to be free: completion restores its recursive
count before consuming the opaque, globally unique ticket. The runtime must park
and later complete the original syscall under matching TLS/CPU state; this crate
does not execute or host-block a guest thread. Admission and polling work are
bounded, with at most one pending condition wait per admitted thread. Mutex
destruction refuses while any condition waiter still retains it; condition
destruction refuses while unnotified waiters remain. Timed waits, targeted wakeups,
signal dispatch and module-unload cleanup remain unsupported. Destruction revokes
backing and clears the slot atomically. Stale or
cross-kind slots, reinitializing live objects and failed publication retain
explicit refusal/rollback behavior. POSIX wrappers return positive errno values
without changing the errno cell; the same signal identifier has distinct direct
kernel and Posix-library result domains. Only pinned scopes are registered.

`ReadOnlyRoot::directory` consumes an explicit directory handle and reopens
it with the capability resolver's required semantics. Lookup opens requested
regular files on demand inside that root, with no admission-time enumeration
or per-file inventory. Windows handles deny writers/replacement while an open
descriptor or query retains them, and the root remains protected from rename.
Host changes between separate file opens remain possible; read-only does not
mean a snapshot of the directory tree. Guest code cannot create/write/remove
host files. Canonical guest names, DOS device/ADS/ambiguous alias rejection,
bounded names and resolver work complement `cap-std`'s traversal/symlink
containment. Metadata queries use the actual opened handle.

Directory-capability roots support read-only directory opens, ordinary reads,
seek, fstat/stat, getdirentries/getdents and reachability. The existing file
inventory remains an explicit set of regular files. One trailing slash is
accepted for directories; a regular file with a trailing slash or O_DIRECTORY
returns ENOTDIR. Other canonical path and host device/alias rules remain.

Directory opens capture an owned, bounded list with the pinned source's
512-byte blocks: little-endian inode hash, record length, type, byte name
length and terminated UTF8 name. Four-byte-aligned records never cross a
block; each block's last record extends through its zero padding. Logical
dot entries are added without host parent access. Other entries retain host
enumeration order and each entry's own type, including Windows directory links
without following them. The source hash's reversed final 1..3 bytes are retained.
Unreadable enumeration, non-UTF8 names and names over 255 bytes stop explicitly.

Each descriptor owns its captured bytes and independent cursor. Ordinary reads
can return partial records, but getdirentries/getdents require a 512-aligned
position within the snapshot and int32 count at least 512; count rounds down
to complete blocks. Getdirentries optionally writes the old int64 position.
The complete payload/base transaction commits before advancing the cursor;
bad backing, overlap and budget failures leave those outputs and position
unchanged. EOF does not access an unused nonnull buffer, but still writes a
requested base output. Pread on a directory remains source Unsupported.
Directory fstat uses the retained packed length; stat captures a fresh bounded
list. Both expose directory mode 040777, 512-byte blocks and source current times,
never a host directory inode length.

Defaults bound each snapshot by the 1 MiB scratch policy and aggregate retained
buffer capacity by `max_directory_bytes` (8 MiB). `Usage::directory_bytes`
tracks actual retained capacity and close releases it; descriptor/work/I/O
budgets remain shared. Allocation/admission failure publishes no partial
descriptor. Host changes during enumeration may be observed; the retained
packed buffer itself stays unchanged. Unbound names have no ambient fallback.

The separately scoped POSIX wrappers cover open/close/read/pread/lseek/fstat,
stat, clock_gettime/clock_getres and nanosleep, with only their exact pinned
exports and aliases. Recognized source errors return -1 and write a real
four-byte guest errno cell: read/pread/lseek and the source's int64 fstat
return all 64 bits set, while int wrappers use zero-extended 32-bit -1.
Successful calls preserve errno. POSIX close accepts descriptors 0..2 without
closing host streams; read validates a null buffer before descriptors and
refuses actual standard-input reads. Unported delegates, backing faults,
allocation and resource limits remain typed failures without a fake -1.

`get_error_addr` returns a stable, initially zero guest integer address for the
admitted initial thread. It never returns host thread-local memory. The core
requires the actual source/thread binding and preflights the whole live cell
for RW/NX access. Both library scopes share the same cell; independent kernel
instances have independent backing. Lazy allocation uses an internal 16 KiB
RW/NX page and the ordinary handle, mapped-byte and work budgets, separate
from the pthread prefix. Guest mapping services cannot unmap or protect it.
Unbound adapters refuse before POSIX input or output. This is initial-thread
storage, not additional guest-thread execution or a firmware TCB-offset claim.
Both scoped `getpagesize` exports return the pinned 16 KiB page size.

`Dlsym` delegates a bounded guest name and normalized int32 runtime handle to an
explicitly image-bound `GuestAccess::module_symbol`. Its default is Unsupported;
unavailable integration cannot become a missing-export success. The core selects
only actual supplied modules, checks their owned export declarations and live
targets, and charges cumulative work. After a real lookup the eight-byte output
is transactional. This service has its own explicit pinned int32 result handling:
known missing handles/exports write null and return ESRCH; null symbol writes
null and returns EFAULT, while null output returns EFAULT without a write. The
int32 code is zero-extended in RAX. Guest backing faults, limits, unsupported
lookup forms remain typed, without a blanket adapter-error mapping.
This neither opens a host library nor starts, discovers or unloads guest modules.

`GetProcParam` delegates to an explicitly image-bound backing owner. The default
`GuestAccess::process_parameters` refuses with `Unsupported`; ordinary memory
alone cannot imply a missing parameter header or supply a fabricated null. The
core derives the actual primary executable's `PT_SCE_PROCPARAM` guest address,
preflights its whole live readable extent and returns that address without
allocation, payload copying or structure/version guesses. Known primary absence
returns zero under the pinned source behavior. Shared-primary blocks, empty
extents, unreadable ranges and unavailable integration remain typed failures.
The owner bounds metadata/page work separately; the retained kernel budget still
charges attempts. A provider's process/module header does not replace the primary.
This getter does not decode a complete firmware process-parameter ABI.

| Subsystem | Implemented behavior | Explicit boundaries |
| --- | --- | --- |
| Direct memory | Bounded first-fit physical ranges, source alignment/search validation, alias-preserving direct maps across allocated pieces, and whole/partial release that withdraws every affected alias subrange | The unchecked source wrapper's unowned-release no-op, pools, GPU/backend effects or flag profiles unsupported by the adapter remain explicit failures |
| Flexible memory | Explicit configured pool, zeroed mappings, source name/protection/alignment checks, fixed/no-overwrite profile, whole or subrange unmap/protect, and optional transactional range/protection query outputs | No replacement of existing fixed maps or unsupported compatibility/cache behavior; unmap retains the aligned 16 KiB admission profile |
| Files | Explicit read-only package capabilities and separate owned writable save mounts; checked file IO, separate logical descriptor cursors and source FileStat codec | No ambient host-path fallback, writable package root, device/archive/socket service or directory pread. Actual directory roots expose bounded source-packed enumeration/read/seek/status; inventories remain explicit regular-file sets |
| Semaphores | Owned counts/attributes, FIFO/priority cooperative waits, reserved grants, deadlines/remaining time and signal/cancel/delete/poll | Blocking requires the runtime continuation interface; negative signal count and creation options remain unsupported |
| Event flags | Owned patterns, AND/OR, clear-none/all/bits, transactional pattern/time outputs, cooperative waits and cancel/delete cohort drains | Poll during an active drain is typed Unsupported; raw calls cannot block; unknown source-abort mode encodings remain Unsupported |
| Time | Real std realtime/timeval, source monotonic aliases with a runtime-owned epoch, native TSC/process counters and frequencies through the bound adapter, process microseconds from the same TSC epoch, bounded host sleeps and checked nanosleep | CPU-time clocks and clock resolution remain Unsupported. Positive nanosleep with a remainder output is Unsupported because this trait cannot preflight a future write before elapsed host time |
| Process/thread context | Explicit positive runtime PID/TID, owned GPO/GPI state, equality, source 4096-byte pthread guest prefix materialized in an owned 16 KiB logical mapping; mutexes, keys and native once initializers described above | Scalar kernel invocation alone does not schedule threads. The composed worker executes created threads with independent TLS and stacks; join/detach/explicit exit, cleanup callbacks and full firmware TCB/DTV remain incomplete |

The source pthread prefix starts with little-endian i32 thread ID and 4092
reserved zero bytes. Its internal mapping cannot be unmapped/protected through
ordinary guest mapping services. Repeated self queries return the same actual
guest address. The explicit process identity replaces the source's constant
PID 100 with owned configured state; the initial TID is not evidence of a
started native guest thread. Opaque sync handle tags/serials and configured
capacities are PentaX policies, not firmware pointer or memory-size facts.

`KernelConfiguredFlexibleMemorySize` has one exact libkernel module 1.1 /
libkernel library 1 registration, NID `n1-v6FgU7MQ`. Its one argument is an
eight-byte guest output: null returns the normal kernel int32 EINVAL value
`0x80020016`; otherwise one checked transaction writes the immutable configured
capacity and returns zero. Backing, policy and work failures remain typed stops.
`Limits::flexible_memory_bytes` defaults to 64 MiB for compact fixtures; the
explicit runtime memory profile selects the pinned 1 GiB pool alongside
12800 MiB of direct memory. Configuration must be nonzero, aligned to 16 KiB
and below the pinned 13824 MiB total. These are configured capacities, not host
RAM, free memory, available bytes or a promise that backing admission succeeds.

Actual flexible and named-flexible maps additionally share that pool. Admission
charges a bounded census of current owned mapping extents before any backing or
output mutation. Protection splits preserve explicit membership and unmapping
releases its mapped extent. Direct aliases, internal runtime pages and adopted
image-protection records remain outside the pool. Mapping, protection and unmap
never change the configured value. Existing aggregate, host, virtual-address,
handle, scratch and work limits remain independent and can refuse earlier.
Physical allocation preserves the source's zero-alignment rule: the search
start is unchanged even when it is not page aligned. Explicit physical
alignment and virtual mapping alignment retain their separate checks.

The monotonic origin differs from the fork's host performance-counter origin;
only the source clock domains/aliases and monotonic relation are retained.
Process counter epochs are initialized on first process-time use and retained
per kernel, while raw TSC reads and both frequency services share the admitted
host domain. The Windows adapter requires invariant TSC and obtains its frequency
from CPUID or one 100 ms host-clock calibration. Unbound adapters refuse.
Positive sleeps still block the host dispatcher; scheduling sleeps independently
for each guest thread and guest-signal interruption remain unfinished. Limits
bound each sleep before it blocks. Unsupported operations never return placeholder success.

## Limits and validation

`LoadStartModule` delegates its existing-program branch to an image/file-bound
owner. Zero uint32 flags and a null options pointer are required. The bounded
guest path is resolved only through explicit retained capabilities; it cannot
name a host file. The adapter must have observed the supplied module's actual
initializer return. It commits its incremented load count with an optional
four-byte zero result and returns the actual positive module handle. Args/argp
are opaque and unused in this pinned existing-program branch. New/unbound files
and unavailable initialization remain `Unsupported`, without invented ENOENT
or generic errno conversion. This hook performs no new mapping or guest call.


Thread attributes retain pinned defaults (1 MiB stack, 4 KiB guard, affinity
0x7f, priority 700, policy 1, inherited scheduling and joinable state) in owned
Rust metadata with a zeroed 64-byte guest prefix. Stack/affinity/guard values use
full guest widths; priority and policy retain exact signed values. Stack setters
store configuration without mapping or dereferencing the proposed stack. The
minimum configured size is 16 KiB. Getstack publishes both outputs atomically.
Creation/destruction share the bounded pthread mapping owner and reject stale,
cross-kind and live reinitialization attempts. POSIX wrappers return positive
errors without changing errno; source-fatal encodings remain Unsupported.

AttrGet supports the actual initial-thread identity and copies its prepared
stack geometry after rechecking owned RW/NX pages and both unmapped guards.
Inspection is bounded to 8 MiB; missing entry integration, other threads or
changed backing refuse without overwriting the destination attributes. Current
thread affinity and priority come from the same retained context used by direct
thread queries and semaphore ordering. Stack inspection for other threads and
join remain unfinished.

`PthreadSetaffinity` and `PthreadGetaffinity` target the actual owned thread
handle, even while another context runs. Creation copies the configured full-width
mask; later changes to, or destruction of, the creation attributes cannot change
that copy. The setter retains any uint64 mask, including zero, matching the pinned
source. The getter transactionally writes exactly eight bytes. Null handles return
ESRCH before null-output validation; a null getter output returns EINVAL. Unowned
handles remain typed stops without dereferencing caller-supplied pointers. Work
admission precedes mutation and a failed output leaves state unchanged. These
functions retain guest metadata; they do not change Windows processor affinity
or promise parallel guest execution. Registrations use exact libkernel1.1 scope
and library version1, with prior marker IDs preserved.

Read/write locks retain an owned zeroed 256-byte guest prefix, writer ownership
and per-thread recursive reader counts. Any owning thread can read while owning a writer;
unlock releases its reader holds before its writer, matching the pinned source.
Write recursion, including try-write and timed-write, returns EDEADLK. Destroying
a held lock returns EBUSY; unlocking an unheld lock returns EPERM. Zero slots
initialize lazily on read, write and unlock, but not on try-write. Creation and
destruction use the shared atomic page/slot transactions and never reuse an old
pthread address. Cross-kind and stale references remain explicit failures.

Rwlock attributes own 64-byte zeroed prefixes and retain any signed type value.
The pinned lock initializer ignores the attribute argument, so it is not read.
The exact kernel and POSIX registrations preserve their distinct error domains;
the POSIX-library try-lock exports intentionally retain direct kernel results.
Timed calls can acquire an available lock without waiting. Native blocking
read/write calls now use scheduler-owned opaque wait tickets, bound to the
actual selected TLS, process owner and pending executor continuation. New readers
wait behind queued writers, but existing readers can recurse and a writer can
read its own lock. Writers are not FIFO. Acquisition eligibility precedes timeout
even after a late scheduler turn. SCE timed calls take low32 relative monotonic
microseconds, not a POSIX absolute timespec. Only the existing POSIX write alias
uses positive errno; the POSIX-library try aliases retain direct kernel results.

Reader ownership pairs share one preallocated `max_thread_contexts` record pool
across all locks, independently of lock handles. Multiple locks per thread are
supported until this explicit finite allocation policy refuses. Each queued
reader reserves its eventual record before enrollment; acquisition, timeout or
internal cancellation releases that reservation exactly once. Waiter storage
has the same independent finite count bound. Queue membership never sleeps the
kernel host or returns success before acquisition. Internal ticket cancellation
is cleanup only, not a guest pthread_cancel implementation. Destroying a lock
with retained tickets returns EBUSY. Failed work/slot/storage admission preserves
ownership and queue state; a quota-refused new lazy read does not publish its slot.

One retained budget bounds total attempts, guest read/write bytes, work, live
guest resources, scratch peak, per-I/O bytes, mapped bytes and sleep duration.
Failed checked memory accesses retain attempted byte/work charges. Resource
state and logical file positions commit only after successful outputs. Initial
file capability inventories are separately bounded by `max_handles`; dynamic
descriptors share the global live-resource budget with sync and memory state.
`max_handles` has an explicit absolute admission cap of 65,536. Allocation
vectors reserve capacity fallibly during construction, and per-call buffers
respect scratch/I/O limits before allocation. Owned file names are capped at
1024 bytes, with source service-specific name checks retained where present.

`Kernel::handle_usage()` provides an allocation-free census of the actual
retained owner tables, independent of the running budget counter. Its closed
numeric categories cover memory allocations and mappings, open files, distinct
system modules, kernel synchronization objects, queues and event registrations,
NP requests, AMPR bindings, pthread keys, POSIX semaphores and fiber records.
The pthread-object count is a subset of memory and is never added twice.
Free slots in the fixed key table and reusable NP table are excluded; module
references share one record, while finalized fiber records remain retained.
`HandleUsage::accounted()` sums the owner categories for comparison with `live`.
Agreement checks accounting consistency; it does not prove that every retained
object is still needed or that guest cleanup occurred.

The default shared live-resource allowance is 256. Only the supervised worker's
explicit `long-startup` execution profile selects a finite allowance of 4096;
other execution profiles retain 256, independently of the memory profile.
These are deliberate PentaX resource policies, not firmware limits. Native
backing, mapped bytes, thread counts, I/O sizes and memory permissions remain
independently bounded. The worker observes the census once after prepared
execution completes or refuses, reporting only closed numeric totals and
per-type counts without guest identifiers, addresses or names.

Non-UTF8 filesystem names remain Unsupported. Mapping labels accept up to
31 non-NUL bytes without a UTF-8 requirement. Protection decoding follows the
pinned source: a nonzero word with none of its five CPU/GPU access bits returns
EINVAL; other bits remain queryable metadata and grant no additional access.
The native CPU adapter still refuses GPU-access profiles.

Protection ranges round outward to 16 KiB pages as in pinned `ProtectMemory`;
zero rounded length succeeds before checking the protection. Unmap and protect
validate complete coverage, including late holes and protected runtime-owned
pages. Unmap holes return source EACCES; an unmapped protection span retains the
source-aborting branch as a typed Unsupported stop. Complete split metadata,
fallible name copies, scratch/work and the shared handle total are admitted
before the backing effect. Only successful effects publish the new records.
Splits preserve their original allocation identity and physical offset. Adjacent
pieces rejoin only when identity, protection and other attributes match; separate
allocations retain separate query extents as a PentaX policy. Staged vectors keep
enough reserved capacity for later admitted map publication without allocation.
The native adapter supports anonymous decommit and fresh zeroed reuse, and
retained pagefile sections for actual direct physical aliases. Direct-map
unmap/remap preserves the shared bytes. Physical release requires complete
allocation coverage, then removes every alias intersection and splits surviving
physical/virtual records. A request may span adjacent allocated pieces. All
replacement handles, vectors, names, scratch and work are admitted before the
single backing effect; only successful completion publishes kernel metadata.
Output writes into aliases being removed are refused before any backing change.
The backing owner separately bounds total virtual mappings and unique committed
storage, counting physical aliases once for the latter. GPU/cache behavior is
not provided by these CPU views.

The native owner retains section offsets independently of logical physical
pieces. It stages all alias rollback data before detaching views, restores
original data and permissions on a recoverable failure before section closure,
and poisons the owner if restoration fails or an irreversible partial section
close occurs. Freed physical holes can receive new zeroed backing without
changing surviving neighbors. A partially referenced Windows section remains
fully charged to the native commitment cap until its last physical piece is
released; logical freed bytes are not reported as reclaimed OS backing.

Original tests cover same-backing direct alias/remap preservation, six-argument
allocation, transactional mapping/query and output rollback, source pthread
prefix/state lifetime, aggregate admission limits, sync timeout/consume/clear
rules, invalid clocks/times, and real read-only temporary-file behavior including
pread and failure-preserved logical offsets. Tests access no game or existing
user files and execute no guest code. This is a library-service port, not proof
of complete PS5 ABI coverage, startup, rendering or commercial game execution.

`KernelSetThreadDtors`, `KernelSetThreadAtexitCount` and
`KernelSetThreadAtexitReport` retain three independent process-wide guest callback
slots as opaque integers under the existing attempt budget. Null registration
leaves the selected slot empty; an existing nonnull callback cannot be replaced,
matching the pinned
source's unsupported branch. The source signature is void. Registration performs
no guest reads, writes or callback execution. Thread exit remains explicitly
unsupported; registering a target does not claim that cleanup ran or make the
stored address executable authority.

`KernelRtldSetApplicationHeapAPI` reads the complete ten-pointer guest table
under the shared byte/work budget before publishing a replacement. It retains
malloc/free (slots 0/1) and optional posix_memalign (slot 6); a null table or
missing malloc/free clears all three. Failed guest access leaves the previous
registration intact. The pointers remain process-owned data, never host function
pointers. Registration invokes no allocator; checked guest-callback dispatch and
the source's special scriptingGetMem lookup hook remain separate runtime work.

Protection requests can now adopt previously untracked pages from explicitly
loaded input images. The same-owner adapter must preflight the complete rounded
span against its admitted image plan and actual committed pages; generated
providers, runtime storage and reservation holes provide no such authority.
New mapping identities, query metadata and managed-byte/handle accounting are
staged under shared work/scratch/resource limits, then publish only after the
whole native protection transaction succeeds. Repeated changes reuse those
records; queries preserve the original protection word, including non-access
metadata. Adoption allocates no backing and does not increase native committed
bytes. Subsequent unmap releases exactly the adopted kernel accounting.

Trusted Rust service composition can initialize a bounded runtime-owned RW/NX
data blob using its selected guest address. Staging allocation, page rounding,
identity/handle/byte/work limits and actual backing/output admission precede
publication. Initializer or backing failure leaves mapping metadata/accounting
unchanged. Runtime-owned blobs cannot be unmapped/protected by guest services;
the owning session releases them. This helper adds no guest syscall attempt.

Share's legacy Initialize/Terminate preserve stateless success. Its fifteen
extended roles follow the later libShare registration, including a corrected
FeaturePermit that rejects a zero low-word mask. Capture requests optionally
store an invalid request ID and return NOT_SUPPORTED; status calls clear their
exact four-/sixteen-byte outputs atomically. Opaque callbacks and overlay input
are neither retained nor executed. No heap, thread, device, recording or host
media operation is created. Attempt/work/I/O budgets remain authoritative.
Only exact canonical identities and independently demonstrated full-prefix
aliases are registered. Source order, duplicate resolution and attribution are
documented in NOTICE and upstream provenance.

ContentExport, ContentSearch and ContentDelete retain three independent fixed
initialization flags in the pinned silent profile. A null parameter word returns
the service's exact error; otherwise it marks only that service initialized.
The verified logging macro skips all diagnostic field reads in silent mode.
Allocator pointers and requested sizes are therefore neither read nor executed,
and no host media operation is created. Work/attempt refusal leaves flags
unchanged. ContentSearch uses its exact module version 1.0. This initialization
profile does not implement content export, search or deletion operations.

`AcmContextCreate` implements the pinned creation-only ACM profile: one opaque
process-owned uint32 identifier, starting at one, is written to exactly four
guest bytes. Shared handle/work/byte limits and the complete live output
transaction precede identifier publication. Rejected outputs consume no context
or identifier; exhaustion refuses without wrapping or reuse. Null takes a typed
unsupported path rather than an invented numeric error. Only canonical
`Acm/Acm` and the exact full-prefix `libSceAcm/libSceAcm` identity are registered,
both module1.1/library1. Context destruction, batch submission, waiting and DSP
execution remain unsupported; creating an identifier does not process audio.

Event-flag continuations retain the exact thread and pending syscall while its
full CPU/TLS context is parked. AND/OR predicates are checked before deadlines;
successful pattern/remaining-time output commits before clearing bits. Single
waiter flags return EPERM before even a satisfied predicate. Cancel writes the
actual cohort count before replacing bits and marks that cohort canceled;
Delete marks it deleted. Their callers remain parked until every old waiter
commits its output. Failed output retains the ticket, bits and drain membership.
Cancel/Delete do not return early, and polling/readiness cannot consume a wake.

Bounded deferred operations serialize native mutations and new waits behind a
drain; admitted timeout values and the existing monotonic wait-clock start are
retained across that barrier. An internal deferred-to-wait/drain transition
keeps the same syscall parked and yields another scheduling pass. This is an
explicit cooperative compatibility policy, not host thread race equivalence.
The source ignores FIFO/priority attributes, so no priority wake order is
claimed. Multi-waiter Poll during a cancel/delete drain is explicitly unsupported
instead of reproducing the source race that can mutate cancellation bits. Live
Poll remains immediate; its Single EPERM precedence is preserved during a drain.

The four stateless AudioOut2 helpers follow the pinned source without opening
audio hardware. Initialize only returns zero and retains no initialization flag.
ContextResetParam writes exactly 64 bytes, including the six source defaults and
zero reserved words. ContextQueryMemory reads only the four-byte queue_depth at
offset 12 before writing its eight-byte size; zero depth selects 4 and every other
uint32 depth uses the source formula. Input/output overlap observes that snapshot.
GetSpeakerInfo writes exactly 80 bytes with the source stereo availability/angles,
zero padding and ignored flags. Each output commits atomically under the shared
byte/work limits. Null source-fatal arguments produce typed Unsupported rather
than invented numeric errors. Exact canonical AudioOut 1.1 / AudioOut2 1 and the
demonstrated libSceAudioOut 1.1 / libSceAudioOut2 1 identities are admitted. These
helpers allocate no handle, audio context, port or device and do not play or mix
audio.

The remaining eleven demonstrated AudioOut2 lifecycle roles use an explicit
logical headless profile: at most16 contexts,256 ports,32 internal sinks and
4096 users (also subject to the shared handle quota). Contexts, ports and users
count once in the independent `audio2` handle census; sinks are bounded internal
components. No physical device is present and no audible playback is claimed.
Metadata is admitted lazily, with each actual allocation capacity checked before
the next allocation. Retained PCM and stereo mixes share a16MiB ceiling and the
shared scratch ceiling includes old storage and simultaneous staging. Grains
are capped at4096 before effects, avoiding the source's larger-u32 duration wrap.

Ports own snapshots of accepted PCM; object ports actually mix signed16-bit or
float mono into stereo with the pinned gain/ambisonics coefficients. Ordinary
ports retain their PCM and gains for logical sink submission. Accepted silent
ambisonics still produces the source's zero stereo batch. Committed-only
`audio_mix_frames` and `audio_logical_output_batches` count evaluated object
sample frames and nonempty accepted logical batches, respectively. They measure
neither physical playback nor DSP completion. Counter overflow refuses before
publishing the queue, mix or sink clock.

Queue backpressure uses the existing process-microsecond clock and source grain
duration. Each logical sink separately retains its exact fractional48kHz (or
declared port-rate) sample remainder. Blocking operations retain opaque tickets
and park the actual guest syscall; no host sleep occurs inside dispatch. A paced
submission retains a global lease over its buffers and sink ownership until its
deadline. Other stateful operations defer behind it, including other contexts.
Readiness observes only clock/owner state; completion requires the enrolled
thread and the native adapter's real pending-token proof. Raw synchronous calls
refuse any needed continuation before effects. A blocking unknown context is
explicitly unsupported instead of looping forever or returning fake success.

The global lease deliberately serializes complete stateful publication more
strongly than upstream's pre-lock work: GetState defaults and provisional opens
wait with the operation, and PortCreate's ID order is determined at that point.
Pure pre-lock null/error precedence remains immediate. Ordinary noncontended
calls retain source-error PortCreate ID consumption; guest, allocation and
policy failures retain all IDs and state. ContextCreate models its output store
before its queue-depth read through an atomic staged overlay. Physical aliases
that cannot prove this ordering refuse. This profile does not claim host race
equivalence, physical-device queue behavior, mastering or additional DSP APIs.

VideoOut notifications use their own retained event subscriptions (filter -13),
distinct from AGC interrupts. The native adapter supplies actual observed
vblank ticks and acknowledged flip completions; numeric metadata alone creates
neither a core Frame nor presentation authority. Default coalescing retains the
newest payload and source counters saturated at15 while preserving active order.
The existing bounded queue/handle/work policy applies; there is no unbounded
legacy copy queue. Failed guest delivery leaves the pending event unchanged.
Closed queues and retired port generations cannot receive stale notifications.
The source lineage and explicit presentation boundaries are documented in
`docs/worker-frame-bridge.md` at the repository root and in NOTICE.


The local ConvertKeycode keyboard-type query preserves the public source
contract: null output returns 0x80bc0031 before a negative signed int32 user
returns 0x80bc0010. Nonnegative IDs write exactly four zero bytes through
checked guest backing. This stateless local type does not enumerate a host
keyboard, initialize IME or claim text conversion. Only the explicit
libSceConvertKeycode module1.0/library1 pair is registered.

### Fiber transfers

The Fiber profile includes Run, Switch, ReturnToThread, GetSelf, context-size
checks and the source frame-pointer query alongside initialization and lifecycle.
Transfers retain generation-bound objects and per-thread Run roots. Idle fibers
may resume on another guest pthread; RUNNING ownership remains exclusive. Saved
80-byte ABI records are observations, never native execution authority. The
native owner separately proves executable entry, owned stack and continuation,
then publishes object and optional result writes atomically after leaving the
old stack. A saved output pointer is checked again when its fiber resumes.

Only canonical source NIDs and the three observed paired-prefix execution
aliases are added. Original provider ordinals remain unchanged. Context-size
checks and the zero frame-pointer result reproduce the pinned implementation.
Object records retain the existing lifetime handle cap; reinitializing an object
reuses its slot with a fresh generation. No commercial compatibility follows.
