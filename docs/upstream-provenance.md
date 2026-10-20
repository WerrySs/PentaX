# Upstream provenance

The three non-MMX/non-EVEX PMADDWD forms use original Rust signed-word
multiplication and wrapping dword addition following the
[Intel Volume 2B PMADDWD contract](https://cdrdv2-public.intel.com/835752/253667-sdm-vol-2b.pdf)
and [Volume 2A Type 4 alignment rules](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf).
No manual pseudocode, external emulator implementation, host vector intrinsic
or commercial guest content is copied into the checked mediator.

The three non-EVEX PCMPGTQ/VPCMPGTQ forms use original checked Rust signed
integer comparisons, following the
[Intel Volume 2B instruction contract](https://cdrdv2-public.intel.com/835752/253667-sdm-vol-2b.pdf)
and [Volume 2A Type 4 alignment rules](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf).
Only architectural operand, signed-lane, mask, upper-register and encoding
contracts are used. No manual pseudocode, external emulator implementation,
host vector operation or commercial guest content is copied.

The mediated RDTSC/RDTSCP register effects follow the
[Intel Volume 2B instruction entries](https://cdrdv2-public.intel.com/835752/253667-sdm-vol-2b.pdf).
Original Rust integrates the existing platform counter shared with kernel time
services. The explicit single-virtual-processor TSC_AUX policy is zero; host AUX
and processor identities are not exposed. No manual pseudocode, third-party
instruction implementation or game content is copied. The existing platform
counter's frequency source, calibration and kernel epoch contracts are retained.

PHMINPOSUW and VPHMINPOSUW use original checked Rust integer comparisons,
following the [Intel Volume 2B instruction contract](https://cdrdv2-public.intel.com/835752/253667-sdm-vol-2b.pdf)
and [Volume 2A Type 4 alignment rules](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf).
Only unsigned minimum, first-index tie selection, zeroing, source width and
encoding contracts are used. No manual pseudocode, third-party implementation,
host vector operation or game content is copied into the mediator.

The eight VEX VPERMILPS/PD forms are original Rust following
[Intel Volume 2C's VPERMILPD/PS entries](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2c-manual.pdf).
Variable PD follows the diagram and VEX operation's control bit1; older prose
incorrectly says bit0. Immediate PD uses successive per-qword bits, whereas PS
repeats four selectors within each 128-bit lane. No manual pseudocode,
third-party implementation or game content is copied into the checked mediator.

The four VEX immediate half/quadword permutations are original Rust following
[Intel Volume 2C's VPERM2F128/I128 and VPERMPD/Q entries](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2c-manual.pdf).
Only bit selection, zero overrides, encoding and full memory-read contracts are
used. The checked mediator snapshots all source bytes before an aliased write,
with no host floating-point operation, copied manual pseudocode, third-party
implementation or game content.

The two VEX 128-bit lane extractions are original Rust following
[Intel Volume 2C's VEXTRACTF128/I128 entries](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2c-manual.pdf)
and [Volume 2A Tables 2-16/2-23](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf).
Only architectural bit-transfer, encoding and memory contracts are used; no
manual pseudocode, third-party runtime or game content is copied. Existing
whole-span checked writes replace unchecked guest memory stores.

The twelve VEX packed PS/PD-to-signed-DQ and signed-DQ-to-PS/PD conversions
extend the original integer-only scalar conversion helpers. Widths, rounding
and exceptions follow the [Intel SDM Volume 2A conversion entries](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf),
with [Volume 1 sections 11.5.2.2, 11.5.3.1 and 14.12](https://cdrdv2-public.intel.com/843827/253665-sdm-vol-1-dec-24.pdf)
for absent denormal exceptions, per-lane aggregation and AVX. No third-party
arithmetic implementation, manual pseudocode, host floating-point operation,
guest content or title-specific bypass is included in the production mediator.

VEX VMOVMSKPS/PD sign extraction is original Rust, following the
[Intel SDM Volume 2B MOVMSKPS/PD entries](https://cdrdv2-public.intel.com/835752/253667-sdm-vol-2b.pdf)
(printed pages 4-92 through 4-95). It extends the checked byte-mask path with
single/double element strides. No floating-point operation, source mutation,
host AVX instruction, copied implementation or game data is used.

VEX mask comparisons are original Rust, using
[Intel SDM Volume 2A's comparison predicate table and CMPPS/PD/SS/SD entries](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf)
and [Volume 1's NaN priority, DAZ and packed exception aggregation rules](https://cdrdv2-public.intel.com/843827/253665-sdm-vol-1-dec-24.pdf).
Fixed integer classification and ordering produce architectural masks and
guest status bits without executing host AVX or floating-point instructions.
The implementation copies no manual pseudocode or third-party emulator code.
Original synthetic tests independently compare copied values against hardware
where AVX is available, restoring host control before returning to Rust.

The seven SystemService notice, identity and lifecycle roles adapt
[`src/libs/libSystemService.cpp`](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/libs/libSystemService.cpp),
independently verified Git blob `690b813be6ceac9c707e06c5fff6ec3c70d13acb`.
Their canonical module/library scope is `SystemService` 1.1/library 1. Only
the separately observed `GetNoticeScreenSkipFlag` and `HideSplashScreen`
full-prefix imports add `libSceSystemService` 1.1/library 1 registrations;
mixed namespaces and additional prefix variants are not inferred.

The notice getter stores one configured bool byte; `emulatorConfig.h`'s
default false and `Config::SkipNoticeScreen` become the owned additive
`system.skip_notice_screen` preference, passed through the existing configuration
path. Both notice setters deliberately retain the source's no-op behavior.
Splash hiding, power tick and abnormal-termination reporting retain their
silent no-op contracts without reading the report's opaque pointer or invoking
host facilities. ParamGetString retains signed-low32 ID 6, null/zero-size error
precedence, full64 capacity, the short-buffer error and a NUL-terminated exact
copy. The reference's fixed `Kyty` name is adapted explicitly to the `PentaX`
system-name profile. Checked seven-byte output preserves all trailing bytes;
unknown IDs remain typed source-fatal refusals. All outputs use the same
bounded atomic guest transaction instead of unchecked source stores.

Created-thread cleanup adapts `CleanupThread`, `RunThread` and
`DestructPthreadSpecific` from
[`src/kernel/pthread.cpp`](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/kernel/pthread.cpp),
verified Git blob `a7fa3003f534af01a119f117954ffe899c982351`, with header
`f5930a4aeae3f79a80d362f81a68862e48730871`. The source runs thread-local Cxa
destructors, current `ThreadDtors()`, then pthread-key destructors before host
TLS deletion and `almost_done`. PentaX currently admits no `CxaThreadAtexit`
registrations, so no such records can be silently skipped; separate process
`cxa_atexit` entries are not consumed. Count/Report callbacks in pinned
`libKernel.cpp` (`b02a4d77404f839371d96d1f549b53043e77d25e`) belong to module
unloading, and are excluded from thread-exit eligibility and invocation.

The bounded Rust cursor preserves ascending 256-slot live scans, whole-slot
exchange before eligibility, current odd-generation matching, clear-before-call
and four destructor passes. A callback may repopulate or replace keys; later
slots are observed in the same pass and earlier slots in the next. Completion
uses uniquely owned thread/context/serial tickets and actual current native
returns, preserving original entry RAX separately. Every callback target, source,
stack, TLS context, executor and call sequence is admitted; copied historical,
foreign and nested returns cannot authorize outer cleanup. Code/stack/resource
refusal retains pending cleanup and faults never mark the child returned.
Actual execution shares existing native budgets and cooperative scheduling.
No host thread/function-pointer calls or catalogue growth are added. Created
storage retains its existing bounded runtime-owner lifetime; cancellation,
unwinding, join/detach reclamation and thread-local Cxa registration remain
explicit further work. These are emulator source-profile contracts under the
same GPLv2 terms, not an independent firmware specification.

PlayGo adapts
[`src/libs/libPlayGo.cpp`](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/libs/libPlayGo.cpp),
verified Git blob `94f0093bd0013d266ed2e32c877f11393ba84a9c`, and
[`src/loader/systemContent.cpp`](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/loader/systemContent.cpp),
blob `b86fcff76047852df9fe10b47c6a8c070602e640`. Supporting headers are
`systemContent.h` (`778847641ed3d8d4ec54b675dc329f0af834319b`) and
`libs/errno.h` (`e84e56ceec6d6a1149cd33fd0907538da496afdb`). Exact config
blobs `744b7f13b847240369a55ff33186beb171f98b7d` and
`c6f9d9e8e64e28edef40141443bd693f1affbd2b` confirm PlayGoHack defaults off.
All use the same owner-authorized revision and existing GPLv2 terms; the
references describe that emulator profile rather than an independent firmware
specification. Binary magic/count, literal byte-tag XML and contiguous manifest
IDs supply real selected-package metadata in source order. A complete twelve-byte
binary prefix replaces unchecked short reads; fallback inputs have a 16-MiB
ceiling plus precharged linear parser work. No metadata files are bundled.

All eighteen PlayGo module 1.0/library 1 roles preserve exact error precedence,
low-width scalars, static handle 1, optional masks and count-based progress.
Initialization leaves the opaque work buffer and diagnostic-only reserved field
unread. Locus stores are privately staged and later ID reads observe backing
aliases; a source BAD_CHUNK publishes its admitted valid prefix, whereas typed
backing/policy failures publish nothing. List generation retains count-last
overlap behavior. The source prefetch/todo no-ops retain no download requests.
Only seven independently observed full-prefix aliases accompany the canonical
exports, yielding 1263 catalogue entries and 64/65 module/library descriptors.
Old service IDs, sites, data RVAs and independent 2048-entry/64-KiB limits stay
fixed. Current provider code/image is 0xfa58/0xfa68 bytes. Production names now
cross descriptor ID 64; actual binding tests cover both canonical and prefixed
PlayGo identities, with oversized shifted tables explicitly refused by budget.

The extended Share contracts adapt
[`src/libs/libShare.cpp`](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/libs/libShare.cpp),
verified Git blob `a38efbeefac2742da3dbe8248651a983150abb07`. The pinned
`libs.cpp` InitAll registers InitNet before InitShare. Exact
`src/loader/symbolDatabase.cpp` and `.h` blobs
`992e43f9b5a81c29e6c4ca614105934c8cfcde78` and
`bf3f64aba32485490e8d8b31a3a9271055991789` verify that duplicate normalized
identities use `insert_or_assign`: the later `Share::ShareFeaturePermit` replaces
the legacy always-success definition. PentaX therefore corrects the original
Permit role's null-mask result and source label while retaining its marker,
discriminant and unique identity. All source pins use the same owner-authorized
revision `209dffe9032d72749257ce18680d506b891284de` under existing GPLv2 terms.

All fifteen extended roles preserve the silent reference profile. Capture calls
ignore their input parameter, optionally store a four-byte invalid request ID,
then return NOT_SUPPORTED. Current status clears exactly 16 bytes; running status
clears four. Mask checks use the low 32 bits and reject zero. Parameter/title
setters require a nonnull opaque pointer; string fields are logging-only.
Register/Unregister do not retain or invoke callbacks, and overlay performs no
host access. Outputs use bounded atomic guest writes; invalid arguments precede
destination access. No host capture/menu or recorded-media success is invented.
Fourteen new canonical rows plus only four independently observed full-prefix
aliases append 18 entries, bringing the catalogue to 1238. The existing Permit
row is reused. Marker/descriptor/data identities and the 2048-entry/64-KiB limits
remain fixed; the current provider code/image is 0xf530/0xf540 bytes.

ContentExport, ContentSearch and ContentDelete initialization adapt
[`src/libs/libs.cpp:156..283`](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/libs/libs.cpp#L156),
verified Git blob `c366dadb7c02e94ed3f67c1c951dec53bd294694`. The associated
registration header `src/libs/libs.h` and logging header
`src/common/logging/log.h` have verified blobs
`d1220ed30bd58c8407d30b29b58f04d7374f6c00` and
`56416cbc800c84e753c941bf3c73644430d24b08`. All pins are the same
owner-authorized revision `209dffe9032d72749257ce18680d506b891284de`, under
the source's existing GPLv2 terms. The LOGF macro evaluates diagnostic fields
only outside silent mode. The silent profile therefore checks the original
parameter word for null, returns its exact service-specific error, and otherwise
sets the matching fixed initialization flag without reading guest memory.
Allocator callbacks, sizes, reserved bytes and user data are logging-only;
no export, search, deletion, callback or allocation operation occurs. Flags are
owned per process and survive repeated initialization/null errors. Work/attempt
limits precede flag mutation; no dynamic resource or new handle is created.

The three exact canonical FUNC bindings plus only the independently observed
libSceContentExport alias bring the catalogue to 1220. ContentSearch has module
version **1.0**, while Export/Delete use 1.1; library versions are all 1.
The original provider now serializes canonical one-to-three-digit u16 selectors
using the existing linker encoder, with exact variable name sizes and no per-name
heap allocation. Production descriptor IDs and all historic markers/objects
stay fixed. Original complete-provider fixtures cross IDs 63/64 and check real
symbolic imports; larger source images still refuse at the independent 64-KiB
limit, rather than silently widening code or registration budgets.

The legacy Share lifecycle source is
[`src/libs/libNet.cpp:2622..2658`](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/libs/libNet.cpp#L2622)
from the owner-authorized reference fork, verified Git blob
`d540a4d0f62a9c0fa8393e83164717ae5e6dcbcb`, under its existing GPLv2 terms.
The exact Contents API identity matches retained bytes. This source can require
access and does not establish an independently public firmware ABI.
Initialize takes `(size_t heap_size, int thread_priority, u64 affinity_mask)`;
FeaturePermit takes a `u32` flag word; Terminate takes no arguments. That file
defines three logging-only success calls, but final registration replaces Permit
with the extended implementation documented above. Initialize/Terminate preserve
their stateless success with bounded work accounting and no heap/thread/host
operation or argument logging. Three original canonical Share module1.1/library1
FUNC identities plus only observed libSceShare Initialize/Terminate append five
registrations, bringing the catalogue to1216. Unchanged2048-entry/64KiB caps and
all historic marker/RW/NX object RVAs remain independent constraints.

The raw Mouse/Keyboard catalogue and disconnected source profile adapt
[`src/libs/libPad.cpp`](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/libs/libPad.cpp#L201)
at authorized revision `209dffe9032d72749257ce18680d506b891284de`, verified
Git blob `ab9a861fed955b3a30e12275c9b60f312c6478f3`, under the same GPLv2
terms. Contents API identity and retained bytes match at the exact ref. This
owner-fork source may require access and is not independently public firmware
documentation. The source defines all ten roles in libPad.cpp itself.

Ten canonical Mouse/Keyboard module 1.1/library 1 FUNC registrations plus
only observed libSceMouse Init/Open/Read append 13 entries, bringing the
provider to 1211 within unchanged 2048-entry/64 KiB code-image caps. All old
marker and RW/NX object RVAs stay fixed. No prefixed MouseClose, raw Keyboard
or mixed-axis aliases are inferred. Every argument uses the GP register bank;
KeyboardGetKey2Char's output is the sixth argument at R9.

The source is stateless: Open returns 1 for admitted type/index, ignores user
and opaque param, and every positive handle is accepted independently of prior
calls. MouseRead clears 40*num bytes; KeyboardRead clears 96*num for 1..16,
ReadState clears 96, and GetKey2Char clears 20 including padding and returns
no character. PentaX uses checked sizing, actual capacity/I/O/work admission
and one atomic guest write. Mouse has no source positive-count cap; its
existing I/O output budget (1 MiB by default) gives a typed limit before any
effect. No donor device state, host input, translation table, callback or
connection success is fabricated. This profile remains separate from IME
callbacks and any later physical-input integration.

The IME source profile adapts the owner-authorized reference fork's
[`src/libs/ime.cpp`](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/libs/ime.cpp)
and associated ime.h/imeCommon.cpp/imeCommon.h at revision
`209dffe9032d72749257ce18680d506b891284de`, under the same GPLv2 terms.
Their verified Git blobs are respectively
`08c196095ba5ef49b07a4fca8af7d82916abde3a`,
`c53e1d706705e29f7ad6d721ae955e682800ed1f`,
`61ff36c53aaf5f4f952a152f3aeea4d7c087b68e` and
`c527b691af7322a9ab06cf53a43121222ca9be90`. Contents API hashes at the
exact ref match retained bytes; these private-fork links may require access
and do not constitute independently public firmware documentation.

Thirteen exact canonical Ime module 1.1/library 1 FUNC rows plus only the
individually observed libSceIme Update/KeyboardOpen identities append 15
registrations, bringing the provider to 1198. The unchanged 2048-entry and
64 KiB code-image caps still apply; prior marker and RW/NX object RVAs stay
fixed. There are no inferred mixed-axis or sibling aliases. ParamInit is
void; all thirteen roles use GP arguments. The source-derived local profile
has real owned UTF16 text, separate disconnected keyboard state, exact output
footprints and queued callbacks. Resource enumeration clears its output even
when returning an error; keyboard info does not fabricate a device. Native
Update dispatch snapshots text before keyboard, guards only text generation
and preserves effects of actual reentrant callback returns. Host input, UI,
text filters and external-keyboard filters require separate integration;
these bindings do not establish those capabilities or a presented frame.

The Font/FontFt source contracts and catalogue adapt
[`src/libs/libFont.cpp`](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/libs/libFont.cpp)
(Git blob `c3b58f805f0781105a9a7cf216bf4750b3e57d5e`) and
[`src/libs/libFontFt.cpp`](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/libs/libFontFt.cpp)
(Git blob `3cfee91fb323157c0bdc07572cb44fb6ad26a5dc`) at the exact authorized
revision `209dffe9032d72749257ce18680d506b891284de`, under the same GPLv2
terms. Cached Git blob hashes match independently verified Contents API values
at that ref. Seventy Font NIDs cover sixty-eight functions; FontFt adds two
selection functions. Canonical Font/FontFt module1.1/library1 identities are
finite. Only fifty-four individually observed full-prefixed NIDs are added;
mixed axes, other versions and unobserved siblings remain excluded. All three
canonical horizontal-render NIDs share one function; only `kAenWy1Zw5o` also
has its demonstrated `libSceFont` identity. No NID-only fallback is enabled.

These126 appended registrations bring the provider to1183 entries within the
unchanged2048-entry and64 KiB code-image caps. Existing markers and RW/NX data
RVAs remain fixed. GP arity is separate from vector capture: FontMemoryInit
receives its seventh GP value from captured callee RSP+8, while horizontal
rendering uses five GP values and XMM0/1 floats. Returned structures require
actual owned guest storage; donor host object/vector addresses are not guest
addresses. Source memory callbacks are retained data, not invoked by Init/Term;
support-font/cache functions are logging-only no-ops. Real parser callbacks,
glyph metrics and bitmap writes remain distinct capabilities from registration.

The donor uses stb_truetype, ambient Roboto searches and an FFmpeg VGA bitmap
fallback. This catalogue copies none of those assets or unrestricted host
search behavior and provides no third-party asset/rasterizer license by itself.
PentaX's selected Rust backend uses `skrifa 0.44.0` (std, no default features)
for unhinted default-coordinate TTF/CFF/CFF2 outlines and an original4x4
nonzero-winding grayscale rasterizer. The licensed bundled Hack-Regular font
from `epaint_default_fonts 0.37.0` replaces the source's ambient Roboto/VGA
fallback. This is a deliberate font/metrics/pixel difference, with one regular
fallback rather than invented bold/italic style coverage; source-equivalent
pixels and full shaping are not claimed. Font input is capped8 MiB; images
use128-byte pitch and dimensions at most128. TTF validation bounds composite
depth16,nodes1024,points16384; contours have at most16384 edges and draw scratch
at most2 MiB. CFF expansion charges at most262144 shared cumulative charstring
bytes before each subroutine/seac expansion, with library recursion bound10.
Malformed/complex data refuses; actual allocation capacities and iteration
work remain admitted. Dependency/font notices are separate from these GPL
source-contract adaptations. Bindings, fixed fallback statistics or returned
surface descriptors alone are no evidence of glyph pixels, GPU presentation
or a visible frame.

Writing objects publish real guest step/letter/metrics views with a256-byte
caller structure. Reinitialization reuses its authenticated owned arena and
commits the new payload/caller output atomically, preserving prior bytes and
state on refusal. Cloned caller structures cannot transfer another writing
owner's authority; string/character/font dependencies remain retained. Writing
lines reserve a finite4096-step arena (262208 bytes,17 rounded guest pages),
keeping earlier64-byte views stable through append. This bounded stable-storage
policy replaces the donor vector's potentially invalidated views. Clear resets
cursor/source metrics; real cursor advancement, ordered overlapping outputs
and release-with-output transactions preserve source effects without fake
success or discarding unrelated guest bytes.

Five Entitlement Access operations adapt
[`src/libs/libNet.cpp`](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/libs/libNet.cpp),
lines1596-1737, at revision `209dffe9032d72749257ce18680d506b891284de`,
Git blob `d540a4d0f62a9c0fa8393e83164717ae5e6dcbcb`, under the same GPLv2
terms recorded in the kernel NOTICE. Canonical `NpEntitlementAccess`
module1.1/library1 admits all five roles. Only Initialize and List additionally
have independently observed matching `libSceNpEntitlementAccess` identities;
mixed names, unobserved prefixed roles and other versions remain excluded.
Seven appended markers preserve every prior site and the fixed RW/NX data
RVAs. The independent2048-entry ceiling and64 KiB code-image cap remain;
no runtime cap is raised by this family.

PentaX uses an explicit local virtual full-package profile (SKU3) with an empty
default addon catalogue and bounded configurable local records. Replacing the
source's three hardcoded GTA-V records with that empty default is a deliberate
local policy, not discovered DLC or PSN/account ownership. No donor addon table,
purchase result, authentication, key material or backend connection is copied
or fabricated. Initialize remains stateless, clears exactly32 boot bytes and
never reads its opaque nonnull input. Queries retain source argument widths,
null precedence,28-byte records, list count/clear/copy ordering and output/input
overlap effects; GetEntitlementKey returns the source no-key error without
reading its label or touching key bytes. Checked bounded staging and one atomic
guest transaction replace unchecked stores and preserve old bytes on refusal.

The forty-one Universal Data System exports adapt `src/libs/libTrophy.cpp`
lines 548-1323 and the trophy/rule/persistence helpers in the exact three source
files and GPLv2 revision identified below. Forty-one canonical
`NpUniversalDataSystem` module 1.1/library 1 registrations and twenty independently
observed exact `libSceNpUniversalDataSystem` registrations are admitted. Other
names, versions and unobserved prefixed roles are excluded. The complete kernel
provider retains its append-only entries under an explicit 2048-entry ceiling; the independent
64 KiB image cap and prior marker/data locations are retained.

Owner-local UDS contexts retain source max-eight and error precedence. Opaque
objects/arrays/events use real owned RW/NX guest mappings and closed mapping
identities. Event creation deep-copies integer properties and exposes a distinct
event-owned mutable property view. Source arrays retain no elements; only
int32/uint32/uint64 object values persist, and the other setters erase the named
integer. Floating arguments affect excluded logging only, so their source
effects require no guessed GP values or new vector-dependent dispatch.
Literal `{}`/size 3 serialization and zeroed 24/56-byte statistics remain explicit
source fallbacks. No host address, nested ownership, PSN transport or callback
execution is inferred.

Posted events load the same actual package/user records as Trophy2 queries.
Supported direct-latest unsigned UDS rules and typed `_UnlockTrophy` events
update filtered unlocks, UTC epoch-microsecond timestamps and source-linked
platinum awards. Per-title `trophies-u{user}-s{label}.json` records are replaced
through capability-relative exclusive temporary files, sync and final rename.
PentaX stages RAM/cache updates until that replacement succeeds; typed storage
failure leaves prior RAM/file data unchanged. This is an explicit stronger
transaction policy than the source RAM-first/log-only save failure. Legacy
ambient saves and popup/audio/PSN side effects are excluded. UDS metadata,
Trophy2 state and live staging share the 64 MiB admission limit, plus bounded
record/key/string and common work/handle/mapping/save-I/O quotas.

The fifteen Trophy2 contracts adapt the pinned fork's
[`src/libs/libTrophy.cpp`](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/libs/libTrophy.cpp),
blob `216922e65cb8736f0bb663931d38764cf89dde76`, with
[`src/common/trophies.h`](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/common/trophies.h),
blob `d30211cea048bc1941618a1d25717f8fb48b5c32`, and
[`src/common/trophies.cpp`](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/common/trophies.cpp),
blob `cec6e76b553fb253e64ff946c292a9c5c6a0c6f1`. All use revision
`209dffe9032d72749257ce18680d506b891284de`. Fifteen canonical `NpTrophy2`
module1.1/library1 bindings and the five independently observed lifecycle
`libSceNpTrophy2` bindings are finite exact entries; mixed names, other versions
and unobserved prefixed query exports are excluded.

Owner-local contexts, label-keyed packages and per-user state retain source
duplicate/max-eight precedence, monotonic IDs, optional outputs and persistent
caches. Registration reads actual UCP files through the admitted `/app0` mount;
absence of that capability is distinct from an actually missing/invalid package.
The checked UCP/JSON reader preserves locale fallback, first duplicate archive
entry, last duplicate JSON member, definitions, text, icons, progressive fields
and supported direct unsigned latest-value UDS rules. The flat JSON index
borrows input strings and allocates decoded retained data fallibly. Source
bounds remain 1 GiB input, 4096 archive entries, 1000 trophies and 50 groups.
Additional explicit policies limit JSON depth to64, structural nodes to65536,
decoded retained string source spans to1 MiB and actual retained plus staged
metadata to64 MiB. Inputs, parser storage and old/new capacities share the
kernel scratch ceiling (64 MiB by default, with no eager reservation), work
and cumulative I/O admission. A policy refusal remains typed.

Real saved unlock IDs and valid epoch-microsecond timestamps determine query
records and weighted percentages. The existing selected per-title save root
uses numeric `trophies-u<user>-s<label>.json` records, bounded to1 MiB; reads
preserve files. Source global `_Trophies`/legacy `_SaveData` discovery and
migration outside that capability are not inferred. All fifteen exports are
readers of persistent unlock state; UDS event production and persistence writes
remain separate work. Queries stage exact records, ordinal array selections and
counts before one atomic output; both seven-argument arrays obtain count from
the actual captured callee RSP+8. Recoverable output failures preserve state
and all guest outputs. Unsupported overlap is explicit. Source icons return
missing with optional size zero; source callback/handle no-ops do not imply
asynchronous work, delivery, cancellation or guest callback execution.

The four Game Intent exports adapt
[`libNet.cpp` lines1996..2081](https://github.com/WerrySs/KytyPS5-Performance/blob/209dffe9032d72749257ce18680d506b891284de/src/libs/libNet.cpp#L1996-L2081),
Git blob `d540a4d0f62a9c0fa8393e83164717ae5e6dcbcb`. Source module/library
`NpGameIntent`1.1/1 and the separately observed exact `libSceNpGameIntent`
1.1/1 import profile are admitted; mixed names and other versions are excluded.
Initialization and termination retain the source's no-state success. Receive
returns the source no-event error after atomically writing only user/type
bytes8..45 and data bytes308..16700; the 16704-byte structure's size, explicit
padding, reserved bytes and alignment tail remain unchanged. Property lookup
validates only null inputs/zero size, writes one NUL and returns value-not-found.
Source host diagnostic string/pointer logging is excluded, with no input
payload read or full-buffer access inferred from it. Bounded actual staging
capacity, retained work/write accounting and checked same-owner transactional
output replace raw stores. These complete four pinned fallback contracts do
not supply Game Intent production, event parsing, network delivery or PSN.

The libc mspace family follows the GPLv2 source
[AnyPS5 Mspace.cpp](https://github.com/boykopovar/AnyPS5/blob/68323bae7494bc4b5d19776c02b4be86f35536c9/core/libs/prx/libc/src/Mspace.cpp),
commit `68323bae7494bc4b5d19776c02b4be86f35536c9`, Git blob
`01e8de2a22cb1bfd13a5a2441dc96e3a0e3e28f3`. Its public export tooling,
SysV ABI declarations and 40-byte managed-size layout establish 13 regular and
10 PAF function contracts. One separately observed import admits the exact
`libSceLibcInternal` module 1.1 / library 1 profile for Create. Related exports
are restricted to this same profile; other names and version aliases are not
inferred from the upstream source, which does not declare SCE descriptors.

PentaX uses caller-provided, same-owner RW/NX guest backing. Create reserves the
first 64 bytes logically without writing a header or creating a mapping;
Destroy removes only its own arena metadata. Best-fit allocation, 32-byte
rounding, coalescing, nesting inside live allocations, adjacent growth, copied
realignment, temporary old-plus-new peak usage, PAF owner arguments and source
statistics/error ordering follow that pin. Parent destruction/free does not
invent cascading child cleanup. Later operations revalidate selected backing.
Source errors use the current guest thread's actual errno cell; unsupported
backing, resource limits and PAF options remain typed host-side refusals.

Private fallible plans precede the sole guest output and infallible metadata
commit. Live arenas share the kernel handle quota and have an explicit libc
heap census; aggregate chunks are limited to 65,536 and retained plus staged
metadata to 16 MiB. Staged vector capacities and copied/zeroed payload share
existing scratch, I/O, byte and work budgets. No host pointer, allocator address,
global host errno or invented successful allocation is exposed to the guest.

The [upstream review](upstream-review.md) records six additional repositories
at immutable revisions, their relevant memory/rendering designs and bounded
follow-up work. Reviewing a repository does not mean its code is included.

The optional Windows sparse-file physical backend is original PentaX Rust,
implemented from the Microsoft file-mapping, sparse-file and access-control
contracts linked in the [native-memory documentation](../crates/pentax-platform/src/native_memory/README.md).
No implementation from those additional repositories is copied into this backend.

The canonical `LibcInternal` dependency flag follows public
[KytyPS5 libC.cpp](https://github.com/KytyPS5/KytyPS5/blob/f186b35706ddbc3faf5513f326e2335d224802f7/src/libs/libC.cpp#L612),
revision `f186b35706ddbc3faf5513f326e2335d224802f7`, blob
`7ed98856bb8d3bc972875525b3d8e7e6fd85b47a`: a mutable namespace-static
`uint32_t` initialized to one, registered through `LIB_OBJECT` under library1
and module1.1. The same contract exists in original MIT Kyty revision
`4733b7e1c91b10554a52007903d74dc76c39a230`,
`source/emulator/src/Libs/LibC.cpp`, blob
`c424fdc77102c93cb63f3036cf7667ca319ce582` (copyright2021 InoriRus).
PentaX uses four aligned little-endian bytes in each guest process's RW/NX data
page, preserving guest writes rather than returning a host-global pointer or
reinitializing the object at lookup. Its GLOBAL/PROTECTED ELF metadata is PentaX's
generated-provider policy, not information supplied by `LIB_OBJECT`. This does
not assert firmware-wide layout or complete libc support. That public revision's
`src/loader/symbolDatabase.cpp` normalizes the `libSce` prefix separately for module
and library names before comparing complete version/type keys. PentaX registers
this object's four resulting canonical/prefixed scope pairs, sharing one
storage object. The same finite pairs are registered for the thirteen existing
memory/string functions observed under `LibcInternal` library version 1 and module
1.1 in pinned `libC.cpp`. Their 39 additional ELF symbols reuse the existing
canonical stubs and service IDs; no new libc behavior is implemented. The two
trace-query registrations remain unchanged. This is an explicit generated
catalogue extension; ordinary resolver key comparisons remain exact, with no
prefix rewriting or NID-only fallback.

The four legacy PREFETCHh locality hints are mediated as ignored hints in
original Rust, following [Intel SDM Volume 2, PREFETCHh](https://cdrdv2-public.intel.com/835757/325383-sdm-vol-2abcd.pdf).
They do not compute an effective address, access guest data or issue a host
prefetch. Exact encodings, prefix/declared-feature policy, continuation authority
and bounded work remain checked; cache flush and other prefetch extensions are
separate unsupported operations. No manual text or upstream implementation is
copied into the handler.

The exact VEX.128/256 VMOVNTDQA load forms extend the original Rust vector
move engine using the [Intel SDM Volume 2B MOVNTDQA contract](https://cdrdv2-public.intel.com/835752/253667-sdm-vol-2b.pdf).
Only the optional streaming cache hint is ignored. Complete aligned guest data
reads, upper-lane behavior and failure-before-publication rules remain checked.
No manual text, upstream implementation or host AVX instruction is used.

The exact VEX.256 VPERMPS and VPERMD forms use the original checked vector
engine following the [Intel instruction reference, VPERMPS and VPERMD](https://cdrdv2-public.intel.com/782156/325383-sdm-vol-2abcd.pdf).
Each destination dword selects across the complete eight-dword source; indices
are snapshotted separately from source data. Full memory spans remain required
even when some source elements are never selected. This is bitwise Rust
mediation, with no host AVX execution or floating-point conversion. No manual
text or external implementation is copied.

The system display-query ABI is adapted from pinned
`src/libs/libSystemService.cpp` at `209dffe9032d72749257ce18680d506b891284de`:
the HDR luminance query returns that emulated profile's three float fields,
and the safe-area query writes only its ratio while retaining reserved bytes.
PentaX replaces raw pointer stores with one checked transactional output and
retains the source null-parameter error. Exact reviewed namespace spellings
and versions remain required. These queries report the pinned emulated display
profile; they do not inspect or establish host HDR rendering support.

The application-content integer query adapts pinned `libAppContent.cpp`:
the low32 parameter ID, FULL SKU constant3, absent-metadata zero for the four
user-defined parameters, and exact int32 output. Unknown IDs preserve the initial
zero store before an explicit unsupported refusal; null outputs retain a typed
source-abort refusal. PentaX replaces raw pointer stores with checked outputs.
The selected-image profile supplies no SFO metadata; loaded-metadata integration
and content mounts remain unfinished. This query does not grant an entitlement.

NetCtl adapts the ten registrations in pinned `libNet.cpp` and the corresponding
`network.cpp` disconnected-control implementation at the same revision. Per-owner
fixed callback slots replace global records; checked guest stores preserve the
source's state/result/NAT/info ABI, lifecycle and error ordering. Native callback
continuations retain the disconnected event, slot iteration and mark-after-return
contract while independently attesting the owner, source, thread and actual call
frame. Host adapter queries and online branches are disabled in that pinned
profile; PentaX does not fabricate their results or call guest pointers as host
functions. Recursive checks remain explicitly unsupported.

The thirteen network startup exports additionally adapt `src/libs/network.cpp/h`,
`src/libs/libNet.cpp` and `src/libs/errno.h` at
`209dffe9032d72749257ce18680d506b891284de`: NetInit/GetNetErrorAddr,
pool create/destroy, resolver create/destroy/abort, SSL init/term and CA get/free,
and HTTP init/term. Exact reviewed Net/Ssl/Http and `libSce` spellings retain
library1/module1.1 scopes. Bounded process-owned metadata replaces global tables;
lowest-free pool IDs1..31, resolver IDs0..31 and 32 typed SSL/HTTP slots retain
source reuse and dependency/error ordering. Destruction does not cascade into
dependents. Signed negative pool sizes remain accepted metadata; source-aborting
flags/zero sizes and unsafe null pool names remain typed refusals. Resolver abort
retains the source's argument-ignoring no-op, without claiming cancellation.

Pinned `libNet.cpp` declares SSL module1.1/library1 only. A closed metadata
inspection separately observed the known public `SslInit` and `SslTerm` import
identities with both module and library named `libSceSsl`, module2.1/library1.
PentaX adds exactly those two bindings using the reviewed one-argument
`SslInit(uint64 size)` and `SslTerm(int32 id)` ABI and the same owned lifecycle
state. This is an explicit PentaX compatibility inference from the requested
scopes, not an upstream 2.1 declaration or proof of complete SSL SDK support.
It supplies no 2.1 CA exports, unprefixed 2.1 aliases or other version bindings.

The [platform startup hook](../crates/pentax-platform/NOTICE) adapts
`EnsureSocketBackend` and requests actual Windows `WSAStartup(2.2)`. Successful
initialization is retained and failed initialization may retry; NetInit retains
the pinned zero return even when the backend reports failure. Neither NetInit
nor the source NetTerm clears the metadata tables. NetTerm is unregistered in
the reviewed export table, and the pinned source contains no WSACleanup.
A separate checked four-byte cell per guest thread replaces host thread-local
net errno: successful Net wrappers preserve it, negative results retain the
source conversion, and SSL/HTTP results leave it unchanged. Guest pointers are
never exposed as host TLS addresses.

SSL CA get/free preserve null-before-ID checks and replace raw stores with one
checked transaction over the exact 24-byte object. Both clear the two pointers
and count; Get then returns NOT_FOUND and Free returns zero without reading or
freeing the prior pointers. This subset allocates no network pool backing,
loads no certificates and provides no connectivity, DNS execution, sockets or
packet operations. The [kernel notice](../crates/pentax-kernel/NOTICE) retains
the source attribution; admission limits and fault-atomic writes are PentaX
policies, not additional firmware behavior.

The HTTP2 metadata subset adapts twenty-one `LibHttp2` registrations and
implementations in `src/libs/libNet.cpp` at
`209dffe9032d72749257ce18680d506b891284de`, retaining exact Http2/libSceHttp2
library1/module1.1 scopes. Context, template and request lifecycle, header/content
metadata, boolean/SSL options, callback registrations and timeout setters replace
global maps with bounded per-process Rust records. Context initialization retains
its four arguments without Net/SSL dependency or size/concurrency checks. Three
independent monotonic IDs start1 and never reuse deleted ordinals. Context and
template deletion retain their source cascades. Overflow is an explicit refusal.

Opaque source strings retain nullable-user-agent and required-null-before-ID
ordering. PentaX charges actual retained text capacities against an aggregate
`max_io_bytes` bound and header records against an aggregate `max_handles` bound;
all three object families additionally share the kernel live-handle allowance.
Checked reads, bounded work and pre-publication allocation replace unchecked
pointer/string operations and unbounded source containers. These limits are
PentaX policies, not firmware capacities. Header mode remains ignored and
duplicates append; request options take lookup precedence over colliding
template IDs, while new requests retain independent false/zero/null defaults.
Opaque callback words are stored without invocation. The metadata calls have no
guest output spans and preserve both errno domains. An original supplied-image
consumer reaches all twenty-one exports through the production worker and
observes real errors, ID progression and cascading cleanup with a final
independent owner census. This subset supplies no send/read, response or async
operations, pool backing, HTTP transport or TLS guarantee.

The nineteen NpWebApi2 operations adapt
[`src/libs/libNet.cpp` at f186b35706ddbc3faf5513f326e2335d224802f7](https://github.com/KytyPS5/KytyPS5/blob/f186b35706ddbc3faf5513f326e2335d224802f7/src/libs/libNet.cpp#L2010),
verified Git blob `1667ec6e6e16f412b71cd2746afc1bcee7f7c106`.
The public registration table at lines2356..2375 supplies exact NIDs and
module1.1/library1 scopes. The same pin's
[`symbolDatabase.cpp` lines10..20](https://github.com/KytyPS5/KytyPS5/blob/f186b35706ddbc3faf5513f326e2335d224802f7/src/loader/symbolDatabase.cpp#L10)
removes a leading `libSce` independently from module and library names. PentaX
declares only the four resulting NpWebApi2/libSceNpWebApi2 combinations, with
ordinary exact version/type matching and all previous provider sites preserved.

Bounded per-owner Rust contexts replace the source context map, retaining pool
rounding, positive IDs, termination and statistics error ordering. Statistics
write the 28 defined field bytes without assigning tail padding. Requests retain
opaque group/path/method bytes and their independent IDs; delete/abort, header
queries and empty-response reads preserve the local source behavior. Send resets
response state, publishes checked optional outputs and returns NOT_SIGNED_IN;
it performs no HTTP or PSN operation. User, push-handle, filter and callback
creation bodies mostly log arguments and advance counters upstream. PentaX
retains bounded data-only receipts for ownership/accounting, without inferring
authentication, push subscriptions, queued events or callback execution.
Logging-only header/push deletion helpers and the empty timeout tick grant no
additional behavior. Checked transactional outputs, fallible preallocation,
monotonic-ID exhaustion, shared live-handle quotas and retained-capacity/work
limits are PentaX resource policies rather than firmware guarantees.

The RUDP bootstrap subset adapts the three registrations and implementations in
`src/libs/libRudp.cpp` at `209dffe9032d72749257ce18680d506b891284de`:
`RudpInit`, `RudpEnableInternalIOThread` and `RudpSetEventHandler`. Owned
initialization and handler/argument state replace the source globals. Pool
arguments are ignored, initialization preserves the handler, and handler
replacement retains null values without dereferencing or calling guest words.
Enabling the internal I/O thread remains the source's explicit no-op, including
ignored stack-size/priority arguments and no initialization prerequisite. These
three source zero results establish neither a running I/O thread nor RUDP
transport, packet handling or event delivery.

The APR path-resolution ABI adapts `AprShared::ResolvePathsCommon`,
`ResolveOnePath` and `Apr::ResolveFilepathsToIdsAndFileSizes` from the same pinned
`src/libs/libAmpr.cpp`. Exact guest strings retain stable sequence IDs and cached
misses; live per-item reads and ordered ID/size/error-index stores preserve
successful prefixes and input/output aliases. PentaX uses its read-only directory
capabilities, bounded fallible metadata cache and checked output transactions,
with the existing active-thread POSIX errno conversion. No host path is formed
from guest text, no tree inventory is required and no file handle is retained per
cached identity. Failed current-item publication retains no new cache entry;
distinct physical output aliases remain explicitly unsupported. This is metadata
resolution, not asynchronous file-command execution or proof of a title frame.

AMPR read-record construction and measurement further adapt the pinned
`libAmpr.cpp` numeric range checks, 20/24-byte record sizing, opcode, retained
arguments, header commit ordering and gather continuation. The Rust port reads
the seventh SysV argument through checked captured-stack access and preserves
direct kernel result widths. Bounded owned queues replace global vectors;
ordered transactional overlays preserve record/header virtual aliases and
refuse unsupported physical aliases. Source exact-object/queued-object lookup,
cold snapshot retention and reset/clear/destruct lifetimes remain explicit.
Constructing a command neither resolves its file ID nor performs its read;
submission executes those retained arguments against the same file owner.

The four APR submit/wait exports further adapt pre-execution submission receipt
allocation, inner-error/record-offset completion, sequential ID/result stores,
and consuming waits from the same pinned file. Current-size positional reads
preserve short/EOF tails and successful prefixes. Bounded capability-backed IO,
typed guest/resource failures and per-owner receipt limits replace raw copies,
ambient paths and global maps. Exact-name hash collision chains and an indexed
successful-ID table retain each name once while avoiding per-read path scans.
No other command kinds or asynchronous completion events are implied.

PentaX includes modified Rust adaptations of
[KytyPS5-Performance](https://github.com/WerrySs/KytyPS5-Performance/tree/209dffe9032d72749257ce18680d506b891284de)
at commit `209dffe9032d72749257ce18680d506b891284de`, derived from
[KytyPS5](https://github.com/KytyPS5/KytyPS5). The pinned fork is private;
the public upstream remains a research reference, not a claim that its current
source is identical to the pinned fork.

The source project specifies GPL version 2 only. PentaX's combined distribution
and Cargo metadata therefore use `GPL-2.0-only`. Original files marked
`GPL-2.0-or-later` retain their individual broader grant. Adapted files carry
their specific SPDX notices; this document does not replace those grants.
The full GPL version 2 terms are in [LICENSE](../LICENSE). The original
`Copyright (c) 2021 InoriRus` notice and complete MIT permission and warranty
terms retained by Kyty are preserved in [Kyty-MIT.txt](../LICENSES/Kyty-MIT.txt).
Binary third-party notices include both this provenance and that full notice.

The checked Rust string-instruction mediator is original PentaX code based on
the architectural MOVS/LODS/STOS/CMPS/SCAS and REP definitions in the
[Intel SDM Volume 2B](https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-2b-manual.pdf).
It uses the existing decoder, owned backing and integer comparison flags;
hardware reference tests operate only on original test arrays. No
upstream implementation or manual text is copied into this component.

The bounded VEX equality, byte-mask and flag-test mediator is original Rust
based on the PCMPEQB/W/D/Q, PMOVMSKB and PTEST definitions in
[Intel SDM Volume 2](https://cdrdv2-public.intel.com/789581/325383-sdm-vol-2abcd.pdf).
It models exact decoder-selected 128/256-bit forms over retained guest state;
original test inputs cover source aliasing, element widths, mask order, flags
and permission failures without executing host AVX instructions. No manual
text, title code or private binary-derived fixture is copied.

Event queues adapt pinned `src/kernel/eventQueue.cpp/h` (Git blobs
`0ebe8a26b1eedba2d9017b3da80e2f432b200b5c` and
`d043c34f00ef9772f0d948457ac04ea2cf0add5c`) and the sixteen exact libkernel
registrations for creation/deletion/waits, user events, timers and field getters.
PentaX uses bounded owner-local records and actual cooperative caller
continuations instead of global host objects or blocking host condition
variables. Default pending-event coalescing/order and timer semantics remain
explicit; optional unbounded legacy delivery and invented graphics/video/AMPR
notifications are not introduced. Guest names/records are request data and
never host pointers. Original fixtures and controlled clocks establish these
contracts without copying title bytes.

Graphics notifications further adapt pinned `renderer/sync.cpp/h`,
`renderContext.cpp/h`, `referenceClock.h`, `kernel/eventQueueFilters.h` and
the four AGC event wrappers/registrations. Owner-local GRAPHICS subscriptions,
source saturation/reset behavior and exact getter fields replace global host
callback maps. An exclusive prepared delivery admits all targets before label
writeback and publishes only after an actual admitted release retires behind
prior Vulkan fences. Native/vendor RELEASE_MEM forms retain source data and
interrupt selectors and the masked context; host TSC scaling follows the source
100MHz arithmetic/overflow domain. GDS and general graphics/presentation remain
unfinished. No original fixture copies guest shader/packet bytes from a title.

The packed string comparison mediator is original Rust derived from the
PCMPxSTRx validity and immediate-control specification in section 4.1 and the
four instruction definitions in
[Intel SDM Volume 2B, June 2024](https://cdrdv2-public.intel.com/825757/253667-sdm-vol-2b.pdf).
It models both legacy and VEX forms with implicit or signed explicit lengths.
Fixed SSE4.2 hardware references use original arrays only; production execution
uses Rust semantics over admitted guest snapshots and never executes host AVX.

## Adapted components

The kernel data exports adapt the pinned `libKernel.cpp` stack-check guard and
program-name OBJECT identities, guard initializer and 511-byte name bound. The
Rust factory owns a separate writable, nonexecuting guest page and one admitted
RELATIVE relocation to its name buffer. Entry preparation writes the name buffer
and argc/argv together before preloading. Ordinary weak-address resolution and
the opt-in missing-weak zero-value profile are original PentaX policies based on
the [ELF symbol binding rules](https://gabi.xinuos.com/elf/05-symtab.html#symbol-binding).
The pinned fork's unresolved-import call stubs are not adopted. TLS and loaded
symbol queries retain their separate policies; existing live bindings cannot
change when a later module introduces a strong provider.

The native atomic prefix policy and mandatory operand admission are original
PentaX code, based on LOCK and compare-exchange in
[Intel SDM Volume 2A](https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf).
Only decoder-validated forms pass existing semantic/feature gates. Natural
alignment and mandatory isolated owned-memory checks are PentaX admission
policies. Native emission retains the CPU instruction; no upstream code or
manual text is copied into an emulated load/store sequence.

Direct kernel-library error results additionally follow the pinned
`kernel/fileSystem.h`, `memory.h`, `semaphore.h`, `eventFlag.h`, `pthread.h`
and `libs/errno.h`, checked against their exact `libKernel.cpp` registrations.
The read/pread/lseek `int64_t` functions promote negative kernel error ints;
the audited `int` functions retain the low 32-bit code, with zero extension
as PentaX's result-register policy. `invoke_library` performs one actual typed
service call and converts only supported source errno in these explicit domains.
Raw invoke, unbound domains, unsupported semantics, allocation, policy limits
and backing faults retain typed failures. Multi-output overlap is an explicit
Unsupported admission policy. POSIX wrappers that set thread-local errno and
return -1, and raw-kernel-syscall flag conventions, are separate unported ABIs.

The retained-file module path adapts the pinned `KernelLoadStartModule`
existing-program branch and `Program::load_count` (initially zero): an exact
explicit guest-file capability identifies an actual supplied shared module,
and a completed observed preload allows reference-count reuse with optional
int32 zero output and the real handle. Original-input comparison, bounded Rust
registry state, transactional output/count admission and opaque native-return
evidence are PentaX ownership rules. The explicit new-file path additionally
adapts `LoadProgram`, `Relocate` and `StartModule(args, argp, null)`, starting its
real load count at one, recording the actual initializer int32 result and
returning its real handle. A negative initializer result remains an explicit
stop. Rust file tickets, preserving source append, transactional native staging,
suspended nested calls, active epoch proofs and cumulative budgets are PentaX
ownership rules. Host path lookup, stop/unload and complete
startup remain unfinished. No host module pointer is passed through the ABI.

The `tls_get_addr` service adapts the pinned two-word `TlsInfo` descriptor and
module-plus-offset lookup. The source's internal host `Program*` is replaced by
the private runtime identity produced by PentaX's typed DTPMOD64 relocation.
Lookup returns the actual prepared guest block of the admitted main thread;
host pointers are never recovered from guest words. Eager bounded Variant II
storage, append-only TLS templates, unchanged old offsets, checked extents and
copy/rollback transactions are explicit PentaX policies. They do not reproduce
the source's host-thread allocator or establish firmware DTV compatibility.

The explicit native preload additionally adapts pinned `RuntimeLinker::Execute`
and `StartModule`: one caller-selected shared module's biased `DT_INIT` is called
with three zero arguments before main entry. Core preparation rejects duplicate
tags, unsupported module roles and targets outside that module's initialized
immutable source. The worker retains one actual thread/TLS and cumulative engine
across calls, preserving guest writes and stopping on unavailable semantics.
This does not copy host-stack expansion, patches, unresolved-success stubs or an
input-order constructor sweep, and does not implement module shutdown.

The image-bound `KernelGetProcParam` path adapts pinned `libKernel.cpp` and
`RuntimeLinker::GetProcParam`. The Rust owner uses the explicit primary executable,
checks its whole live parameter extent and returns a guest integer address.
Unbound integration, empty extents and shared-primary parameter shapes remain
explicitly unsupported. Missing primary metadata retains the source null result;
no host-pointer cast, parameter snapshot or guessed structure is used.

The opt-in loaded-symbol query additionally adapts pinned `libKernel.cpp`
`KernelDlsym`, `kernel_symbol_to_nid`, `sha1_digest`, and
`src/loader/symbolDatabase.cpp` lookup order. The Rust transform streams bounded
name/salt blocks with no allocation. Canonical raw-NID fallback stays inside one
actual supplied image; ordinary import resolution and exact declarations are
unchanged. Weak/absolute/unsupported forms, same-rank ambiguity and formatted
database-name prefixes refuse. The host `scriptingGetMem` callback fallback and
library-name alias normalization are not retained. Explicit int32 miss/null
outputs preserve the pinned behavior through checked guest transactions.

| Rust component | Pinned source | PentaX modifications, 2026-10-17 and 2026-10-18 |
| --- | --- | --- |
| `pentax-platform/src/windows/entry.rs` and `stack.rs` | `src/loader/runtimeLinker.cpp` and `runtimeLinker.h`, `RunEntry` and fixed argument layout | Owned guarded RW/NX stack, private original fixture, checked layout, Win64 nonvolatile GPR/SIMD/control-state preservation and TEB restoration; no arbitrary guest entry API |
| Core prepared main-call context | `src/loader/runtimeLinker.cpp`, `EntryParams`, `Execute` and `RunEntry` | Source/owner-bound checked entry storage, argc=1 and three argv slots, owned terminated program name, zero root frame and mediated outer return on a guarded RW/NX stack. One transaction follows complete target/guard/budget preflight. The original consumer verifies actual argument/frame reads; module lifecycle and the supplied callback's behavior are not inferred |
| Core TLS storage, main-thread admission and modeled segment access | `src/loader/runtimeLinker.cpp`, `TlsGetAddr` and `TlsMainGetAddr` | Explicit observed 64-byte zeroed self-pointer header, owned RW/NX page, moved upper guard and checked main-thread provenance. Guest FS MOV accesses use checked Rust memory semantics; raw byte scanning, host-segment mutation and removal of FS stores are not used. No complete firmware TCB/DTV or scheduler is claimed |
| `pentax-core/src/process/hle_strings*` | `src/libs/libC.cpp`, string routines and registrations | Nine bounded guest-memory services, explicit work/byte/scratch accounting, fallible whole-write preparation and structured failures; guest integers never become host pointers |
| `pentax-shader` | `src/graphics/shader/recompiler/frontend/decode/{ShaderDecoder.h,ShaderDecoder.cpp,OpcodeTable.h,ScalarAluOps.cpp,ExportOps.cpp,VectorAluOps.cpp,MemoryOps.cpp,ImageOps.cpp}` and `src/graphics/shader/shader.h` | Owned linear syntax span, fallible capacity/work limits, complete observed family maps, vector modifiers, memory/image/address metadata and typed unsupported cases; see the crate's [NOTICE](../crates/pentax-shader/NOTICE) |
| `pentax-kernel` and `pentax-core/src/process/hle/kernel_*` | `src/libs/libKernel.cpp`, `src/libs/libAmpr.cpp`, qualified registration metadata and the kernel/system service sources listed in the crate's [NOTICE](../crates/pentax-kernel/NOTICE) | Exact scoped provider identities, bounded stateful service subsets, AMPR header/binding lifecycle, explicit unsupported branches, read-only file capabilities and transactional outputs over the prepared native owner; the kernel crate has no independent execution entry point or firmware ABI claim |
| `pentax-platform/src/native_memory/windows/transaction.rs` and `windows/physical/release.rs` | `src/kernel/memory.cpp`, mapping publication, shared physical backing, GetAllocatedSpan/FindMappings/ReleaseDirectMemoryInternal and rollback paths | Prospective output/alias checks, private/physical backing, virtual and physical subrange release, retained placeholders and section offsets, staged rollback metadata, no-allocation output copies and poisoning after failed restoration or irreversible partial closure; pools and host cache modes remain unsupported |
| `pentax-shader-compiler` | Shader CFG, frontend, IR passes and SPIR-V backend listed in the crate's [NOTICE](../crates/pentax-shader-compiler/NOTICE) | Safe bounded Rust subset with typed stage/resource profiles, iterative graph/SSA walks, fallible arenas, actual output feature manifest and explicit unsupported forms; original programs independently validated with the pinned official SPIR-V Tools |
| `pentax-gpu/src/compute*` and `pentax-core/src/gpu.rs` | Pinned `pm4.h`, `pm4Handlers.cpp` compute register/dispatch paths and `shader.cpp` initial compute ABI | Explicit known state, bounded acyclic single-buffer wave32 compilation, owned input/push data, same-owner pause through GPU completion/writeback, bounded stateful multi-dispatch/indirect call-chain plans, ordered memory writes/waits, live indirect arguments and alias/permission admission; asynchronous scheduling and presentation remain unfinished |
| `pentax-core/src/gpu/driver.rs` and HLE graphics provider/runtime | `src/libs/libAgcDriver.cpp`, `src/libs/agc.{h,cpp}` and `src/graphics/guest_gpu/graphicsRun.{h,cpp}` | Exact Graphics5Driver and paired Graphics5/libSceAgc 1 / module 1.1 exports, four checked single/batch compute-submit ABIs and AgcSuspendPoint frame-boundary/reset state after synchronous completion; source null/zero-count ordering, bounded metadata capture and cumulative same-owner GPU admission; independent compute state per observed ACB queue, terminal partial-execution failure policy and original native x64 consumers; no draw/async/presentation claim; see the [core NOTICE](../crates/pentax-core/NOTICE) |
| `pentax-platform/src/vulkan` | Host Vulkan device, resource, descriptor, pipeline, command and readback paths listed in the module's [NOTICE](../crates/pentax-platform/src/vulkan/NOTICE) | Owned Rust RAII resources, closed factories and opaque prepared compute products, bounded dynamic dispatch/resource contracts, explicit synchronization and fenced readback; no caller-supplied raw SPIR-V or complete guest renderer |

The compiler also transcribes selected official SPIR-V enumerants from Khronos
SPIRV-Headers at `01e0577914a75a2569c846778c2f93aa8e6feddd`.
The original source-header notice remains in `spirv/spec.rs`; the complete
applicable MIT terms and Khronos modification notice are retained in
[Khronos-MIT.txt](../crates/pentax-shader-compiler/Khronos-MIT.txt) and included
in binary third-party notices. The compiler's default rejection of undefined
guest registers does not change the pinned source's false defaults for private
scalar-mask cache metadata.

The generated HLE provider catalogue uses the pinned public-source registration
identities and an original ELF producer. Their presence is not a claim of a
complete firmware ABI or implementation of every registered upstream service.
Upstream no-op and success placeholders are not treated as implemented coverage.

The kernel POSIX subset also adapts pinned `src/libs/libKernel.cpp` wrappers,
`errno.h`'s `KernelToPosix` and `errno.cpp`'s initially zero thread-local cell.
Lexical registration scopes are retained: `Posix` is a distinct versioned library
inside module `libkernel`, even when a libkernel initializer calls its function.
Host `thread_local` storage is replaced by a checked guest-owned initial-thread
RW/NX page with stable pointer identity, resource accounting and protected
lifetime. The source's int64 fstat wrapper retains its distinct -1 promotion.
Success preserves errno; unsupported source branches and backing faults do not
become source errno. Additional guest-thread/TLS ABI compatibility is not inferred.

Read-only directory roots retain the pinned filesystem mount concept while replacing host pathname concatenation with [cap-std 4.0.3](https://docs.rs/cap-std/4.0.3/cap_std/fs/struct.Dir.html) capability-relative lookup. Guest canonical-name and Windows alias/device policies, explicit root admission, per-call resolver work and retained read-only handles are PentaX policies. No directory tree is discovered at admission, and no ambient host fallback is retained. Directory reachability, source-packed descriptors/enumeration/read/seek/status now adapt pinned fileSystem.cpp/h, common/hash.h and common/file.cpp/h, with dot/type behavior reviewed in sysWindowsFileIO.cpp and sysLinuxFileIO.cpp. Name hashes retain the unusual reversed 1..3-byte tail; no standard hash is substituted. Host-order entries use their own type without following outside targets, and logical dot entries require no parent lookup. Checked little-endian packing, bounded owned buffers, aggregate storage accounting, atomic payload/base outputs and post-commit cursor updates replace unchecked pointers and partially published source state. Directory pread remains explicitly unsupported. The new dependency uses its own applicable license terms, retained in complete binary third-party notices.

Guest-thread mutexes additionally adapt pinned `kernel/pthread.cpp/h`,
`libKernel.cpp` and `errno.h`: eight-byte pointer slots, copied type attributes,
lazy zero/adaptive initialization, owner/count transitions, recursive overflow,
busy/deadlock/permission errors and relative/absolute timedlock conversion.
POSIX pthread wrappers return positive errno without changing the errno cell;
direct functions retain int32 kernel errors. Zeroed guest prefixes retain the
pinned reserved256-byte mutex and reserved64-byte attribute areas; private
locking state stays in bounded Rust vectors. Per-object RW/NX pages and slot
publication/revocation use the existing atomic memory transaction. A shared
handle accounts for both object and mapping. A bounded monotonic address cursor
prevents reissuing destroyed pthread identities. No host pointer or host-private
layout is exposed. A source-bound paused thread with matching admitted TLS is
required before payload access. Protocols 0/1/2 retain pinned Windows metadata
storage and copy-on-initialization. The source comments out winpthreads priority
configuration; all stored protocols therefore retain real exclusion and the
same cooperative wait mechanics without fabricated priority inheritance.
Blocking and timed-lock actions enroll a bounded globally unique ticket for the
resolved mutex and active thread, preserving the original pending syscall.
Ready polling grants no ownership; completion after exact executor-phase and
thread checks acquires a free mutex or reports the source timeout domain.
POSIX absolute realtime values retain the source relative-microsecond conversion;
actual waits use monotonic deadlines and the runtime's bounded idle-delay path.
Waiter-aware destruction is an explicit lifetime guard around source undefined
deletion. Direct invocation without a continuation still refuses foreign
contention. Source-aborting invalid protocols, live-object reinitialization and
static-unload tracking remain explicit limits. No C++ source is vendored.

Pthread keys also adapt the pinned 256-entry process table and generation-tagged
thread-local values. Lowest-free-index allocation and source four-byte key
outputs are preserved. Bounded Rust ownership replaces process-global host
state; one admitted initial guest thread owns the lazily allocated value table.
Guest output commits before allocation is published. Deletion advances a
generation so recycled keys cannot reveal stale data. Guest values and retained
destructor words remain opaque u64 integers. Direct/POSIX errors and the null
pointer result domain retain their source distinctions without errno writes.
Thread-exit/destructor execution remains unsupported; registration does not
establish a full guest-thread lifecycle or expose host callback capabilities.

POSIX `pthread_once` adapts the pinned validation, four-byte state and void
initializer completion ordering. The source's direct host function-pointer
call is replaced by a source/owner-bound core ticket, checked nested frame and
opaque actual return. Matching executor/call sequence, target, stack/guards
and arguments are required before DONE publication; request possession is
not execution evidence. The same guest TLS/stack/kernel budgets and suspended
caller survive nested once calls and explicit module initialization across
source append. Nested return preserves the continuing guest MXCSR environment
instead of replacing flags raised by the initializer with a caller snapshot;
the independent original consumer raises and observes real masked divide-by-zero.
Waiting, signals and cancellation remain unsupported. No
upstream success placeholder or direct guest-to-host callback is retained.

The native bridge is verified with original isolated diagnostics. Its prepared
loop combines checked ordinary memory accesses, modeled guest FS operations and
attested services; this does not establish guest fault recovery or commercial-game
startup. The shader compiler emits SPIR-V for its documented subset, and separate
original Vulkan compute/graphics diagnostics verify completed readback. No game
rendering, presentation or compatibility follows from those diagnostics.

`KernelSetThreadDtors`, `KernelSetThreadAtexitCount` and
`KernelSetThreadAtexitReport` adapt pinned `src/kernel/pthread.cpp`/`pthread.h` and its
exact `libKernel.cpp` registrations and count/report setters. PentaX retains
three independent guest callback slots as numeric process-owned state, preserving the void signature, initially null slot and
explicit nonnull replacement refusal. No host callback cast or cleanup execution
is introduced. Any later thread-exit/module-unload executor must revalidate the live target;
thread exit currently remains an explicit unsupported operation.

`KernelRtldSetApplicationHeapAPI` adapts pinned `libKernel.cpp` and
`RuntimeLinker::SetApplicationHeapApi`. A checked 80-byte guest read replaces
host pointer-array dereferences before publishing slots 0/1/6. Null and missing
required callbacks clear the registration; replacement is allowed. These are
numeric callback records, not native call capabilities. The source's
`scriptingGetMem` lookup hook and allocator invocation are not synthesized by
registration. Existing direct/flexible memory and static TLS retain their own
explicit storage paths.


`__cxa_atexit` registration follows public KytyPS5
`f186b35706ddbc3faf5513f326e2335d224802f7`, `src/libs/libC.cpp`
(blob `7ed98856bb8d3bc972875525b3d8e7e6fd85b47a`), lines517–529 and796,
with `LibcInternal` library1/module1.1 at line612. Its public NID is
`tsvEmnenz48`. Original MIT Kyty
`4733b7e1c91b10554a52007903d74dc76c39a230`, LibC.cpp
(blob `c424fdc77102c93cb63f3036cf7667ca319ce582`), lines88–101/249,
corroborates storing the three opaque words and returning int32 zero.
AnyPS5 `6bd669e40c817b9883e6fae4f69dda69195e546c`,
`core/libs/prx/libc/src/RuntimeSupport.cpp`
(blob `8fc522804af8e98504d22399afb8e621ef581c0f`), lines60–67, and
`VabiMacros.hpp` (blob `1eed45e459bdfbf92547ae2ec669d485a3718a19`)
independently establish the SysV callback/argument/DSO tuple. Nulls and duplicates
are stored; registration performs no dereference or callback invocation and
defines no guest errno mapping for host allocation failures. PentaX retains a
bounded per-process Rust registry and typed admission failures instead of source
global host pointers or an unbounded vector. The public prefix rule supports
only the explicit reviewed scope pairs. The separate `libc`/`libc` registration
is not inferred here. Finalization is not implemented by registration; the
sources differ in null handling, zero-DSO selection and reentrant consumption,
so their shutdown behavior must not be mixed or silently replaced by host atexit.

The `snprintf` intermediate source profile adapts the same public
`f186b35706ddbc3faf5513f326e2335d224802f7` revision. Verified Git blobs:

| Public source | Blob |
| --- | --- |
| `src/libs/libC.cpp` | `7ed98856bb8d3bc972875525b3d8e7e6fd85b47a` |
| `src/libs/guestPrintf.cpp` | `afc61624f385f2e223625db1ff5777c2367c8197` |
| `src/libs/guestPrintf.h` | `0e71438e6d32cf6d14a6fa4ce68dc1eeab1b7184` |
| `src/libs/vaContext.h` | `399d0cc06f43449b88f1627572b2566fa9917445` |
| `src/common/stringUtils.h` | `89f7217e51a29425e4b13f95677ee35bb0b9191a` |
| `src/loader/symbolDatabase.cpp` | `992e43f9b5a81c29e6c4ca614105934c8cfcde78` |

[`libC.cpp` lines612..627/793](https://github.com/KytyPS5/KytyPS5/blob/f186b35706ddbc3faf5513f326e2335d224802f7/src/libs/libC.cpp#L612)
supplies the public NID `eLdDw6l0-bU`, canonical LibcInternal library1/module1.1
and wrapper. The independent module/library prefix rule supplies only the four
finite canonical/`libSce` pairs. The appended original marker preserves all
previous service IDs/sites and data addresses: 17 markers, 67 ELF symbols and the
dynamic table at 0x300 keep its new 0x200 stub separate from metadata.

[`vaContext.h` lines8..55/107..122/199..205](https://github.com/KytyPS5/KytyPS5/blob/f186b35706ddbc3faf5513f326e2335d224802f7/src/libs/vaContext.h#L8)
and [`guestPrintf.cpp` lines796..801](https://github.com/KytyPS5/KytyPS5/blob/f186b35706ddbc3faf5513f326e2335d224802f7/src/libs/guestPrintf.cpp#L796)
establish three fixed GP arguments, remaining RCX/R8/R9, unconditional raw
XMM0..7 capture without an AL gate, independent register cursors and one shared
scalar overflow cursor from the actual guest callee RSP+8. The Rust parser,
integer/float converters and bounded UTF-16 path retain the public algorithms
and quirks, including wrapping width parsing, unknown-specifier output without
a `%n` store, source floating rounding, and the source's final embedded-NUL
prefix copy with full logical return count. No host `snprintf`/`va_list` or
title-specific pattern is used.

The final copy at source lines772..783 aborts on truncation or zero capacity;
it is not graceful C99 snprintf. PentaX instead refuses those cases, malformed
formats, invalid UTF-16 and source-undefined numeric/high-precision cases with
typed pre-write `UnsupportedFormat`. Finite shared work/byte/capacity budgets,
fallible staging and one complete checked guest write are PentaX policies.
This capability does not establish complete libc support.

The pinned formatter credits Marco Paland, 2014-2019, PALANDesign Hannover,
Germany, under MIT, with exponential conversion contributed by Martijn Jasperse.
The [core NOTICE](../crates/pentax-core/NOTICE) retains those attributions and
the full MIT grant, conditions and disclaimer alongside the source tree's GPLv2
terms. The Rust adaptation does not claim original authorship of the algorithms.

The JSON allocator constructor has public NID `-hJRce8wn1U` and C2 export
`_ZN3sce4Json12MemAllocatorC2Ev`. Pinned [KytyPS5 libJson2.cpp](https://github.com/KytyPS5/KytyPS5/blob/f186b35706ddbc3faf5513f326e2335d224802f7/src/libs/libJson2.cpp)
(blob `d6e968f019fa61e0adc5224cd20be1ecd3261487`) supplies the canonical
`Json2` library1/module1.1 identity and a return-this stub. The reviewed public
prefix rule supports four explicitly enumerated canonical/prefixed pairs.
A separate exact `libSceJson` module / `libSceJson2` library identity is
corroborated by the [public shadPS4 diagnostic report](https://github.com/shadps4-emu/shadPS4/issues/4764).
No global name normalization, wildcard version or library-only resolution is used.

The observed constructor effect instead follows one coherent implementation:
[AnyPS5 Export.cpp](https://github.com/boykopovar/AnyPS5/blob/6bd669e40c817b9883e6fae4f69dda69195e546c/core/libs/prx/libSceJson2/Export.cpp#L520)
(blob `744ba1497aadfb6b38518618642be50231a86b58`), table520-529 and
constructor583-585. Its SysV void constructor writes the first pointer-sized
object word to a table address point after two zero prefix words. All six
observed virtual slots have the same terminal handler. PentaX generates its own
placement-bound guest table and marker addresses in immutable guest pages;
it never stores host implementation pointers or throws the source's host C++
exception. A virtual call produces `UnavailableJsonAllocatorVirtual`, without
completing the call. Constructor admission attests the current factory/source,
owner, markers, permissions and table before an atomic eight-byte guest write.
The void transport value is not an object-return contract. The sources do not
establish a complete class size, alignment, RTTI, virtual-slot arguments or
allocator semantics. Destructor, initialization and allocation behavior remain
separate work; return-this stubs do not establish that those effects are absent.

The separate Null-value constructor follows the same exact KytyPS5 `f186b357`
`libJson2.cpp` blob: `JsonValue` lines53-70 asserts a 32-byte representation and
uint32 type at offset28; `JsonValueInit`146-150 zeroes the whole object and sets
Null0. Constructor521-530 explicitly accepts null and returns self. Public
NIDs `qBMjqyBn3OM` and `-wa17B7TGnw` are registered at1092-1093. PentaX writes
exactly32 zero bytes through the paused guest owner, including the source's
padding initialization. Whole-span preflight prevents partial boundary writes.
Four reviewed canonical/prefixed scopes cover both public identities; only
the separately observed `qBMjqyBn3OM` uses the distinct `libSceJson` module /
`libSceJson2` library pair. These remain exact module1.1/library1 rows.
The generated provider's per-symbol manifest distinguishes this marker from
the allocator constructor and unexported terminal marker. AnyPS5's Value
representation instead uses an opaque host-owned Node pointer and is not
combined with this guest layout. Full JSON allocation, parsing and lifecycle
semantics are separate work.

Primitive Value operations use the same exact pinned KytyPS5 blob without
combining the incompatible AnyPS5 Node representation. `JsonValueClear`253-277
resets parent/root, union and type but preserves four padding bytes. Copy279-294
clears the destination, assigns its type from the source, then reads source
type again to select a payload operation; distinct overlap must observe that
ordering. Constructors initialize all32bytes before later copy/type operations.
`JsonValueClearMethod`1059-1076 additionally restores parent/root fields.
Boolean and integer constructors/setters retain their source register-width
effects; no host bool or C++ container pointer is reinterpreted in Rust.

GetBoolean/GetInteger/GetReal return pointers to the union at offset16 for
matching types, otherwise pointers to distinct stable zero constants. This is
verified in exact source lines744-771: GetReal is `const double*`, not a floating
return register. PentaX supplies separate immutable guest zero words and attests
their bytes and permissions with the allocator table. Double constructor571-580
initializes the complete object, stores type4 and its double payload, and returns
self. SetDouble683-692 clears the old value before storing type4 and its payload.
Their SysV arguments are self in RDI and double in XMM0. PentaX copies the low64
raw XMM0 bits from its actual PRE-SYSCALL snapshot, without a host float cast or
normalization. Both functions append four canonical rows plus their independently
confirmed distinct module/library row, preserving every earlier export and marker.
Null payload cleanup/count follows the source's nullable
helpers; nonzero Array/Object payloads stop explicitly until real ownership and
destruction are implemented. Unknown type defaults follow source dispatch.

Seventeen appended primitive markers use eighteen public NIDs, including two
destructor identities. Their four reviewed canonical/prefixed pairs are finite;
only fifteen individually observed public imports gain the separate exact
`libSceJson` module / `libSceJson2` library pair. No absent TypeCtor, ClearMethod
or alternate destructor identity gains that pair. Existing constructor and
terminal marker addresses and the allocator table address point stay stable.

The String batch uses the same pinned Kyty blob: mutable/const helper overloads
80-98, String creation/destruction162-169/222-228, Value cleanup/copy253-294,
CString/String Value constructors582-602, SetString693-702, GetString728-733,
ToString/Serialize803-827 and String lifecycle829-876. The eight-byte String
wrapper's impl stays opaque; PentaX substitutes checked guest identities and
owned guest byte storage, never a Rust or C++ container pointer. A const Value
still holds a mutable String pointee, so copy/ToString can materialize a zero
impl through the mutable helper. Const null/zero wrappers instead read empty.
Constructors initialize in source order and do not implicitly destroy previous
contents. ToString(String) returns raw contents, and Serialize retains the
source's raw quote wrapping rather than adding unimplemented escaping.
Unported Real/container formatting remains explicit instead of using a guessed
Rust float representation. Twelve new public identities append exact canonical
rows; eleven gain the independently observed fifth scope, while StringAssign
remains canonical-only. Earlier export rows and guest marker/table/default
addresses remain stable during metadata growth within the original code page.

`LibcInternalExt::LibcHeapGetTraceInfo` adapts the same pinned `libC.cpp` source:
the exact extension-library registration, 32-byte Info record, 64-bit mask and
64-entry state table. The original unknown words are preserved. PentaX owns a
zero-initialized guest data page for each provider instance and publishes guest
addresses under retained memory/service bounds. Calls retain guest-written
values; new owners receive fresh storage. The generated provider remains
source-bound, with separate immutable code and non-executable writable data.

The explicit matching `libSce`-prefixed trace-query pair follows the pinned
`SymbolDatabase::UpdateName` prefix rule. It shares the same guest trace storage
and implementation, with independently attested export sites. PentaX does not
apply source-wide name normalization or a global NID-only fallback; unrelated
names and conflicting versions retain exact resolver errors.

`KernelMapNamedDirectMemory` adapts pinned `src/kernel/memory.cpp` and its
seven-argument `libKernel.cpp` registration. The native service token retains
callee RSP; a checked, budgeted read above the return address supplies the
seventh argument only after normal provider/owner admission. The source's null
name and 32-byte length errors retain their int32 result domain. Mapping labels
are bounded byte strings, with no UTF-8 requirement. The existing direct-memory
owner supplies real physical aliases, checked permissions and transactional
mapping/output publication. Missing stack capture and invalid guest backing
remain typed failures; no guessed name or successful mapping is synthesized.

Memory protection decoding follows pinned DecodeMemoryProtection: access derives
from CPU/GPU bits only, while range queries retain the original protection word.
Nonzero words with no defined access bits return EINVAL. Metadata cannot add
unrequested CPU permissions; CPU and GPU access bits are retained independently.
KernelIsAddressSanitizerEnabled follows the pinned disabled profile and the
absence of guest ASan instrumentation, independently of host build sanitizers.


The runtime memory profile adopts only the pinned `PhysicalMemory::TotalSize`
(13824 MiB) and default 1024 MiB flexible subtraction from `src/kernel/memory.cpp`
at revision `209dffe9032d72749257ce18680d506b891284de`. The resulting direct
capacity is a source profile, not a firmware-wide assertion. Host availability
selection, the 32 GiB virtual window and lazy placeholder implementation are
PentaX policies; they do not authorize equal host commitment or additional APIs.


The CPU/GPU access split follows the same pinned `DecodeMemoryProtection`: CPU
bits0x1/0x2/0x4 and GPU bits0x10/0x20 describe distinct consumers, while host data
protection admits the required read/write union. PentaX always keeps backing NX,
enforces CPU rights before native operands and GPU rights before command/resource
reads and output publication. Its synchronous Vulkan bridge retains the exclusive
owner through upload, fence and writeback, making successful results visible to
all physical aliases. This implements no asynchronous GPU cache policy.

The VEX-encoded BMI1/BMI2 GPR admission follows Intel SDM Volume 2A,
section 2.6 and Tables 2-28/2-29 in the
[2024 documentation changes](https://cdrdv2-public.intel.com/812380/252046-sdm-change-document.pdf).
Only the thirteen listed GPR families are admitted with declared feature bits;
actual host feature checks, source attestation, isolated memory validation and
GPR/RFLAGS context handling remain in the native executor. No AVX/EVEX vector
execution or software BMI fallback is introduced. Test encodings and operands
are original, not extracted from supplied software.

VEX broadcasts and packed integer add/subtract follow the VBROADCAST,
VPBROADCASTB/W/D/Q, PADD and PSUB entries in
[Intel SDM Volume 2, 2023](https://cdrdv2-public.intel.com/789581/325383-sdm-vol-2abcd.pdf).
Original bounded Rust loops read scalar/tuple widths, repeat source bits and
wrap arithmetic independently at element boundaries. They retain the prepared
owner's memory checks, source aliases, flags and upper-half behavior without
requiring host AVX. No manual implementation code or binary fixture is copied.

VEX VPSHUFD, VPSHUFHW and VPSHUFLW follow the PSHUFD/HW/LW entries in the
same Intel manual. Immediate selectors operate independently in each 128-bit
lane; word shuffles retain the unselected half. Full 128/256-bit source reads,
alias snapshots and 128-bit upper clearing use the existing checked Rust path.
Fixtures use original operands and explicit independent selection tables.

Condition objects adapt the pinned PthreadCond/Condattr and POSIX wrapper paths
in `kernel/pthread.cpp` and exact `libKernel.cpp` registrations. PentaX retains
reserved prefixes, clock selection, lazy initialization, transactional lifetime
and error domains. Empty-set notifications are valid only under the existing
initial-thread admission; no blocked condition waiter or additional guest thread
can exist in that profile. Waiter scheduling and targeted/timed waits are not
implemented. The generated-provider registration bound is 256 instead of 128
to hold the expanded fixed catalogue; mapped image and execution limits remain.

Read/write lock lifecycle, recursive reader acquisition, writer-owned reads,
reader-first unlock and self-writer EDEADLK follow the same pinned pthread.cpp.
Try-write uniquely does not initialize a zero slot. Attribute types are retained
as arbitrary signed ints; the pinned initializer ignores its attribute argument.
Twenty-two exact public registrations are appended without changing existing
marker indices, including the POSIX-library try functions' direct kernel result
domain. The owned initial-thread state admits no contended scheduler waits or
additional guest threads. Guest prefixes and lifetime transactions reuse the
existing PentaX pthread backing owner.

Thread attributes additionally port pinned PthreadAttrInit/Destroy/Get and the
registered stack, guard, affinity, detach and scheduling accessors. The initial
values, 16 KiB minimum stack size, null/low-pointer distinctions, signed priority
preservation and separate POSIX result domains follow pthread.cpp. Thirty-nine
exact registrations are appended, including scope-specific aliases. Unregistered
reference functions do not acquire invented exports. Rust retains configuration
outside the guest's 64-byte reserved prefix; setting a stack grants no mapping
or execution authority. Initial-thread snapshots use PentaX's actual prepared
entry stack and live guards, not host stack discovery or guessed source geometry.
This adds no thread creation, host scheduling, join or contended wait support.

The unavailable-import diagnostic table contains 441 scoped public LIB_FUNC
registrations from the pinned libKernel.cpp/libC.cpp. It preserves exact module,
library and version identities and returns only static reference function names.
It is separate from callable providers and never changes resolver admission;
unknown input identities produce no public label. Bounded lookup replaces local
temporary instrumentation when identifying the next missing startup service.

The diagnostic-only extension covers the other exact LIB_FUNC registrations in
the pinned revision's src/libs/*.cpp files, including their declared descriptor
versions. All 441 existing labels were checked against the extracted catalogue.
One duplicate Share registration uses the lexically first reference namespace;
its function name and complete export identity agree. No callable implementation
or guessed identity is added by this table.

Unavailable diagnostics retain exact `public_function` matching and separately
report a `reference_function` only when the pinned catalogue has an unambiguous
label for the same NID. `reference_match` distinguishes descriptor-version and
scope differences, ambiguous references and absent entries. These hints neither
identify the input as a compatible export nor alter resolver/provider behavior;
all missing calls still stop without a return value.

Three additional diagnostic-only HTTP2 references adapt explicit Gen5
`Nid`/`ExportName`/`LibraryName` attributes from
[SharpEmu Http2Exports.cs at b0fa905fa10682107813dd4c53c48a18005859d6](https://github.com/sharpemu/sharpemu/blob/b0fa905fa10682107813dd4c53c48a18005859d6/src/SharpEmu.Libs/Network/Http2Exports.cs#L202):
`sceHttp2SetMinSslVersion`, `sceHttp2CreateCookieBox` and `sceHttp2SetCookieBox`
(lines 202-205, 280-283 and 293-296; source Git blob
`c3dd043c06b95462950f1227bce70c3875378849`). The source retains Copyright (C)
2026 SharpEmu Emulator Project and GPL-2.0-or-later; its
[GPL version 2 license](https://github.com/sharpemu/sharpemu/blob/b0fa905fa10682107813dd4c53c48a18005859d6/LICENSE.txt)
and matching copyright/SPDX notice are retained by this data adaptation.
Only a byte-exact `libSceHttp2` library and one of these public NIDs can produce
a static `library-reference` hint, and only when the existing catalogue has no
reference. Conflicting catalogue labels refuse as ambiguous. The source does
not establish a module or descriptor versions, so the data never enters exact
name lookup, provider registration or import resolution. No C# service code,
execution ABI or behavior is copied or inferred, and a hint still leaves the
call unavailable. Unknown input identifiers remain unlabeled and unprinted.

Additional `public-name-hash-reference` candidates derive solely from the
GPL-2.0-or-later [SharpEmu public names corpus at the same immutable revision](https://github.com/sharpemu/sharpemu/blob/b0fa905fa10682107813dd4c53c48a18005859d6/scripts/ps5_names.txt),
Git blob `53f57fa8affbe06af3882bb6b6167eee1b680ed1`. The source
`REUSE.toml` attributes that corpus to SharpEmu Emulator Project without a year.
The source generator trims each nonempty name, hashes its UTF-8 bytes with the
published fixed suffix using SHA-1 and encodes the reversed first eight bytes
as its unpadded NID. A bounded offline pass checks the complete pinned corpus,
deduplicates identical names and refuses distinct-name hash collisions before
selecting protocol-safe public rows. Unknown input identifiers are never
persisted or transmitted; retained names and keys come from the public index.
These are diagnostic candidates, with no library, module, version, symbol-type
or ABI evidence. They never enter exact-name lookup, provider registration or
resolution. Existing public declarations retain their qualification, while
conflicting public labels produce an ambiguous-reference refusal.

The separate `reference_library` category compares an attested import's library
against existing static public library names, optionally removing one exact
case-sensitive `libSce` prefix for comparison. Returned categories are slices
of public constants, never input strings. It supplies neither a function name
nor an export/version binding; libraries absent from the public set stay unnamed.

The application-module path table and load-before-reference ordering follow
[KytyPS5's independently pinned Sysmodule change1125a0013eea7bdf54cda852c515ae0c45ef7e06](https://github.com/KytyPS5/KytyPS5/blob/1125a0013eea7bdf54cda852c515ae0c45ef7e06/src/libs/libSysmodule.cpp),
Git blob `10e43457331ef1a4b6d7475d864e90a003c07ee2`, GPL version2. This public
change lists thirteen modules loaded from `/app0/sce_module` on request and
starts a newly mapped module before retaining the Sysmodule reference. PentaX
adapts that ordering through its existing bounded source-epoch append, relocation,
TLS and real nested-call proof. Only an explicitly supplied file capability may
be selected. The runtime never searches firmware or enumerates game directories.
Original Sysmodule arguments remain bound to the suspended parent, while the
module initializer receives three zero arguments. A successful Sysmodule call
returns zero, independently of the actual positive initializer result or the
distinct KernelLoadStartModule handle. Reentrant mapped self-loads retain their
reference without proving outer completion or recursively starting the image.
Missing initializers and negative starts remain typed failures. Public paths
and metadata are not replacement implementations of the module's exports.

The kernel provider also offers its 46 Posix registrations under the explicit
`libScePosix` spelling, following the pinned SymbolDatabase::UpdateName prefix
rule. These aliases append to the original 191 entries, retain the same Rust
service/result semantics and keep module/library versions exact. No NID-only
resolution fallback is enabled. The generated image still fits one code page
and one data page; existing service marker indices do not change.

AGC public/internal register defaults additionally translate
`src/libs/agcRegisterDefaults.inc` at the same pinned revision. Its Git blob is
`f02dff5f6b023c25e30bec79343d1ee83692be1b`; the Rust data records all121 unique
arrays, source version aliases, derived public table-0 patches for11/12 and the
internal13 register-value patch. `agc.cpp` defines the64-byte guest descriptor,
independent per-version pointer tables and uint32 version fallback to13;
`graphics/shader/shader.h` defines each register pair as two uint32 fields.
Source global pointers/vectors become one bounded per-owner RW/NX guest blob
published through the retained kernel transaction. Original native consumers
and a canonical source-data oracle cover actual pointers, all register/type
values and pointer offsets, aliases, version behavior and resource refusal.

Shader creation further ports `AgcCreateShader`, program-register address
patching and normal `ShaderMapUserData` from the same pinned revision. Header
fields retain exact guest widths and source relocation order; nonnull relative
pointer addition uses modulo-u64 semantics before live memory checks. Retained
shader metadata is per owner, not a host pointer or executable certificate.
The Rust compute path consumes matching declared code extents and revalidates
actual GPU backing. Optional source title-hash overrides are outside this port.

VideoOutRegisterBuffers2, SubmitChangeBufferAttribute2 and UnregisterBuffers
further port the same pinned videoOut.cpp's retained four groups and sixteen
buffers per port. Fixed Rust arrays stage the full registration/replacement
before publication; exact source int32 errors, captured SysV category/option
arguments, ignored reserved descriptor words and close-generation cleanup remain.
The normalized surface port follows imageInfo.h's seven format policies and
two DCC control profiles, tile.cpp's single-level RenderTarget64KB block geometry
and thin4KiB DCC coverage, and image.cpp/regionDefinitions.h's40-bit range checks.
Verified source blobs: tile.cpp16d86b8cb6b2ac6248fe81ddbab3091c0763788e,
imageInfo.h89c3958c887e3a8ba3a196d9fd4c3d05213f8853,
image.cppacd747c79db942657d5ac6ac4eb5290daff64d00.
Source validation does not dereference pixel/metadata backing at registration;
PentaX likewise retains geometry without granting access. Future draw/present
consumers must admit live same-owner backing. No source host vectors, guest
pointer logging or title-specific overrides are copied.

The fused shader size query and both fusion exports port pinned agc.cpp680..824
and its register-search/address-patch helpers, with agc.h SizeAlign, shader.h's
96-byte Shader and eight-byte register layouts, and pm4.h's GS/HS identities.
Rust preserves conditional type/count reads, both checksum occurrences, source
resource masks, modern shared-VGPR recomputation versus legacy maximum, optional
scratch and address-pair behavior, userdata differences and the guest stage-bit
error's already-copied header. Fixed staging preserves source ordering for the
same virtual bytes and refuses unsupported distinct physical aliases. Adjacent
final byte edits become checked contiguous transaction outputs. Hard admission
failures publish no partial state; no shader execution or frame is fabricated.

Both AgcCreateInterpolantMapping variants further port agc.cpp's1215..1443
semantic packing, F16/default fields, first matching output and identity tail,
with shader.h's four-byte ShaderSemantic and pm4.h's SPI_PS_INPUT_CNTL_0.
The functions retain conditional/null reads and source table order, through
checked staged overlays. Source counts above32 remain supported up to a finite
256-record bound; larger or rewritten counts fail without truncated success.
Pure flag results are checked against separately compiled pinned C++ functions:
32768 original cases per variant, covering all F16 modes/defaults, flat/custom/
linear flags, present/missing outputs and hardware-map truncation. FNV64 oracle
digests are e694da7c67365325 and6ac11e27f8dacb25 in the documented fixture loop
order. The project includes Rust fixtures and expected results, not a C++ runtime
or any data from the supplied game.

The FMA3 mediator is original Rust following Intel SDM Volume1 section14.5 and
Volume2C's exact VEX FMA forms, linked in the native-dispatch note. Fixed integer
storage retains every finite product/addend bit before one rounding. No manual
implementation, third-party soft-float code, guest bytes or upstream title
override is copied. Independent original hardware references check copied
synthetic bits only and restore host control before Rust resumes; production
does not execute host FMA or depend on host AVX state. Opcode admission remains
an exact decoder catalogue, excluding EVEX/FMA4.

Runtime physical-section commitment is original Rust using Microsoft's
documented SEC_RESERVE, VirtualAlloc(MEM_COMMIT), placeholder and shared-view
contracts, linked in native_memory/README.md. First-view commitment is tracked
per section offset; no source allocator, guest fault handler, data file or host
policy override is copied. Full retained capacity remains budgeted independently
of actual commitment. Original OS-state/alias/rollback fixtures use synthetic
data only and cover both reserved and eager section profiles.

## Guest thread priorities

The seven priority roles follow `PthreadGetprio`, `PthreadSetprio`,
`Posix::pthread_setprio`, both scheduling-parameter wrappers and the two scalar
priority queries in the pinned authorized fork's `src/kernel/pthread.cpp`
at `209dffe9032d72749257ce18680d506b891284de`, Git blob
`a7fa3003f534af01a119f117954ffe899c982351`. The nine canonical NIDs and exact
module/library scopes follow `src/libs/libKernel.cpp`, blob
`b02a4d77404f839371d96d1f549b53043e77d25e`. All use module `libkernel` 1.1;
the two kernel calls and scalar queries use library `libkernel` 1, while the
five POSIX wrapper entries use library `Posix` 1. No additional prefix aliases
are inferred. Existing registration IDs and provider sites remain fixed.

The modified Rust implementation preserves full-width owned thread handles,
signed low-32-bit priority/policy arguments, four-byte outputs, policy-before-
priority store ordering (including aliases), null error precedence, positive
POSIX errors without errno writes, and unrestricted signed priority metadata.
Invalid guest backing and unowned handles produce typed refusals. Staged writes
and metadata commits replace unchecked source pointers; absent host failures
are not invented. Scalar min/max queries ignore policy and need no thread.

The source maps guest priorities <=478 to high, >=733 to low, and the remaining
values to normal. A created thread starting with single-bit affinity latches a
one-class demotion once; the main thread does not. PentaX preserves those class
rules with an explicit cooperative scheduling policy: bounded turns of 4096,
2048 and 1024 translated blocks respectively. Runnable contexts continue to
rotate fairly; Windows process/thread priority and affinity are untouched.
These ratios are PentaX policy rather than a reproduction of Windows scheduling.
Exact guest metadata remains separate from effective execution class:
`pthread_setschedparam` updates attributes only, while `PthreadSetprio` changes
the running class. Initial child activation uses its then-current attributes;
later affinity changes never relatch its demotion. Priority inheritance and
priority-ordered kernel wait queues are outside this change.
