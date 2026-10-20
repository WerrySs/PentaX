// SPDX-License-Identifier: GPL-2.0-or-later

//! The read-only inspection entry point.

use pentax_loader::ExecutableView;

use crate::hash::{plan_inspection, validate_indices};
use crate::limits::NameBudget;
use crate::records::{decode_relocations, decode_symbols};
use crate::strings::StringTable;
use crate::tables::{PlannedTable, read_table};
use crate::{Inspection, Limits, LinkError, LinkMetadata, LinkTables, TableExtent, TableKind};

/// Inspect the linking tables of an opened executable without modifying it.
///
/// The runtime plan is derived from `view` itself, so payload selection,
/// dynlib backing and dynamic regions cannot be supplied, forged or mismatched
/// by the caller. Every complete extent is checked against `limits` before
/// table copies. An unsized standard symbol table can use the count in a
/// budget-checked eight-byte SysV hash header. A dynamic section without a
/// symbol table can still be inspected when all relocations use symbol zero.
///
/// # Errors
///
/// Returns [`LinkError`] when the loader rejects the image, a table extent is
/// unknown or malformed, a budget is exceeded, a name is invalid, or a
/// relocation references a symbol that the complete symbol table lacks.
pub fn inspect(view: &ExecutableView<'_>, limits: &Limits) -> Result<Inspection, LinkError> {
    let plan = view.runtime_plan()?;
    let Some(dynamic) = &plan.dynamic else {
        return Ok(Inspection::NotDynamic);
    };
    let (tables, hash) = plan_inspection(view, dynamic, limits)?;

    let dynlib = plan.dynlib_data;
    let read = |table: Option<PlannedTable>| {
        table
            .map(|table| read_table(view, dynlib, &table))
            .transpose()
    };
    let hash_bytes = read(hash.map(|hash| hash.table))?;
    if let (Some(bytes), Some(hash)) = (hash_bytes.as_deref(), hash.as_ref()) {
        validate_indices(bytes, hash)?;
    }
    let symbol_bytes = read(tables.symbols)?;
    let string_bytes = read(tables.strings)?;
    let rela_bytes = read(tables.rela)?;
    let plt_bytes = read(tables.plt_rela)?;

    let strings = string_bytes.as_deref().map(StringTable::new);
    let mut budget = NameBudget::new(limits.max_name_bytes);
    let symbols = match symbol_bytes.as_deref() {
        Some(bytes) => decode_symbols(bytes, strings.as_ref(), &mut budget)?,
        None => Vec::new(),
    };
    // Only a complete table with a declared or hash-derived extent counts.
    let symbol_count = symbol_bytes.is_some().then_some(symbols.len());
    let rela = decode_relocations(
        TableKind::Rela,
        rela_bytes.as_deref().unwrap_or_default(),
        symbol_count,
    )?;
    let plt_rela = decode_relocations(
        TableKind::PltRela,
        plt_bytes.as_deref().unwrap_or_default(),
        symbol_count,
    )?;

    let extents = LinkTables {
        strings: extent(tables.strings),
        symbols: extent(tables.symbols),
        rela: extent(tables.rela),
        plt_rela: extent(tables.plt_rela),
        sysv_hash: extent(hash.map(|hash| hash.table)),
    };
    Ok(Inspection::Dynamic(LinkMetadata::new(
        extents,
        symbols,
        rela,
        plt_rela,
        tables.table_bytes,
        budget.used(),
        hash.map(|hash| (hash.buckets, hash.chains)),
    )))
}

fn extent(table: Option<PlannedTable>) -> Option<TableExtent> {
    table.map(|table| TableExtent {
        address: table.address,
        size: table.size,
    })
}
