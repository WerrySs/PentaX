// SPDX-License-Identifier: GPL-2.0-only
// Original bounded guest-arena fixtures; no host allocator pointers or games.

use super::*;
use crate::tests::{ARG, TestGuest};
use crate::{Limits, Service::*};

const BASE: u64 = ARG + 0x400;
const OUT: u64 = ARG + 0x80;
type ArenaSnapshot = (u64, u64, u64, u64, Vec<Chunk>, usize);

fn fixture() -> (MspaceState, Budget, TestGuest) {
    let mut guest = TestGuest::new();
    guest.put(0, &[0xa7; PAGE as usize]);
    (
        MspaceState::default(),
        Budget::new(Limits::default()).unwrap(),
        guest,
    )
}

fn call(
    state: &mut MspaceState,
    budget: &mut Budget,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<MspaceResult, Failure> {
    let mut a = [0; 6];
    a[..values.len()].copy_from_slice(values);
    state.invoke(service, CallArgs(a), guest, budget)
}

fn value(result: Result<MspaceResult, Failure>) -> u64 {
    let MspaceResult::Value(value) = result.unwrap() else {
        panic!("expected ordinary heap result");
    };
    value
}

fn create(state: &mut MspaceState, budget: &mut Budget, guest: &mut TestGuest, size: u64) {
    assert_eq!(
        call(
            state,
            budget,
            guest,
            MspaceCreate,
            &[u64::MAX, BASE, size, 0]
        ),
        Ok(MspaceResult::Value(BASE))
    );
}

fn alloc(state: &mut MspaceState, budget: &mut Budget, guest: &mut TestGuest, size: u64) -> u64 {
    value(call(state, budget, guest, MspaceMalloc, &[BASE, size]))
}

fn census(state: &MspaceState, budget: &Budget) {
    assert_eq!(state.retained_handle_count(), budget.usage().handles_live);
    assert_eq!(
        state.chunks,
        state
            .arenas
            .iter()
            .map(|arena| arena.chunks.len())
            .sum::<usize>()
    );
}

fn snapshot(state: &MspaceState) -> Vec<ArenaSnapshot> {
    state
        .arenas
        .iter()
        .map(|arena| {
            (
                arena.base,
                arena.end,
                arena.in_use,
                arena.peak,
                arena.chunks.clone(),
                arena.chunks.capacity(),
            )
        })
        .collect()
}

#[test]
fn create_validation_base_identity_header_retention_and_recreate_are_real_guest_arena_semantics() {
    let (mut state, mut budget, mut guest) = fixture();
    for args in [
        [0, 0, 96, 0],
        [0, BASE + 1, 96, 0],
        [0, BASE, 95, 0],
        [0, u64::MAX - 15, 96, 0],
        [0, BASE, 96, 2],
    ] {
        assert_eq!(
            call(&mut state, &mut budget, &mut guest, MspaceCreate, &args),
            Ok(invalid(0))
        );
    }
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            PafMspaceCreate,
            &[u64::MAX, 0, 0, 2, 1]
        ),
        Err(Failure::Unsupported("PAF Mspace creation options"))
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceCreate,
            &[u64::MAX, BASE, 96, (9 << 32) | 1]
        ),
        Ok(MspaceResult::Value(BASE))
    );
    assert_eq!(guest.get(BASE, 96), [0xa7; 96]);
    assert_eq!(alloc(&mut state, &mut budget, &mut guest, 0), BASE + 64);
    assert_eq!(guest.get(BASE, 96), [0xa7; 96]);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceCreate,
            &[0, BASE, 96, 0]
        ),
        Ok(invalid(0))
    );
    let before = snapshot(&state);
    assert!(matches!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceCreate,
            &[0, ARG + PAGE, 96, 0]
        ),
        Err(Failure::Guest(_))
    ));
    assert_eq!(snapshot(&state), before);
    assert_eq!(
        call(&mut state, &mut budget, &mut guest, MspaceDestroy, &[BASE]),
        Ok(MspaceResult::Value(0))
    );
    assert_eq!(
        call(&mut state, &mut budget, &mut guest, MspaceDestroy, &[BASE]),
        Ok(invalid(u64::from(u32::MAX)))
    );
    create(&mut state, &mut budget, &mut guest, 96);
    assert_eq!(guest.get(BASE, 96), [0xa7; 96]);
    census(&state, &budget);
}

#[test]
fn best_fit_size_then_address_splits_and_coalesces_without_wiping_guest_bytes() {
    let (mut state, mut budget, mut guest) = fixture();
    create(&mut state, &mut budget, &mut guest, 1024);
    let blocks = (0..6)
        .map(|_| alloc(&mut state, &mut budget, &mut guest, 64))
        .collect::<Vec<_>>();
    for pointer in [blocks[0], blocks[2]] {
        assert_eq!(
            call(
                &mut state,
                &mut budget,
                &mut guest,
                MspaceFree,
                &[BASE, pointer]
            ),
            Ok(MspaceResult::Value(0))
        );
    }
    assert_eq!(alloc(&mut state, &mut budget, &mut guest, 33), blocks[0]);
    assert_eq!(alloc(&mut state, &mut budget, &mut guest, 1), blocks[2]);
    assert_eq!(
        alloc(&mut state, &mut budget, &mut guest, 1),
        blocks[2] + 32
    );
    for pointer in [
        blocks[0],
        blocks[1],
        blocks[2],
        blocks[2] + 32,
        blocks[3],
        blocks[4],
        blocks[5],
    ] {
        assert_eq!(
            call(
                &mut state,
                &mut budget,
                &mut guest,
                MspaceFree,
                &[BASE, pointer]
            ),
            Ok(MspaceResult::Value(0))
        );
    }
    assert_eq!(
        state.arenas[0].chunks,
        [Chunk {
            start: BASE + 64,
            end: BASE + 1024,
            used: false
        }]
    );
    assert_eq!(alloc(&mut state, &mut budget, &mut guest, 960), BASE + 64);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceMalloc,
            &[BASE, 1]
        ),
        Ok(exhausted(0))
    );
    assert_eq!(guest.get(BASE, 1024), [0xa7; 1024]);
    census(&state, &budget);
}

#[test]
fn calloc_zeroes_only_the_requested_product_and_overflow_precedes_handle_lookup() {
    let (mut state, mut budget, mut guest) = fixture();
    create(&mut state, &mut budget, &mut guest, 1024);
    let pointer = value(call(
        &mut state,
        &mut budget,
        &mut guest,
        MspaceCalloc,
        &[BASE, 3, 7],
    ));
    assert_eq!(guest.get(pointer, 21), [0; 21]);
    assert_eq!(guest.get(pointer + 21, 11), [0xa7; 11]);
    assert_eq!(state.arenas[0].in_use, 32);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceCalloc,
            &[0, u64::MAX, 2]
        ),
        Ok(exhausted(0))
    );
    let before = snapshot(&state);
    guest.refuse = true;
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceCalloc,
            &[BASE, 2, 20]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(snapshot(&state), before);
    guest.refuse = false;
    let zero = value(call(
        &mut state,
        &mut budget,
        &mut guest,
        MspaceCalloc,
        &[BASE, 0, u64::MAX],
    ));
    assert_eq!(guest.get(zero, 32), [0xa7; 32]);
    assert_eq!(state.arenas[0].in_use, 64);
    census(&state, &budget);
}

#[test]
fn realloc_preserves_rounded_padding_counts_temporary_peak_and_commits_only_after_copy() {
    let (mut state, mut budget, mut guest) = fixture();
    create(&mut state, &mut budget, &mut guest, 4096);
    let first = alloc(&mut state, &mut budget, &mut guest, 33);
    let blocker = alloc(&mut state, &mut budget, &mut guest, 64);
    let pattern = (0..64u8).collect::<Vec<_>>();
    guest.put(first - ARG, &pattern);
    let before = snapshot(&state);
    guest.refuse = true;
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceRealloc,
            &[BASE, first, 160]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(snapshot(&state), before);
    assert_eq!(guest.get(first, 64), pattern);
    guest.refuse = false;
    let moved = value(call(
        &mut state,
        &mut budget,
        &mut guest,
        MspaceRealloc,
        &[BASE, first, 160],
    ));
    assert_ne!(moved, first);
    assert_eq!(guest.get(moved, 64), pattern);
    assert_eq!(guest.get(moved + 64, 96), [0xa7; 96]);
    assert_eq!((state.arenas[0].in_use, state.arenas[0].peak), (224, 288));
    assert_eq!(
        value(call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceRealloc,
            &[BASE, moved, 1]
        )),
        moved
    );
    assert_eq!(
        value(call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceMallocUsableSize,
            &[moved]
        )),
        160
    );
    let before = snapshot(&state);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceRealloc,
            &[BASE, moved, 4096]
        ),
        Ok(exhausted(0))
    );
    assert_eq!(snapshot(&state), before);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceFree,
            &[BASE, blocker]
        ),
        Ok(MspaceResult::Value(0))
    );
    assert_eq!(
        value(call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceRealloc,
            &[BASE, moved, 320]
        )),
        moved
    );
    assert_eq!((state.arenas[0].in_use, state.arenas[0].peak), (320, 320));
    assert_eq!(
        value(call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceRealloc,
            &[BASE, moved, 0]
        )),
        0
    );
    assert_eq!(state.arenas[0].in_use, 0);
    assert_ne!(
        value(call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceRealloc,
            &[BASE, 0, 0]
        )),
        0
    );
    census(&state, &budget);
}

#[test]
fn alignment_variants_preserve_zero_rules_posix_output_and_source_errno_distinctions() {
    let (mut state, mut budget, mut guest) = fixture();
    create(&mut state, &mut budget, &mut guest, 4096);
    let pointer = value(call(
        &mut state,
        &mut budget,
        &mut guest,
        MspaceReallocalign,
        &[BASE, 0, 100, 0],
    ));
    assert_eq!(pointer & 127, 0);
    for alignment in [0, 3, 100] {
        assert_eq!(
            value(call(
                &mut state,
                &mut budget,
                &mut guest,
                MspaceReallocalign,
                &[BASE, pointer, alignment, 0]
            )),
            pointer
        );
    }
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceReallocalign,
            &[0, u64::MAX, 1 << 63, 1]
        ),
        Ok(MspaceResult::Value(0))
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceMemalign,
            &[0, 0, 1]
        ),
        Ok(invalid(0))
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceFree,
            &[u64::MAX, 0]
        ),
        Ok(MspaceResult::Value(0))
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceFree,
            &[BASE, pointer + 1]
        ),
        Ok(invalid(0))
    );
    let relaxed = value(call(
        &mut state,
        &mut budget,
        &mut guest,
        MspaceAlignedAlloc,
        &[BASE, 128, 33],
    ));
    assert_eq!(relaxed & 127, 0);
    guest.put(OUT - ARG, &0x1122334455667788u64.to_le_bytes());
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspacePosixMemalign,
            &[0, OUT, 3, 8]
        ),
        Ok(MspaceResult::Value(22))
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspacePosixMemalign,
            &[0, OUT, 8, 8]
        ),
        Ok(invalid(22))
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspacePosixMemalign,
            &[BASE, OUT, 8, 8192]
        ),
        Ok(exhausted(12))
    );
    assert_eq!(guest.u64(OUT - ARG), 0x1122334455667788);
    let before = snapshot(&state);
    guest.refuse = true;
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspacePosixMemalign,
            &[BASE, OUT, 8, 32]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(snapshot(&state), before);
    guest.refuse = false;
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspacePosixMemalign,
            &[BASE, OUT, 8, 32]
        ),
        Ok(MspaceResult::Value(0))
    );
    assert_eq!(guest.u64(OUT - ARG) & 31, 0);
    census(&state, &budget);
}

#[test]
fn stats_write_only_four_u64s_after_size_check_and_usable_size_requires_exact_live_start() {
    let (mut state, mut budget, mut guest) = fixture();
    create(&mut state, &mut budget, &mut guest, 0x811);
    let pointer = alloc(&mut state, &mut budget, &mut guest, 1);
    let header = [40, 0, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96];
    guest.put(OUT - ARG, &header);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceMallocStats,
            &[BASE, OUT]
        ),
        Ok(MspaceResult::Value(0))
    );
    assert_eq!(guest.get(OUT, 8), header);
    for (offset, expected) in [(8, 0x811), (16, 0x811), (24, 32), (32, 32)] {
        assert_eq!(guest.u64(OUT - ARG + offset), expected);
    }
    assert_eq!(guest.get(OUT + 40, 8), [0xa7; 8]);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceMallocStatsFast,
            &[0, 0]
        ),
        Ok(MspaceResult::Value(22))
    );
    guest.put(OUT - ARG, &39u16.to_le_bytes());
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceMallocStatsFast,
            &[0, OUT]
        ),
        Ok(MspaceResult::Value(22))
    );
    guest.put(OUT - ARG, &40u16.to_le_bytes());
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceMallocStatsFast,
            &[0, OUT]
        ),
        Ok(invalid(22))
    );
    assert_eq!(
        value(call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceMallocUsableSize,
            &[0]
        )),
        0
    );
    assert_eq!(
        value(call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceMallocUsableSize,
            &[pointer]
        )),
        32
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceMallocUsableSize,
            &[pointer + 1]
        ),
        Ok(invalid(0))
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceFree,
            &[BASE, pointer]
        ),
        Ok(MspaceResult::Value(0))
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceMallocUsableSize,
            &[pointer]
        ),
        Ok(invalid(0))
    );
    census(&state, &budget);
}

#[test]
fn nested_arenas_survive_parent_free_and_destroy_with_descending_usable_size_lookup() {
    let (mut state, mut budget, mut guest) = fixture();
    create(&mut state, &mut budget, &mut guest, 0x2800);
    let region = alloc(&mut state, &mut budget, &mut guest, 4096);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceCreate,
            &[0, region + 4096, 96, 0]
        ),
        Ok(invalid(0))
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceCreate,
            &[0, region, 4096, 1]
        ),
        Ok(MspaceResult::Value(region))
    );
    let inner = value(call(
        &mut state,
        &mut budget,
        &mut guest,
        MspaceMalloc,
        &[region, 64],
    ));
    assert_eq!(
        value(call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceMallocUsableSize,
            &[region]
        )),
        4096
    );
    assert_eq!(
        value(call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceMallocUsableSize,
            &[inner]
        )),
        64
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceCreate,
            &[0, inner, 96, 0]
        ),
        Ok(invalid(0))
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceFree,
            &[BASE, region]
        ),
        Ok(MspaceResult::Value(0))
    );
    assert_eq!(
        call(&mut state, &mut budget, &mut guest, MspaceDestroy, &[BASE]),
        Ok(MspaceResult::Value(0))
    );
    assert_eq!(state.retained_handle_count(), 1);
    assert_ne!(
        value(call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceMalloc,
            &[region, 64]
        )),
        0
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceFree,
            &[region, inner]
        ),
        Ok(MspaceResult::Value(0))
    );
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceDestroy,
            &[region]
        ),
        Ok(MspaceResult::Value(0))
    );
    create(&mut state, &mut budget, &mut guest, 0x2800);
    census(&state, &budget);
}

#[test]
fn all_ten_paf_wrappers_share_regular_state_and_ignore_owner_words_without_dereference() {
    let (mut state, mut budget, mut guest) = fixture();
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            PafMspaceCreate,
            &[u64::MAX, BASE, 4096, 0, 0]
        ),
        Ok(MspaceResult::Value(BASE))
    );
    let first = value(call(
        &mut state,
        &mut budget,
        &mut guest,
        PafMspaceMalloc,
        &[BASE, 64, u64::MAX],
    ));
    let aligned = value(call(
        &mut state,
        &mut budget,
        &mut guest,
        PafMspaceMemalign,
        &[BASE, 64, 64, u64::MAX],
    ));
    let zero = value(call(
        &mut state,
        &mut budget,
        &mut guest,
        PafMspaceCalloc,
        &[BASE, 3, 7, u64::MAX],
    ));
    assert_eq!(guest.get(zero, 21), [0; 21]);
    let grown = value(call(
        &mut state,
        &mut budget,
        &mut guest,
        PafMspaceRealloc,
        &[BASE, first, 128, u64::MAX],
    ));
    let moved = value(call(
        &mut state,
        &mut budget,
        &mut guest,
        PafMspaceReallocalign,
        &[BASE, grown, 256, 160, u64::MAX],
    ));
    assert_eq!(moved & 255, 0);
    guest.put(OUT - ARG, &40u16.to_le_bytes());
    for service in [PafMspaceMallocStats, PafMspaceMallocStatsFast] {
        assert_eq!(
            call(&mut state, &mut budget, &mut guest, service, &[BASE, OUT]),
            Ok(MspaceResult::Value(0))
        );
    }
    for pointer in [aligned, zero, moved] {
        assert_eq!(
            call(
                &mut state,
                &mut budget,
                &mut guest,
                PafMspaceFree,
                &[BASE, pointer, u64::MAX]
            ),
            Ok(MspaceResult::Value(0))
        );
    }
    assert_eq!(state.arenas[0].in_use, 0);
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            PafMspaceDestroy,
            &[BASE]
        ),
        Ok(MspaceResult::Value(0))
    );
    census(&state, &budget);
}

#[test]
fn quotas_numeric_overflow_and_backing_revocation_refuse_without_publishing_allocator_changes() {
    for limits in [
        Limits {
            max_handles: 0,
            ..Default::default()
        },
        Limits {
            max_work: 0,
            ..Default::default()
        },
        Limits {
            max_scratch_bytes: 1,
            max_name_bytes: 1,
            max_io_bytes: 1,
            ..Default::default()
        },
    ] {
        let (mut state, _, mut guest) = fixture();
        let mut budget = Budget::new(limits).unwrap();
        assert!(matches!(
            call(
                &mut state,
                &mut budget,
                &mut guest,
                MspaceCreate,
                &[0, BASE, 4096, 0]
            ),
            Err(Failure::Limit(_))
        ));
        assert!(state.arenas.is_empty());
        assert_eq!(state.arenas.capacity(), 0);
        census(&state, &budget);
    }
    let (mut state, mut budget, mut guest) = fixture();
    create(&mut state, &mut budget, &mut guest, 4096);
    let pointer = alloc(&mut state, &mut budget, &mut guest, 64);
    let before = snapshot(&state);
    for (service, args) in [
        (MspaceMalloc, vec![BASE, u64::MAX]),
        (MspaceRealloc, vec![BASE, pointer, u64::MAX]),
        (MspaceReallocalign, vec![BASE, pointer, 32, u64::MAX]),
    ] {
        assert_eq!(
            call(&mut state, &mut budget, &mut guest, service, &args),
            Ok(exhausted(0))
        );
        assert_eq!(snapshot(&state), before);
    }
    budget.limits.max_io_bytes = 4;
    assert_eq!(
        call(
            &mut state,
            &mut budget,
            &mut guest,
            MspaceCalloc,
            &[BASE, 1, 8]
        ),
        Err(Failure::Limit("Mspace transfer bytes"))
    );
    assert_eq!(snapshot(&state), before);
    struct Revoked<'a>(&'a mut TestGuest);
    impl GuestAccess for Revoked<'_> {
        fn read(&self, address: u64, bytes: &mut [u8]) -> Result<(), GuestError> {
            self.0.read(address, bytes)
        }
        fn apply(
            &mut self,
            effect: crate::MemoryEffect,
            writes: &[GuestWrite<'_>],
        ) -> Result<(), GuestError> {
            self.0.apply(effect, writes)
        }
    }
    let mut revoked = Revoked(&mut guest); // Missing exact backing proof is a refusal.
    assert_eq!(
        state.invoke(
            MspaceFree,
            CallArgs([BASE, pointer, 0, 0, 0, 0]),
            &mut revoked,
            &mut budget
        ),
        Err(Failure::Guest(GuestError::UnsupportedMapping))
    );
    assert_eq!(snapshot(&state), before);
    assert_eq!(
        storage::metadata(MAX_METADATA_BYTES, 1),
        Err(Failure::Limit("Mspace metadata"))
    );
    census(&state, &budget);
}
