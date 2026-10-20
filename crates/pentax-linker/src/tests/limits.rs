// SPDX-License-Identifier: GPL-2.0-or-later

use super::fixture::{Domain, Tables, image, null_symbol, rela, symbol};
use super::{DOMAINS, ZERO, inspect_bytes, sample_tables};
use crate::{LimitKind, Limits, LinkError, LinkMetadata};

fn with(change: impl FnOnce(&mut Limits)) -> Limits {
    let mut limits = Limits::default();
    change(&mut limits);
    limits
}

fn exceeded(limit: LimitKind, requested: u64, maximum: u64) -> LinkError {
    LinkError::LimitExceeded {
        limit,
        requested,
        maximum,
    }
}

fn dynamic(bytes: &[u8], limits: &Limits) -> LinkMetadata {
    inspect_bytes(bytes, limits)
        .unwrap()
        .into_metadata()
        .unwrap()
}

#[test]
fn every_budget_is_inclusive_and_tiny_limits_are_respected() {
    for domain in DOMAINS {
        let bytes = image(domain, &sample_tables());
        let run = |limits: Limits| inspect_bytes(&bytes, &limits);
        assert_eq!(run(ZERO), Err(exceeded(LimitKind::TableBytes, 208, 0)));
        assert_eq!(
            run(with(|limits| limits.max_table_bytes = 207)),
            Err(exceeded(LimitKind::TableBytes, 208, 207))
        );
        assert_eq!(
            run(with(|limits| limits.max_symbols = 3)),
            Err(exceeded(LimitKind::Symbols, 4, 3))
        );
        // Names of 4 and 3 bytes fit; the 5-byte third name does not.
        assert_eq!(
            run(with(|limits| limits.max_name_bytes = 11)),
            Err(LinkError::NameBudgetExceeded {
                symbol: 3,
                name_offset: 10,
                used: 7,
                maximum: 11
            })
        );
        let exact = Limits {
            max_table_bytes: 208,
            max_name_bytes: 12,
            max_symbols: 4,
            max_relocations: 4,
        };
        assert_eq!(dynamic(&bytes, &exact).counts().name_bytes, 12);
    }
}

#[test]
fn zero_limits_accept_present_but_empty_tables() {
    let tables = Tables {
        symbols: Some(Vec::new()),
        rela: Some(Vec::new()),
        ..Tables::default()
    };
    for domain in DOMAINS {
        let metadata = dynamic(&image(domain, &tables), &ZERO);
        assert!(metadata.symbol_table_present());
        assert_eq!(metadata.counts().symbols, 0);
    }
}

#[test]
fn regular_and_plt_relocations_share_one_limit() {
    let tables = Tables {
        rela: Some([rela(0, 0, 8, 1), rela(8, 0, 8, 2)].concat()),
        plt_rela: Some([rela(16, 0, 7, 0), rela(24, 0, 7, 0)].concat()),
        ..Tables::default()
    };
    for domain in DOMAINS {
        let bytes = image(domain, &tables);
        // Each table alone fits within three records; together they hold four.
        assert_eq!(
            inspect_bytes(&bytes, &with(|limits| limits.max_relocations = 3)),
            Err(exceeded(LimitKind::Relocations, 4, 3))
        );
        let metadata = dynamic(&bytes, &with(|limits| limits.max_relocations = 4));
        assert_eq!(metadata.counts().rela + metadata.counts().plt_rela, 4);
    }
}

#[test]
fn shared_name_offsets_are_charged_for_every_retained_copy() {
    let mut records = vec![null_symbol()];
    records.extend([symbol(1, 0x12, 0, 0, 0, 0); 4]);
    let tables = Tables {
        strings: Some(b"\0abcdefghij\0".to_vec()),
        symbols: Some(records.concat()),
        ..Tables::default()
    };
    let bytes = image(Domain::Dynlib, &tables);
    assert_eq!(
        inspect_bytes(&bytes, &with(|limits| limits.max_name_bytes = 39)),
        Err(LinkError::NameBudgetExceeded {
            symbol: 4,
            name_offset: 1,
            used: 30,
            maximum: 39
        })
    );
    let metadata = dynamic(&bytes, &with(|limits| limits.max_name_bytes = 40));
    assert_eq!(metadata.counts().name_bytes, 40);
}

#[test]
fn offsets_into_one_long_tail_are_bounded_by_the_name_budget() {
    let mut strings = vec![0];
    strings.extend([b'a'; 64]);
    strings.push(0);
    let mut records = vec![null_symbol()];
    records.extend((1..=64).map(|offset| symbol(offset, 0x12, 0, 0, 0, 0)));
    let tables = Tables {
        strings: Some(strings),
        symbols: Some(records.concat()),
        ..Tables::default()
    };
    let bytes = image(Domain::Guest, &tables);
    // The second lookup scans at most 37 bytes of its 63-byte name.
    assert_eq!(
        inspect_bytes(&bytes, &with(|limits| limits.max_name_bytes = 100)),
        Err(LinkError::NameBudgetExceeded {
            symbol: 2,
            name_offset: 2,
            used: 64,
            maximum: 100
        })
    );
    let metadata = dynamic(&bytes, &Limits::default());
    assert_eq!(metadata.counts().name_bytes, (1..=64).sum::<u64>());
    assert_eq!(
        metadata.symbols()[64].name.as_ref().unwrap().as_bytes(),
        b"a"
    );
}
