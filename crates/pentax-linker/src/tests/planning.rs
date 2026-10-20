// SPDX-License-Identifier: GPL-2.0-or-later

//! Internal planning and extraction checks. These forge loader regions that
//! the public entry point never accepts from callers, to exercise defensive
//! paths that `pentax-loader` currently rejects first.

use pentax_loader::{DynamicAddress, DynamicRegion, DynamicTable, open_executable};

use super::fixture::{Domain, Tables, address, image};
use crate::tables::{PlannedTable, plan_tables, read_table};
use crate::{LimitKind, Limits, LinkError, TableKind};

fn region(size: Option<u64>, entry_size: Option<u64>) -> Option<DynamicRegion> {
    Some(DynamicRegion {
        address: DynamicAddress::DynlibOffset(0),
        size,
        entry_size,
    })
}

fn dynamic(
    symbols: Option<DynamicRegion>,
    rela: Option<DynamicRegion>,
    plt_rela: Option<DynamicRegion>,
) -> DynamicTable {
    DynamicTable {
        entries: Vec::new(),
        strings: None,
        symbols,
        rela,
        plt_rela,
    }
}

#[test]
fn widths_partial_records_and_unknown_extents_are_structural_errors() {
    let limits = Limits::default();
    assert_eq!(
        plan_tables(&dynamic(None, region(Some(25), Some(24)), None), &limits),
        Err(LinkError::PartialRecord {
            table: TableKind::Rela,
            size: 25,
            entry_size: 24
        })
    );
    assert_eq!(
        plan_tables(&dynamic(region(Some(48), Some(16)), None, None), &limits),
        Err(LinkError::InvalidEntrySize {
            table: TableKind::Symbols,
            entry_size: Some(16),
            expected: 24
        })
    );
    assert_eq!(
        plan_tables(&dynamic(None, None, region(Some(24), None)), &limits),
        Err(LinkError::InvalidEntrySize {
            table: TableKind::PltRela,
            entry_size: None,
            expected: 24
        })
    );
    assert_eq!(
        plan_tables(&dynamic(None, None, region(None, Some(24))), &limits),
        Err(LinkError::UnknownExtent {
            table: TableKind::PltRela,
            address: DynamicAddress::DynlibOffset(0)
        })
    );
}

#[test]
fn declared_sizes_are_bounded_before_any_allocation() {
    let limits = Limits::default();
    // The largest multiple of 24 representable in u64.
    let huge = u64::MAX - 15;
    let overflow = plan_tables(
        &dynamic(
            None,
            region(Some(huge), Some(24)),
            region(Some(huge), Some(24)),
        ),
        &limits,
    );
    assert!(matches!(
        overflow,
        Err(LinkError::SizeOverflow { .. } | LinkError::HostSizeOverflow { .. })
    ));
    let giant = plan_tables(&dynamic(region(Some(huge), Some(24)), None, None), &limits);
    assert!(matches!(
        giant,
        Err(LinkError::LimitExceeded {
            limit: LimitKind::TableBytes,
            ..
        } | LinkError::HostSizeOverflow { .. })
    ));
    // Record counts are bounded independently of the byte budget.
    let half = Some(24 * 600_000);
    assert_eq!(
        plan_tables(
            &dynamic(None, region(half, Some(24)), region(half, Some(24))),
            &limits
        ),
        Err(LinkError::LimitExceeded {
            limit: LimitKind::Relocations,
            requested: 1_200_000,
            maximum: 1_000_000
        })
    );
    assert_eq!(
        plan_tables(
            &dynamic(region(Some(24 * 1_000_001), Some(24)), None, None),
            &limits
        ),
        Err(LinkError::LimitExceeded {
            limit: LimitKind::Symbols,
            requested: 1_000_001,
            maximum: 1_000_000
        })
    );
}

#[test]
fn table_reads_use_their_own_domain_and_complete_bounds() {
    let tables = Tables {
        strings: Some(b"\0abc\0".to_vec()),
        ..Tables::default()
    };
    let bytes = image(Domain::Dynlib, &tables);
    let view = open_executable(&bytes).unwrap();
    let dynlib = view.runtime_plan().unwrap().dynlib_data;
    let table = |address, size: u64| PlannedTable {
        kind: TableKind::Strings,
        address,
        size,
        length: usize::try_from(size).unwrap(),
        records: 0,
    };
    let read = |address, size, dynlib| read_table(&view, dynlib, &table(address, size));

    assert_eq!(
        &*read(DynamicAddress::DynlibOffset(1), 4, dynlib).unwrap(),
        b"abc\0"
    );
    for offset in [2, u64::MAX] {
        assert_eq!(
            read(DynamicAddress::DynlibOffset(offset), 4, dynlib),
            Err(LinkError::TableOutOfBounds {
                table: TableKind::Strings,
                offset,
                size: 4,
                available: 5
            })
        );
    }
    assert_eq!(
        read(DynamicAddress::DynlibOffset(0), 1, None),
        Err(LinkError::MissingDynlibData {
            table: TableKind::Strings
        })
    );

    // The same bytes through their original guest virtual address.
    let guest = address(Domain::Guest, 1);
    assert_eq!(
        &*read(DynamicAddress::GuestVirtual(guest), 3, None).unwrap(),
        b"abc"
    );
    // File offset zero is not guest address zero, and the initialized image
    // ends one byte into this read; the following bytes are BSS.
    for (address, size) in [(0, 4), (guest + 3, 4)] {
        assert!(matches!(
            read(DynamicAddress::GuestVirtual(address), size, dynlib),
            Err(LinkError::Load(_))
        ));
    }
}
