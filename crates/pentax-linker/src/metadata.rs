// SPDX-License-Identifier: GPL-2.0-or-later

//! Owned inspection results.

use pentax_loader::DynamicAddress;

use crate::{Relocation, Symbol};

/// A successful inspection. Failures are reported as [`LinkError`](crate::LinkError).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inspection {
    /// The image has no `PT_DYNAMIC` header, so it declares no linking tables.
    NotDynamic,
    /// A dynamic section exists. Tables it does not declare are empty.
    Dynamic(LinkMetadata),
}

impl Inspection {
    #[must_use]
    pub fn metadata(&self) -> Option<&LinkMetadata> {
        match self {
            Self::NotDynamic => None,
            Self::Dynamic(metadata) => Some(metadata),
        }
    }

    #[must_use]
    pub fn into_metadata(self) -> Option<LinkMetadata> {
        match self {
            Self::NotDynamic => None,
            Self::Dynamic(metadata) => Some(metadata),
        }
    }
}

/// Where an inspected table came from, in its original address domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableExtent {
    pub address: DynamicAddress,
    pub size: u64,
}

/// Extents of the tables that were inspected. The string table is only
/// inspected, and therefore only listed, alongside a symbol table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LinkTables {
    pub strings: Option<TableExtent>,
    pub symbols: Option<TableExtent>,
    pub rela: Option<TableExtent>,
    pub plt_rela: Option<TableExtent>,
    /// Present only when a standard SysV hash established the symbol extent.
    pub sysv_hash: Option<TableExtent>,
}

/// Summary figures derived from the decoded records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LinkCounts {
    pub symbols: usize,
    /// Symbols with a nonzero name offset.
    pub named_symbols: usize,
    pub rela: usize,
    pub plt_rela: usize,
    /// Regular and PLT records whose symbol index is nonzero.
    pub symbol_references: usize,
    /// Inspected SysV hash buckets; zero when no hash was needed.
    pub sysv_hash_buckets: u32,
    /// Inspected SysV hash chains, equal to the derived symbol count.
    pub sysv_hash_chains: u32,
    /// Declared bytes of every inspected table.
    pub table_bytes: u64,
    /// Owned name bytes charged against `Limits::max_name_bytes`.
    pub name_bytes: u64,
}

/// Complete, owned and structurally checked linking records.
///
/// Fields are private so the records and their accounting stay consistent;
/// values only come from [`inspect`](crate::inspect).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkMetadata {
    tables: Box<LinkTables>,
    symbols: Vec<Symbol>,
    rela: Vec<Relocation>,
    plt_rela: Vec<Relocation>,
    table_bytes: u64,
    name_bytes: u64,
    hash_counts: Option<(u32, u32)>,
}

impl LinkMetadata {
    pub(crate) fn new(
        tables: LinkTables,
        symbols: Vec<Symbol>,
        rela: Vec<Relocation>,
        plt_rela: Vec<Relocation>,
        table_bytes: u64,
        name_bytes: u64,
        hash_counts: Option<(u32, u32)>,
    ) -> Self {
        Self {
            tables: Box::new(tables),
            symbols,
            rela,
            plt_rela,
            table_bytes,
            name_bytes,
            hash_counts,
        }
    }

    #[must_use]
    pub fn tables(&self) -> LinkTables {
        *self.tables
    }

    /// Whether a complete symbol table was inspected. An empty but present
    /// table still establishes a symbol count of zero.
    #[must_use]
    pub fn symbol_table_present(&self) -> bool {
        self.tables.symbols.is_some()
    }

    /// Symbols in table order, including the reserved zeroth entry.
    #[must_use]
    pub fn symbols(&self) -> &[Symbol] {
        &self.symbols
    }

    #[must_use]
    pub fn rela(&self) -> &[Relocation] {
        &self.rela
    }

    #[must_use]
    pub fn plt_rela(&self) -> &[Relocation] {
        &self.plt_rela
    }

    #[must_use]
    pub fn counts(&self) -> LinkCounts {
        LinkCounts {
            symbols: self.symbols.len(),
            named_symbols: self
                .symbols
                .iter()
                .filter(|symbol| symbol.name_offset != 0)
                .count(),
            rela: self.rela.len(),
            plt_rela: self.plt_rela.len(),
            symbol_references: self
                .rela
                .iter()
                .chain(&self.plt_rela)
                .filter(|relocation| relocation.references_symbol())
                .count(),
            table_bytes: self.table_bytes,
            name_bytes: self.name_bytes,
            sysv_hash_buckets: self.hash_counts.map_or(0, |counts| counts.0),
            sysv_hash_chains: self.hash_counts.map_or(0, |counts| counts.1),
        }
    }
}
