// SPDX-License-Identifier: GPL-2.0-or-later

//! Bounded standard SysV hash extents, without hash lookup or linking.
//!
//! The gABI uses the same u32 layout for ELF32 and ELF64: two counts,
//! `bucket[nbucket]`, then `chain[nchain]`. `nchain` establishes the complete
//! dynamic symbol count. <https://gabi.xinuos.com/elf/08-dynamic.html#hash-table>

use pentax_loader::{DynamicAddress, DynamicTable, ExecutableView};

use crate::records::ELF64_SYMBOL_SIZE;
use crate::tables::{PlannedTable, TablePlan, plan_tables, plan_with_symbol_extent};
use crate::{Limits, LinkError, TableKind};

const DT_HASH: i64 = 4;
const HEADER_SIZE: u64 = 8;
const WORD_SIZE: u64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlannedHash {
    pub(crate) table: PlannedTable,
    pub(crate) buckets: u32,
    pub(crate) chains: u32,
}

/// Only an unsized standard symbol table can consume DT_HASH. Explicit symbol
/// sizes require no hash reads, and SCE offsets never consume standard hashes.
pub(crate) fn plan_inspection(
    view: &ExecutableView<'_>,
    dynamic: &DynamicTable,
    limits: &Limits,
) -> Result<(TablePlan, Option<PlannedHash>), LinkError> {
    let pointer = hash_pointer(dynamic)?;
    let Some(symbols) = dynamic.symbols else {
        return Ok((plan_tables(dynamic, limits)?, None));
    };
    if symbols.size.is_some() || !matches!(symbols.address, DynamicAddress::GuestVirtual(_)) {
        return Ok((plan_tables(dynamic, limits)?, None));
    }
    let Some(pointer) = pointer else {
        // GNU hash and SCE size tags do not establish a standard symbol count.
        return Ok((plan_tables(dynamic, limits)?, None));
    };

    // Charge the fixed stack header and every already-known table first. No
    // source hash bytes are read when even this minimum exceeds the budget.
    plan_with_symbol_extent(dynamic, 0, HEADER_SIZE, limits)?;
    let mut header = [0; HEADER_SIZE as usize];
    view.read_initialized_vaddr(pointer, &mut header)?;
    let buckets = u32::from_le_bytes(header[..4].try_into().expect("four-byte header word"));
    let chains = u32::from_le_bytes(header[4..].try_into().expect("four-byte header word"));
    if buckets == 0 || chains == 0 {
        return Err(LinkError::InvalidSysvHashCounts { buckets, chains });
    }
    let size = u64::from(buckets)
        .checked_add(u64::from(chains))
        .and_then(|words| words.checked_mul(WORD_SIZE))
        .and_then(|arrays| arrays.checked_add(HEADER_SIZE))
        .ok_or(LinkError::SizeOverflow {
            context: "SysV hash table size",
        })?;
    let symbol_size =
        u64::from(chains)
            .checked_mul(ELF64_SYMBOL_SIZE)
            .ok_or(LinkError::SizeOverflow {
                context: "SysV hash symbol extent",
            })?;
    // Check all combined bytes and record counts before any complete table is
    // copied. Untrusted header counts cannot drive unbounded allocations.
    let tables = plan_with_symbol_extent(dynamic, symbol_size, size, limits)?;
    let length = usize::try_from(size).map_err(|_| LinkError::HostSizeOverflow {
        table: TableKind::SysvHash,
        size,
    })?;
    pointer.checked_add(size).ok_or(LinkError::SizeOverflow {
        context: "SysV hash address range",
    })?;
    Ok((
        tables,
        Some(PlannedHash {
            table: PlannedTable {
                kind: TableKind::SysvHash,
                address: DynamicAddress::GuestVirtual(pointer),
                size,
                length,
                records: 0,
            },
            buckets,
            chains,
        }),
    ))
}

fn hash_pointer(dynamic: &DynamicTable) -> Result<Option<u64>, LinkError> {
    let mut pointer = None;
    for entry in dynamic.entries.iter().filter(|entry| entry.tag == DT_HASH) {
        if pointer.replace(entry.value).is_some() {
            return Err(LinkError::DuplicateHashPointer);
        }
    }
    Ok(pointer)
}

/// The entire initialized table must already have been read. Zero is the
/// reserved STN_UNDEF terminator; all nonzero entries must name real symbols.
/// Chains are not traversed, so this does not implement or validate lookup.
pub(crate) fn validate_indices(bytes: &[u8], hash: &PlannedHash) -> Result<(), LinkError> {
    for (position, word) in bytes[HEADER_SIZE as usize..].chunks_exact(4).enumerate() {
        let symbol = u32::from_le_bytes(word.try_into().expect("four-byte hash word"));
        if symbol != 0 && symbol >= hash.chains {
            let buckets = hash.buckets as usize;
            let (array, index) = if position < buckets {
                ("bucket", position)
            } else {
                ("chain", position - buckets)
            };
            return Err(LinkError::SysvHashIndexOutOfRange {
                array,
                index,
                symbol,
                symbol_count: hash.chains,
            });
        }
    }
    Ok(())
}
