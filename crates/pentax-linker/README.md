# pentax-linker

`lookup_module_export` is a separate opt-in query over one retained module. It
uses the pinned bounded salted SHA-1/name transform and canonical raw-NID
fallback only within that module's offered export declarations. The ordinary
exact import resolver has no new NID fallback or priority. FUNC precedes OBJECT;
selected-rank ambiguity, weak/absolute/unsupported types and formatted database
prefixes refuse. Query export/key/work bounds are separate from the original
source inspection. Attempts retain charged work even on error, including hash,
metadata, key-copy and target-selection visits, for a cumulative runtime owner.
The streaming transform allocates no payload and has independent hashlib/public
registration padding vectors. Selected metadata borrows its exact source and
provides no live mapping, module-start or execution authority.

Bounded, read-only ELF64 linking metadata and image-bound relocation plans for PentaX.

This crate inspects linking records, plans AMD64 relative writes, and offers
whole-session symbolic planning and typed TLS binding under explicit policies below. It does **not**:

- compute NIDs from names or infer platform version compatibility;
- apply writes to guest memory or run indirect resolvers;
- load modules, map guest memory, or execute guest code;
- implement HLE libraries, runtime TLS storage/lookup, or anything GPU-related;
- read files, write to disk, or keep copies of source bytes beyond the
  returned names and records.

## API

`plan_missing_functions` is a separate source-retaining classification query over
the same exact resolver. It deduplicates absent GLOBAL FUNC identities across
ordinary address relocations while preserving role-scoped names, exact versions
and references. Missing weak/object imports and nonzero R_X86_64_64 function
addends refuse. GLOB_DAT/JUMP_SLOT retain the existing ignored-addend rule.
TLS and unknown relocation records remain separate deferred work. Function,
metadata, key and lookup budgets precede retention; debug summaries omit input
identifiers. The result supplies no address, patch, complete-link result or
execution authority, and `plan_symbolic` keeps its strict behavior. A core
unavailable-function factory must still pass ordinary exact linking/preparation.

```rust,ignore
let view = pentax_loader::open_executable(&bytes)?;
match pentax_linker::inspect(&view, &pentax_linker::Limits::default())? {
    pentax_linker::Inspection::NotDynamic => { /* no PT_DYNAMIC header */ }
    pentax_linker::Inspection::Dynamic(metadata) => {
        let _ = (metadata.symbols(), metadata.rela(), metadata.plt_rela());
        let _ = metadata.counts();
    }
}
```

`inspect` takes only an `ExecutableView` and `Limits`. It derives the runtime
plan internally, so callers cannot supply a forged or mismatched plan that
would select different payloads. The result is an owned `LinkMetadata`, or a
structured `LinkError`. An image without `PT_DYNAMIC` is reported as
`Inspection::NotDynamic`, which is distinct from both failure and a dynamic
section with no tables.

`plan_relative` builds a separate, image-bound plan:

```rust,ignore
let view = pentax_loader::open_executable(&bytes)?;
let plan = pentax_linker::plan_relative(&view, &pentax_linker::Limits::default(), load_bias)?;
let _ = (plan.runtime_plan(), plan.load_bias());
let _ = (plan.patches(), plan.deferred());
```

The plan's constructor and fields are private. Its runtime mappings and
inspection are both derived from the same immutable view, so a caller cannot
mix relocation records with another image's plan. `runtime_plan()` returns
only an immutable reference. Planning performs no I/O, memory mapping or writes.

## Relative planning

The [AMD64 psABI relocation specification](https://gitlab.com/x86-psABIs/x86-64-ABI/-/blob/master/x86-64-ABI/object-files.tex)
defines `R_X86_64_NONE` (type 0) as no operation and `R_X86_64_RELATIVE`
(type 8) as `B + A` in an eight-byte field for LP64. A relative record must
have symbol index zero. Each planned value retains the low 64 bits of the
load bias plus signed explicit addend, including wrapping and negative
sentinels. These bits are data; they are not checked for canonicality or
whether they point into mapped memory. Target arithmetic is checked separately.

Each complete eight-byte relative target must lie inside the union of logical
`PT_LOAD` and standalone `PT_SCE_RELRO` mappings, including BSS. Unaligned
fields and fields crossing adjacent logical mappings are valid. Gaps, rounded
page padding and `p_align` padding cannot supply missing bytes. The original
field end, biased target and biased field end must fit in `u64`. Nonempty
overlapping logical mappings are rejected even if the image needs no relative
writes. Target permissions do not forbid a relocation: a later staging writer
may fill an initially read-only or executable mapping before final protection.

Patches are sorted by biased target address and must not overlap. Exact
duplicates are rejected. The [gABI composition rules](https://gabi.xinuos.com/elf/06-reloc.html#relocation-entry)
give consecutive same-offset records special semantics; a sequence containing
RELATIVE is rejected as unsupported instead of planning an isolated `B + A`.

Every other relocation type, including symbol relocations, indirect relative
resolvers and unknown values, is retained in `deferred()` with its original
table, zero-based record index and raw fields. No resolution, target width,
value or write is invented for those records. `NONE` produces neither a patch
nor a deferred record. Both regular and PLT RELA tables are considered, and
their combined record count, including `NONE`, remains subject to the same
`Limits::max_relocations` inspection budget before decoding. Output has at
most one patch or deferred descriptor per inspected record. Patches and
deferred records never imply that the image is fully linked or executable.

Failures return a structured `RelocationError`, with original record context
where applicable. No partial plan is returned and source bytes remain unchanged.

## Whole-session symbolic planning

```rust,ignore
let inputs = [
    pentax_linker::ModuleInput::new(pentax_loader::open_executable(&primary)?, primary_bias),
    pentax_linker::ModuleInput::new(pentax_loader::open_executable(&provider)?, provider_bias),
];
let plan = pentax_linker::plan_symbolic(&inputs, &pentax_linker::SymbolicLimits::default())?;
let _ = (plan.modules(), plan.primary_entry(), plan.patches(), plan.deferred());
```

Input zero is the primary image; an empty session is rejected. Its entry must
start in its own executable logical mapping. Other modules may have entry
zero. Each `PlannedModule` retains its input-order `ModuleId`, runtime plan,
original RELRO ranges and load bias. All nonempty biased logical ranges and
their 16 KiB page coverage must fit in `u64`; different modules cannot share
a mapped page. A downstream staging API may impose additional address-space,
bias-alignment or RELRO protection restrictions.

`ExactDeclarationV1` is a PentaX policy, not a documented SCE compatibility rule.
An undefined qualified symbol's selectors refer to its own image's import
library and needed module. An offered definition's selectors refer to its
own image's export library and module-info roles. Images without an export
library scope offer no providers. Resolved identities compare exact
module name bytes, major/minor version, library name bytes, complete library
version, canonical NID and supported symbol type. Numeric selector equality
across images has no meaning. There is no NID-only, filename, `DT_NEEDED`
ordering, normalized-name or global bare-symbol fallback. Each role may use
its classic or observed `_1` encoding; different roles and different images
may use different encodings. Matching uses decoded semantics, so the encoding
identity does not block an otherwise exact declaration match. A role with
both encodings produces `SceDescriptors::Conflicted` and is rejected as
`UnsupportedDescriptors` before complete linking tables or names are read.

External address providers may be GLOBAL or WEAK DEFAULT or PROTECTED FUNC/OBJECT definitions
with their image's own module-info/export-library declarations. The generic
[ELF visibility rules](https://gabi.xinuos.com/elf/05-symtab.html#symbol-visibility)
make PROTECTED definitions externally visible while binding self-references
within their defining image. This admission retains the existing exact
identity, type, section, logical-range and address checks.
Ordinary address imports select one strong definition ahead of weak definitions,
independently of input order. Without a strong definition, exactly one matching
weak definition is admitted. Multiple strong or weak-only definitions remain
ambiguous even if their value bits coincide. Missing weak imports remain errors
by default. The explicit `MissingWeakPolicy::NullAddressV1` profile instead gives
an absent declared WEAK FUNC/OBJECT an ELF zero symbol value, retaining
`Resolution::UnresolvedWeak` provenance and normal R_X86_64_64 addend arithmetic.
It creates no callable substitute and masks no version or type conflict. TLS and
module-local export queries retain their separate GLOBAL-only provider policy.
Appending a provider cannot change an already published weak or null binding.
These are bounded PentaX policies based on the
[ELF binding rules](https://gabi.xinuos.com/elf/05-symtab.html#symbol-binding),
not a claim of complete SCE interposition compatibility. Wrong types and unsupported
binding/visibility/section categories produce typed diagnostics. Hidden,
internal and LOCAL definitions are not offered externally.

Within that provider scan, a canonical qualified definition is unoffered
only when both its selectors are absent from the owned roles and present in
the needed-module/import-library roles. This exact false/false/true/true
membership produces `ProviderExclusionReason::NotOfferedImportScope` before
any provider key or copied identity names are allocated. It never publishes
an imported address or claims to identify a thunk. Partial, mixed and absent
pairs retain typed `MissingDeclaration` errors. If both complete pairs exist,
only the owned export declarations supply identity, regardless of imported
names or versions. Malformed qualified names, duplicate IDs and role
conflicts keep their errors before exclusion. This decision neither changes
SHN_UNDEF handling nor grants resolution of an excluded defined reference.

If exact name/NID identity exists only at another version, `VersionMismatch`
reports the requested versions and one original candidate's available
versions and symbol identity, chosen deterministically in version order.
The diagnostic is bounded and not exhaustive. Type is checked after an
exact-version lookup: an exact-version wrong type reports `TypeMismatch`;
an alternate-version diagnostic may describe a record whose type also
differs. Neither diagnostic enables fallback binding.

LOCAL definitions and GLOBAL HIDDEN/PROTECTED definitions can resolve within
their own image, using supported types. DEFAULT nonlocal defined references
are rejected because general interposition is not implemented. Ordinary
definitions use checked `provider_bias + st_value`; the full nonzero symbol
size must lie in logical mappings, including BSS and adjacent ranges. A
zero-sized symbol must start inside a logical mapping. ABS OBJECT values
and LOCAL ABS NOTYPE constants use `st_value` without bias or a mapping test.
COMMON, TLS, reserved section indices, IFUNC and other unsupported types are
explicit errors. JUMP_SLOT requires a FUNC definition starting in executable
logical bytes; ABS functions are unsupported.

The [AMD64 psABI](https://gitlab.com/x86-psABIs/x86-64-ABI/-/blob/master/x86-64-ABI/object-files.tex)
defines `R_X86_64_64` (1) as S+A and `GLOB_DAT` (6) / `JUMP_SLOT` (7) as S.
The latter two ignore the explicit addend. Planning also includes RELATIVE
and NONE with their existing rules. ELF symbol index zero supplies S=0 and
has explicit `ZeroSymbolIndex` provenance; it is distinct from an unresolved
nonzero symbol. Data arithmetic wraps at 64 bits and does not require a
canonical or mapped pointer. Address arithmetic remains checked. Every
planned target needs eight logical bytes, even in BSS or across adjacency;
page padding does not supply storage. Combined relative/symbolic writes
cannot overlap, including duplicates across regular/PLT tables. Consecutive
same-offset sequences involving a supported write are rejected as unsupported
composition. Other relocation types remain explicit deferred records.

`SymbolicLinkPlan`, module and patch fields are private with read-only
getters. Each patch retains its consumer, original table/record and either
definition/ABS provider symbol identity, relative origin or symbol-zero
origin. The caller cannot inject detached records or replacement biases.
All checks finish before a plan is returned; the linker never writes source
or guest bytes. A deferred-free plan only describes this narrow policy and
does not establish startup ABI, TLS, HLE or execution readiness.

`provider_exclusions()` exposes read-only `ProviderExclusion` records with
original `SymbolRef` and reason; `provider_exclusion_count()` counts those
records. They contain no names or addresses. This evidence covers only the
existing scan of defined, nonlocal, nonhidden/internal, named qualified
candidates in images declaring an export library, rather than all symbols
in all images. Records remain ordered by input image and original symbol.

`DescriptorVersions` retains the requested and available module major/minor
and opaque library version without encoding-specific naming.

`SymbolicLimits.inspection` and `.sce` are aggregate session allowances,
including names, tables, symbols, relocations and retained dynamic records;
the single metadata-name limit remains per name. Every declared dynamic
entry is charged before runtime parsing. All known table/symbol/relocation/
descriptor totals are checked across every image before any complete linking
table read or decode. SysV inference may first read its bounded eight-byte
header. Both descriptor encodings, attributes, `DT_NEEDED` and both opaque
original-filename tags count as retained records. Filename values are never
read as names and do not choose or conflict with an encoding; even `u64::MAX`
does not consume name bytes. Only decoded roles and `DT_NEEDED` need metadata
names. Metadata-only strings are counted too. Name inspection uses remaining
aggregate allowances. Additional bounds cover module count, examined provider
candidates (including exclusions), patches, copied identity-key bytes,
descriptor/identity comparisons, index operations and candidate visits.
All four role searches consume lookup steps; retaining an exclusion consumes
another step before allocation. The aggregate candidate allowance bounds the
diagnostic count. Excluded rows consume no identity-key bytes. Provider lookup uses an ordered index, with cached
consumer resolutions and at most two identities in ambiguity diagnostics.

## SCE descriptors and selectors

```rust,ignore
let view = pentax_loader::open_executable(&bytes)?;
let limits = (pentax_linker::Limits::default(), pentax_linker::SceLimits::default());
if let Some(sce) = pentax_linker::inspect_sce(&view, &limits.0, &limits.1)?.metadata() {
    let _ = (sce.link(), sce.needed(), sce.descriptors(), sce.attributes());
    let _ = (sce.original_filenames(), sce.qualified_symbols(), sce.import_associations());
}
```

`inspect_sce` is a separate, read-only entry point; `inspect` and
`plan_relative` are unchanged. It takes only the view and two budgets and
derives the runtime plan, generic `LinkMetadata` and descriptors from that
one view. Results have private fields and immutable accessors.

Each of the four descriptor roles has a classic tag and an observed PS5 `_1`
tag:

| Role | Classic | `_1` |
| --- | --- | --- |
| `ModuleInfo` (this image's module) | `0x6100000d` | `0x61000043` |
| `NeededModule` | `0x6100000f` | `0x61000045` |
| `ExportLibrary` | `0x61000013` | `0x61000047` |
| `ImportLibrary` | `0x61000015` | `0x61000049` |

Both encodings are decoded from the retained `u64` with explicit shifts: a
name offset in bits 0..32, a raw `u16` version in bits 32..48 and an ID in
bits 48..64. A module version's major is the high byte (bits 40..48) and its
minor the low byte (bits 32..40); the raw `u16` is also kept. A library
version stays one opaque `u16` with no major/minor split. Name offsets are
relative to the dynamic string table, in its own address domain; they are
never guest addresses. Every record keeps its dynamic-entry index, tag and
raw value and reports its `DescriptorEncoding` (`Classic` or
`Variant1Observed`).

The encoding is selected per role from that role's own tags only. A role
with neither tag is absent, one tag selects that encoding, and both tags are
a `RoleConflict`. Different roles may use different encodings, so a classic
`ModuleInfo` beside `_1` imports decodes normally. `SceDescriptors` is
`Absent` (no descriptor tag of any role, distinct from
`SceInspection::NotDynamic`), `Decoded(DecodedDescriptors)`, or
`Conflicted(ConflictedDescriptors)`. A conflict keeps both sides raw in entry
order; neither is named, decoded, merged, deduplicated, preferred or
ID-compared. Roles without a conflict are still decoded and validated, but any
conflict suppresses every import association. Treating a same-role mix as a
conflict is PentaX policy modelled on KytyPS5's per-role checks, not a
claimed ABI prohibition. Filename markers, attributes and the ELF ABI version
never select an encoding.

Module/library and export/import roles are separate ID scopes in either
encoding; equal numbers in different roles are unrelated. A repeated ID within
one decoded role is a `DuplicateDescriptorId` error, naming the role's
encoding, even when both values match. This is PentaX ambiguity policy, not a
claimed ABI rule. `DT_SCE_MODULE_ATTR`, `DT_SCE_EXPORT_LIB_ATTR` and
`DT_SCE_IMPORT_LIB_ATTR` are kept verbatim as opaque values beside either
encoding; no flag meaning or constant is assumed. Raw `DT_NEEDED` entries and
their names are kept separately, with no mapping to SCE descriptors and no
added file extension.

`DT_SCE_ORIGINAL_FILENAME` (`0x61000009`) and `DT_SCE_ORIGINAL_FILENAME_1`
(`0x61000041`) are opaque original-filename markers, listed by
`original_filenames()` with their complete `u64`. They are not descriptor
roles: no string is looked up for them, whatever their value, no filename or
path is decoded, and they neither select an encoding nor affect otherwise
valid descriptors. An image with only filename markers has absent
descriptors.

`SelectorId`, `NidToken` and `QualifiedNid` are pure byte helpers. Selectors
are positional radix-64 integers over
`ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+-`, most
significant digit first (`0 = A`, `64 = BA`, `65535 = P--`). Canonical
selectors have 1..=3 digits, no leading `A` unless the value is zero, and a
value that fits `u16`. A NID token has 11 alphabet bytes whose final digit has
zero low two bits. A qualified name is exactly `NID#library#module`. These are
PentaX validation rules derived from the observed encoder, not confirmed PS5
acceptance rules. NIDs are never computed from names.

Every symbol name containing `#` is listed by symbol index with its parse
result; names without `#` stay ordinary. For `SceDescriptors::Decoded`, each
undefined (`SHN_UNDEF`) GLOBAL or WEAK qualified symbol is matched against the
image's own import-library and needed-module IDs, whichever encoding each of
those two roles uses, and missing selectors are reported as
`SelectorMatch::Missing`. Absent or conflicted descriptors make no
association. Defined, absolute, common and reserved section indices are
identified but never treated as providers. A match is a declaration only; it
does not name a provider, compare versions or resolve an address. There is no
NID-only fallback and no cross-module lookup.

`SceLimits` bounds declared `PT_DYNAMIC` records (checked from program-header
metadata before the loader parses or allocates entries), retained
`DT_NEEDED`, descriptor (either encoding, including both raw sides of a
conflict), attribute and filename-marker records (checked before output
allocation), total metadata name bytes and single-name bytes. Defaults are
65,536, 4,096, 1 MiB and 4,096; they are PentaX policy.

A string table is planned only when `DT_NEEDED` or a decoded descriptor role
needs a name; conflicting roles and filename markers never need one. A string
table used only for metadata names is planned with its complete extent even
without a symbol table and is charged to `Limits::max_table_bytes` before
generic table copies or decoding; a table shared with the symbol table is
counted once. Standard and SCE table pointers keep their existing selection;
`_1` tags never change it. Name scans stay inside the declared table, scan at
most one byte beyond the smaller remaining budget, and stop at the first
failure. Every retained name, including aliases, is charged. Failures return
a structured `SceError` naming the dynamic entry, tag and name offset.

Layouts and tags were cross-checked against pinned upstream implementations,
whose code was not copied:
[shadPS4 `elf.h`](https://github.com/shadps4-emu/shadPS4/blob/e6c79a5fe0ea2f3d03fcb9c5c50cf61af063f3f5/src/core/loader/elf.h),
[`module.h`](https://github.com/shadps4-emu/shadPS4/blob/e6c79a5fe0ea2f3d03fcb9c5c50cf61af063f3f5/src/core/module.h)
and [`module.cpp`](https://github.com/shadps4-emu/shadPS4/blob/e6c79a5fe0ea2f3d03fcb9c5c50cf61af063f3f5/src/core/module.cpp);
[KytyPS5 `elf.h`](https://github.com/KytyPS5/KytyPS5/blob/763d3be3a9bc4e5a068d7cf52015030d6c03308f/src/loader/elf.h)
and [`runtimeLinker.cpp`](https://github.com/KytyPS5/KytyPS5/blob/763d3be3a9bc4e5a068d7cf52015030d6c03308f/src/loader/runtimeLinker.cpp);
[SharpEmu `SelfLoader.cs`](https://github.com/sharpemu/sharpemu/blob/80e81c9aab38e87e98d955e2c78f037fbb7f4a3f/src/SharpEmu.Core/Loader/SelfLoader.cs).
The `_1` field layout is corroborated by two implementation families. KytyPS5's
`runtimeLinker.cpp` reads all four `_1` words with its classic helpers and
checks classic/`_1` coexistence separately per role. The independent PS5
producer SharpProspero packs the same three fields in
[`DynamicWriter.cs`](https://github.com/SvenGDK/SharpProspero/blob/5dfdb4b0b166e83dc9ae275001afb695f12c7174/tools/SharpProspero.Link/DynamicWriter.cs),
documents the module version as major-high in
[`StubLibrary.cs`](https://github.com/SvenGDK/SharpProspero/blob/5dfdb4b0b166e83dc9ae275001afb695f12c7174/tools/SharpProspero.Link/StubLibrary.cs)
and [`ModuleEditor.cs`](https://github.com/SvenGDK/SharpProspero/blob/5dfdb4b0b166e83dc9ae275001afb695f12c7174/tools/SharpProspero.Prx/ModuleEditor.cs),
and reads module-info and export-library words in
[`PrxImage.cs`](https://github.com/SvenGDK/SharpProspero/blob/5dfdb4b0b166e83dc9ae275001afb695f12c7174/tools/SharpProspero.Prx/PrxImage.cs).
SharpEmu corroborates only the `NeededModule_1` name offset. These are
emulator and producer observations, not Sony documentation; producer default
versions and generated IDs are not required values.

Deferred: firmware acceptance of any `_1` value or classic/`_1` combination,
provider lookup, module or library version compatibility, filename decoding,
address resolution, relocation writes and PS5 execution. No real images were
used; all tests use original synthetic fixtures.

## Typed TLS bindings

`plan_symbolic` continues to defer AMD64 TLS relocation kinds 16/17/18 and
reject ordinary address relocations against STT_TLS. The separate opt-in
`plan_tls_bindings(&base, &TlsBindingLimits::default())` borrows that exact
immutable base and resolves nonzero-symbol DTPMOD64, DTPOFF64 and TPOFF64
records to typed provider identities and module-template offsets. It computes
no numeric TLS patch values, runtime module IDs or TP layout.

The private-construction `TlsBindingPlan` exposes its exact `base_plan()`,
address-sorted `bindings()` and unchanged `remaining_deferred()` records.
Each binding retains its original deferred index, consumer, source and checked
eight-byte biased target. Definitions retain the owning module, optional SymbolRef, typed offset, size and
PT_TLS header identity. Every deferred record is bound once or remains explicit;
an unknown relocation is never assigned a width or silently marked complete.

`GenericAmd64ExactDeclarationV1` preserves the existing exact module/library
roles, names, versions, canonical NIDs, provider exclusions and weak/preemption
restrictions. TLS resolution has a separate cache/result from ordinary guest
addresses. A defined STT_TLS value of zero is a valid offset when the symbol
index is nonzero. The selected offset and symbol size must fit its retained
PT_TLS memory template; source image VA and load bias never alter that offset.
DTPMOD64 with symbol index zero binds directly to the consumer's nonempty PT_TLS
(local dynamic model); its definition has no SymbolRef. The runtime derives the
actual TLS ID from that retained module/template identity, not an image bias,
null-symbol entry, arbitrary provider or host pointer. This follows the
[ELF local dynamic model](https://github.com/aosp-mirror/platform_bionic/blob/55060f004b0edc30cadfaacc44c923847223cfed/docs/elf-tls.md#local-dynamic-ld).
DTPOFF64/TPOFF64 with symbol zero remain unsupported, and all DTPMOD64 records
require a zero addend. Missing/empty local templates are rejected explicitly.
These bounds do not establish a complete console TLS ABI.

Targets use existing complete consumer-owned logical coverage, including BSS
and adjacent mappings, excluding padding/gaps. TLS composition and combined
ordinary/TLS eight-byte overlaps are rejected before returning a plan. Module,
all-record, binding, symbol/range/index/merge work, export, lookup and key-byte
budgets are aggregate and checked before retention or traversal. No memory
write, template copy, runtime TLS lookup or execution occurs.

## Records

Layouts follow the ELF specification:

- [Symbol table](https://gabi.xinuos.com/elf/05-symtab.html): 24-byte ELF64
  entries with a `u32` name offset, `u8` info, `u8` other, `u16` section index,
  `u64` value and `u64` size.
- [Relocation](https://gabi.xinuos.com/elf/06-reloc.html): 24-byte ELF64 RELA
  entries with a `u64` offset, a `u64` info field (symbol index in the high 32
  bits and type in the low 32 bits) and a signed `i64` addend.

All raw fields are preserved by inspection. `SymbolBinding`, `SymbolType` and
`SymbolVisibility` are views over those raw fields. They name the generic gABI
values and report OS- or processor-specific ranges, but they never reject or
reinterpret values. Inspected relocation types are kept as raw `u32` values;
relative planning only interprets the two operations described above.

Names are arbitrary bytes taken from bounded, NUL-terminated strings inside
the declared string table. `SymbolName::to_utf8` is an optional view, and UTF-8
is never required. A zero name offset, as in the reserved zeroth symbol, yields
`name: None` rather than an invented placeholder.

## Address domains

`pentax-loader` already assigns each dynamic table to one address domain, and
this crate keeps that assignment:

- `DynamicAddress::GuestVirtual` (standard tags): the complete declared range
  is read with `ExecutableView::read_initialized_vaddr`, so BSS, gaps and
  partial coverage are errors.
- `DynamicAddress::DynlibOffset` (SCE tags): the table is a checked slice of
  the plan's own `PT_SCE_DYNLIBDATA` payload.

The two domains are never converted into each other or into file offsets.

## Completeness rules

- Each inspected table needs a complete extent. An unsized standard
  `DT_SYMTAB` may use a standard `DT_HASH` in its original guest address
  domain. The [gABI SysV hash layout](https://gabi.xinuos.com/elf/08-dynamic.html#hash-table)
  has two `u32` counts followed by `nbucket` bucket words and `nchain` chain
  words. Its `nchain` establishes the symbol count, including the reserved
  zeroth symbol. Both counts must be nonzero, every bucket and chain index
  must be zero or below `nchain`, and the full initialized table must exist.
  Duplicate `DT_HASH` pointers are rejected. The inspected hash extent,
  bucket/chain counts and complete bytes are included in result accounting.
- Without a supported bounded count, inspection returns
  `LinkError::UnknownExtent`. GNU hash inference and hash lookup are not
  implemented. SCE size tags, section headers and relocation indices never
  substitute for a standard symbol-table extent. An explicit symbol size
  requires no hash reads.
- A dynamic section without a symbol table can still be inspected when every
  relocation uses symbol index zero (`STN_UNDEF`). A nonzero index is checked
  against the complete symbol count, but it is never resolved.
- `inspect` reads the string table only alongside a symbol table, because
  symbol names are its only consumer there. It is required only when some
  symbol has a nonzero name offset. `inspect_sce` also reads it for SCE
  metadata names, as described above.
- Record widths must be 24 bytes. Partial records, out-of-range or
  unterminated name offsets, and out-of-range symbol indices are all errors.

## Limits

`Limits` bounds the combined declared table bytes, total owned name bytes,
symbol count and combined regular plus PLT relocation count. An inspected
SysV hash is part of the same table-byte budget. Defaults are
64 MiB, 64 MiB, 1,000,000 and 1,000,000. Declared sizes are checked with
checked arithmetic before full tables are read or allocated. For hash-derived
extents, the fixed eight-byte header and already-known table sizes are checked
first, then the complete aggregate size and symbol count are checked before
any table copies. Zero and very small limits are honoured.

Every retained name is charged to the name budget, including repeated
references to one shared string. Each name lookup scans at most one byte more
than the remaining name budget. Total scanning is therefore linear in the
budget plus the symbol count, even for many offsets into one long string tail.

`append_symbolic_module` extends an immutable source plan by one input while
preserving every predecessor's original byte slice, load bias, runtime/SCE
analysis, ordinary binding, deferred record and provider exclusion. Changed or
ambiguous old bindings refuse. Input/metadata preservation and complete symbolic
planning have separate finite limits. Private source/parent identities describe
lineage only; they authorize no file read, native mapping, initializer or return.
