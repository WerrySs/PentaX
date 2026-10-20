// SPDX-License-Identifier: GPL-2.0-or-later

//! Read-only SCE module/library descriptor and qualified NID selector
//! inspection.
//!
//! Each descriptor role is decoded from its classic tag or its observed PS5
//! `_1` tag, chosen independently per role. The `_1` layout is corroborated by
//! KytyPS5 and SharpProspero, not by Sony documentation. A role with both tags
//! is kept raw as an explicit conflict. Original-filename markers are opaque.
//! Nothing is resolved, hashed, loaded or executed, and no provider or version
//! compatibility is evaluated.

mod descriptors;
mod error;
mod inspection;
mod limits;
mod metadata;
mod selectors;

pub use descriptors::{
    AttributeRecord, AttributeRole, ConflictedDescriptors, DT_SCE_EXPORT_LIB, DT_SCE_EXPORT_LIB_1,
    DT_SCE_EXPORT_LIB_ATTR, DT_SCE_IMPORT_LIB, DT_SCE_IMPORT_LIB_1, DT_SCE_IMPORT_LIB_ATTR,
    DT_SCE_MODULE_ATTR, DT_SCE_MODULE_INFO, DT_SCE_MODULE_INFO_1, DT_SCE_NEEDED_MODULE,
    DT_SCE_NEEDED_MODULE_1, DT_SCE_ORIGINAL_FILENAME, DT_SCE_ORIGINAL_FILENAME_1,
    DecodedDescriptors, DescriptorDirection, DescriptorEncoding, DescriptorNamespace,
    DescriptorRole, DynamicSource, LibraryDescriptor, MetadataName, ModuleDescriptor, NeededRecord,
    OriginalFilenameRecord, RoleConflict, SceDescriptors,
};
pub use error::SceError;
pub use inspection::inspect_sce;
pub use limits::{SceLimitKind, SceLimits};
pub use metadata::{
    ImportAssociation, QualifiedSymbol, SceInspection, SceMetadata, SelectorMatch, SymbolSection,
};
pub use selectors::{
    NID_TOKEN_LEN, NidToken, QualifiedField, QualifiedNameError, QualifiedNid, SELECTOR_ALPHABET,
    SelectorId, SelectorText, TokenError,
};

#[cfg(test)]
mod tests;
