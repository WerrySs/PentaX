// SPDX-License-Identifier: GPL-2.0-or-later

//! Structural dynamic parsing only: no symbol resolution or relocation writes.
//! Standard addresses: https://gabi.xinuos.com/elf/08-dynamic.html
//! SCE offsets: https://github.com/shadps4-emu/shadPS4/blob/main/src/core/module.cpp

use super::RuntimeError;
use crate::{LoadError, read_u64};
use std::{error::Error, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicAddress {
    GuestVirtual(u64),
    DynlibOffset(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynamicEntry {
    pub tag: i64,
    pub value: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynamicRegion {
    pub address: DynamicAddress,
    /// None means the complete table extent is not established. Such a region
    /// cannot be iterated as a complete table or used as a resolver bound.
    pub size: Option<u64>,
    pub entry_size: Option<u64>,
}

/// Known table descriptions plus every entry preceding DT_NULL. Unknown tags
/// are retained, not declared implemented. Full import resolution is deferred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicTable {
    pub entries: Vec<DynamicEntry>,
    pub strings: Option<DynamicRegion>,
    pub symbols: Option<DynamicRegion>,
    pub rela: Option<DynamicRegion>,
    pub plt_rela: Option<DynamicRegion>,
}

impl DynamicTable {
    pub(super) fn regions(&self) -> impl Iterator<Item = DynamicRegion> + '_ {
        [self.strings, self.symbols, self.rela, self.plt_rela]
            .into_iter()
            .flatten()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DynamicError {
    InvalidSize {
        size: usize,
    },
    MissingTerminator,
    DuplicateTag {
        tag: i64,
    },
    MixedAddressDomains {
        standard: i64,
        sce: i64,
    },
    MissingCompanion {
        tag: i64,
        required: i64,
    },
    InvalidEntrySize {
        tag: i64,
        value: u64,
        expected: u64,
    },
    InvalidTableSize {
        tag: i64,
        size: u64,
        entry_size: u64,
    },
    UnsupportedPltRelocations {
        value: u64,
    },
}

impl fmt::Display for DynamicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSize { size } => {
                write!(f, "payload size {size} must be a nonzero multiple of 16")
            }
            Self::MissingTerminator => write!(f, "DT_NULL is missing within the declared payload"),
            Self::DuplicateTag { tag } => write!(f, "duplicate singleton tag {tag:#x}"),
            Self::MixedAddressDomains { standard, sce } => write!(
                f,
                "conflicting standard/SCE tags {standard:#x} and {sce:#x}"
            ),
            Self::MissingCompanion { tag, required } => {
                write!(f, "tag {tag:#x} requires tag {required:#x}")
            }
            Self::InvalidEntrySize {
                tag,
                value,
                expected,
            } => write!(f, "tag {tag:#x} entry size {value} must be {expected}"),
            Self::InvalidTableSize {
                tag,
                size,
                entry_size,
            } => write!(
                f,
                "tag {tag:#x} table size {size} is not divisible by {entry_size}"
            ),
            Self::UnsupportedPltRelocations { value } => write!(
                f,
                "PLT relocation format {value} is unsupported; expected DT_RELA (7)"
            ),
        }
    }
}
impl Error for DynamicError {}
impl From<DynamicError> for LoadError {
    fn from(error: DynamicError) -> Self {
        RuntimeError::Dynamic(error).into()
    }
}

/// Parse bounded 16-byte dynamic records. Addresses retain their distinct
/// domains; runtime_plan validates known regions against their correct backing.
/// The parser never follows unbounded pointers or performs relocations.
pub fn parse_dynamic(bytes: &[u8]) -> Result<DynamicTable, LoadError> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(16) {
        return Err(DynamicError::InvalidSize { size: bytes.len() }.into());
    }
    let mut entries = Vec::new();
    let mut terminated = false;
    for raw in bytes.chunks_exact(16) {
        let tag = read_u64(raw, 0) as i64;
        if tag == 0 {
            terminated = true;
            break;
        }
        entries.push(DynamicEntry {
            tag,
            value: read_u64(raw, 8),
        });
    }
    if !terminated {
        return Err(DynamicError::MissingTerminator.into());
    }
    let strings = region(
        &entries,
        [5, 0x61000035],
        [Some(10), Some(0x61000037)],
        None,
        None,
    )?;
    let symbols = region(
        &entries,
        [6, 0x61000039],
        [Some(39), Some(0x6100003f)],
        Some([11, 0x6100003b]),
        Some(24),
    )?;
    let rela = region(
        &entries,
        [7, 0x6100002f],
        [Some(8), Some(0x61000031)],
        Some([9, 0x61000033]),
        Some(24),
    )?;
    let plt_rela = region(
        &entries,
        [23, 0x61000029],
        [Some(2), Some(0x6100002d)],
        None,
        Some(24),
    )?;
    if let Some(plt_region) = plt_rela {
        let (table_tag, format_tag) =
            if matches!(plt_region.address, DynamicAddress::GuestVirtual(_)) {
                (23, 20)
            } else {
                (0x61000029, 0x6100002b)
            };
        let value = singleton(&entries, format_tag)?.ok_or(DynamicError::MissingCompanion {
            tag: table_tag,
            required: format_tag,
        })?;
        if value != 7 {
            return Err(DynamicError::UnsupportedPltRelocations { value }.into());
        }
    }
    Ok(DynamicTable {
        entries,
        strings,
        symbols,
        rela,
        plt_rela,
    })
}

fn singleton(entries: &[DynamicEntry], tag: i64) -> Result<Option<u64>, LoadError> {
    let mut found = None;
    for entry in entries.iter().filter(|entry| entry.tag == tag) {
        if found.is_some() {
            return Err(DynamicError::DuplicateTag { tag }.into());
        }
        found = Some(entry.value);
    }
    Ok(found)
}

fn region(
    entries: &[DynamicEntry],
    pointers: [i64; 2],
    sizes: [Option<i64>; 2],
    widths: Option<[i64; 2]>,
    expected_width: Option<u64>,
) -> Result<Option<DynamicRegion>, LoadError> {
    let values = [
        singleton(entries, pointers[0])?,
        singleton(entries, pointers[1])?,
    ];
    if values[0].is_some() && values[1].is_some() {
        return Err(DynamicError::MixedAddressDomains {
            standard: pointers[0],
            sce: pointers[1],
        }
        .into());
    }
    let (family, pointer) = if let Some(value) = values[0] {
        (0, value)
    } else if let Some(value) = values[1] {
        (1, value)
    } else {
        return Ok(None);
    };
    if let [Some(standard), Some(sce)] = sizes
        && singleton(entries, standard)?.is_some()
        && singleton(entries, sce)?.is_some()
    {
        return Err(DynamicError::MixedAddressDomains { standard, sce }.into());
    }
    if let Some([standard, sce]) = widths
        && singleton(entries, standard)?.is_some()
        && singleton(entries, sce)?.is_some()
    {
        return Err(DynamicError::MixedAddressDomains { standard, sce }.into());
    }
    let size = if let Some(tag) = sizes[family] {
        singleton(entries, tag)?
    } else {
        None
    };
    // Standard DT_SYMTAB historically has no size tag. Preserve that explicit
    // unknown extent; a later resolver must establish its own bounded count.
    if size.is_none() && !(family == 0 && pointers[0] == 6) {
        return Err(DynamicError::MissingCompanion {
            tag: pointers[family],
            required: sizes[family].unwrap_or(0),
        }
        .into());
    }
    if let Some(tags) = widths {
        let tag = tags[family];
        let value = singleton(entries, tag)?.ok_or(DynamicError::MissingCompanion {
            tag: pointers[family],
            required: tag,
        })?;
        if let Some(expected) = expected_width
            && value != expected
        {
            return Err(DynamicError::InvalidEntrySize {
                tag,
                value,
                expected,
            }
            .into());
        }
    }
    if let (Some(size), Some(width)) = (size, expected_width)
        && size % width != 0
    {
        return Err(DynamicError::InvalidTableSize {
            tag: pointers[family],
            size,
            entry_size: width,
        }
        .into());
    }
    Ok(Some(DynamicRegion {
        address: if family == 0 {
            DynamicAddress::GuestVirtual(pointer)
        } else {
            DynamicAddress::DynlibOffset(pointer)
        },
        size,
        entry_size: expected_width,
    }))
}
