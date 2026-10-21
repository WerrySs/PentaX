// SPDX-License-Identifier: GPL-2.0-or-later

//! Bounded, read-only ELF64 linking metadata for PentaX.
//!
//! [`inspect`] decodes the dynamic symbol table, the regular RELA table and the
//! PLT RELA table referenced by an executable's dynamic section into owned
//! records. [`plan_relative`] also prepares checked, image-bound eight-byte
//! writes for symbol-independent AMD64 RELATIVE relocations while retaining
//! other types as explicit deferred records. [`plan_symbolic`] additionally
//! plans whole-session symbol writes under an explicit exact SCE
//! declaration policy. [`plan_tls_bindings`] separately selects typed TLS
//! definitions without inventing runtime IDs, TP layout or numeric patches.
//! These operations never write guest memory, discover
//! modules, emulate libraries, set up TLS or execute guest code. The separate
//! opt-in [`lookup_module_export`] hashes a bounded name for one retained module.
//!
//! [`inspect_sce`] additionally reads SCE module and library descriptors and
//! qualified `NID#library#module` symbol names from the same view. Each
//! descriptor role uses its classic tag or its observed PS5 `_1` tag, selected
//! per role; a role with both is reported as an explicit conflict. It neither
//! hashes names, resolves imports nor evaluates version compatibility.
//!
//! Record layouts follow the ELF specification's
//! [symbol table](https://gabi.xinuos.com/elf/05-symtab.html) description
//! (24-byte ELF64 entries: `u32` name offset, `u8` info, `u8` other, `u16`
//! section index, `u64` value, `u64` size) and its
//! [relocation](https://gabi.xinuos.com/elf/06-reloc.html) description (24-byte
//! ELF64 RELA entries: `u64` offset, `u64` info whose high 32 bits are the
//! symbol index and low 32 bits the type, `i64` signed addend). Unknown
//! binding, type, visibility and relocation values are preserved rather than
//! interpreted.
//!
//! Table addresses keep the domain established by `pentax-loader`: standard
//! tags hold original guest virtual addresses, read from initialized load
//! bytes, while SCE tags hold offsets into `PT_SCE_DYNLIBDATA`. The two domains
//! are never converted into each other. Metadata/relocation code is original
//! Rust; the opt-in name/NID transform adapts pinned Kyty source with retained
//! provenance. There are no third-party dependencies or I/O.

#![forbid(unsafe_code)]

mod error;
mod hash;
mod inspection;
mod limits;
mod metadata;
mod records;
mod relative;
mod sce;
mod strings;
mod symbolic;
mod tables;

pub use symbolic::{
    ModuleExport, ModuleExportIdentity, ModuleExportIdentityAttempt, ModuleExportQuery,
    ModuleLookupAttempt, ModuleLookupError, ModuleLookupLimits, lookup_module_export,
    lookup_qualified_module_export,
};

pub use error::{LimitKind, LinkError, TableKind};
pub use inspection::inspect;
pub use limits::Limits;
pub use metadata::{Inspection, LinkCounts, LinkMetadata, LinkTables, TableExtent};
pub use pentax_loader::DynamicAddress;
pub use records::{
    ELF64_RELA_SIZE, ELF64_SYMBOL_SIZE, Relocation, Symbol, SymbolBinding, SymbolName, SymbolType,
    SymbolVisibility,
};
pub use relative::{
    DeferredRelocation, RelativePatch, RelativeRelocationPlan, RelocationError, RelocationSource,
    plan_relative,
};
pub use sce::{
    AttributeRecord, AttributeRole, ConflictedDescriptors, DT_SCE_EXPORT_LIB, DT_SCE_EXPORT_LIB_1,
    DT_SCE_EXPORT_LIB_ATTR, DT_SCE_IMPORT_LIB, DT_SCE_IMPORT_LIB_1, DT_SCE_IMPORT_LIB_ATTR,
    DT_SCE_MODULE_ATTR, DT_SCE_MODULE_INFO, DT_SCE_MODULE_INFO_1, DT_SCE_NEEDED_MODULE,
    DT_SCE_NEEDED_MODULE_1, DT_SCE_ORIGINAL_FILENAME, DT_SCE_ORIGINAL_FILENAME_1,
    DecodedDescriptors, DescriptorDirection, DescriptorEncoding, DescriptorNamespace,
    DescriptorRole, DynamicSource, ImportAssociation, LibraryDescriptor, MetadataName,
    ModuleDescriptor, NID_TOKEN_LEN, NeededRecord, NidToken, OriginalFilenameRecord,
    QualifiedField, QualifiedNameError, QualifiedNid, QualifiedSymbol, RoleConflict,
    SELECTOR_ALPHABET, SceDescriptors, SceError, SceInspection, SceLimitKind, SceLimits,
    SceMetadata, SelectorId, SelectorMatch, SelectorText, SymbolSection, TokenError, inspect_sce,
};
pub use symbolic::{
    DescriptorVersions, FallbackPolicy, FallbackTransition, MissingFunctionImport,
    MissingFunctionPlan, MissingFunctionReason, MissingWeakPolicy, ModuleId, ModuleInput,
    PlannedModule, ProviderExclusion, ProviderExclusionReason, Resolution,
    SessionDeferredRelocation, SourceIdentity, SymbolRef, SymbolicBudget, SymbolicError,
    SymbolicExtensionError, SymbolicExtensionLimits, SymbolicLimits, SymbolicLinkPlan,
    SymbolicPatch, TlsBinding, TlsBindingBudget, TlsBindingError, TlsBindingLimits, TlsBindingPlan,
    TlsBindingPolicy, TlsDefinition, TlsOffset, TlsRelocationKind, UnsupportedSymbol,
    append_symbolic_module, append_symbolic_module_with_fallbacks,
    append_symbolic_modules_with_fallbacks, plan_missing_functions,
    plan_missing_functions_for_extension, plan_symbolic, plan_tls_bindings,
};

#[cfg(test)]
mod tests;
