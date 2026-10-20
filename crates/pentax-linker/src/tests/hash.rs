// SPDX-License-Identifier: GPL-2.0-or-later

//! Original standard ELF fixtures with bounded SysV hash symbol extents.

use pentax_loader::{DynamicAddress, open_executable};

use super::fixture::{
    Domain, SCE_SYMENT, SCE_SYMTAB, SCE_SYMTABSZ, address, assemble, null_symbol, rela, symbol,
};
use super::inspect_bytes;
use crate::{LimitKind, Limits, LinkError, TableExtent, TableKind};

const HASH_OFFSET: usize = 104;
const KNOWN_BYTES: u64 = 6 + 24 + 24;
const SYMBOL_BYTES: u64 = 2 * 24;
const HASH_BYTES: u64 = 8 + 4 * (2 + 2);
const TOTAL_BYTES: u64 = KNOWN_BYTES + SYMBOL_BYTES + HASH_BYTES;

fn header(buckets: u32, chains: u32) -> Vec<u8> {
    [buckets.to_le_bytes(), chains.to_le_bytes()].concat()
}

fn hash(buckets: &[u32], chains: &[u32]) -> Vec<u8> {
    let mut bytes = header(buckets.len() as u32, chains.len() as u32);
    for word in buckets.iter().chain(chains) {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes
}

fn valid_hash() -> Vec<u8> {
    hash(&[1, 0], &[0, 0])
}

fn hash_image(hash: &[u8], change: impl FnOnce(&mut Vec<(i64, u64)>)) -> Vec<u8> {
    let at = |offset| address(Domain::Guest, offset);
    let mut data = b"\0puts\0\0\0".to_vec();
    data.extend_from_slice(&null_symbol());
    data.extend_from_slice(&symbol(1, 0x12, 0, 0, 0, 0));
    data.extend_from_slice(&rela(0x2000, 1, 1, -8));
    data.extend_from_slice(&rela(0x3000, 1, 7, 0));
    assert_eq!(data.len(), HASH_OFFSET);
    data.extend_from_slice(hash);
    let mut entries = vec![
        (5, at(0)),
        (10, 6),
        (6, at(8)),
        (11, 24),
        (7, at(56)),
        (8, 24),
        (9, 24),
        (23, at(80)),
        (2, 24),
        (20, 7),
        (4, at(HASH_OFFSET)),
    ];
    change(&mut entries);
    assemble(Domain::Guest, &entries, &data)
}

#[test]
fn standard_hash_establishes_complete_symbols_and_accounts_for_all_hash_bytes() {
    let bytes = hash_image(&valid_hash(), |entries| {
        // An SCE-only size is deliberately different. It cannot establish a
        // standard table extent, and the standard hash remains authoritative.
        entries.push((SCE_SYMTABSZ, 24 * 999));
    });
    let view = open_executable(&bytes).unwrap();
    let original = view.runtime_plan().unwrap().dynamic.unwrap();
    assert_eq!(original.symbols.unwrap().size, None);
    let metadata = inspect_bytes(&bytes, &Limits::default())
        .unwrap()
        .into_metadata()
        .unwrap();
    assert_eq!(metadata.symbols().len(), 2);
    assert_eq!(
        metadata.symbols()[1].name.as_ref().unwrap().as_bytes(),
        b"puts"
    );
    assert_eq!(metadata.rela()[0].symbol_index(), 1);
    assert_eq!(metadata.plt_rela()[0].symbol_index(), 1);
    assert_eq!(
        metadata.tables().symbols,
        Some(TableExtent {
            address: DynamicAddress::GuestVirtual(address(Domain::Guest, 8)),
            size: SYMBOL_BYTES,
        })
    );
    assert_eq!(
        metadata.tables().sysv_hash,
        Some(TableExtent {
            address: DynamicAddress::GuestVirtual(address(Domain::Guest, HASH_OFFSET)),
            size: HASH_BYTES,
        })
    );
    let counts = metadata.counts();
    assert_eq!((counts.sysv_hash_buckets, counts.sysv_hash_chains), (2, 2));
    assert_eq!(counts.symbols, 2);
    assert_eq!(counts.table_bytes, TOTAL_BYTES);
    // Inspection does not mutate the loader's preserved dynamic metadata.
    assert_eq!(view.runtime_plan().unwrap().dynamic.unwrap(), original);
}

#[test]
fn explicit_symbol_sizes_require_no_hash_reads_or_accounting() {
    let bytes = hash_image(&[], |entries| {
        entries.push((39, SYMBOL_BYTES));
        entries.iter_mut().find(|entry| entry.0 == 4).unwrap().1 = 0;
    });
    let metadata = inspect_bytes(&bytes, &Limits::default())
        .unwrap()
        .into_metadata()
        .unwrap();
    assert_eq!(metadata.counts().symbols, 2);
    assert_eq!(metadata.tables().sysv_hash, None);
    assert_eq!(metadata.counts().sysv_hash_buckets, 0);
    assert_eq!(metadata.counts().sysv_hash_chains, 0);
    assert_eq!(metadata.counts().table_bytes, KNOWN_BYTES + SYMBOL_BYTES);

    // A standard pointer must never be followed as an SCE dynlib offset.
    let bytes = assemble(
        Domain::Dynlib,
        &[
            (SCE_SYMTAB, 0),
            (SCE_SYMTABSZ, 24),
            (SCE_SYMENT, 24),
            (4, 0),
        ],
        &null_symbol(),
    );
    let metadata = inspect_bytes(&bytes, &Limits::default())
        .unwrap()
        .into_metadata()
        .unwrap();
    assert_eq!(metadata.counts().symbols, 1);
    assert_eq!(metadata.tables().sysv_hash, None);
}

#[test]
fn duplicate_hash_pointers_are_rejected_even_with_an_explicit_symbol_size() {
    for explicit in [false, true] {
        for duplicate in [0, address(Domain::Guest, HASH_OFFSET)] {
            let bytes = hash_image(&valid_hash(), |entries| {
                entries.push((4, duplicate));
                if explicit {
                    entries.push((39, SYMBOL_BYTES));
                }
            });
            assert_eq!(
                inspect_bytes(&bytes, &Limits::default()),
                Err(LinkError::DuplicateHashPointer)
            );
        }
    }
}

#[test]
fn no_supported_hash_preserves_unknown_extent_without_cross_domain_guessing() {
    for extra in [
        None,
        Some((SCE_SYMTABSZ, SYMBOL_BYTES)),
        Some((0x6fff_fef5, 0)),
    ] {
        let bytes = hash_image(&valid_hash(), |entries| {
            entries.retain(|entry| entry.0 != 4);
            if let Some(entry) = extra {
                entries.push(entry);
            }
        });
        assert_eq!(
            inspect_bytes(&bytes, &Limits::default()),
            Err(LinkError::UnknownExtent {
                table: TableKind::Symbols,
                address: DynamicAddress::GuestVirtual(address(Domain::Guest, 8)),
            })
        );
    }
}

#[test]
fn tiny_and_aggregate_budgets_precede_hash_reads_and_complete_table_copies() {
    let unreadable = hash_image(&[], |entries| {
        entries.iter_mut().find(|entry| entry.0 == 4).unwrap().1 = u64::MAX;
    });
    for maximum in 0..8 {
        let limits = Limits {
            max_table_bytes: maximum,
            ..Limits::default()
        };
        assert_eq!(
            inspect_bytes(&unreadable, &limits),
            Err(LinkError::LimitExceeded {
                limit: LimitKind::TableBytes,
                requested: KNOWN_BYTES + 8,
                maximum,
            })
        );
    }
    let bytes = hash_image(&valid_hash(), |_| {});
    assert_eq!(
        inspect_bytes(
            &bytes,
            &Limits {
                max_table_bytes: TOTAL_BYTES - 1,
                ..Limits::default()
            }
        ),
        Err(LinkError::LimitExceeded {
            limit: LimitKind::TableBytes,
            requested: TOTAL_BYTES,
            maximum: TOTAL_BYTES - 1,
        })
    );
    let exact = Limits {
        max_table_bytes: TOTAL_BYTES,
        max_symbols: 2,
        max_relocations: 2,
        max_name_bytes: 4,
    };
    assert!(inspect_bytes(&bytes, &exact).is_ok());
    assert_eq!(
        inspect_bytes(
            &bytes,
            &Limits {
                max_symbols: 1,
                ..exact
            }
        ),
        Err(LinkError::LimitExceeded {
            limit: LimitKind::Symbols,
            requested: 2,
            maximum: 1,
        })
    );
}

#[test]
fn truncated_headers_arrays_and_wrapping_or_wrong_domain_pointers_fail() {
    let complete = valid_hash();
    for length in 0..complete.len() {
        let bytes = hash_image(&complete[..length], |_| {});
        assert!(
            matches!(
                inspect_bytes(&bytes, &Limits::default()),
                Err(LinkError::Load(_))
            ),
            "hash length {length}"
        );
    }
    for pointer in [HASH_OFFSET as u64, u64::MAX - 3, u64::MAX] {
        let bytes = hash_image(&complete, |entries| {
            entries.iter_mut().find(|entry| entry.0 == 4).unwrap().1 = pointer;
        });
        assert!(matches!(
            inspect_bytes(&bytes, &Limits::default()),
            Err(LinkError::Load(_))
        ));
    }
}

#[test]
fn zero_counts_and_invalid_bucket_or_chain_indices_are_rejected() {
    for (buckets, chains) in [(0, 0), (0, 2), (2, 0)] {
        let bytes = hash_image(&header(buckets, chains), |_| {});
        assert_eq!(
            inspect_bytes(&bytes, &Limits::default()),
            Err(LinkError::InvalidSysvHashCounts { buckets, chains })
        );
    }
    for (position, array, index, symbol) in [
        (0, "bucket", 0, 2),
        (1, "bucket", 1, u32::MAX),
        (2, "chain", 0, 2),
        (3, "chain", 1, u32::MAX),
    ] {
        let mut hash = valid_hash();
        let offset = 8 + position * 4;
        hash[offset..offset + 4].copy_from_slice(&symbol.to_le_bytes());
        let bytes = hash_image(&hash, |_| {});
        assert_eq!(
            inspect_bytes(&bytes, &Limits::default()),
            Err(LinkError::SysvHashIndexOutOfRange {
                array,
                index,
                symbol,
                symbol_count: 2,
            })
        );
    }
}

#[test]
fn giant_header_declarations_fail_budgets_before_array_allocation_or_reads() {
    for (buckets, chains) in [(u32::MAX, 2), (2, u32::MAX), (u32::MAX, u32::MAX)] {
        let bytes = hash_image(&header(buckets, chains), |_| {});
        assert!(matches!(
            inspect_bytes(&bytes, &Limits::default()),
            Err(LinkError::LimitExceeded {
                limit: LimitKind::TableBytes,
                ..
            })
        ));
    }
    let bytes = hash_image(&header(1, u32::MAX), |_| {});
    assert_eq!(
        inspect_bytes(
            &bytes,
            &Limits {
                max_table_bytes: u64::MAX,
                ..Limits::default()
            }
        ),
        Err(LinkError::LimitExceeded {
            limit: LimitKind::Symbols,
            requested: u64::from(u32::MAX),
            maximum: Limits::default().max_symbols,
        })
    );
}

#[test]
fn aggregate_accounting_checks_overflow_including_hash_bytes() {
    let bytes = hash_image(&valid_hash(), |_| {});
    let view = open_executable(&bytes).unwrap();
    let mut dynamic = view.runtime_plan().unwrap().dynamic.unwrap();
    dynamic.strings.as_mut().unwrap().size = Some(u64::MAX - 7);
    let limits = Limits {
        max_table_bytes: u64::MAX,
        ..Limits::default()
    };
    assert!(matches!(
        crate::tables::plan_with_symbol_extent(&dynamic, SYMBOL_BYTES, HASH_BYTES, &limits),
        Err(LinkError::SizeOverflow { .. } | LinkError::HostSizeOverflow { .. })
    ));
}
