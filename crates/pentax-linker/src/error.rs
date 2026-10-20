// SPDX-License-Identifier: GPL-2.0-or-later

//! Structured, actionable inspection failures.

use std::{error::Error, fmt};

use pentax_loader::{DynamicAddress, LoadError};

/// A linking table referenced by the dynamic section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableKind {
    Strings,
    Symbols,
    Rela,
    PltRela,
    SysvHash,
}

impl fmt::Display for TableKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Strings => "dynamic string table",
            Self::Symbols => "dynamic symbol table",
            Self::Rela => "RELA table",
            Self::PltRela => "PLT RELA table",
            Self::SysvHash => "SysV symbol hash table",
        })
    }
}

/// A count or byte budget checked before complete table reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LimitKind {
    /// Combined declared size of every inspected table.
    TableBytes,
    Symbols,
    /// Regular and PLT RELA records together.
    Relocations,
}

impl LimitKind {
    /// The [`Limits`](crate::Limits) field controlling this budget.
    #[must_use]
    pub const fn field(self) -> &'static str {
        match self {
            Self::TableBytes => "max_table_bytes",
            Self::Symbols => "max_symbols",
            Self::Relocations => "max_relocations",
        }
    }
}

impl fmt::Display for LimitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TableBytes => "table bytes",
            Self::Symbols => "symbol count",
            Self::Relocations => "combined relocation count",
        })
    }
}

/// Why linking metadata cannot be inspected. Nothing is partially returned.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinkError {
    /// The executable or its runtime plan was rejected by `pentax-loader`.
    Load(LoadError),
    /// The table has no complete extent. Standard symbols may use a bounded
    /// SysV hash count, but section headers, SCE size tags and relocation
    /// indices are never used as a substitute.
    UnknownExtent {
        table: TableKind,
        address: DynamicAddress,
    },
    InvalidEntrySize {
        table: TableKind,
        entry_size: Option<u64>,
        expected: u64,
    },
    PartialRecord {
        table: TableKind,
        size: u64,
        entry_size: u64,
    },
    /// A declared size cannot be addressed by this host.
    HostSizeOverflow {
        table: TableKind,
        size: u64,
    },
    SizeOverflow {
        context: &'static str,
    },
    /// DT_HASH is a singleton pointer in the standard guest address domain.
    DuplicateHashPointer,
    /// A hash must have buckets and include the reserved zeroth symbol.
    InvalidSysvHashCounts {
        buckets: u32,
        chains: u32,
    },
    SysvHashIndexOutOfRange {
        array: &'static str,
        index: usize,
        symbol: u32,
        symbol_count: u32,
    },
    LimitExceeded {
        limit: LimitKind,
        requested: u64,
        maximum: u64,
    },
    MissingDynlibData {
        table: TableKind,
    },
    TableOutOfBounds {
        table: TableKind,
        offset: u64,
        size: u64,
        available: u64,
    },
    MissingStringTable {
        symbol: usize,
        name_offset: u32,
    },
    NameOffsetOutOfRange {
        symbol: usize,
        name_offset: u32,
        table_size: u64,
    },
    UnterminatedName {
        symbol: usize,
        name_offset: u32,
    },
    /// Retaining this name would exceed `Limits::max_name_bytes`.
    NameBudgetExceeded {
        symbol: usize,
        name_offset: u32,
        used: u64,
        maximum: u64,
    },
    /// A relocation names a nonzero symbol, but no symbol table exists.
    MissingSymbolTable {
        table: TableKind,
        relocation: usize,
        symbol: u32,
    },
    SymbolIndexOutOfRange {
        table: TableKind,
        relocation: usize,
        symbol: u32,
        symbol_count: usize,
    },
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(error) => write!(f, "executable: {error}"),
            Self::UnknownExtent { table, address } => write!(
                f,
                "{table} at {} has no complete extent; a declared size or supported bounded extent is required",
                Location(*address)
            ),
            Self::InvalidEntrySize {
                table,
                entry_size: Some(value),
                expected,
            } => write!(
                f,
                "{table} entry size {value} is unsupported; expected {expected}"
            ),
            Self::InvalidEntrySize {
                table,
                entry_size: None,
                expected,
            } => write!(f, "{table} declares no entry size; expected {expected}"),
            Self::PartialRecord {
                table,
                size,
                entry_size,
            } => write!(
                f,
                "{table} size {size:#x} is not a whole number of {entry_size}-byte records"
            ),
            Self::HostSizeOverflow { table, size } => {
                write!(f, "{table} size {size:#x} cannot be addressed on this host")
            }
            Self::SizeOverflow { context } => write!(f, "{context} overflows 64-bit arithmetic"),
            Self::DuplicateHashPointer => f.write_str("duplicate singleton DT_HASH pointer"),
            Self::InvalidSysvHashCounts { buckets, chains } => write!(
                f,
                "SysV hash declares {buckets} buckets and {chains} chains; both counts must be nonzero"
            ),
            Self::SysvHashIndexOutOfRange {
                array,
                index,
                symbol,
                symbol_count,
            } => write!(
                f,
                "SysV hash {array}[{index}] references symbol {symbol}, but nchain is {symbol_count}"
            ),
            Self::LimitExceeded {
                limit,
                requested,
                maximum,
            } => write!(
                f,
                "{limit} {requested} exceeds Limits::{} = {maximum}; raise it only for trusted input",
                limit.field()
            ),
            Self::MissingDynlibData { table } => write!(
                f,
                "{table} uses an SCE dynlib offset, but the image has no PT_SCE_DYNLIBDATA"
            ),
            Self::TableOutOfBounds {
                table,
                offset,
                size,
                available,
            } => write!(
                f,
                "{table} at dynlib offset {offset:#x} of {size:#x} bytes exceeds PT_SCE_DYNLIBDATA size {available:#x}"
            ),
            Self::MissingStringTable {
                symbol,
                name_offset,
            } => write!(
                f,
                "symbol {symbol} has name offset {name_offset:#x}, but the dynamic section has no string table"
            ),
            Self::NameOffsetOutOfRange {
                symbol,
                name_offset,
                table_size,
            } => write!(
                f,
                "symbol {symbol} name offset {name_offset:#x} is outside the {table_size:#x}-byte string table"
            ),
            Self::UnterminatedName {
                symbol,
                name_offset,
            } => write!(
                f,
                "symbol {symbol} name at offset {name_offset:#x} has no NUL terminator inside the string table"
            ),
            Self::NameBudgetExceeded {
                symbol,
                name_offset,
                used,
                maximum,
            } => write!(
                f,
                "symbol {symbol} name at offset {name_offset:#x} exceeds Limits::max_name_bytes = {maximum} after {used} retained bytes; raise it only for trusted input"
            ),
            Self::MissingSymbolTable {
                table,
                relocation,
                symbol,
            } => write!(
                f,
                "{table} record {relocation} references symbol {symbol}, but the dynamic section has no symbol table"
            ),
            Self::SymbolIndexOutOfRange {
                table,
                relocation,
                symbol,
                symbol_count,
            } => write!(
                f,
                "{table} record {relocation} references symbol {symbol}, but the symbol table has {symbol_count} entries"
            ),
        }
    }
}

impl Error for LinkError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        if let Self::Load(error) = self {
            Some(error)
        } else {
            None
        }
    }
}

impl From<LoadError> for LinkError {
    fn from(error: LoadError) -> Self {
        Self::Load(error)
    }
}

/// Human-readable address that names its domain.
struct Location(DynamicAddress);

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            DynamicAddress::GuestVirtual(address) => write!(f, "guest address {address:#x}"),
            DynamicAddress::DynlibOffset(offset) => {
                write!(f, "PT_SCE_DYNLIBDATA offset {offset:#x}")
            }
        }
    }
}
