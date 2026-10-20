// SPDX-License-Identifier: GPL-2.0-or-later

//! The read-only SCE inspection entry point: dynamic-record preflight,
//! budgeted metadata names, image-bound assembly and import selector
//! association.

use std::collections::BTreeMap;

use pentax_loader::{DynamicEntry, ExecutableView, PT_DYNAMIC};

use crate::limits::ensure_within;
use crate::sce::descriptors::{
    AttributeRecord, ConflictedDescriptors, DecodedDescriptors, DescriptorRole, DynamicSource,
    LibraryDescriptor, MetadataName, ModuleDescriptor, NeededRecord, OriginalFilenameRecord,
    RoleConflict, SceDescriptors, TagClass,
};
use crate::sce::metadata::{
    ImportAssociation, QualifiedSymbol, SceInspection, SceMetadata, SelectorMatch, SymbolSection,
};
use crate::sce::selectors::{QualifiedNid, SelectorId};
use crate::sce::{SceError, SceLimitKind, SceLimits};
use crate::tables::{PlannedTable, plan_tables, read_table};
use crate::{
    Inspection, LimitKind, Limits, LinkError, LinkMetadata, SymbolBinding, TableExtent, TableKind,
    inspect,
};

const DYNAMIC_RECORD_SIZE: u64 = 16;

/// Inspect SCE module and library descriptors and qualified symbol names of
/// an opened executable without modifying it.
///
/// Every input is derived from `view`. Before the loader parses the dynamic
/// section, each `PT_DYNAMIC` header's declared size is checked against
/// `sce_limits.max_dynamic_entries`, so a giant payload cannot drive the entry
/// allocation. Generic linking metadata is then inspected with `link_limits`
/// exactly as [`inspect`] does, and retained record counts and the complete
/// string-table extent are checked before descriptor output or name copies.
///
/// Each descriptor role selects the classic or observed `_1` encoding from
/// its own tags; a role with both is kept raw as a conflict. Nothing is
/// resolved, loaded, written or executed. Undefined qualified symbols are
/// only matched against this image's own decoded import IDs, and only when no
/// role conflicts.
///
/// # Errors
///
/// Returns [`SceError`] when the image or its linking tables are rejected, a
/// budget is exceeded, a name is invalid, or a decoded descriptor role
/// repeats an ID.
pub fn inspect_sce(
    view: &ExecutableView<'_>,
    link_limits: &Limits,
    sce_limits: &SceLimits,
) -> Result<SceInspection, SceError> {
    preflight_dynamic(view, sce_limits)?;
    let plan = view.runtime_plan().map_err(LinkError::from)?;
    let Some(dynamic) = &plan.dynamic else {
        return Ok(SceInspection::NotDynamic);
    };
    let census = Census::of(&dynamic.entries);
    ensure_records(census.retained(), sce_limits)?;

    // Only a decoded name needs the string table. Its extent and host size
    // are validated now, independently of whether a symbol table exists.
    let strings = if census.needs_names() {
        dynamic
            .strings
            .map(|region| {
                let size = region.size.ok_or(LinkError::UnknownExtent {
                    table: TableKind::Strings,
                    address: region.address,
                })?;
                let length = usize::try_from(size).map_err(|_| LinkError::HostSizeOverflow {
                    table: TableKind::Strings,
                    size,
                })?;
                Ok::<_, LinkError>(PlannedTable {
                    kind: TableKind::Strings,
                    address: region.address,
                    size,
                    length,
                    records: 0,
                })
            })
            .transpose()?
    } else {
        None
    };

    // With no symbol table, generic inspection does not consume strings.
    // Include a metadata-only string table in the known aggregate before
    // generic inspection can copy or decode any relocation table.
    if let Some(table) = strings
        && dynamic.symbols.is_none()
    {
        let generic = plan_tables(dynamic, link_limits)?;
        let combined =
            generic
                .table_bytes
                .checked_add(table.size)
                .ok_or(LinkError::SizeOverflow {
                    context: "combined table bytes",
                })?;
        ensure_within(LimitKind::TableBytes, combined, link_limits.max_table_bytes)?;
    }

    let Inspection::Dynamic(link) = inspect(view, link_limits)? else {
        // Both plans come from the same immutable view.
        return Ok(SceInspection::NotDynamic);
    };

    // A string table already inspected alongside symbols is counted once.
    let mut table_bytes = link.counts().table_bytes;
    if let Some(table) = strings
        && link.tables().strings.is_none()
    {
        table_bytes = table_bytes
            .checked_add(table.size)
            .ok_or(LinkError::SizeOverflow {
                context: "combined table bytes",
            })?;
        ensure_within(
            LimitKind::TableBytes,
            table_bytes,
            link_limits.max_table_bytes,
        )?;
    }
    let string_bytes = strings
        .map(|table| read_table(view, plan.dynlib_data, &table))
        .transpose()?;

    let mut names = Names::new(string_bytes.as_deref(), sce_limits);
    let records = Records::decode(&dynamic.entries, &census, &mut names)?;
    let (qualified, associations) = qualify(&link, records.index.as_ref());

    Ok(SceInspection::Dynamic(Box::new(SceMetadata {
        link,
        strings: strings.map(|table| TableExtent {
            address: table.address,
            size: table.size,
        }),
        table_bytes,
        name_bytes: names.used,
        needed: records.needed,
        descriptors: records.descriptors,
        attributes: records.attributes,
        filenames: records.filenames,
        qualified,
        associations,
    })))
}

/// Check every declared `PT_DYNAMIC` record count from program-header
/// metadata alone. The loader still validates divisibility, `DT_NULL` and
/// duplicate headers afterwards.
fn preflight_dynamic(view: &ExecutableView<'_>, limits: &SceLimits) -> Result<(), SceError> {
    for header in &view.metadata().program_headers {
        if header.kind == PT_DYNAMIC {
            let requested = header.file_size / DYNAMIC_RECORD_SIZE;
            if requested > limits.max_dynamic_entries {
                return Err(SceError::LimitExceeded {
                    limit: SceLimitKind::DynamicEntries,
                    requested,
                    maximum: limits.max_dynamic_entries,
                });
            }
        }
    }
    Ok(())
}

fn ensure_records(requested: u64, limits: &SceLimits) -> Result<(), SceError> {
    if requested > limits.max_descriptor_records {
        Err(SceError::LimitExceeded {
            limit: SceLimitKind::DescriptorRecords,
            requested,
            maximum: limits.max_descriptor_records,
        })
    } else {
        Ok(())
    }
}

/// Counts taken before any output is allocated.
#[derive(Debug, Default)]
struct Census {
    needed: usize,
    /// Descriptor records by role position, then encoding position.
    descriptors: [[usize; 2]; 4],
    attributes: usize,
    filenames: usize,
}

impl Census {
    fn of(entries: &[DynamicEntry]) -> Self {
        let mut census = Self::default();
        for entry in entries {
            match TagClass::of(entry.tag) {
                TagClass::Needed => census.needed += 1,
                TagClass::Descriptor(role, encoding) => {
                    census.descriptors[role.position()][encoding.position()] += 1;
                }
                TagClass::Attribute(_) => census.attributes += 1,
                TagClass::OriginalFilename => census.filenames += 1,
                TagClass::Other => {}
            }
        }
        census
    }

    fn count(&self, role: DescriptorRole) -> usize {
        self.descriptors[role.position()].iter().sum()
    }

    /// Any descriptor role has a record, in either encoding.
    fn present(&self) -> bool {
        self.descriptors.iter().flatten().any(|&count| count != 0)
    }

    /// Both of a role's encodings are present.
    fn conflicts(&self, role: DescriptorRole) -> bool {
        self.descriptors[role.position()]
            .iter()
            .all(|&count| count != 0)
    }

    /// The role is present in exactly one encoding.
    fn decodes(&self, role: DescriptorRole) -> bool {
        self.count(role) != 0 && !self.conflicts(role)
    }

    fn retained(&self) -> u64 {
        // Every count is bounded by the parsed entry count, which itself fits
        // in memory, so this sum cannot overflow u64.
        let descriptors: usize = self.descriptors.iter().flatten().sum();
        [self.needed, descriptors, self.attributes, self.filenames]
            .into_iter()
            .map(|count| count as u64)
            .sum()
    }

    /// Only `DT_NEEDED` and decoded roles read names. Conflicting roles and
    /// filename markers never do.
    fn needs_names(&self) -> bool {
        let decoded = DescriptorRole::ALL.iter().any(|&role| self.decodes(role));
        self.needed != 0 || decoded
    }
}

/// Bounded lookup of metadata names inside the one declared string table.
///
/// A lookup scans at most one byte more than the smaller of the remaining
/// total budget and the single-name limit, never past the declared table.
/// Each success charges exactly the retained length and the first failure
/// stops inspection, so total scanning stays linear in the budget plus the
/// record count even for many offsets into one long unterminated tail.
struct Names<'a> {
    table: Option<&'a [u8]>,
    used: u64,
    maximum: u64,
    single: u64,
}

impl<'a> Names<'a> {
    const fn new(table: Option<&'a [u8]>, limits: &SceLimits) -> Self {
        Self {
            table,
            used: 0,
            maximum: limits.max_metadata_name_bytes,
            single: limits.max_single_metadata_name_bytes,
        }
    }

    fn take(&mut self, source: DynamicSource, name_offset: u64) -> Result<MetadataName, SceError> {
        let table = self.table.ok_or(SceError::MissingStringTable {
            source,
            name_offset,
        })?;
        let tail = usize::try_from(name_offset)
            .ok()
            .and_then(|start| table.get(start..))
            .filter(|tail| !tail.is_empty())
            .ok_or(SceError::NameOffsetOutOfRange {
                source,
                name_offset,
                table_size: table.len() as u64,
            })?;
        let remaining = self.maximum.saturating_sub(self.used);
        let allowed = remaining.min(self.single);
        let window = usize::try_from(allowed)
            .unwrap_or(usize::MAX)
            .saturating_add(1)
            .min(tail.len());
        let scanned = &tail[..window];
        let Some(length) = scanned.iter().position(|&byte| byte == 0) else {
            return Err(if window == tail.len() {
                SceError::UnterminatedName {
                    source,
                    name_offset,
                }
            } else if self.single <= remaining {
                SceError::NameTooLong {
                    source,
                    name_offset,
                    maximum: self.single,
                }
            } else {
                SceError::NameBudgetExceeded {
                    source,
                    name_offset,
                    used: self.used,
                    maximum: self.maximum,
                }
            });
        };
        // `length <= allowed <= remaining`, so the charge stays in budget.
        self.used += length as u64;
        Ok(MetadataName::new(&scanned[..length]))
    }
}

/// Retained records in dynamic-entry order.
struct Records {
    needed: Vec<NeededRecord>,
    attributes: Vec<AttributeRecord>,
    filenames: Vec<OriginalFilenameRecord>,
    descriptors: SceDescriptors,
    /// Present only for [`SceDescriptors::Decoded`].
    index: Option<DescriptorIndex>,
}

/// One ID scope per decoded role, mapping an ID to its source and position in
/// that role's descriptor slice.
#[derive(Default)]
struct DescriptorIndex {
    scopes: [BTreeMap<u16, (DynamicSource, usize)>; 4],
}

impl DescriptorIndex {
    fn lookup(&self, role: DescriptorRole, id: SelectorId) -> SelectorMatch {
        self.scopes[role.position()]
            .get(&id.value())
            .map_or(SelectorMatch::Missing, |&(_, position)| {
                SelectorMatch::Declared(position)
            })
    }
}

impl Records {
    fn decode(
        entries: &[DynamicEntry],
        census: &Census,
        names: &mut Names<'_>,
    ) -> Result<Self, SceError> {
        // Every capacity below was bounded by `SceLimits::max_descriptor_records`.
        let mut needed = Vec::with_capacity(census.needed);
        let mut attributes = Vec::with_capacity(census.attributes);
        let mut filenames = Vec::with_capacity(census.filenames);
        let capacity = |role| {
            if census.decodes(role) {
                census.count(role)
            } else {
                0
            }
        };
        let mut decoded = DecodedDescriptors {
            encodings: [None; 4],
            module_info: Vec::with_capacity(capacity(DescriptorRole::ModuleInfo)),
            needed_modules: Vec::with_capacity(capacity(DescriptorRole::NeededModule)),
            export_libraries: Vec::with_capacity(capacity(DescriptorRole::ExportLibrary)),
            import_libraries: Vec::with_capacity(capacity(DescriptorRole::ImportLibrary)),
        };
        // Raw sources by role and encoding, kept only for conflicting roles.
        let mut raw: [[Vec<DynamicSource>; 2]; 4] = Default::default();
        for role in DescriptorRole::ALL {
            if census.conflicts(role) {
                raw[role.position()] = census.descriptors[role.position()].map(Vec::with_capacity);
            }
        }
        let mut ids = DescriptorIndex::default();

        for (index, entry) in entries.iter().enumerate() {
            let source = DynamicSource {
                index,
                tag: entry.tag,
                value: entry.value,
            };
            match TagClass::of(entry.tag) {
                TagClass::Needed => {
                    let name = names.take(source, entry.value)?;
                    needed.push(NeededRecord::new(source, name));
                }
                // A conflicting role is never decoded, ID-checked or named.
                TagClass::Descriptor(role, encoding) if census.conflicts(role) => {
                    raw[role.position()][encoding.position()].push(source);
                }
                TagClass::Descriptor(role, encoding) => {
                    // Check the scope before scanning the name. A decoded
                    // role has one encoding, so both entries share it.
                    let id = (entry.value >> 48) as u16;
                    let scope = &mut ids.scopes[role.position()];
                    if let Some(&(first, _)) = scope.get(&id) {
                        return Err(SceError::DuplicateDescriptorId {
                            role,
                            encoding,
                            id,
                            first,
                            second: source,
                        });
                    }
                    let name = names.take(source, u64::from(entry.value as u32))?;
                    let position = match role {
                        DescriptorRole::ModuleInfo => push(
                            &mut decoded.module_info,
                            ModuleDescriptor::new(source, encoding, name),
                        ),
                        DescriptorRole::NeededModule => push(
                            &mut decoded.needed_modules,
                            ModuleDescriptor::new(source, encoding, name),
                        ),
                        DescriptorRole::ExportLibrary => push(
                            &mut decoded.export_libraries,
                            LibraryDescriptor::new(source, encoding, name),
                        ),
                        DescriptorRole::ImportLibrary => push(
                            &mut decoded.import_libraries,
                            LibraryDescriptor::new(source, encoding, name),
                        ),
                    };
                    scope.insert(id, (source, position));
                    decoded.encodings[role.position()] = Some(encoding);
                }
                TagClass::Attribute(role) => attributes.push(AttributeRecord::new(role, source)),
                TagClass::OriginalFilename => filenames.push(OriginalFilenameRecord::new(source)),
                TagClass::Other => {}
            }
        }

        let conflicts: Vec<_> = DescriptorRole::ALL
            .into_iter()
            .zip(raw)
            .filter(|(role, _)| census.conflicts(*role))
            .map(|(role, [classic, variant1])| RoleConflict::new(role, classic, variant1))
            .collect();
        let (descriptors, index) = if !conflicts.is_empty() {
            let conflicted = ConflictedDescriptors { decoded, conflicts };
            (SceDescriptors::Conflicted(conflicted), None)
        } else if census.present() {
            (SceDescriptors::Decoded(decoded), Some(ids))
        } else {
            (SceDescriptors::Absent, None)
        };
        Ok(Self {
            needed,
            attributes,
            filenames,
            descriptors,
            index,
        })
    }
}

/// Push and return the new element's position.
fn push<T>(records: &mut Vec<T>, record: T) -> usize {
    records.push(record);
    records.len() - 1
}

/// Record every `#`-containing symbol name and, for conflict-free decoded
/// descriptors, match undefined GLOBAL/WEAK qualified symbols against import
/// IDs. Output is bounded by the inspected symbol count; each lookup is a
/// logarithmic map search.
fn qualify(
    link: &LinkMetadata,
    ids: Option<&DescriptorIndex>,
) -> (Vec<QualifiedSymbol>, Option<Vec<ImportAssociation>>) {
    let mut qualified = Vec::new();
    let mut associations = ids.map(|_| Vec::new());
    for (index, symbol) in link.symbols().iter().enumerate() {
        let Some(name) = &symbol.name else {
            continue;
        };
        if !name.as_bytes().contains(&b'#') {
            continue;
        }
        let parsed = QualifiedNid::parse(name.as_bytes());
        let binding = symbol.binding();
        let section = SymbolSection::of(symbol.section_index);
        qualified.push(QualifiedSymbol::new(index, parsed, section, binding));
        // SHN_UNDEF, not a zero value, marks a reference. No NID-only
        // fallback, export lookup or cross-module search is attempted.
        if let (Some(ids), Some(associations), Ok(qualified_name)) =
            (ids, associations.as_mut(), parsed)
            && section == SymbolSection::Undefined
            && matches!(binding, SymbolBinding::GLOBAL | SymbolBinding::WEAK)
        {
            associations.push(ImportAssociation::new(
                index,
                qualified_name,
                ids.lookup(DescriptorRole::ImportLibrary, qualified_name.library()),
                ids.lookup(DescriptorRole::NeededModule, qualified_name.module()),
            ));
        }
    }
    (qualified, associations)
}
