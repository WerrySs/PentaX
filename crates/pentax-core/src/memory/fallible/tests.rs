// SPDX-License-Identifier: GPL-2.0-or-later

use super::{FallibleWriteError, GuestMemory, MemoryError, PAGE_SIZE, Permissions, reserve_exact};
use crate::memory::ADDRESS_LIMIT;

const BASE: u64 = 0x4000;

#[derive(Debug, PartialEq, Eq)]
struct State {
    pages: Vec<(u64, Permissions, Option<Vec<u8>>)>,
    mapped: u64,
    resident: u64,
}

fn state(memory: &GuestMemory) -> State {
    State {
        pages: memory
            .pages
            .iter()
            .map(|(&base, page)| (base, page.permissions, page.data.clone()))
            .collect(),
        mapped: memory.mapped_bytes(),
        resident: memory.resident_bytes(),
    }
}

fn mixed_pages(resident_limit: u64) -> GuestMemory {
    let mut memory = GuestMemory::new(resident_limit, 3 * PAGE_SIZE);
    memory
        .map(BASE, 3 * PAGE_SIZE, Permissions::READ_WRITE)
        .unwrap();
    memory.write(BASE, &vec![0x5a; PAGE_SIZE as usize]).unwrap();
    memory
}

#[test]
fn each_reservation_failure_preserves_resident_and_lazy_pages_then_retry_commits() {
    let bytes: Vec<u8> = (0..PAGE_SIZE + 6)
        .map(|index| (index % 251) as u8)
        .collect();
    let address = BASE + PAGE_SIZE - 3;
    // Pending descriptors, first missing backing, second missing backing.
    for fail_at in 1..=3 {
        let mut memory = mixed_pages(3 * PAGE_SIZE);
        let original = state(&memory);
        let mut attempts = 0;
        let result = memory.write_with_reservation_gate(address, &bytes, || {
            attempts += 1;
            if attempts == fail_at {
                Err(FallibleWriteError::Allocation)
            } else {
                Ok(())
            }
        });
        assert_eq!(result, Err(FallibleWriteError::Allocation));
        assert_eq!(attempts, fail_at);
        assert_eq!(state(&memory), original);

        memory.write_fallible(address, &bytes).unwrap();
        let mut actual = vec![0; (3 * PAGE_SIZE) as usize];
        memory.read(BASE, &mut actual).unwrap();
        let mut expected = vec![0; actual.len()];
        expected[..PAGE_SIZE as usize].fill(0x5a);
        let first = PAGE_SIZE as usize - 3;
        expected[first..first + bytes.len()].copy_from_slice(&bytes);
        assert_eq!(actual, expected);
        assert_eq!(memory.resident_bytes(), 3 * PAGE_SIZE);
        assert_eq!(memory.mapped_bytes(), original.mapped);
        for page in memory.pages.values() {
            assert_eq!(page.permissions, Permissions::READ_WRITE);
            assert_eq!(page.data.as_ref().unwrap().len(), PAGE_SIZE as usize);
        }
    }
}

#[test]
fn existing_backing_needs_no_reservation_even_when_gate_always_refuses() {
    let mut memory = mixed_pages(3 * PAGE_SIZE);
    let mut attempts = 0;
    memory
        .write_with_reservation_gate(BASE + 7, &[1, 2, 3], || {
            attempts += 1;
            Err(FallibleWriteError::Allocation)
        })
        .unwrap();
    assert_eq!(attempts, 0);
    assert_eq!(memory.resident_bytes(), PAGE_SIZE);
    let mut actual = [0; 5];
    memory.read(BASE + 6, &mut actual).unwrap();
    assert_eq!(actual, [0x5a, 1, 2, 3, 0x5a]);
}

fn preflight_failure(mut memory: GuestMemory, address: u64, error: MemoryError) {
    let before = state(&memory);
    let mut attempts = 0;
    assert_eq!(
        memory.write_with_reservation_gate(address, &[7; 4], || {
            attempts += 1;
            Err(FallibleWriteError::Allocation)
        }),
        Err(FallibleWriteError::Memory(error))
    );
    assert_eq!(attempts, 0);
    assert_eq!(state(&memory), before);
}

#[test]
fn full_destination_faults_precede_any_reservation_or_write() {
    let mut unmapped = GuestMemory::new(2 * PAGE_SIZE, 2 * PAGE_SIZE);
    unmapped
        .map(BASE, PAGE_SIZE, Permissions::READ_WRITE)
        .unwrap();
    unmapped.write(BASE + PAGE_SIZE - 2, &[1, 2]).unwrap();
    preflight_failure(
        unmapped,
        BASE + PAGE_SIZE - 2,
        MemoryError::Unmapped(BASE + PAGE_SIZE),
    );

    let mut protected = mixed_pages(3 * PAGE_SIZE);
    protected
        .protect(BASE + PAGE_SIZE, PAGE_SIZE, Permissions::READ)
        .unwrap();
    preflight_failure(
        protected,
        BASE + PAGE_SIZE - 2,
        MemoryError::PermissionDenied(BASE + PAGE_SIZE),
    );

    preflight_failure(
        mixed_pages(PAGE_SIZE),
        BASE + PAGE_SIZE - 2,
        MemoryError::BudgetExceeded,
    );
    preflight_failure(
        mixed_pages(3 * PAGE_SIZE),
        u64::MAX - 1,
        MemoryError::InvalidRange,
    );
    preflight_failure(
        mixed_pages(3 * PAGE_SIZE),
        ADDRESS_LIMIT - 1,
        MemoryError::InvalidRange,
    );
}

#[test]
fn full_read_preflight_rejects_late_source_faults_without_output_changes() {
    for second in [None, Some(Permissions::WRITE)] {
        let mut memory = GuestMemory::new(2 * PAGE_SIZE, 2 * PAGE_SIZE);
        memory
            .map(BASE, PAGE_SIZE, Permissions::READ_WRITE)
            .unwrap();
        memory.write(BASE + PAGE_SIZE - 2, &[1, 2]).unwrap();
        if let Some(permissions) = second {
            memory
                .map(BASE + PAGE_SIZE, PAGE_SIZE, permissions)
                .unwrap();
        }
        let before = state(&memory);
        let error = if second.is_some() {
            MemoryError::PermissionDenied(BASE + PAGE_SIZE)
        } else {
            MemoryError::Unmapped(BASE + PAGE_SIZE)
        };
        assert_eq!(
            memory.preflight_read(BASE + PAGE_SIZE - 2, 4),
            Err(error.clone())
        );
        let mut output = [9; 4];
        assert_eq!(memory.read(BASE + PAGE_SIZE - 2, &mut output), Err(error));
        assert_eq!(output, [9; 4]);
        assert_eq!(state(&memory), before);
    }
}

#[test]
fn successful_span_preflights_never_materialize_lazy_backing() {
    let memory = mixed_pages(3 * PAGE_SIZE);
    let before = state(&memory);
    memory.preflight_read(BASE, 3 * PAGE_SIZE).unwrap();
    memory.preflight_write(BASE, 3 * PAGE_SIZE).unwrap();
    assert_eq!(state(&memory), before);
}

#[test]
fn exact_page_boundaries_and_budget_admit_only_touched_backing() {
    let mut memory = GuestMemory::new(PAGE_SIZE, 3 * PAGE_SIZE);
    memory
        .map(BASE, 3 * PAGE_SIZE, Permissions::READ_WRITE)
        .unwrap();
    memory
        .write_fallible(BASE + PAGE_SIZE, &vec![3; PAGE_SIZE as usize])
        .unwrap();
    assert_eq!(memory.resident_bytes(), PAGE_SIZE);
    assert!(memory.pages[&BASE].data.is_none());
    assert!(memory.pages[&(BASE + 2 * PAGE_SIZE)].data.is_none());
    let before = state(&memory);
    assert_eq!(
        memory.write_fallible(BASE + 2 * PAGE_SIZE - 1, &[1, 2]),
        Err(FallibleWriteError::Memory(MemoryError::BudgetExceeded))
    );
    assert_eq!(state(&memory), before);
}

#[test]
fn empty_accesses_keep_existing_address_independent_policy() {
    let mut memory = GuestMemory::new(0, 0);
    let mut attempts = 0;
    memory
        .write_with_reservation_gate(u64::MAX, &[], || {
            attempts += 1;
            Err(FallibleWriteError::Allocation)
        })
        .unwrap();
    memory.write_fallible(u64::MAX, &[]).unwrap();
    memory.write(u64::MAX, &[]).unwrap();
    memory.read(u64::MAX, &mut []).unwrap();
    memory.preflight_read(u64::MAX, 0).unwrap();
    memory.preflight_write(u64::MAX, 0).unwrap();
    assert_eq!(attempts, 0);
    assert_eq!(memory.resident_bytes(), 0);
    assert_eq!(memory.mapped_bytes(), 0);
    assert_eq!(
        memory.map(0, 0, Permissions::READ),
        Err(MemoryError::InvalidRange)
    );
    assert_eq!(
        memory.protect(0, 0, Permissions::READ),
        Err(MemoryError::InvalidRange)
    );
}

#[test]
fn capacity_overflow_maps_the_actual_reservation_error_without_oom() {
    let mut buffer = Vec::<u8>::new();
    let mut attempts = 0;
    let result = reserve_exact(&mut buffer, usize::MAX, &mut || {
        attempts += 1;
        Ok(())
    });
    assert_eq!(result, Err(FallibleWriteError::Allocation));
    assert_eq!(attempts, 1);
    assert!(buffer.is_empty());
    assert_eq!(buffer.capacity(), 0);
}
