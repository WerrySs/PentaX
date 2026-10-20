// SPDX-License-Identifier: GPL-2.0-or-later

use super::{CONTEXT, PAD, bytes, committed, run, run_with, stopped, terminal, write};
use crate::{
    InspectError, LimitKind, Limits, RECORD_STORAGE_BYTES as SLOT, StopCategory, StopReason,
    Storage, inspect, limits::reserve_exact, report::Record,
};

fn with(change: impl FnOnce(&mut Limits)) -> Limits {
    let mut limits = Limits::default();
    change(&mut limits);
    limits
}

fn resource(limit: LimitKind, requested: u64, maximum: u64) -> StopReason {
    StopReason::Resource {
        limit,
        requested,
        maximum,
    }
}

fn pads(count: usize) -> Vec<u32> {
    vec![PAD; count]
}

#[test]
fn unaligned_lengths_are_rejected_before_limits_and_ownership() {
    let zero = Limits {
        max_input_bytes: 0,
        max_records: 0,
        max_record_bytes: 0,
        max_visited_dwords: 0,
    };
    for length in [1, 2, 3, 5, 7, 4097] {
        let input = vec![0x80_u8; length];
        let expected = Err(InspectError::UnalignedLength {
            bytes: u64::try_from(length).unwrap(),
        });
        assert_eq!(inspect(&input, &Limits::default()), expected);
        assert_eq!(inspect(&input, &zero), expected);
    }
}

#[test]
fn input_bytes_limit_is_inclusive_and_checked_before_ownership() {
    let input = bytes(&pads(2));
    let exact = inspect(&input, &with(|l| l.max_input_bytes = 8)).unwrap();
    assert!(exact.is_complete());
    assert_eq!(exact.accounting().input_capacity_bytes(), 8);

    let over = Err(InspectError::LimitExceeded {
        limit: LimitKind::InputBytes,
        requested: 8,
        maximum: 7,
    });
    assert_eq!(inspect(&input, &with(|l| l.max_input_bytes = 7)), over);

    let zero = with(|l| l.max_input_bytes = 0);
    assert!(inspect(&[], &zero).unwrap().is_complete());
    assert_eq!(
        inspect(&input[..4], &zero),
        Err(InspectError::LimitExceeded {
            limit: LimitKind::InputBytes,
            requested: 4,
            maximum: 0,
        })
    );
}

#[test]
fn visited_budget_covers_long_padding_streams() {
    let words = pads(1000);
    assert!(run_with(&words, &with(|l| l.max_visited_dwords = 1000)).is_complete());

    let short = run_with(&words, &with(|l| l.max_visited_dwords = 999));
    assert_eq!(
        stopped(&short),
        (999, resource(LimitKind::VisitedDwords, 1000, 999))
    );
    assert_eq!(
        (short.committed_packets(), short.visited_dwords()),
        (999, 999)
    );

    let zero = run_with(&words, &with(|l| l.max_visited_dwords = 0));
    assert_eq!(
        stopped(&zero),
        (0, resource(LimitKind::VisitedDwords, 1, 0))
    );
    assert_eq!(zero.committed_packets(), 0);
    assert!(run_with(&[], &with(|l| l.max_visited_dwords = 0)).is_complete());
}

#[test]
fn visited_budget_charges_the_frame_before_body_checks() {
    let words = [vec![PAD], write(CONTEXT, 0, 0x10, &[1, 2])].concat();
    assert!(run_with(&words, &with(|l| l.max_visited_dwords = 5)).is_complete());

    let short = run_with(&words, &with(|l| l.max_visited_dwords = 4));
    let (at, reason) = stopped(&short);
    assert_eq!((at, reason), (1, resource(LimitKind::VisitedDwords, 5, 4)));
    assert_eq!(reason.category(), StopCategory::Resource);
    assert_eq!(reason.header(), None);
    // Only the header was charged; the prefix is untouched.
    assert_eq!(short.visited_dwords(), 2);
    assert_eq!(short.committed_packets(), 1);
}

#[test]
fn record_count_limit_stops_long_padding_streams() {
    let words = pads(1000);
    assert!(run_with(&words, &with(|l| l.max_records = 1000)).is_complete());

    let short = run_with(&words, &with(|l| l.max_records = 999));
    assert_eq!(
        stopped(&short),
        (999, resource(LimitKind::Records, 1000, 999))
    );
    assert_eq!(short.accounting().record_capacity(), 999);
    // The stopped packet was examined before the commit check.
    assert_eq!(short.visited_dwords(), 1000);

    let zero = run_with(&words, &with(|l| l.max_records = 0));
    assert_eq!(stopped(&zero), (0, resource(LimitKind::Records, 1, 0)));
    assert_eq!(zero.accounting().record_capacity_bytes(), 0);
}

#[test]
fn record_limit_is_checked_after_a_valid_packet_is_decoded() {
    let words = [write(CONTEXT, 0, 0x10, &[1]), vec![PAD]].concat();
    let short = run_with(&words, &with(|l| l.max_records = 1));
    assert_eq!(stopped(&short), (3, resource(LimitKind::Records, 2, 1)));
    assert_eq!(short.committed_dwords(), 3);
}

#[test]
fn record_storage_bytes_are_charged_per_reserved_slot() {
    let words = pads(1000);
    let exact = run_with(&words, &with(|l| l.max_record_bytes = 1000 * SLOT));
    assert!(exact.is_complete());
    assert_eq!(exact.accounting().record_capacity_bytes(), 1000 * SLOT);

    let short = run_with(&words, &with(|l| l.max_record_bytes = 1000 * SLOT - 1));
    assert_eq!(
        stopped(&short),
        (
            999,
            resource(LimitKind::RecordBytes, 1000 * SLOT, 1000 * SLOT - 1)
        )
    );
    assert_eq!(short.accounting().record_capacity_bytes(), 999 * SLOT);

    for tiny in [0, SLOT - 1] {
        let inspection = run_with(&words, &with(|l| l.max_record_bytes = tiny));
        assert_eq!(
            stopped(&inspection),
            (0, resource(LimitKind::RecordBytes, SLOT, tiny))
        );
    }
}

#[test]
fn record_reservation_is_the_smallest_bound() {
    let words = pads(10);
    let capacity = |limits: Limits| run_with(&words, &limits).accounting();

    let by_input = capacity(Limits::default());
    assert_eq!(by_input.record_capacity(), 10);
    assert_eq!(by_input.record_capacity_bytes(), 10 * SLOT);
    assert_eq!(by_input.owned_capacity_bytes(), 40 + 10 * SLOT);

    assert_eq!(capacity(with(|l| l.max_records = 3)).record_capacity(), 3);
    let by_bytes = capacity(with(|l| l.max_record_bytes = 5 * SLOT + 1));
    assert_eq!(by_bytes.record_capacity(), 5);
    assert_eq!(by_bytes.record_capacity_bytes(), 5 * SLOT);

    let unbounded = Limits {
        max_input_bytes: u64::MAX,
        max_records: u64::MAX,
        max_record_bytes: u64::MAX,
        max_visited_dwords: u64::MAX,
    };
    assert_eq!(capacity(unbounded).record_capacity(), 10);
}

#[test]
fn allocation_failure_is_typed_and_returns_no_storage() {
    // The global allocator cannot be made to fail here; an unrepresentable
    // exact request exercises the same fallible path and its typed mapping.
    assert!(matches!(
        reserve_exact::<Record>(Storage::Records, usize::MAX),
        Err(InspectError::AllocationFailed {
            storage: Storage::Records,
            ..
        })
    ));
    assert!(matches!(
        reserve_exact::<u8>(Storage::Input, usize::MAX),
        Err(InspectError::AllocationFailed {
            storage: Storage::Input,
            ..
        })
    ));
    let reserved = reserve_exact::<Record>(Storage::Records, 3).unwrap();
    assert!(reserved.is_empty() && reserved.capacity() >= 3);
}

#[test]
fn resource_stops_keep_the_prefix_like_other_stops() {
    let words = [write(CONTEXT, 0, 1, &[2]), pads(3)].concat();
    let full = run_with(&words, &with(|l| l.max_visited_dwords = 4));
    assert_eq!(stopped(&full).0, 4);
    // The same prefix is committed when the fifth dword is rejected for syntax.
    let mut syntax = words[..4].to_vec();
    syntax.push(0);
    terminal(&syntax, 4);
    assert_eq!(committed(&full), committed(&run(&syntax)));
    assert_eq!(full.committed_packets(), 2);
}
