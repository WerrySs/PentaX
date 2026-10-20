// SPDX-License-Identifier: GPL-2.0-or-later

use super::error::charge;
use super::{
    ModuleId, ModuleInput, PlannedModule, SymbolicBudget as B, SymbolicError as E, SymbolicLimits,
};
use crate::relative::logical_ranges;
use crate::{
    DT_SCE_EXPORT_LIB, DT_SCE_EXPORT_LIB_1, DT_SCE_EXPORT_LIB_ATTR, DT_SCE_IMPORT_LIB,
    DT_SCE_IMPORT_LIB_1, DT_SCE_IMPORT_LIB_ATTR, DT_SCE_MODULE_ATTR, DT_SCE_MODULE_INFO,
    DT_SCE_MODULE_INFO_1, DT_SCE_NEEDED_MODULE, DT_SCE_NEEDED_MODULE_1, DT_SCE_ORIGINAL_FILENAME,
    DT_SCE_ORIGINAL_FILENAME_1, LinkError, SceDescriptors, TableKind, inspect_sce,
};
use pentax_loader::{PT_DYNAMIC, RUNTIME_PAGE_SIZE};

/// Only runtime metadata and (if necessary) a fixed eight-byte SysV header
/// are read before all aggregate table/record extents have passed.
pub(super) fn prepare<'a>(
    inputs: &[ModuleInput<'a>],
    limits: &SymbolicLimits,
) -> Result<Vec<PlannedModule<'a>>, E> {
    if inputs.is_empty() {
        return Err(E::EmptySession);
    }
    charge(&mut 0, inputs.len() as u64, limits.max_modules, B::Modules)?;
    let mut dynamic_entries = 0;
    for input in inputs {
        for header in &input.view.metadata().program_headers {
            if header.kind == PT_DYNAMIC {
                charge(
                    &mut dynamic_entries,
                    header.file_size / 16,
                    limits.sce.max_dynamic_entries,
                    B::DynamicEntries,
                )?;
            }
        }
    }
    let (mut table_bytes, mut symbols, mut relocations, mut descriptors) = (0, 0, 0, 0);
    let mut modules = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        let id = ModuleId(index);
        let runtime = input.view.runtime_plan().map_err(|error| E::Link {
            module: id,
            error: error.into(),
        })?;
        if let Some(dynamic) = &runtime.dynamic {
            let (tables, _) =
                crate::hash::plan_inspection(&input.view, dynamic, &limits.inspection)
                    .map_err(|error| E::Link { module: id, error })?;
            let mut needs_names = false;
            // Per-role presence only: bit one is classic, bit two observed.
            // A conflict will become SceDescriptors::Conflicted; reject that
            // policy case here before full tables or any descriptor names.
            let mut encodings = [0_u8; 4];
            for entry in &dynamic.entries {
                match entry.tag {
                    1
                    | DT_SCE_MODULE_INFO
                    | DT_SCE_NEEDED_MODULE
                    | DT_SCE_EXPORT_LIB
                    | DT_SCE_IMPORT_LIB
                    | DT_SCE_MODULE_INFO_1
                    | DT_SCE_NEEDED_MODULE_1
                    | DT_SCE_EXPORT_LIB_1
                    | DT_SCE_IMPORT_LIB_1 => {
                        needs_names = true;
                        let role = match entry.tag {
                            DT_SCE_MODULE_INFO | DT_SCE_MODULE_INFO_1 => Some(0),
                            DT_SCE_NEEDED_MODULE | DT_SCE_NEEDED_MODULE_1 => Some(1),
                            DT_SCE_EXPORT_LIB | DT_SCE_EXPORT_LIB_1 => Some(2),
                            DT_SCE_IMPORT_LIB | DT_SCE_IMPORT_LIB_1 => Some(3),
                            _ => None,
                        };
                        if let Some(role) = role {
                            encodings[role] |= if matches!(
                                entry.tag,
                                DT_SCE_MODULE_INFO_1
                                    | DT_SCE_NEEDED_MODULE_1
                                    | DT_SCE_EXPORT_LIB_1
                                    | DT_SCE_IMPORT_LIB_1
                            ) {
                                2
                            } else {
                                1
                            };
                        }
                        charge(
                            &mut descriptors,
                            1,
                            limits.sce.max_descriptor_records,
                            B::DescriptorRecords,
                        )?;
                    }
                    DT_SCE_MODULE_ATTR
                    | DT_SCE_EXPORT_LIB_ATTR
                    | DT_SCE_IMPORT_LIB_ATTR
                    | DT_SCE_ORIGINAL_FILENAME
                    | DT_SCE_ORIGINAL_FILENAME_1 => {
                        charge(
                            &mut descriptors,
                            1,
                            limits.sce.max_descriptor_records,
                            B::DescriptorRecords,
                        )?;
                    }
                    _ => {}
                }
            }
            if encodings.contains(&3) {
                return Err(E::UnsupportedDescriptors { module: id });
            }
            charge(
                &mut table_bytes,
                tables.table_bytes,
                limits.inspection.max_table_bytes,
                B::TableBytes,
            )?;
            // Generic inspection omits strings when no symbol table exists;
            // descriptor-only names nevertheless consume their full table.
            if needs_names
                && tables.strings.is_none()
                && let Some(region) = dynamic.strings
            {
                let size = region.size.ok_or(E::Link {
                    module: id,
                    error: LinkError::UnknownExtent {
                        table: TableKind::Strings,
                        address: region.address,
                    },
                })?;
                charge(
                    &mut table_bytes,
                    size,
                    limits.inspection.max_table_bytes,
                    B::TableBytes,
                )?;
            }
            charge(
                &mut symbols,
                tables.symbols.map_or(0, |t| t.records),
                limits.inspection.max_symbols,
                B::Symbols,
            )?;
            for table in [tables.rela, tables.plt_rela].into_iter().flatten() {
                charge(
                    &mut relocations,
                    table.records,
                    limits.inspection.max_relocations,
                    B::Relocations,
                )?;
            }
        }
        let ranges =
            logical_ranges(&runtime).map_err(|error| E::Relocation { module: id, error })?;
        modules.push(PlannedModule {
            input: input.view.source_bytes(),
            id,
            runtime,
            load_bias: input.load_bias,
            metadata: None,
            ranges,
        });
    }
    validate_mappings(&modules)?;
    let (mut symbol_names, mut metadata_names) = (0, 0);
    for (input, module) in inputs.iter().zip(&mut modules) {
        let mut link_limits = limits.inspection;
        link_limits.max_name_bytes -= symbol_names;
        let mut sce_limits = limits.sce;
        sce_limits.max_metadata_name_bytes -= metadata_names;
        module.metadata = inspect_sce(&input.view, &link_limits, &sce_limits)
            .map_err(|error| E::Sce {
                module: module.id,
                error,
            })?
            .into_metadata();
        if let Some(metadata) = &module.metadata {
            if matches!(metadata.descriptors(), SceDescriptors::Conflicted(_)) {
                return Err(E::UnsupportedDescriptors { module: module.id });
            }
            // Each inspection received the remaining allowance, so these
            // additions cannot overflow or exceed the aggregate budgets.
            symbol_names += metadata.link().counts().name_bytes;
            metadata_names += metadata.metadata_name_bytes();
        }
    }
    Ok(modules)
}

fn validate_mappings(modules: &[PlannedModule<'_>]) -> Result<(), E> {
    let mut pages: Vec<(u64, u64, ModuleId)> = Vec::new();
    for module in modules {
        let mut own: Vec<(u64, u64, ModuleId)> = Vec::new();
        for range in &module.ranges {
            let fail = || E::BiasedMappingOverflow {
                module: module.id,
                segment: range.segment,
            };
            let start = range.start.checked_add(module.load_bias).ok_or_else(fail)?;
            let end = range.end.checked_add(module.load_bias).ok_or_else(fail)?;
            let page_start = start & !(RUNTIME_PAGE_SIZE - 1);
            let page_end =
                end.checked_add(RUNTIME_PAGE_SIZE - 1).ok_or_else(fail)? & !(RUNTIME_PAGE_SIZE - 1);
            if let Some(last) = own.last_mut()
                && page_start <= last.1
            {
                last.1 = last.1.max(page_end);
            } else {
                own.push((page_start, page_end, module.id));
            }
        }
        pages.extend(own);
        for (range, relro) in module.runtime.relro.iter().enumerate() {
            if relro.size == 0 {
                continue;
            }
            relro
                .address
                .checked_add(module.load_bias)
                .and_then(|start| start.checked_add(relro.size))
                .ok_or(E::BiasedRelroOverflow {
                    module: module.id,
                    range,
                })?;
        }
    }
    pages.sort_unstable_by_key(|range| range.0);
    for pair in pages.windows(2) {
        if pair[1].0 < pair[0].1 {
            return Err(E::OverlappingModulePages {
                first: pair[0].2,
                second: pair[1].2,
                address: pair[1].0,
            });
        }
    }
    Ok(())
}

pub(super) fn primary_entry(module: &PlannedModule<'_>) -> Result<u64, E> {
    let entry = module.runtime.metadata.entry;
    if !module.runtime.segments.iter().any(|segment| {
        segment.flags.executable()
            && segment.vaddr <= entry
            && entry - segment.vaddr < segment.mem_size
    }) {
        return Err(E::PrimaryEntryOutsideExecutable { address: entry });
    }
    entry
        .checked_add(module.load_bias)
        .ok_or(E::PrimaryEntryOverflow)
}
