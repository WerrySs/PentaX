// SPDX-License-Identifier: GPL-2.0-or-later

//! Original operation, span and budget tests over owned guest memory.

use super::*;
use crate::memory::Permissions;

const BASE: u64 = 0x10_0000;
const SECOND: u64 = BASE + PAGE_SIZE;
type F = KnownFunction;

fn memory(pages: u64) -> GuestMemory {
    let mut memory = GuestMemory::new(pages * PAGE_SIZE, pages * PAGE_SIZE);
    memory
        .map(BASE, pages * PAGE_SIZE, Permissions::READ_WRITE)
        .unwrap();
    memory
}

fn budget() -> ServiceBudget {
    ServiceBudget::new(MemoryServiceLimits::default())
}

fn call(
    budget: &mut ServiceBudget,
    memory: &mut GuestMemory,
    function: F,
    rdi: u64,
    rsi: u64,
    rdx: u64,
) -> Result<u64, MemoryServiceError> {
    let attempt = budget.begin_attempt()?;
    invoke(attempt, function, AbiArgs { rdi, rsi, rdx }, memory, budget).map(|result| result.rax)
}

fn bytes(memory: &GuestMemory, address: u64, size: usize) -> Vec<u8> {
    let mut output = vec![0; size];
    memory.read(address, &mut output).unwrap();
    output
}

fn pattern(size: usize, seed: u8) -> Vec<u8> {
    (0..size)
        .map(|index| (index as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

#[test]
fn builtin_budget_eligibility_is_pure_at_exact_limits_and_checked_overflows() {
    let snapshot = |budget: &ServiceBudget| {
        (
            budget.attempts,
            budget.total_bytes,
            budget.work,
            budget.calls,
            budget.string_reads,
            budget.string_writes,
            budget.string_scratch_peak,
            budget.reservations,
            budget.refuse_reservation,
        )
    };
    for function in [F::Memcpy, F::Memmove, F::Memset] {
        let exact_work = 19 + SITE_LOOKUP_WORK + work(function, 8).unwrap();
        for boundary in 0..7 {
            let mut limits = MemoryServiceLimits {
                max_attempts: 3,
                max_span_bytes: 8,
                max_scratch_bytes: 8,
                max_total_bytes: 21,
                max_work_units: exact_work,
                ..MemoryServiceLimits::default()
            };
            match boundary {
                1 => limits.max_attempts = 2,
                2 => limits.max_span_bytes = 7,
                3 => limits.max_scratch_bytes = 7,
                4 => limits.max_total_bytes = 20,
                5 => limits.max_work_units -= 1,
                6 => limits.max_work_units = 19 + SITE_LOOKUP_WORK - 1,
                _ => {}
            }
            let mut budget = ServiceBudget::new(limits);
            budget.attempts = 2;
            budget.total_bytes = 13;
            budget.work = 19;
            budget.calls[F::Memset.index()] = 1;
            budget.refuse_reservation = Some(0);
            let before = snapshot(&budget);
            for _ in 0..3 {
                assert_eq!(budget.can_admit_builtin(function, 8), boundary == 0);
                assert_eq!(snapshot(&budget), before);
            }
        }
    }
    let mut budget = ServiceBudget::new(MemoryServiceLimits {
        max_attempts: 1,
        max_span_bytes: 0,
        max_scratch_bytes: 0,
        max_total_bytes: 0,
        max_work_units: SITE_LOOKUP_WORK + 1,
        ..MemoryServiceLimits::default()
    });
    for function in [F::Memcpy, F::Memmove, F::Memset] {
        assert!(budget.can_admit_builtin(function, 0));
    }
    for function in [
        F::Memcmp,
        F::Strlen,
        F::CxaAtExit,
        F::HeapGetTraceInfo,
        F::Snprintf,
    ] {
        assert!(!budget.can_admit_builtin(function, 0));
    }
    budget.limits.max_work_units = SITE_LOOKUP_WORK;
    assert!(!budget.can_admit_builtin(F::Memset, 0)); // Zero length still has fixed operation work.
    budget.limits.max_attempts = u64::MAX;
    budget.limits.max_span_bytes = u64::MAX;
    budget.limits.max_scratch_bytes = u64::MAX;
    budget.limits.max_total_bytes = u64::MAX;
    budget.limits.max_work_units = u64::MAX;
    for boundary in 0..5 {
        budget.attempts = if boundary == 0 { u64::MAX } else { 0 };
        budget.total_bytes = if boundary == 1 { u64::MAX } else { 0 };
        budget.work = match boundary {
            2 => u64::MAX - SITE_LOOKUP_WORK + 1, // Site lookup addition overflows.
            3 => u64::MAX - SITE_LOOKUP_WORK,     // Site fits; fixed operation addition overflows.
            _ => 0,
        };
        let length = match boundary {
            1 => 1,
            4 => u64::MAX,
            _ => 0,
        };
        let before = snapshot(&budget);
        assert!(!budget.can_admit_builtin(F::Memset, length));
        assert_eq!(snapshot(&budget), before);
    }
}

#[test]
fn memcpy_copies_unaligned_cross_page_spans_and_returns_the_full_destination() {
    let mut memory = memory(3);
    let mut budget = budget();
    let source = BASE + 0x123;
    let destination = SECOND - 5;
    let input = pattern(100, 9);
    memory.write(source, &input).unwrap();
    let rax = call(
        &mut budget,
        &mut memory,
        F::Memcpy,
        destination,
        source,
        100,
    )
    .unwrap();
    assert_eq!(rax, destination);
    assert_eq!(bytes(&memory, destination, 100), input);
    assert_eq!(bytes(&memory, destination - 1, 1), [0]);
    assert_eq!(bytes(&memory, destination + 100, 1), [0]);
    assert_eq!(budget.calls(F::Memcpy), 1);
    assert_eq!(budget.total_bytes(), 100);
}

#[test]
fn memcpy_refuses_every_nonempty_overlap_but_allows_adjacent_spans() {
    let mut memory = memory(1);
    let input = pattern(64, 1);
    memory.write(BASE, &input).unwrap();
    for (destination, source) in [
        (BASE, BASE),
        (BASE + 8, BASE),
        (BASE, BASE + 8),
        (BASE + 31, BASE),
    ] {
        let mut budget = budget();
        assert_eq!(
            call(&mut budget, &mut memory, F::Memcpy, destination, source, 32),
            Err(MemoryServiceError::UnsupportedOverlap)
        );
        assert_eq!(budget.completed(), 0);
        assert_eq!(bytes(&memory, BASE, 64), input);
    }
    let mut budget = budget();
    call(&mut budget, &mut memory, F::Memcpy, BASE + 32, BASE, 32).unwrap();
    assert_eq!(bytes(&memory, BASE + 32, 32), &input[..32]);
}

#[test]
fn memmove_snapshots_the_source_in_both_overlap_directions_and_identical_spans() {
    let input = pattern(48, 3);
    for (destination, source) in [(BASE + 8, BASE), (BASE, BASE + 8), (BASE, BASE)] {
        let mut memory = memory(1);
        memory.write(BASE, &input).unwrap();
        let mut expected = input.clone();
        let (d, s) = ((destination - BASE) as usize, (source - BASE) as usize);
        expected.copy_within(s..s + 40, d);
        let rax = call(
            &mut budget(),
            &mut memory,
            F::Memmove,
            destination,
            source,
            40,
        )
        .unwrap();
        assert_eq!(rax, destination);
        assert_eq!(bytes(&memory, BASE, 48), expected);
    }
}

#[test]
fn memset_fills_only_the_low_int_byte_and_ignores_high_register_bits() {
    let mut memory = memory(2);
    let mut budget = budget();
    let destination = SECOND - 3;
    let rax = call(
        &mut budget,
        &mut memory,
        F::Memset,
        destination,
        0xffff_ffff_0000_01ab,
        7,
    )
    .unwrap();
    assert_eq!(rax, destination);
    let mut expected = [0xab; 9];
    expected[0] = 0;
    expected[8] = 0;
    assert_eq!(bytes(&memory, destination - 1, 9), expected);
    memory.write(BASE, &[9; 4]).unwrap();
    // Bit 32 and bit 8 are outside the converted unsigned char.
    call(&mut budget, &mut memory, F::Memset, BASE, 0x1_0000_0100, 4).unwrap();
    assert_eq!(bytes(&memory, BASE, 4), [0; 4]);
    assert_eq!(budget.calls(F::Memset), 2);
}

#[test]
fn memcmp_orders_unsigned_bytes_with_a_deterministic_zero_extended_result() {
    let mut memory = memory(1);
    memory.write(BASE, &[1, 2, 0x01, 9]).unwrap();
    memory.write(BASE + 0x100, &[1, 2, 0x80, 0]).unwrap();
    let mut budget = budget();
    let less = call(&mut budget, &mut memory, F::Memcmp, BASE, BASE + 0x100, 4).unwrap();
    let greater = call(&mut budget, &mut memory, F::Memcmp, BASE + 0x100, BASE, 4).unwrap();
    let equal = call(&mut budget, &mut memory, F::Memcmp, BASE, BASE + 0x100, 2).unwrap();
    // Signed char ordering would invert the first two results.
    assert_eq!(less, 0xffff_ffff);
    assert_eq!(less as u32 as i32, -1);
    assert_eq!(greater, 1);
    assert_eq!(equal, 0);
    let empty = call(&mut budget, &mut memory, F::Memcmp, 0, 0, 0).unwrap();
    assert_eq!(empty, 0);
    assert_eq!(budget.calls(F::Memcmp), 4);
}

#[test]
fn zero_length_accepts_any_pointer_below_the_limit_without_mapping() {
    let mut memory = memory(1);
    let mut budget = budget();
    for pointer in [0, 1, ADDRESS_LIMIT - 1] {
        for function in F::MEMORY {
            let rax = call(&mut budget, &mut memory, function, pointer, pointer, 0).unwrap();
            let expected = if function == F::Memcmp { 0 } else { pointer };
            assert_eq!(rax, expected);
        }
    }
    for (rdi, rsi, argument) in [
        (ADDRESS_LIMIT, 0, PointerArgument::First),
        (0, ADDRESS_LIMIT, PointerArgument::Second),
        (u64::MAX, 0, PointerArgument::First),
    ] {
        assert_eq!(
            call(&mut budget, &mut memory, F::Memcpy, rdi, rsi, 0),
            Err(MemoryServiceError::AddressOverflow { argument })
        );
    }
    // Memset's RSI is a value, not a pointer.
    call(&mut budget, &mut memory, F::Memset, 0, u64::MAX, 0).unwrap();
    // Each zero-length call still costs an attempt and its fixed work.
    assert_eq!(budget.attempts(), 16);
    assert_eq!(budget.total_bytes(), 0);
    assert_eq!(budget.work(), 16 * SITE_LOOKUP_WORK + 16);
    assert_eq!(budget.completed(), 13);
}

#[test]
fn nonempty_spans_reject_null_and_overflowing_ends() {
    let mut memory = memory(1);
    let mut budget = budget();
    for (function, rdi, rsi, expected) in [
        (
            F::Memcpy,
            0,
            BASE,
            MemoryServiceError::NullPointer {
                argument: PointerArgument::First,
            },
        ),
        (
            F::Memmove,
            BASE,
            0,
            MemoryServiceError::NullPointer {
                argument: PointerArgument::Second,
            },
        ),
        (
            F::Memset,
            0,
            1,
            MemoryServiceError::NullPointer {
                argument: PointerArgument::First,
            },
        ),
        (
            F::Memcmp,
            BASE,
            0,
            MemoryServiceError::NullPointer {
                argument: PointerArgument::Second,
            },
        ),
        (
            F::Memcpy,
            ADDRESS_LIMIT - 1,
            BASE,
            MemoryServiceError::AddressOverflow {
                argument: PointerArgument::First,
            },
        ),
        (
            F::Memcmp,
            BASE,
            u64::MAX,
            MemoryServiceError::AddressOverflow {
                argument: PointerArgument::Second,
            },
        ),
    ] {
        assert_eq!(
            call(&mut budget, &mut memory, function, rdi, rsi, 2),
            Err(expected)
        );
    }
    assert_eq!(budget.completed(), 0);
    // A span ending exactly at the limit is arithmetically valid but unmapped.
    assert!(matches!(
        call(&mut budget, &mut memory, F::Memset, ADDRESS_LIMIT - 2, 0, 2),
        Err(MemoryServiceError::Memory(MemoryError::Unmapped(_)))
    ));
}

#[test]
fn invalid_final_pages_of_either_span_leave_destination_bytes_and_residency_unchanged() {
    const THIRD: u64 = SECOND + PAGE_SIZE;
    for case in 0..4 {
        let mut memory = GuestMemory::new(4 * PAGE_SIZE, 4 * PAGE_SIZE);
        memory
            .map(BASE, 2 * PAGE_SIZE, Permissions::READ_WRITE)
            .unwrap();
        let resident_span = BASE + 0x100;
        memory.write(resident_span, &[7; 16]).unwrap();
        let resident = memory.resident_bytes();
        // Every span starts in mapped pages and crosses into THIRD. Lazy
        // pages before THIRD must not become resident.
        let (function, rdi, rsi, error) = match case {
            0 => (
                F::Memcpy,
                resident_span,
                THIRD - 4,
                MemoryError::Unmapped(THIRD),
            ),
            1 => (
                F::Memmove,
                resident_span,
                THIRD - 4,
                MemoryError::Unmapped(THIRD),
            ),
            2 => {
                memory.map(THIRD, PAGE_SIZE, Permissions::READ).unwrap();
                (
                    F::Memcpy,
                    THIRD - 4,
                    resident_span,
                    MemoryError::PermissionDenied(THIRD),
                )
            }
            _ => {
                memory.map(THIRD, PAGE_SIZE, Permissions::READ).unwrap();
                (
                    F::Memset,
                    THIRD - 4,
                    0x55,
                    MemoryError::PermissionDenied(THIRD),
                )
            }
        };
        let mut budget = budget();
        assert_eq!(
            call(&mut budget, &mut memory, function, rdi, rsi, 8),
            Err(MemoryServiceError::Memory(error))
        );
        assert_eq!(bytes(&memory, resident_span, 16), [7; 16]);
        assert_eq!(bytes(&memory, THIRD - 4, 4), [0; 4]);
        assert_eq!(memory.resident_bytes(), resident);
        assert_eq!(budget.completed(), 0);
    }
}

#[test]
fn memcmp_full_span_validation_wins_over_an_early_difference() {
    let mut memory = memory(1);
    memory.write(BASE, &[1]).unwrap();
    memory.write(BASE + PAGE_SIZE - 16, &[2]).unwrap();
    let mut budget = budget();
    // The first bytes differ, but the second span's final page is unmapped.
    assert_eq!(
        call(
            &mut budget,
            &mut memory,
            F::Memcmp,
            BASE,
            BASE + PAGE_SIZE - 16,
            32
        ),
        Err(MemoryServiceError::Memory(MemoryError::Unmapped(
            BASE + PAGE_SIZE
        )))
    );
    assert_eq!(budget.completed(), 0);
    // Work is charged for both complete spans, not up to the difference.
    assert_eq!(budget.work(), SITE_LOOKUP_WORK + 5 * 32 + 1);
}

#[test]
fn destination_resident_budget_is_preflighted_before_scratch_or_writes() {
    let mut memory = GuestMemory::new(PAGE_SIZE, 2 * PAGE_SIZE);
    memory
        .map(BASE, 2 * PAGE_SIZE, Permissions::READ_WRITE)
        .unwrap();
    memory.write(BASE, &[4; 8]).unwrap();
    let mut budget = budget();
    budget.refuse_reservation = Some(0);
    // One more lazy page would exceed residency; this fails before scratch.
    assert_eq!(
        call(&mut budget, &mut memory, F::Memset, SECOND - 1, 9, 2),
        Err(MemoryServiceError::Memory(MemoryError::BudgetExceeded))
    );
    assert_eq!(memory.resident_bytes(), PAGE_SIZE);
    assert_eq!(bytes(&memory, SECOND - 1, 2), [0, 0]);
    assert_eq!(budget.reservations, 0);
}

#[test]
fn injected_scratch_reservation_failures_change_no_guest_state() {
    for (function, refuse) in [
        (F::Memcpy, 0),
        (F::Memmove, 0),
        (F::Memset, 0),
        (F::Memcmp, 0),
        (F::Memcmp, 1),
    ] {
        let mut memory = memory(2);
        memory.write(BASE, &pattern(32, 5)).unwrap();
        memory.write(SECOND - 8, &[6; 8]).unwrap();
        let resident = memory.resident_bytes();
        let mut budget = budget();
        budget.refuse_reservation = Some(refuse);
        if function != F::Memcmp {
            // Eligibility predicts quotas, not a successful allocation. A
            // genuine provider/helper error still retains its admitted charges.
            assert!(budget.can_admit_builtin(function, 32));
        }
        assert_eq!(
            call(&mut budget, &mut memory, function, SECOND - 8, BASE, 32),
            Err(MemoryServiceError::Allocation)
        );
        assert_eq!(bytes(&memory, SECOND - 8, 8), [6; 8]);
        assert_eq!(bytes(&memory, SECOND, 24), [0; 24]);
        assert_eq!(memory.resident_bytes(), resident);
        assert_eq!(budget.completed(), 0);
        assert_eq!(budget.reservations, refuse + 1);
        assert_eq!(budget.attempts(), 1);
        assert_eq!(budget.total_bytes(), 32);
        assert_eq!(
            budget.work(),
            SITE_LOOKUP_WORK + work(function, 32).unwrap()
        );
        budget.refuse_reservation = None;
        call(&mut budget, &mut memory, function, SECOND - 8, BASE, 32).unwrap();
    }
}

#[test]
fn span_scratch_attempt_byte_and_work_caps_are_inclusive_and_retained() {
    let limits = MemoryServiceLimits {
        max_span_bytes: 16,
        max_scratch_bytes: 24,
        max_attempts: 5,
        max_total_bytes: 40,
        max_work_units: u64::MAX,
        max_exit_callbacks: 4096,
    };
    let mut memory = memory(1);
    let mut budget = ServiceBudget::new(limits);
    assert!(budget.can_admit_builtin(F::Memset, 16));
    call(&mut budget, &mut memory, F::Memset, BASE, 1, 16).unwrap();
    let first_work = SITE_LOOKUP_WORK + work(F::Memset, 16).unwrap();
    assert!(!budget.can_admit_builtin(F::Memset, 17));
    assert_eq!(
        call(&mut budget, &mut memory, F::Memset, BASE, 1, 17),
        Err(MemoryServiceError::LimitExceeded {
            kind: ServiceLimit::SpanBytes,
            requested: 17,
            maximum: 16,
        })
    );
    assert_eq!(
        (budget.attempts(), budget.total_bytes(), budget.work()),
        (2, 16, first_work + SITE_LOOKUP_WORK)
    );
    // Memcmp holds two snapshots: 2 * 13 exceeds the scratch cap.
    assert_eq!(
        call(&mut budget, &mut memory, F::Memcmp, BASE, BASE + 64, 13),
        Err(MemoryServiceError::LimitExceeded {
            kind: ServiceLimit::ScratchBytes,
            requested: 26,
            maximum: 24,
        })
    );
    call(&mut budget, &mut memory, F::Memcmp, BASE, BASE + 64, 12).unwrap();
    assert_eq!(budget.total_bytes(), 28);
    let before_refusal = budget.work();
    assert!(!budget.can_admit_builtin(F::Memcpy, 13));
    assert_eq!(
        call(&mut budget, &mut memory, F::Memcpy, BASE + 32, BASE, 13),
        Err(MemoryServiceError::LimitExceeded {
            kind: ServiceLimit::TotalBytes,
            requested: 41,
            maximum: 40,
        })
    );
    assert_eq!(budget.total_bytes(), 28);
    assert_eq!(budget.work(), before_refusal + SITE_LOOKUP_WORK);
    assert_eq!(budget.attempts(), 5);
    assert!(!budget.can_admit_builtin(F::Memcpy, 0));
    assert_eq!(
        call(&mut budget, &mut memory, F::Memcpy, BASE + 32, BASE, 0),
        Err(MemoryServiceError::LimitExceeded {
            kind: ServiceLimit::Attempts,
            requested: 6,
            maximum: 5,
        })
    );
    // Rejected attempts were charged; the counters never restart.
    assert_eq!(budget.attempts(), 5);
    assert_eq!(budget.work(), before_refusal + SITE_LOOKUP_WORK);
    assert_eq!(budget.completed(), 2);
}

#[test]
fn work_is_precharged_conservatively_and_failures_do_not_reset_it() {
    assert_eq!(work(F::Memset, 0), Some(1));
    assert_eq!(work(F::Memset, 1), Some(2 + PAGE_SIZE + 1));
    assert_eq!(work(F::Memcpy, 2), Some(6 + 2 * PAGE_SIZE + 1));
    assert_eq!(
        work(F::Memmove, PAGE_SIZE),
        Some(3 * PAGE_SIZE + 2 * PAGE_SIZE + 1)
    );
    assert_eq!(
        work(F::Memset, PAGE_SIZE + 2),
        Some(2 * PAGE_SIZE + 4 + 3 * PAGE_SIZE + 1)
    );
    assert_eq!(work(F::Memcmp, 10), Some(51));
    assert_eq!(work(F::Memcpy, u64::MAX), None);

    let exact = SITE_LOOKUP_WORK + work(F::Memset, 8).unwrap();
    let mut memory = memory(1);
    let mut budget = ServiceBudget::new(MemoryServiceLimits {
        max_work_units: exact,
        ..MemoryServiceLimits::default()
    });
    assert!(budget.can_admit_builtin(F::Memset, 8));
    call(&mut budget, &mut memory, F::Memset, BASE, 1, 8).unwrap();
    assert_eq!(budget.work(), exact);
    assert!(!budget.can_admit_builtin(F::Memset, 0));
    assert_eq!(
        budget.begin_attempt().map(|_| ()),
        Err(MemoryServiceError::LimitExceeded {
            kind: ServiceLimit::WorkUnits,
            requested: exact + SITE_LOOKUP_WORK,
            maximum: exact,
        })
    );
    assert_eq!(budget.work(), exact);

    let mut budget = ServiceBudget::new(MemoryServiceLimits {
        max_span_bytes: u64::MAX,
        max_scratch_bytes: u64::MAX,
        max_total_bytes: u64::MAX,
        max_work_units: u64::MAX,
        ..MemoryServiceLimits::default()
    });
    assert!(!budget.can_admit_builtin(F::Memcpy, u64::MAX));
    assert_eq!(
        call(
            &mut budget,
            &mut memory,
            F::Memcpy,
            BASE,
            BASE + 8,
            u64::MAX
        ),
        Err(MemoryServiceError::CounterOverflow {
            kind: ServiceLimit::WorkUnits,
        })
    );
    assert_eq!(
        (budget.attempts(), budget.total_bytes(), budget.work()),
        (1, 0, SITE_LOOKUP_WORK)
    );
    assert_eq!(
        call(
            &mut budget,
            &mut memory,
            F::Memcmp,
            BASE,
            BASE + 8,
            u64::MAX
        ),
        Err(MemoryServiceError::CounterOverflow {
            kind: ServiceLimit::ScratchBytes,
        })
    );
    assert_eq!(
        (budget.attempts(), budget.total_bytes(), budget.work()),
        (2, 0, 2 * SITE_LOOKUP_WORK)
    );
}
