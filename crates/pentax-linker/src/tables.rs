// SPDX-License-Identifier: GPL-2.0-or-later

//! Planning and extraction of complete tables in their own address domain.

use std::borrow::Cow;

use pentax_loader::{DynamicAddress, DynamicRegion, DynamicTable, ExecutableView, ResolvedPayload};

use crate::limits::ensure_within;
use crate::records::{ELF64_RELA_SIZE, ELF64_SYMBOL_SIZE};
use crate::{LimitKind, Limits, LinkError, TableKind};

/// A table whose declared extent and record width have been validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlannedTable {
    pub(crate) kind: TableKind,
    pub(crate) address: DynamicAddress,
    pub(crate) size: u64,
    /// `size` as a host length.
    pub(crate) length: usize,
    /// Number of fixed-width records; zero for the string table.
    pub(crate) records: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TablePlan {
    pub(crate) strings: Option<PlannedTable>,
    pub(crate) symbols: Option<PlannedTable>,
    pub(crate) rela: Option<PlannedTable>,
    pub(crate) plt_rela: Option<PlannedTable>,
    pub(crate) table_bytes: u64,
}

/// Validate extents, widths and every budget without reading or allocating
/// table bytes. Names are the only consumer of the string table at this
/// stage, so it is only inspected alongside a symbol table.
pub(crate) fn plan_tables(dynamic: &DynamicTable, limits: &Limits) -> Result<TablePlan, LinkError> {
    plan_regions(dynamic, dynamic.symbols, 0, limits)
}

/// Plan a standard symbol table whose complete extent was established by a
/// bounded SysV hash header. The hash bytes are charged in the same budget.
pub(crate) fn plan_with_symbol_extent(
    dynamic: &DynamicTable,
    symbol_size: u64,
    hash_size: u64,
    limits: &Limits,
) -> Result<TablePlan, LinkError> {
    let symbols = dynamic.symbols.map(|region| DynamicRegion {
        size: Some(symbol_size),
        ..region
    });
    plan_regions(dynamic, symbols, hash_size, limits)
}

fn plan_regions(
    dynamic: &DynamicTable,
    symbol_region: Option<DynamicRegion>,
    hash_size: u64,
    limits: &Limits,
) -> Result<TablePlan, LinkError> {
    let symbols = plan(TableKind::Symbols, symbol_region, Some(ELF64_SYMBOL_SIZE))?;
    let strings = if symbols.is_some() {
        plan(TableKind::Strings, dynamic.strings, None)?
    } else {
        None
    };
    let rela = plan(TableKind::Rela, dynamic.rela, Some(ELF64_RELA_SIZE))?;
    let plt_rela = plan(TableKind::PltRela, dynamic.plt_rela, Some(ELF64_RELA_SIZE))?;

    let mut table_bytes = hash_size;
    for table in [strings, symbols, rela, plt_rela].into_iter().flatten() {
        table_bytes = table_bytes
            .checked_add(table.size)
            .ok_or(LinkError::SizeOverflow {
                context: "combined table bytes",
            })?;
    }
    ensure_within(LimitKind::TableBytes, table_bytes, limits.max_table_bytes)?;
    ensure_within(LimitKind::Symbols, records(symbols), limits.max_symbols)?;
    let relocations =
        records(rela)
            .checked_add(records(plt_rela))
            .ok_or(LinkError::SizeOverflow {
                context: "combined relocation count",
            })?;
    ensure_within(LimitKind::Relocations, relocations, limits.max_relocations)?;
    Ok(TablePlan {
        strings,
        symbols,
        rela,
        plt_rela,
        table_bytes,
    })
}

fn records(table: Option<PlannedTable>) -> u64 {
    table.map_or(0, |table| table.records)
}

fn plan(
    kind: TableKind,
    region: Option<DynamicRegion>,
    width: Option<u64>,
) -> Result<Option<PlannedTable>, LinkError> {
    let Some(region) = region else {
        return Ok(None);
    };
    let size = region.size.ok_or(LinkError::UnknownExtent {
        table: kind,
        address: region.address,
    })?;
    let records = match width {
        Some(expected) => {
            if region.entry_size != Some(expected) {
                return Err(LinkError::InvalidEntrySize {
                    table: kind,
                    entry_size: region.entry_size,
                    expected,
                });
            }
            if !size.is_multiple_of(expected) {
                return Err(LinkError::PartialRecord {
                    table: kind,
                    size,
                    entry_size: expected,
                });
            }
            size / expected
        }
        None => 0,
    };
    let length =
        usize::try_from(size).map_err(|_| LinkError::HostSizeOverflow { table: kind, size })?;
    Ok(Some(PlannedTable {
        kind,
        address: region.address,
        size,
        length,
        records,
    }))
}

/// Borrow or copy one complete table.
///
/// Dynlib offsets are checked slices of the runtime plan's own
/// `PT_SCE_DYNLIBDATA` payload. Guest addresses are original virtual addresses
/// read in full from initialized load bytes, so BSS, gaps and partial coverage
/// are errors. Neither domain is ever reinterpreted as the other or as a file
/// offset.
pub(crate) fn read_table<'a>(
    view: &ExecutableView<'a>,
    dynlib: Option<ResolvedPayload<'a>>,
    table: &PlannedTable,
) -> Result<Cow<'a, [u8]>, LinkError> {
    match table.address {
        DynamicAddress::DynlibOffset(offset) => {
            let backing = dynlib
                .ok_or(LinkError::MissingDynlibData { table: table.kind })?
                .file_bytes;
            let out_of_bounds = || LinkError::TableOutOfBounds {
                table: table.kind,
                offset,
                size: table.size,
                available: backing.len() as u64,
            };
            let start = usize::try_from(offset).map_err(|_| out_of_bounds())?;
            let end = start.checked_add(table.length).ok_or_else(out_of_bounds)?;
            backing
                .get(start..end)
                .map(Cow::Borrowed)
                .ok_or_else(out_of_bounds)
        }
        DynamicAddress::GuestVirtual(address) => {
            let mut bytes = vec![0; table.length];
            view.read_initialized_vaddr(address, &mut bytes)?;
            Ok(Cow::Owned(bytes))
        }
    }
}
