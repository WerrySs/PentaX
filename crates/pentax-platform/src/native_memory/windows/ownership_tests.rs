// SPDX-License-Identifier: GPL-2.0-only

//! Original data-only ownership/preflight fixtures. No guest execution or FFI.

use super::*;

const GRANULE: u64 = 0x10000;

fn space(size: u64) -> NativeAddressSpace {
    NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), size).unwrap()
}

fn base(owner: &NativeAddressSpace) -> u64 {
    owner.reserved_ranges().next().unwrap().address
}

#[test]
fn identity_survives_moves_and_pause_but_not_owner_replacement() {
    let owner = space(GRANULE);
    let identity = owner.owner_id();
    // Taking ownership through an actual container preserves the identity; no
    // pointer to the Rust allocation or guest reservation defines this token.
    let mut container = vec![owner];
    let mut moved = container.pop().unwrap();
    assert_eq!(moved.owner_id(), identity);
    assert_eq!(moved.pause().unwrap().owner_id(), identity);
    drop(moved);
    let replacement = space(GRANULE);
    assert_ne!(replacement.owner_id(), identity);
    let empty = NativeAddressSpace::new(Default::default()).unwrap();
    assert_ne!(empty.owner_id(), replacement.owner_id());
}

#[test]
fn guard_preflight_accepts_owned_holes_and_rejects_mappings_and_invalid_ranges() {
    let mut owner = space(3 * GRANULE);
    let b = base(&owner);
    owner
        .map_at(b + GRANULE, NATIVE_GUEST_PAGE, NativePermissions::READ)
        .unwrap();
    let operations = owner.operations;
    let paused = owner.pause().unwrap();
    paused.preflight_reserved_unmapped(b, GRANULE).unwrap();
    paused
        .preflight_reserved_unmapped(
            b + GRANULE + NATIVE_GUEST_PAGE,
            2 * GRANULE - NATIVE_GUEST_PAGE,
        )
        .unwrap();
    assert_eq!(
        paused.preflight_reserved_unmapped(b, 2 * GRANULE),
        Err(NativeMemoryError::Overlap)
    );
    assert_eq!(
        paused.preflight_reserved_unmapped(b + GRANULE, NATIVE_GUEST_PAGE),
        Err(NativeMemoryError::Overlap)
    );
    for (address, size, error) in [
        (b, 0, NativeMemoryError::InvalidRange),
        (u64::MAX, NATIVE_GUEST_PAGE, NativeMemoryError::InvalidRange),
        (b + 1, NATIVE_GUEST_PAGE, NativeMemoryError::Unaligned),
        (b, NATIVE_GUEST_PAGE - 1, NativeMemoryError::Unaligned),
        (
            b + 3 * GRANULE - NATIVE_GUEST_PAGE,
            2 * NATIVE_GUEST_PAGE,
            NativeMemoryError::Unmapped(b + 3 * GRANULE),
        ),
    ] {
        assert_eq!(
            paused.preflight_reserved_unmapped(address, size),
            Err(error)
        );
    }
    assert_eq!(owner.operations, operations);
    assert_eq!(owner.mapped_bytes(), NATIVE_GUEST_PAGE);
}

#[test]
fn reservation_union_never_bridges_a_foreign_owned_gap() {
    // Three simultaneously live reservations guarantee the middle one cannot
    // belong to either outside reservation, regardless of OS address selection.
    let mut owners = (0..3).map(|_| space(GRANULE)).collect::<Vec<_>>();
    owners.sort_unstable_by_key(base);
    let mut right = owners.pop().unwrap();
    let middle = owners.pop().unwrap();
    let mut left = owners.pop().unwrap();
    let b = base(&left);
    let right_base = base(&right);
    // Move the actual unique descriptor, rather than inventing reservation
    // metadata. Neither remaining source owner retains a release capability.
    left.reservations.push(right.reservations.pop().unwrap());
    left.reserved += right.reserved;
    right.reserved = 0;
    let paused = left.pause().unwrap();
    paused.preflight_reserved_unmapped(b, GRANULE).unwrap();
    paused
        .preflight_reserved_unmapped(right_base, GRANULE)
        .unwrap();
    assert_eq!(
        paused.preflight_reserved_unmapped(b, right_base + GRANULE - b),
        Err(NativeMemoryError::Unmapped(b + GRANULE))
    );
    assert_eq!(
        paused.preflight_reserved_unmapped(base(&middle), GRANULE),
        Err(NativeMemoryError::Unmapped(base(&middle)))
    );
}

#[test]
fn executable_source_preflight_checks_every_page_without_copying_or_changing_permissions() {
    let mut owner = space(GRANULE);
    let b = base(&owner);
    owner
        .map_at(b, NATIVE_GUEST_PAGE, NativePermissions::READ_EXECUTE)
        .unwrap();
    owner
        .map_at(
            b + NATIVE_GUEST_PAGE,
            NATIVE_GUEST_PAGE,
            NativePermissions::READ,
        )
        .unwrap();
    let operations = owner.operations;
    {
        let paused = owner.pause().unwrap();
        paused.preflight_fetch_source(b, NATIVE_GUEST_PAGE).unwrap();
        assert_eq!(
            paused.preflight_fetch_source(b + NATIVE_GUEST_PAGE - 1, 2),
            Err(NativeMemoryError::PermissionDenied(b + NATIVE_GUEST_PAGE))
        );
        assert_eq!(
            paused.preflight_fetch_source(u64::MAX, 2),
            Err(NativeMemoryError::InvalidRange)
        );
        paused.preflight_fetch_source(u64::MAX, 0).unwrap();
    }
    assert_eq!(owner.operations, operations);
    owner
        .protect(
            b + NATIVE_GUEST_PAGE,
            NATIVE_GUEST_PAGE,
            NativePermissions::READ_EXECUTE,
        )
        .unwrap();
    let paused = owner.pause().unwrap();
    assert_eq!(
        paused.preflight_fetch_source(b + 2 * NATIVE_GUEST_PAGE - 1, 2),
        Err(NativeMemoryError::Unmapped(b + 2 * NATIVE_GUEST_PAGE))
    );
}

#[test]
fn poison_denies_both_preflights_before_even_empty_or_invalid_requests() {
    let mut owner = space(GRANULE);
    let identity = owner.owner_id();
    owner.poisoned = true;
    assert!(matches!(owner.pause(), Err(NativeMemoryError::Poisoned)));
    // This private test seam bypasses public pause to verify direct liveness
    // checks too; no public caller can construct such a poisoned pause.
    let paused = PausedNativeMemory { owner: &mut owner };
    assert_eq!(paused.owner_id(), identity);
    assert_eq!(
        paused.preflight_fetch_source(u64::MAX, 0),
        Err(NativeMemoryError::Poisoned)
    );
    assert_eq!(
        paused.preflight_reserved_unmapped(u64::MAX, 0),
        Err(NativeMemoryError::Poisoned)
    );
}
