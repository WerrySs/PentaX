// SPDX-License-Identifier: GPL-2.0-or-later

//! Raw SCE dynamic descriptor words, scoped descriptor records, opaque
//! attributes and opaque original-filename markers.
//!
//! Each module/library descriptor role has a classic tag and an observed
//! `_1` tag. Classic tags and their packed layout were cross-checked against
//! shadPS4 (`elf.h`, `module.h`) and KytyPS5 (`elf.h`, `runtimeLinker.cpp`).
//! The four `_1` tags use the same packing in KytyPS5's reader and in the
//! independent PS5 producer SharpProspero (`DynamicWriter.cs`, `PrxImage.cs`),
//! whose `StubLibrary.cs` and `ModuleEditor.cs` document the module major
//! version as the high byte of bits 32..48. These are observed implementations,
//! not Sony documentation; they establish neither firmware acceptance nor
//! version compatibility for any value. Words are decoded with explicit shifts
//! from the retained `u64`, never by transposing a host union layout:
//!
//! - module: name offset bits 0..32, minor version 32..40, major version
//!   40..48, ID 48..64;
//! - library: name offset bits 0..32, opaque version `u16` 32..48, ID 48..64.
//!
//! Name offsets are relative to the dynamic string table, not guest addresses
//! or file offsets. Descriptor tags repeat; they are not singleton pointers.
//! The encoding is chosen per role from that role's own tags only. Filename
//! markers, attributes and the ELF ABI version never select it.

use std::fmt;

use crate::sce::selectors::SelectorId;

pub(crate) const DT_NEEDED: i64 = 1;

/// An opaque original-filename marker. It is retained raw and never read.
pub const DT_SCE_ORIGINAL_FILENAME: i64 = 0x6100_0009;
/// This image's own (export) module descriptor.
pub const DT_SCE_MODULE_INFO: i64 = 0x6100_000d;
/// An imported module descriptor.
pub const DT_SCE_NEEDED_MODULE: i64 = 0x6100_000f;
pub const DT_SCE_MODULE_ATTR: i64 = 0x6100_0011;
pub const DT_SCE_EXPORT_LIB: i64 = 0x6100_0013;
pub const DT_SCE_IMPORT_LIB: i64 = 0x6100_0015;
pub const DT_SCE_EXPORT_LIB_ATTR: i64 = 0x6100_0017;
pub const DT_SCE_IMPORT_LIB_ATTR: i64 = 0x6100_0019;

/// An opaque `_1` original-filename marker. Like
/// [`DT_SCE_ORIGINAL_FILENAME`], it is retained raw, never read, and does not
/// select a descriptor encoding.
pub const DT_SCE_ORIGINAL_FILENAME_1: i64 = 0x6100_0041;
/// The observed `_1` tag of [`DescriptorRole::ModuleInfo`].
pub const DT_SCE_MODULE_INFO_1: i64 = 0x6100_0043;
/// The observed `_1` tag of [`DescriptorRole::NeededModule`].
pub const DT_SCE_NEEDED_MODULE_1: i64 = 0x6100_0045;
/// The observed `_1` tag of [`DescriptorRole::ExportLibrary`].
pub const DT_SCE_EXPORT_LIB_1: i64 = 0x6100_0047;
/// The observed `_1` tag of [`DescriptorRole::ImportLibrary`].
pub const DT_SCE_IMPORT_LIB_1: i64 = 0x6100_0049;

/// One dynamic entry exactly as parsed, with its zero-based position among
/// the entries preceding `DT_NULL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DynamicSource {
    pub index: usize,
    pub tag: i64,
    pub value: u64,
}

/// Owned metadata-name bytes, excluding the NUL terminator. UTF-8 is never
/// required and an empty name is kept as empty, never replaced.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct MetadataName(Box<[u8]>);

impl MetadataName {
    pub(crate) fn new(bytes: &[u8]) -> Self {
        Self(bytes.into())
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// A text view, available only when the bytes happen to be valid UTF-8.
    #[must_use]
    pub fn to_utf8(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for MetadataName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "b\"{}\"", self.0.escape_ascii())
    }
}

/// Escaped, never lossy, text for diagnostics.
impl fmt::Display for MetadataName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.escape_ascii())
    }
}

/// Module or library descriptor IDs; the two namespaces never mix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DescriptorNamespace {
    Module,
    Library,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DescriptorDirection {
    Export,
    Import,
}

/// Which tag of a role a descriptor record came from.
///
/// Both encodings share one field layout. The distinction is kept so callers
/// can tell observed `_1` records from classic ones; it is not a global image
/// dialect and implies nothing about other roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DescriptorEncoding {
    /// The classic tag: `0x6100000d`, `0x6100000f`, `0x61000013` or
    /// `0x61000015`.
    Classic,
    /// The `_1` tag: `0x61000043`, `0x61000045`, `0x61000047` or `0x61000049`,
    /// decoded with the layout observed in KytyPS5 and SharpProspero. Firmware
    /// acceptance and version compatibility are not established.
    Variant1Observed,
}

impl DescriptorEncoding {
    pub(crate) const fn position(self) -> usize {
        match self {
            Self::Classic => 0,
            Self::Variant1Observed => 1,
        }
    }
}

impl fmt::Display for DescriptorEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Classic => "classic",
            Self::Variant1Observed => "observed _1",
        })
    }
}

/// A module or library descriptor role. Each role is its own ID scope, in
/// whichever encoding it uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DescriptorRole {
    /// This image's own module.
    ModuleInfo,
    NeededModule,
    ExportLibrary,
    ImportLibrary,
}

impl DescriptorRole {
    pub(crate) const ALL: [Self; 4] = [
        Self::ModuleInfo,
        Self::NeededModule,
        Self::ExportLibrary,
        Self::ImportLibrary,
    ];

    /// The dynamic tag of this role in `encoding`.
    #[must_use]
    pub const fn tag(self, encoding: DescriptorEncoding) -> i64 {
        match (self, encoding) {
            (Self::ModuleInfo, DescriptorEncoding::Classic) => DT_SCE_MODULE_INFO,
            (Self::NeededModule, DescriptorEncoding::Classic) => DT_SCE_NEEDED_MODULE,
            (Self::ExportLibrary, DescriptorEncoding::Classic) => DT_SCE_EXPORT_LIB,
            (Self::ImportLibrary, DescriptorEncoding::Classic) => DT_SCE_IMPORT_LIB,
            (Self::ModuleInfo, DescriptorEncoding::Variant1Observed) => DT_SCE_MODULE_INFO_1,
            (Self::NeededModule, DescriptorEncoding::Variant1Observed) => DT_SCE_NEEDED_MODULE_1,
            (Self::ExportLibrary, DescriptorEncoding::Variant1Observed) => DT_SCE_EXPORT_LIB_1,
            (Self::ImportLibrary, DescriptorEncoding::Variant1Observed) => DT_SCE_IMPORT_LIB_1,
        }
    }

    /// The symbolic tag name, for diagnostics.
    pub(crate) const fn tag_name(self, encoding: DescriptorEncoding) -> &'static str {
        match (self, encoding) {
            (Self::ModuleInfo, DescriptorEncoding::Classic) => "DT_SCE_MODULE_INFO",
            (Self::NeededModule, DescriptorEncoding::Classic) => "DT_SCE_NEEDED_MODULE",
            (Self::ExportLibrary, DescriptorEncoding::Classic) => "DT_SCE_EXPORT_LIB",
            (Self::ImportLibrary, DescriptorEncoding::Classic) => "DT_SCE_IMPORT_LIB",
            (Self::ModuleInfo, DescriptorEncoding::Variant1Observed) => "DT_SCE_MODULE_INFO_1",
            (Self::NeededModule, DescriptorEncoding::Variant1Observed) => "DT_SCE_NEEDED_MODULE_1",
            (Self::ExportLibrary, DescriptorEncoding::Variant1Observed) => "DT_SCE_EXPORT_LIB_1",
            (Self::ImportLibrary, DescriptorEncoding::Variant1Observed) => "DT_SCE_IMPORT_LIB_1",
        }
    }

    #[must_use]
    pub const fn namespace(self) -> DescriptorNamespace {
        match self {
            Self::ModuleInfo | Self::NeededModule => DescriptorNamespace::Module,
            Self::ExportLibrary | Self::ImportLibrary => DescriptorNamespace::Library,
        }
    }

    #[must_use]
    pub const fn direction(self) -> DescriptorDirection {
        match self {
            Self::ModuleInfo | Self::ExportLibrary => DescriptorDirection::Export,
            Self::NeededModule | Self::ImportLibrary => DescriptorDirection::Import,
        }
    }

    pub(crate) const fn position(self) -> usize {
        match self {
            Self::ModuleInfo => 0,
            Self::NeededModule => 1,
            Self::ExportLibrary => 2,
            Self::ImportLibrary => 3,
        }
    }
}

impl fmt::Display for DescriptorRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ModuleInfo => "module info",
            Self::NeededModule => "needed module",
            Self::ExportLibrary => "export library",
            Self::ImportLibrary => "import library",
        })
    }
}

/// Which classic-numbered attribute tag an opaque attribute record came from.
/// Attributes accompany either descriptor encoding and never select one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttributeRole {
    /// `DT_SCE_MODULE_ATTR`.
    Module,
    /// `DT_SCE_EXPORT_LIB_ATTR`.
    ExportLibrary,
    /// `DT_SCE_IMPORT_LIB_ATTR`.
    ImportLibrary,
}

/// How inspection treats a dynamic tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TagClass {
    Needed,
    Descriptor(DescriptorRole, DescriptorEncoding),
    Attribute(AttributeRole),
    OriginalFilename,
    Other,
}

impl TagClass {
    pub(crate) const fn of(tag: i64) -> Self {
        use DescriptorEncoding::{Classic, Variant1Observed};
        match tag {
            DT_NEEDED => Self::Needed,
            DT_SCE_MODULE_INFO => Self::Descriptor(DescriptorRole::ModuleInfo, Classic),
            DT_SCE_NEEDED_MODULE => Self::Descriptor(DescriptorRole::NeededModule, Classic),
            DT_SCE_EXPORT_LIB => Self::Descriptor(DescriptorRole::ExportLibrary, Classic),
            DT_SCE_IMPORT_LIB => Self::Descriptor(DescriptorRole::ImportLibrary, Classic),
            DT_SCE_MODULE_INFO_1 => Self::Descriptor(DescriptorRole::ModuleInfo, Variant1Observed),
            DT_SCE_NEEDED_MODULE_1 => {
                Self::Descriptor(DescriptorRole::NeededModule, Variant1Observed)
            }
            DT_SCE_EXPORT_LIB_1 => {
                Self::Descriptor(DescriptorRole::ExportLibrary, Variant1Observed)
            }
            DT_SCE_IMPORT_LIB_1 => {
                Self::Descriptor(DescriptorRole::ImportLibrary, Variant1Observed)
            }
            DT_SCE_MODULE_ATTR => Self::Attribute(AttributeRole::Module),
            DT_SCE_EXPORT_LIB_ATTR => Self::Attribute(AttributeRole::ExportLibrary),
            DT_SCE_IMPORT_LIB_ATTR => Self::Attribute(AttributeRole::ImportLibrary),
            DT_SCE_ORIGINAL_FILENAME | DT_SCE_ORIGINAL_FILENAME_1 => Self::OriginalFilename,
            _ => Self::Other,
        }
    }
}

const fn name_offset(value: u64) -> u32 {
    value as u32
}

const fn raw_version(value: u64) -> u16 {
    (value >> 32) as u16
}

const fn descriptor_id(value: u64) -> SelectorId {
    SelectorId::new((value >> 48) as u16)
}

/// A module descriptor with its decoded name.
///
/// Fields are views over the retained raw word. The ID is scoped to this
/// image and to the descriptor's role; it is not a vector position,
/// `DT_NEEDED` order, runtime handle or global module ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDescriptor {
    source: DynamicSource,
    encoding: DescriptorEncoding,
    name: MetadataName,
}

impl ModuleDescriptor {
    pub(crate) const fn new(
        source: DynamicSource,
        encoding: DescriptorEncoding,
        name: MetadataName,
    ) -> Self {
        Self {
            source,
            encoding,
            name,
        }
    }

    #[must_use]
    pub const fn source(&self) -> DynamicSource {
        self.source
    }

    #[must_use]
    pub const fn encoding(&self) -> DescriptorEncoding {
        self.encoding
    }

    /// Offset into the dynamic string table, from bits 0..32.
    #[must_use]
    pub const fn name_offset(&self) -> u32 {
        name_offset(self.source.value)
    }

    #[must_use]
    pub const fn name(&self) -> &MetadataName {
        &self.name
    }

    /// The complete `u16` at bits 32..48: major in the high byte, minor in
    /// the low byte.
    #[must_use]
    pub const fn raw_version(&self) -> u16 {
        raw_version(self.source.value)
    }

    /// Bits 32..40.
    #[must_use]
    pub const fn version_minor(&self) -> u8 {
        (self.source.value >> 32) as u8
    }

    /// Bits 40..48.
    #[must_use]
    pub const fn version_major(&self) -> u8 {
        (self.source.value >> 40) as u8
    }

    /// Bits 48..64.
    #[must_use]
    pub const fn id(&self) -> SelectorId {
        descriptor_id(self.source.value)
    }
}

/// A library descriptor with its decoded name.
///
/// The version is one opaque `u16`; no major/minor split is established and
/// compatibility is not evaluated. The ID is scoped like
/// [`ModuleDescriptor::id`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryDescriptor {
    source: DynamicSource,
    encoding: DescriptorEncoding,
    name: MetadataName,
}

impl LibraryDescriptor {
    pub(crate) const fn new(
        source: DynamicSource,
        encoding: DescriptorEncoding,
        name: MetadataName,
    ) -> Self {
        Self {
            source,
            encoding,
            name,
        }
    }

    #[must_use]
    pub const fn source(&self) -> DynamicSource {
        self.source
    }

    #[must_use]
    pub const fn encoding(&self) -> DescriptorEncoding {
        self.encoding
    }

    /// Offset into the dynamic string table, from bits 0..32.
    #[must_use]
    pub const fn name_offset(&self) -> u32 {
        name_offset(self.source.value)
    }

    #[must_use]
    pub const fn name(&self) -> &MetadataName {
        &self.name
    }

    /// The complete `u16` at bits 32..48.
    #[must_use]
    pub const fn version(&self) -> u16 {
        raw_version(self.source.value)
    }

    /// Bits 48..64.
    #[must_use]
    pub const fn id(&self) -> SelectorId {
        descriptor_id(self.source.value)
    }
}

/// An attribute entry, kept verbatim. No bit, flag or ID meaning is assumed
/// and no particular value is required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttributeRecord {
    role: AttributeRole,
    source: DynamicSource,
}

impl AttributeRecord {
    pub(crate) const fn new(role: AttributeRole, source: DynamicSource) -> Self {
        Self { role, source }
    }

    #[must_use]
    pub const fn role(&self) -> AttributeRole {
        self.role
    }

    #[must_use]
    pub const fn source(&self) -> DynamicSource {
        self.source
    }

    /// The complete original value.
    #[must_use]
    pub const fn raw(&self) -> u64 {
        self.source.value
    }
}

/// A [`DT_SCE_ORIGINAL_FILENAME`] or [`DT_SCE_ORIGINAL_FILENAME_1`] entry,
/// kept verbatim; the source tag tells them apart.
///
/// The complete `u64` is retained. No string is read, no filename or path is
/// decoded, and the marker neither selects nor conflicts with a descriptor
/// encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OriginalFilenameRecord {
    source: DynamicSource,
}

impl OriginalFilenameRecord {
    pub(crate) const fn new(source: DynamicSource) -> Self {
        Self { source }
    }

    #[must_use]
    pub const fn source(&self) -> DynamicSource {
        self.source
    }

    /// The complete original value.
    #[must_use]
    pub const fn raw(&self) -> u64 {
        self.source.value
    }
}

/// A raw `DT_NEEDED` entry and its name. It is not mapped to any SCE module or
/// library descriptor, and no file extension or path is inferred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeededRecord {
    source: DynamicSource,
    name: MetadataName,
}

impl NeededRecord {
    pub(crate) const fn new(source: DynamicSource, name: MetadataName) -> Self {
        Self { source, name }
    }

    #[must_use]
    pub const fn source(&self) -> DynamicSource {
        self.source
    }

    /// The complete `d_val`, an offset into the dynamic string table.
    #[must_use]
    pub const fn name_offset(&self) -> u64 {
        self.source.value
    }

    #[must_use]
    pub const fn name(&self) -> &MetadataName {
        &self.name
    }
}

/// Decoded module and library descriptors, each role in dynamic-entry order.
///
/// Every record in one role uses that role's single encoding, reported by
/// each record. Different roles may use different encodings. Every
/// `(role, ID)` pair is unique; equal numeric IDs in different roles are
/// unrelated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedDescriptors {
    /// By role position; `Some` exactly when that role has records.
    pub(crate) encodings: [Option<DescriptorEncoding>; 4],
    pub(crate) module_info: Vec<ModuleDescriptor>,
    pub(crate) needed_modules: Vec<ModuleDescriptor>,
    pub(crate) export_libraries: Vec<LibraryDescriptor>,
    pub(crate) import_libraries: Vec<LibraryDescriptor>,
}

impl DecodedDescriptors {
    /// Records describing this image's own module.
    #[must_use]
    pub fn module_info(&self) -> &[ModuleDescriptor] {
        &self.module_info
    }

    #[must_use]
    pub fn needed_modules(&self) -> &[ModuleDescriptor] {
        &self.needed_modules
    }

    #[must_use]
    pub fn export_libraries(&self) -> &[LibraryDescriptor] {
        &self.export_libraries
    }

    #[must_use]
    pub fn import_libraries(&self) -> &[LibraryDescriptor] {
        &self.import_libraries
    }

    /// The encoding of `role`'s decoded records, or `None` when this set has
    /// none for that role.
    #[must_use]
    pub const fn encoding(&self, role: DescriptorRole) -> Option<DescriptorEncoding> {
        self.encodings[role.position()]
    }
}

/// One role declared with both its classic and its `_1` tag.
///
/// Both sides are kept raw in dynamic-entry order. Neither is decoded,
/// merged, deduplicated or preferred, their IDs are not compared and no name
/// is read. Treating this as a conflict is PentaX policy modelled on
/// KytyPS5's per-role checks, not a claimed ABI prohibition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleConflict {
    role: DescriptorRole,
    classic: Vec<DynamicSource>,
    variant1: Vec<DynamicSource>,
}

impl RoleConflict {
    pub(crate) const fn new(
        role: DescriptorRole,
        classic: Vec<DynamicSource>,
        variant1: Vec<DynamicSource>,
    ) -> Self {
        Self {
            role,
            classic,
            variant1,
        }
    }

    #[must_use]
    pub const fn role(&self) -> DescriptorRole {
        self.role
    }

    /// The role's classic-tag entries; never empty.
    #[must_use]
    pub fn classic(&self) -> &[DynamicSource] {
        &self.classic
    }

    /// The role's `_1`-tag entries; never empty.
    #[must_use]
    pub fn variant1(&self) -> &[DynamicSource] {
        &self.variant1
    }
}

/// Descriptors in which at least one role uses both encodings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictedDescriptors {
    pub(crate) decoded: DecodedDescriptors,
    pub(crate) conflicts: Vec<RoleConflict>,
}

impl ConflictedDescriptors {
    /// Every role without a conflict, decoded and validated exactly as in
    /// [`SceDescriptors::Decoded`]. Conflicting roles are empty here.
    #[must_use]
    pub fn decoded(&self) -> &DecodedDescriptors {
        &self.decoded
    }

    /// One record per conflicting role, in [`DescriptorRole`] declaration
    /// order; never empty.
    #[must_use]
    pub fn conflicts(&self) -> &[RoleConflict] {
        &self.conflicts
    }
}

/// The module and library descriptors of a dynamic section.
///
/// Each role selects its encoding from its own tags only: neither tag means
/// the role is absent, one tag selects that encoding, and both tags are a
/// [`RoleConflict`]. `_1` tags are never aliased to classic tags, and
/// filename markers, attributes and the ELF ABI version select nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceDescriptors {
    /// No descriptor tag of any role. Distinct from an image without
    /// `PT_DYNAMIC`; attributes and filename markers alone are absent.
    Absent,
    /// At least one role, each in a single encoding.
    Decoded(DecodedDescriptors),
    /// At least one role has both encodings. Import selectors are not
    /// associated.
    Conflicted(ConflictedDescriptors),
}

impl SceDescriptors {
    /// The descriptors when no role conflicts.
    #[must_use]
    pub fn decoded(&self) -> Option<&DecodedDescriptors> {
        match self {
            Self::Decoded(descriptors) => Some(descriptors),
            _ => None,
        }
    }
}
