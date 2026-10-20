// SPDX-License-Identifier: GPL-2.0-only

//! Original calls through the production dispatcher and process-owned budget.
use super::*;

fn register(budget: &mut ServiceBudget, words: [u64; 3]) -> Result<u64, MemoryServiceError> {
    let mut unmapped = GuestMemory::new(0, 0);
    let attempt = budget.begin_attempt()?;
    let result = invoke(
        attempt,
        KnownFunction::CxaAtExit,
        AbiArgs {
            rdi: words[0],
            rsi: words[1],
            rdx: words[2],
        },
        &mut unmapped,
        budget,
    );
    assert_eq!(unmapped.resident_bytes(), 0);
    result.map(|result| result.rax)
}

#[test]
fn ordered_opaque_words_nulls_and_duplicates_are_retained_without_guest_memory_access() {
    let mut budget = ServiceBudget::new(MemoryServiceLimits {
        max_span_bytes: 0,
        max_scratch_bytes: 0,
        max_total_bytes: 0,
        max_exit_callbacks: 4,
        ..Default::default()
    });
    let words = [
        [0, 0, 0],
        [u64::MAX, 0x8000_0000_0000_0001, 0xfedc_ba98_7654_3210],
        [u64::MAX, 0x8000_0000_0000_0001, 0xfedc_ba98_7654_3210],
        [0, u64::MAX, u64::MAX],
    ];
    for entry in words {
        assert_eq!(register(&mut budget, entry), Ok(0));
    }
    assert_eq!(budget.exit_callbacks.entries, words);
    assert_eq!(budget.calls(KnownFunction::CxaAtExit), 4);
    assert_eq!(budget.attempts(), 4);
    assert_eq!(budget.total_bytes(), 0);
    assert_eq!(budget.string_reads(), 0);
    assert_eq!(budget.string_writes(), 0);
    assert!(budget.exit_callbacks.entries.capacity() <= 4);
    assert!(!format!("{budget:?}").contains("18446744073709551615"));

    // Moving a live budget into its actual provider context retains records;
    // a new process budget has independent empty storage and counters.
    let budget = budget.with_provider(0x100_0000);
    assert_eq!(budget.exit_callbacks.entries, words);
    let mut fresh = ServiceBudget::new(Default::default());
    assert!(fresh.exit_callbacks.entries.is_empty());
    assert_eq!(fresh.calls(KnownFunction::CxaAtExit), 0);
    assert_eq!(register(&mut fresh, [7, 8, 9]), Ok(0));
    assert_eq!(fresh.exit_callbacks.entries, [[7, 8, 9]]);
    assert_eq!(budget.exit_callbacks.entries, words);
}

#[test]
fn zero_and_inclusive_registration_caps_refuse_without_partial_records_or_completion() {
    for maximum in [0, 2] {
        let mut budget = ServiceBudget::new(MemoryServiceLimits {
            max_exit_callbacks: maximum,
            ..Default::default()
        });
        for index in 0..maximum {
            assert_eq!(register(&mut budget, [index, 2, 3]), Ok(0));
        }
        let before = budget.exit_callbacks.entries.clone();
        let work = budget.work();
        assert_eq!(
            register(&mut budget, [4, 5, 6]),
            Err(MemoryServiceError::LimitExceeded {
                kind: ServiceLimit::ExitCallbacks,
                requested: maximum + 1,
                maximum,
            })
        );
        assert_eq!(budget.exit_callbacks.entries, before);
        assert_eq!(budget.completed(), maximum);
        assert_eq!(budget.attempts(), maximum + 1);
        assert_eq!(budget.work(), work + SITE_LOOKUP_WORK + REGISTER_WORK);
    }
}

#[test]
fn allocation_refusal_keeps_old_records_and_capacity_and_retry_appends_exactly_once() {
    let mut budget = ServiceBudget::new(MemoryServiceLimits {
        max_exit_callbacks: 64,
        ..Default::default()
    });
    budget.refuse_reservation = Some(0);
    assert_eq!(
        register(&mut budget, [1, 2, 3]),
        Err(MemoryServiceError::Allocation)
    );
    assert!(budget.exit_callbacks.entries.is_empty());
    assert_eq!(budget.exit_callbacks.entries.capacity(), 0);
    assert_eq!(budget.completed(), 0);
    assert_eq!(budget.attempts(), 1);
    budget.refuse_reservation = None;
    assert_eq!(register(&mut budget, [1, 2, 3]), Ok(0));
    let capacity = budget.exit_callbacks.entries.capacity();
    assert!(capacity < 64);
    while budget.exit_callbacks.entries.len() < capacity {
        assert_eq!(register(&mut budget, [4, 5, 6]), Ok(0));
    }
    let before = budget.exit_callbacks.entries.clone();
    let completed = budget.completed();
    let attempts = budget.attempts();
    let work = budget.work();
    budget.refuse_reservation = Some(budget.reservations);
    assert_eq!(
        register(&mut budget, [7, 8, 9]),
        Err(MemoryServiceError::Allocation)
    );
    assert_eq!(budget.exit_callbacks.entries, before);
    assert_eq!(budget.exit_callbacks.entries.capacity(), capacity);
    assert_eq!(budget.completed(), completed);
    assert_eq!(budget.attempts(), attempts + 1);
    assert!(budget.work() > work);
    budget.refuse_reservation = None;
    assert_eq!(register(&mut budget, [7, 8, 9]), Ok(0));
    assert_eq!(&budget.exit_callbacks.entries[..before.len()], &before);
    assert_eq!(budget.exit_callbacks.entries.last(), Some(&[7, 8, 9]));
    assert_eq!(budget.completed(), completed + 1);
}

#[test]
fn work_limits_are_inclusive_and_retained_byte_overflow_refuses_before_allocation() {
    let exact = SITE_LOOKUP_WORK + REGISTER_WORK + 24;
    for maximum in [SITE_LOOKUP_WORK + REGISTER_WORK - 1, exact - 1, exact] {
        let mut budget = ServiceBudget::new(MemoryServiceLimits {
            max_work_units: maximum,
            ..Default::default()
        });
        let result = register(&mut budget, [u64::MAX; 3]);
        if maximum == exact {
            assert_eq!(result, Ok(0));
            assert_eq!(budget.work(), exact);
            assert_eq!(budget.exit_callbacks.entries, [[u64::MAX; 3]]);
        } else {
            assert!(matches!(
                result,
                Err(MemoryServiceError::LimitExceeded {
                    kind: ServiceLimit::WorkUnits,
                    ..
                })
            ));
            assert!(budget.exit_callbacks.entries.is_empty());
            assert_eq!(budget.exit_callbacks.entries.capacity(), 0);
            assert_eq!(budget.completed(), 0);
            assert_eq!(budget.attempts(), 1);
            assert_eq!(budget.reservations, 0);
            assert!(budget.work() >= SITE_LOOKUP_WORK);
        }
    }
    let mut budget = ServiceBudget::new(MemoryServiceLimits {
        max_exit_callbacks: u64::MAX,
        ..Default::default()
    });
    assert_eq!(
        register(&mut budget, [1, 2, 3]),
        Err(MemoryServiceError::CounterOverflow {
            kind: ServiceLimit::ExitCallbacks,
        })
    );
    assert!(budget.exit_callbacks.entries.is_empty());
    assert_eq!(budget.reservations, 0);
    assert_eq!(budget.completed(), 0);
}

#[test]
fn exhausted_attempt_budget_cannot_publish_another_registration() {
    let mut budget = ServiceBudget::new(MemoryServiceLimits {
        max_attempts: 1,
        ..Default::default()
    });
    assert_eq!(register(&mut budget, [1, 2, 3]), Ok(0));
    let work = budget.work();
    assert_eq!(
        register(&mut budget, [4, 5, 6]),
        Err(MemoryServiceError::LimitExceeded {
            kind: ServiceLimit::Attempts,
            requested: 2,
            maximum: 1,
        })
    );
    assert_eq!(budget.exit_callbacks.entries, [[1, 2, 3]]);
    assert_eq!(budget.calls(KnownFunction::CxaAtExit), 1);
    assert_eq!(budget.attempts(), 1);
    assert_eq!(budget.work(), work);
}
