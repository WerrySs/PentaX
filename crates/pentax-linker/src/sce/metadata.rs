// SPDX-License-Identifier: GPL-2.0-or-later

//! Owned, image-bound SCE inspection results.

use crate::sce::descriptors::{
    AttributeRecord, NeededRecord, OriginalFilenameRecord, SceDescriptors,
};
use crate::sce::selectors::{QualifiedNameError, QualifiedNid};
use crate::{LinkMetadata, SymbolBinding, TableExtent};

const SHN_UNDEF: u16 = 0;
const SHN_LORESERVE: u16 = 0xff00;
const SHN_ABS: u16 = 0xfff1;
const SHN_COMMON: u16 = 0xfff2;

/// A successful SCE inspection. Failures are reported as
/// [`SceError`](crate::SceError).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceInspection {
    /// The image has no `PT_DYNAMIC` header.
    NotDynamic,
    /// A dynamic section exists, whether or not it has SCE descriptors.
    Dynamic(Box<SceMetadata>),
}

impl SceInspection {
    #[must_use]
    pub fn metadata(&self) -> Option<&SceMetadata> {
        match self {
            Self::NotDynamic => None,
            Self::Dynamic(metadata) => Some(metadata),
        }
    }

    #[must_use]
    pub fn into_metadata(self) -> Option<SceMetadata> {
        match self {
            Self::NotDynamic => None,
            Self::Dynamic(metadata) => Some(*metadata),
        }
    }
}

/// A view of a raw `st_shndx`; the raw value stays in the symbol record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolSection {
    /// `SHN_UNDEF`, regardless of the symbol's value.
    Undefined,
    /// An ordinary section index below `SHN_LORESERVE`.
    Section(u16),
    /// `SHN_ABS`.
    Absolute,
    /// `SHN_COMMON`.
    Common,
    /// Any other reserved, OS-, processor-specific or escape index.
    Reserved(u16),
}

impl SymbolSection {
    #[must_use]
    pub const fn of(section_index: u16) -> Self {
        match section_index {
            SHN_UNDEF => Self::Undefined,
            SHN_ABS => Self::Absolute,
            SHN_COMMON => Self::Common,
            index if index < SHN_LORESERVE => Self::Section(index),
            index => Self::Reserved(index),
        }
    }
}

/// A symbol whose name contains `#`, by original symbol index.
///
/// The original [`SymbolName`](crate::SymbolName) and every raw field remain
/// in [`LinkMetadata::symbols`]. Defined, absolute, common and reserved
/// symbols are identified but not treated as providers or given addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QualifiedSymbol {
    symbol: usize,
    name: Result<QualifiedNid, QualifiedNameError>,
    section: SymbolSection,
    binding: SymbolBinding,
}

impl QualifiedSymbol {
    pub(crate) const fn new(
        symbol: usize,
        name: Result<QualifiedNid, QualifiedNameError>,
        section: SymbolSection,
        binding: SymbolBinding,
    ) -> Self {
        Self {
            symbol,
            name,
            section,
            binding,
        }
    }

    /// Index into [`LinkMetadata::symbols`].
    #[must_use]
    pub const fn symbol(&self) -> usize {
        self.symbol
    }

    /// The canonical qualified name, or why the name is malformed.
    pub const fn name(&self) -> Result<QualifiedNid, QualifiedNameError> {
        self.name
    }

    #[must_use]
    pub const fn section(&self) -> SymbolSection {
        self.section
    }

    #[must_use]
    pub const fn binding(&self) -> SymbolBinding {
        self.binding
    }
}

/// Whether a selector names a descriptor declared by this image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectorMatch {
    /// Index into the matching decoded import descriptor slice.
    Declared(usize),
    /// No descriptor with this ID exists in the import namespace. Nothing is
    /// invented in its place.
    Missing,
}

impl SelectorMatch {
    #[must_use]
    pub const fn is_declared(self) -> bool {
        matches!(self, Self::Declared(_))
    }
}

/// An inspection-only match of one undefined GLOBAL or WEAK qualified symbol
/// against this image's own decoded import declarations.
///
/// The library selector is matched against
/// [`DecodedDescriptors::import_libraries`](crate::DecodedDescriptors::import_libraries)
/// and the module selector against
/// [`DecodedDescriptors::needed_modules`](crate::DecodedDescriptors::needed_modules),
/// whichever encoding each of those roles uses. A match is only a
/// declaration: it does not identify a provider, compare versions, resolve an
/// address, or imply that the import can be satisfied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportAssociation {
    symbol: usize,
    name: QualifiedNid,
    library: SelectorMatch,
    module: SelectorMatch,
}

impl ImportAssociation {
    pub(crate) const fn new(
        symbol: usize,
        name: QualifiedNid,
        library: SelectorMatch,
        module: SelectorMatch,
    ) -> Self {
        Self {
            symbol,
            name,
            library,
            module,
        }
    }

    /// Index into [`LinkMetadata::symbols`].
    #[must_use]
    pub const fn symbol(&self) -> usize {
        self.symbol
    }

    #[must_use]
    pub const fn name(&self) -> QualifiedNid {
        self.name
    }

    #[must_use]
    pub const fn library(&self) -> SelectorMatch {
        self.library
    }

    #[must_use]
    pub const fn module(&self) -> SelectorMatch {
        self.module
    }

    /// Whether both selectors are declared.
    #[must_use]
    pub const fn is_declared(&self) -> bool {
        self.library.is_declared() && self.module.is_declared()
    }
}

/// Complete, owned SCE descriptor and selector metadata.
///
/// Fields are private and values only come from
/// [`inspect_sce`](crate::inspect_sce), so every record, index and count was
/// derived from one executable view. The image remains unlinked and
/// non-executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceMetadata {
    pub(crate) link: LinkMetadata,
    pub(crate) strings: Option<TableExtent>,
    pub(crate) table_bytes: u64,
    pub(crate) name_bytes: u64,
    pub(crate) needed: Vec<NeededRecord>,
    pub(crate) descriptors: SceDescriptors,
    pub(crate) attributes: Vec<AttributeRecord>,
    pub(crate) filenames: Vec<OriginalFilenameRecord>,
    pub(crate) qualified: Vec<QualifiedSymbol>,
    pub(crate) associations: Option<Vec<ImportAssociation>>,
}

impl SceMetadata {
    /// Generic ELF linking metadata, inspected with the caller's `Limits`
    /// exactly as [`inspect`](crate::inspect) would.
    #[must_use]
    pub fn link(&self) -> &LinkMetadata {
        &self.link
    }

    /// The string table used for metadata names, when any were read.
    #[must_use]
    pub fn metadata_strings(&self) -> Option<TableExtent> {
        self.strings
    }

    /// Declared bytes of every table inspected by this call. A string table
    /// shared with the symbol table is counted once.
    #[must_use]
    pub fn table_bytes(&self) -> u64 {
        self.table_bytes
    }

    /// Owned metadata name bytes charged against
    /// `SceLimits::max_metadata_name_bytes`. Symbol names are charged
    /// separately against `Limits::max_name_bytes`.
    #[must_use]
    pub fn metadata_name_bytes(&self) -> u64 {
        self.name_bytes
    }

    /// Raw `DT_NEEDED` entries in dynamic-entry order.
    #[must_use]
    pub fn needed(&self) -> &[NeededRecord] {
        &self.needed
    }

    #[must_use]
    pub fn descriptors(&self) -> &SceDescriptors {
        &self.descriptors
    }

    /// Opaque attribute entries in dynamic-entry order, whatever encoding
    /// each descriptor role uses.
    #[must_use]
    pub fn attributes(&self) -> &[AttributeRecord] {
        &self.attributes
    }

    /// Opaque original-filename markers of both tags, in dynamic-entry order.
    #[must_use]
    pub fn original_filenames(&self) -> &[OriginalFilenameRecord] {
        &self.filenames
    }

    /// Every symbol whose name contains `#`, in symbol order. Names without
    /// `#` are ordinary names and are not listed.
    #[must_use]
    pub fn qualified_symbols(&self) -> &[QualifiedSymbol] {
        &self.qualified
    }

    /// Import selector associations, in symbol order. `None` unless the
    /// descriptors are [`SceDescriptors::Decoded`]: absent descriptors and any
    /// role conflict suppress every association.
    #[must_use]
    pub fn import_associations(&self) -> Option<&[ImportAssociation]> {
        self.associations.as_deref()
    }
}
