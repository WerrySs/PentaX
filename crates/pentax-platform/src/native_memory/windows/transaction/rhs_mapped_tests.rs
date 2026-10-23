// SPDX-License-Identifier: GPL-2.0-only
//! Original exact mapped-span, mutation, backing and refusal consumers.
use super::*;
const PAGE: u64 = NATIVE_GUEST_PAGE;
fn fixture() -> (NativeAddressSpace, NativeRange, NativeRange, NativeRange) {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 16 * PAGE).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(base, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    owner
        .map_at(base + 4 * PAGE, 2 * PAGE, NativePermissions::READ_EXECUTE)
        .unwrap();
    owner
        .map_at(base + 8 * PAGE, 3 * PAGE, NativePermissions::READ_EXECUTE)
        .unwrap();
    (
        owner,
        NativeRange {
            address: base + 64,
            size: 32,
        },
        NativeRange {
            address: base + 4 * PAGE,
            size: 2 * PAGE,
        },
        NativeRange {
            address: base + 8 * PAGE,
            size: 3 * PAGE,
        },
    )
}
fn run(owner: &mut NativeAddressSpace, left: NativeRange, right: NativeRange) -> u64 {
    owner
        .pause()
        .unwrap()
        .preflight_disjoint_backing(left, right, u64::MAX)
        .unwrap()
}
#[test]
fn exact_two_large_rhs_spans_survive_small_return_slots_without_caching_payload_or_rights() {
    let (mut owner, left, a, b) = fixture();
    assert_eq!(run(&mut owner, left, a), 12);
    assert_eq!(run(&mut owner, left, b), 16);
    assert_eq!(owner.alias_rhs_preflight_scans.get(), 2);
    let short = NativeRange {
        address: left.address + 128,
        size: 8,
    };
    for _ in 0..4 {
        assert_eq!(run(&mut owner, left, short), 8);
        assert_eq!(run(&mut owner, left, a), 12);
        assert_eq!(run(&mut owner, left, b), 16);
    }
    assert_eq!(owner.alias_rhs_preflight_scans.get(), 6); // Only four small RHS scans.
    let partial = NativeRange {
        address: a.address + 1,
        size: a.size - 2,
    };
    assert_eq!(run(&mut owner, left, partial), 12);
    assert_eq!(owner.alias_rhs_preflight_scans.get(), 7); // Exact key, no covering proof.
    owner
        .protect(a.address, a.size, NativePermissions::READ_WRITE)
        .unwrap();
    assert_eq!(run(&mut owner, left, a), 12);
    let mut paused = owner.pause().unwrap();
    paused.write(a.address, b"fresh bytes").unwrap();
    let mut actual = [0; 11];
    paused.read(a.address, &mut actual).unwrap();
    assert_eq!(&actual, b"fresh bytes");
    assert!(matches!(
        paused.preflight_fetch_source(a.address, 1),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
}

#[test]
fn successful_and_failed_mutation_entries_invalidate_before_any_reusable_mapping_proof() {
    let (mut owner, left, a, _) = fixture();
    run(&mut owner, left, a);
    let mut scans = owner.alias_rhs_preflight_scans.get();
    assert!(
        owner
            .map_at(a.address, PAGE, NativePermissions::READ)
            .is_err()
    );
    run(&mut owner, left, a);
    scans += 1;
    assert_eq!(owner.alias_rhs_preflight_scans.get(), scans);
    assert!(
        owner
            .protect(a.address + 1, PAGE, NativePermissions::READ)
            .is_err()
    );
    run(&mut owner, left, a);
    scans += 1;
    assert_eq!(owner.alias_rhs_preflight_scans.get(), scans);
    assert!(owner.unmap(a.address + 1, PAGE).is_err());
    run(&mut owner, left, a);
    scans += 1;
    assert_eq!(owner.alias_rhs_preflight_scans.get(), scans);
    assert!(
        owner
            .map_physical(a.address + 1, PAGE, 0, NativePermissions::READ)
            .is_err()
    );
    run(&mut owner, left, a);
    scans += 1;
    assert_eq!(owner.alias_rhs_preflight_scans.get(), scans);
    assert!(owner.release_physical(1, PAGE, 4096).is_err());
    run(&mut owner, left, a);
    scans += 1;
    assert_eq!(owner.alias_rhs_preflight_scans.get(), scans);
    owner
        .compare_replace_words(&[], &Default::default())
        .unwrap();
    run(&mut owner, left, a);
    scans += 1;
    assert_eq!(owner.alias_rhs_preflight_scans.get(), scans);
    {
        let mut paused = owner.pause().unwrap();
        assert!(
            paused
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::Protect {
                            address: a.address + 1,
                            size: PAGE,
                            permissions: NativePermissions::READ
                        },
                        writes: &[]
                    },
                    &Default::default()
                )
                .is_err()
        );
    }
    run(&mut owner, left, a);
    scans += 1;
    assert_eq!(owner.alias_rhs_preflight_scans.get(), scans);
    // None-effect payload writes alter neither map coverage nor the proof.
    owner
        .pause()
        .unwrap()
        .apply_transaction(
            NativeMemoryTransaction {
                effect: NativeMemoryEffect::None,
                writes: &[NativeMemoryWrite {
                    address: left.address,
                    bytes: b"new",
                }],
            },
            &Default::default(),
        )
        .unwrap();
    run(&mut owner, left, a);
    assert_eq!(owner.alias_rhs_preflight_scans.get(), scans);
    owner
        .protect(
            a.address,
            a.size,
            NativePermissions::new(false, false, false),
        )
        .unwrap();
    run(&mut owner, left, a);
    scans += 1;
    assert_eq!(owner.alias_rhs_preflight_scans.get(), scans);
    assert!(matches!(
        owner.pause().unwrap().preflight_read(a.address, 1),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
}

#[test]
fn unmap_and_same_address_shared_remap_recheck_live_backing_and_refuse_actual_aliases() {
    let (mut owner, left, a, _) = fixture();
    run(&mut owner, left, a);
    owner.unmap(a.address, a.size).unwrap();
    assert!(matches!(
        owner
            .pause()
            .unwrap()
            .preflight_disjoint_backing(left, a, u64::MAX),
        Err(NativeMemoryError::Unmapped(_))
    ));
    owner
        .map_at(a.address, a.size, NativePermissions::READ)
        .unwrap();
    assert_eq!(run(&mut owner, left, a), 12);
    owner.allocate_physical(0, 2 * PAGE).unwrap();
    owner.unmap(left.address / PAGE * PAGE, PAGE).unwrap();
    owner.unmap(a.address, a.size).unwrap();
    owner
        .map_physical(
            left.address / PAGE * PAGE,
            PAGE,
            0,
            NativePermissions::READ_WRITE,
        )
        .unwrap();
    owner
        .map_physical(a.address, a.size, 0, NativePermissions::READ)
        .unwrap();
    let mut paused = owner.pause().unwrap();
    paused.write(left.address, b"same backing").unwrap();
    let mut actual = [0; 12];
    paused.read(a.address + 64, &mut actual).unwrap();
    assert_eq!(&actual, b"same backing");
    assert!(matches!(
        paused.preflight_disjoint_backing(left, a, u64::MAX),
        Err(NativeMemoryError::Overlap)
    ));
    // A positive mapped-span hit is never a retained disjointness result.
    assert!(matches!(
        paused.preflight_disjoint_backing(left, a, u64::MAX),
        Err(NativeMemoryError::Overlap)
    ));
}

#[test]
fn exact_budget_error_order_foreign_owner_late_holes_and_live_before_hit_remain_refused() {
    let (mut owner, left, a, _) = fixture();
    run(&mut owner, left, a);
    let missing_left = NativeRange {
        address: a.address + 8 * PAGE,
        size: 32,
    };
    let mut paused = owner.pause().unwrap();
    assert!(matches!(
        paused.preflight_disjoint_backing(missing_left, a, 11),
        Err(NativeMemoryError::BudgetExceeded)
    ));
    assert!(matches!(
        paused.preflight_disjoint_backing(missing_left, a, 12),
        Err(NativeMemoryError::Unmapped(_))
    ));
    let late_hole = NativeRange {
        address: a.address + 1,
        size: a.size,
    };
    assert!(matches!(
        paused.preflight_disjoint_backing(left, late_hole, u64::MAX),
        Err(NativeMemoryError::Unmapped(_))
    ));
    assert_eq!(paused.preflight_disjoint_backing(left, a, 12).unwrap(), 12);
    assert!(matches!(
        paused.preflight_disjoint_backing(
            NativeRange {
                address: a.address + 64,
                size: 32
            },
            a,
            12
        ),
        Err(NativeMemoryError::Overlap)
    ));
    paused.write(left.address, b"still live").unwrap();
    // Poison without clearing to prove liveness wins even an exact warm hit.
    paused.owner.poisoned = true;
    assert!(matches!(
        paused.preflight_disjoint_backing(left, a, 12),
        Err(NativeMemoryError::Poisoned)
    ));
    let (mut other, other_left, _, _) = fixture();
    assert!(matches!(
        other
            .pause()
            .unwrap()
            .preflight_disjoint_backing(other_left, a, u64::MAX),
        Err(NativeMemoryError::Unmapped(_))
    ));
    assert!(other.alias_rhs_preflight.get().iter().all(Option::is_none));
}
