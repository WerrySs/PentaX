// SPDX-License-Identifier: GPL-2.0-only
//! Original bounded actual-owner backing tests; no supplied-image data.

use super::*;

const PAGE: u64 = NATIVE_GUEST_PAGE;

fn space() -> (NativeAddressSpace, u64) {
    let owner = NativeAddressSpace::reserve_any(Default::default(), 32 * PAGE).unwrap();
    let address = owner.reserved_ranges().next().unwrap().address;
    (owner, address)
}

fn span(address: u64, size: u64) -> NativeRange {
    NativeRange { address, size }
}

fn effect(owner: &mut NativeAddressSpace, effect: NativeMemoryEffect) {
    owner
        .pause()
        .unwrap()
        .apply_transaction(
            NativeMemoryTransaction {
                effect,
                writes: &[],
            },
            &Default::default(),
        )
        .unwrap();
}

fn allocate(owner: &mut NativeAddressSpace, size: u64) {
    effect(
        owner,
        NativeMemoryEffect::AllocatePhysical { physical: 0, size },
    );
}

fn shared(owner: &mut NativeAddressSpace, address: u64, size: u64, physical: u64) {
    effect(
        owner,
        NativeMemoryEffect::MapPhysical {
            address,
            size,
            physical,
            permissions: NativePermissions::READ_WRITE,
        },
    );
}

#[test]
fn private_unaligned_spans_keep_exact_old_work_in_both_orders_and_do_not_grant_access() {
    let (mut owner, base) = space();
    owner
        .map_at(base, 2 * PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    owner
        .map_at(base + 4 * PAGE, 8 * PAGE, NativePermissions::READ_EXECUTE)
        .unwrap();
    let output = span(base + PAGE - 2, 4);
    let source = span(base + 4 * PAGE + 3, 8 * PAGE - 6);
    let operations = owner.operations;
    let mut paused = owner.pause().unwrap();
    paused.write(output.address, b"keep").unwrap();
    // Two output pages and eight source pages retain the original charge 47.
    assert_eq!(
        paused.preflight_disjoint_backing(output, source, 47),
        Ok(47)
    );
    assert_eq!(
        paused.preflight_disjoint_backing(source, output, 47),
        Ok(47)
    );
    assert_eq!(
        paused.preflight_disjoint_backing(output, source, 46),
        Err(NativeMemoryError::BudgetExceeded)
    );
    let mut actual = [0; 4];
    paused.read(output.address, &mut actual).unwrap();
    assert_eq!(&actual, b"keep");
    assert!(matches!(
        paused.write(source.address, b"deny"),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    assert_eq!(paused.owner.operations, operations);
}

#[test]
fn private_same_page_touching_ranges_stay_disjoint_and_actual_overlap_refuses_after_budget() {
    let (mut owner, base) = space();
    owner
        .map_at(base, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let paused = owner.pause().unwrap();
    assert_eq!(
        paused.preflight_disjoint_backing(span(base + 1, 7), span(base + 8, 5), 8),
        Ok(8)
    );
    assert_eq!(
        paused.preflight_disjoint_backing(span(base + 8, 5), span(base + 1, 7), 8),
        Ok(8)
    );
    let left = span(base + 2, 10);
    let right = span(base + 11, 1);
    assert_eq!(
        paused.preflight_disjoint_backing(left, right, 7),
        Err(NativeMemoryError::BudgetExceeded)
    );
    assert_eq!(
        paused.preflight_disjoint_backing(left, right, 8),
        Err(NativeMemoryError::Overlap)
    );
    assert_eq!(
        paused.preflight_disjoint_backing(right, left, 8),
        Err(NativeMemoryError::Overlap)
    );
}

#[test]
fn mapped_holes_overflow_empty_spans_and_overlap_keep_full_validation_error_order() {
    let (mut owner, base) = space();
    owner
        .map_at(base, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    owner
        .map_at(base + 2 * PAGE, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let paused = owner.pause().unwrap();
    let incomplete = span(base, 2 * PAGE);
    let overlapping = span(base, 1);
    // Work admission and BOTH complete mapped checks precede virtual overlap.
    assert_eq!(
        paused.preflight_disjoint_backing(incomplete, overlapping, 11),
        Err(NativeMemoryError::BudgetExceeded)
    );
    assert_eq!(
        paused.preflight_disjoint_backing(incomplete, overlapping, 12),
        Err(NativeMemoryError::Unmapped(base + PAGE))
    );
    assert_eq!(
        paused.preflight_disjoint_backing(overlapping, incomplete, 12),
        Err(NativeMemoryError::Unmapped(base + PAGE))
    );
    assert_eq!(
        paused.preflight_disjoint_backing(span(base + PAGE, 1), span(base + 3 * PAGE, 1), 8),
        Err(NativeMemoryError::Unmapped(base + PAGE))
    );
    assert_eq!(
        paused.preflight_disjoint_backing(span(u64::MAX, 2), overlapping, u64::MAX),
        Err(NativeMemoryError::InvalidRange)
    );
    assert_eq!(
        paused.preflight_disjoint_backing(overlapping, span(u64::MAX, 2), u64::MAX),
        Err(NativeMemoryError::InvalidRange)
    );
    assert_eq!(
        paused.preflight_disjoint_backing(span(u64::MAX, 0), span(u64::MAX, 2), 0),
        Ok(0)
    );
}

#[test]
fn same_address_private_to_shared_remap_rechecks_real_aliases_without_retained_proofs() {
    let (mut owner, base) = space();
    let view = base + 8 * PAGE;
    owner
        .map_at(base, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    allocate(&mut owner, PAGE);
    shared(&mut owner, view, PAGE, 0);
    let left = span(base + 3, 4);
    let right = span(view + 3, 4);
    assert_eq!(
        owner
            .pause()
            .unwrap()
            .preflight_disjoint_backing(left, right, 8),
        Ok(8)
    );
    owner.pause().unwrap().write(view + 3, b"view").unwrap();
    owner.unmap(base, PAGE).unwrap();
    shared(&mut owner, base, PAGE, 0);
    assert_eq!(
        owner
            .pause()
            .unwrap()
            .preflight_disjoint_backing(left, right, 8),
        Err(NativeMemoryError::Overlap)
    );
    owner.pause().unwrap().write(base + 3, b"real").unwrap();
    let mut actual = [0; 4];
    owner.pause().unwrap().read(view + 3, &mut actual).unwrap();
    assert_eq!(&actual, b"real");
    // Disjoint byte extents inside the SAME shared page do not alias.
    assert_eq!(
        owner
            .pause()
            .unwrap()
            .preflight_disjoint_backing(span(base + 3, 4), span(view + 7, 4), 8),
        Ok(8)
    );
    owner.unmap(base, PAGE).unwrap();
    owner
        .map_at(base, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    assert_eq!(
        owner
            .pause()
            .unwrap()
            .preflight_disjoint_backing(left, right, 8),
        Ok(8)
    );
    owner.pause().unwrap().read(base + 3, &mut actual).unwrap();
    assert_eq!(actual, [0; 4]);
}

#[test]
fn late_shared_page_in_smaller_unaligned_span_keeps_cartesian_alias_and_disjoint_results() {
    let (mut owner, base) = space();
    let view = base + 8 * PAGE;
    owner
        .map_at(base, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    allocate(&mut owner, 6 * PAGE);
    shared(&mut owner, base + PAGE, PAGE, 0);
    shared(&mut owner, view, 3 * PAGE, 0);
    let mixed = span(base + PAGE - 2, 4);
    let other = span(view, 3 * PAGE);
    // Classification must visit the late shared page rather than trusting the
    // first private two bytes. Its failed scan still uses the old 22 charge.
    assert_eq!(
        owner
            .pause()
            .unwrap()
            .preflight_disjoint_backing(mixed, other, 21),
        Err(NativeMemoryError::BudgetExceeded)
    );
    assert_eq!(
        owner
            .pause()
            .unwrap()
            .preflight_disjoint_backing(mixed, other, 22),
        Err(NativeMemoryError::Overlap)
    );
    assert_eq!(
        owner
            .pause()
            .unwrap()
            .preflight_disjoint_backing(other, mixed, 22),
        Err(NativeMemoryError::Overlap)
    );
    owner.unmap(view, 3 * PAGE).unwrap();
    shared(&mut owner, view, 3 * PAGE, 3 * PAGE);
    assert_eq!(
        owner
            .pause()
            .unwrap()
            .preflight_disjoint_backing(mixed, other, 22),
        Ok(22)
    );
    assert_eq!(
        owner
            .pause()
            .unwrap()
            .preflight_disjoint_backing(other, mixed, 22),
        Ok(22)
    );
}

#[test]
fn foreign_ranges_noaccess_and_poisoned_owner_never_gain_private_shortcut_authority() {
    let (mut owner, base) = space();
    let (mut foreign, other) = space();
    assert_ne!(owner.owner_id(), foreign.owner_id());
    owner
        .map_at(base, 2 * PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    foreign
        .map_at(other, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    assert_eq!(
        owner
            .pause()
            .unwrap()
            .preflight_disjoint_backing(span(base, 1), span(other, 1), 8),
        Err(NativeMemoryError::Unmapped(other))
    );
    owner
        .protect(base, PAGE, NativePermissions::new(false, false, false))
        .unwrap();
    let mut paused = owner.pause().unwrap();
    // Alias metadata never granted data permissions, even for private pages.
    assert_eq!(
        paused.preflight_disjoint_backing(span(base, 1), span(base + PAGE, 1), 8),
        Ok(8)
    );
    assert_eq!(
        paused.write(base, b"x"),
        Err(NativeMemoryError::PermissionDenied(base))
    );
    paused.owner.poisoned = true;
    assert_eq!(
        paused.preflight_disjoint_backing(span(base, 1), span(base + PAGE, 1), 8),
        Err(NativeMemoryError::Poisoned)
    );
    assert_eq!(
        paused.preflight_disjoint_backing(span(u64::MAX, 0), span(u64::MAX, 0), 0),
        Err(NativeMemoryError::Poisoned)
    );
}
