// SPDX-License-Identifier: GPL-2.0-only

//! Original owned-page regressions for the exact positive metadata proof.
//! No guest execution or external input is used by these tests.

use super::*;

const PAGE: u64 = NATIVE_GUEST_PAGE;

fn space() -> (NativeAddressSpace, u64) {
    let owner = NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x40000).unwrap();
    let address = owner.reserved_ranges().next().unwrap().address;
    (owner, address)
}

fn check(owner: &mut NativeAddressSpace, address: u64, size: u64) -> Result<(), NativeMemoryError> {
    owner
        .pause()?
        .preflight_read_write_non_executable(address, size)
}

fn prime(owner: &mut NativeAddressSpace, address: u64, size: u64) {
    check(owner, address, size).unwrap();
    assert_eq!(
        owner.stack_preflight.get(),
        Some(NativeRange { address, size })
    );
}

#[test]
fn exact_span_hits_across_pauses_without_caching_payload_subspans_or_foreign_owner() {
    let (mut owner, base) = space();
    owner
        .map_at(base, 3 * PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let span = NativeRange {
        address: base + 7,
        size: 3 * PAGE - 14,
    };
    prime(&mut owner, span.address, span.size);
    for value in 1..=8_u8 {
        owner.pause().unwrap().write(base + 19, &[value]).unwrap();
        check(&mut owner, span.address, span.size).unwrap();
        let mut actual = [0];
        owner.pause().unwrap().read(base + 19, &mut actual).unwrap();
        assert_eq!(actual, [value]);
    }
    assert_eq!(owner.stack_preflight_scans.get(), 1);
    owner
        .map_at(base + 8 * PAGE, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    assert!(owner.stack_preflight.get().is_none());
    prime(&mut owner, span.address, span.size);
    assert_eq!(owner.stack_preflight_scans.get(), 2);
    // A strict subspan uses a new scan; it is not an interval-permission cache.
    prime(&mut owner, span.address + 1, span.size - 1);
    assert_eq!(owner.stack_preflight_scans.get(), 3);
    assert_eq!(
        check(&mut owner, base, 3 * PAGE + 1),
        Err(NativeMemoryError::Unmapped(base + 3 * PAGE))
    );
    assert!(owner.stack_preflight.get().is_none());
    prime(&mut owner, span.address, span.size);
    let (mut other, other_base) = space();
    other
        .map_at(other_base, 3 * PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    assert_ne!(owner.owner_id(), other.owner_id());
    assert_eq!(
        check(&mut other, span.address, span.size),
        Err(NativeMemoryError::Unmapped(span.address))
    );
    prime(&mut other, other_base, 3 * PAGE);
    assert_eq!(owner.stack_preflight.get(), Some(span));
}

#[test]
fn late_permissions_holes_and_same_address_remap_always_retire_the_old_proof() {
    let (mut owner, base) = space();
    owner
        .map_at(base, 3 * PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    prime(&mut owner, base, 3 * PAGE);
    for rights in [
        NativePermissions::READ.with_gpu(false, true),
        NativePermissions::new(true, true, true),
    ] {
        owner.protect(base + 2 * PAGE, PAGE, rights).unwrap();
        assert!(owner.stack_preflight.get().is_none());
        assert_eq!(
            check(&mut owner, base, 3 * PAGE),
            Err(NativeMemoryError::PermissionDenied(base + 2 * PAGE))
        );
        owner
            .protect(base + 2 * PAGE, PAGE, NativePermissions::READ_WRITE)
            .unwrap();
        prime(&mut owner, base, 3 * PAGE);
    }
    owner.unmap(base + PAGE, PAGE).unwrap();
    assert_eq!(
        check(&mut owner, base, 3 * PAGE),
        Err(NativeMemoryError::Unmapped(base + PAGE))
    );
    owner
        .map_at(base + PAGE, PAGE, NativePermissions::READ)
        .unwrap();
    assert_eq!(
        check(&mut owner, base, 3 * PAGE),
        Err(NativeMemoryError::PermissionDenied(base + PAGE))
    );
    owner
        .protect(base + PAGE, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    prime(&mut owner, base, 3 * PAGE);
    assert_eq!(owner.mapped_bytes(), 3 * PAGE);
}

#[test]
fn failed_mapping_protection_and_unmap_attempts_do_not_leave_a_reusable_proof() {
    let (mut owner, base) = space();
    owner
        .map_at(base, 3 * PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    owner.pause().unwrap().write(base, b"original").unwrap();
    prime(&mut owner, base, 3 * PAGE);
    assert_eq!(
        owner.map_at(base + 1, PAGE, NativePermissions::READ_WRITE),
        Err(NativeMemoryError::Unaligned)
    );
    assert!(owner.stack_preflight.get().is_none());
    prime(&mut owner, base, 3 * PAGE);
    assert_eq!(
        owner.protect(base, PAGE - 1, NativePermissions::READ),
        Err(NativeMemoryError::Unaligned)
    );
    assert!(owner.stack_preflight.get().is_none());
    prime(&mut owner, base, 3 * PAGE);
    assert_eq!(
        owner.unmap(base + 1, PAGE),
        Err(NativeMemoryError::Unaligned)
    );
    assert!(owner.stack_preflight.get().is_none());
    prime(&mut owner, base, 3 * PAGE);
    owner.operations = 0;
    owner.gate = Some(1);
    assert!(matches!(
        owner.protect(base, 3 * PAGE, NativePermissions::READ),
        Err(NativeMemoryError::OperatingSystem { .. })
    ));
    assert!(!owner.poisoned);
    assert!(owner.stack_preflight.get().is_none());
    owner.gate = None;
    prime(&mut owner, base, 3 * PAGE); // Forces a live rewalk after rollback.
    let mut actual = [0; 8];
    owner.pause().unwrap().read(base, &mut actual).unwrap();
    assert_eq!(&actual, b"original");
    let scans = owner.stack_preflight_scans.get();
    owner.operations = 0;
    owner.gate = Some(0);
    assert!(owner.unmap(base, PAGE).is_err());
    assert!(owner.stack_preflight.get().is_none());
    owner.gate = None;
    prime(&mut owner, base, 3 * PAGE);
    assert_eq!(owner.stack_preflight_scans.get(), scans + 1);
}

#[test]
fn physical_alias_release_and_rollback_cannot_reuse_a_virtual_stack_proof() {
    let (mut owner, base) = space();
    owner.allocate_physical(0, 2 * PAGE).unwrap();
    owner
        .map_physical(base, 2 * PAGE, 0, NativePermissions::READ_WRITE)
        .unwrap();
    prime(&mut owner, base, 2 * PAGE);
    owner
        .map_physical(base + 4 * PAGE, 2 * PAGE, 0, NativePermissions::READ_WRITE)
        .unwrap();
    assert!(owner.stack_preflight.get().is_none());
    owner
        .pause()
        .unwrap()
        .write(base + 4 * PAGE, b"shared")
        .unwrap();
    prime(&mut owner, base, 2 * PAGE);
    assert_eq!(
        owner.release_physical(0, 2 * PAGE, 0),
        Err(NativeMemoryError::BudgetExceeded)
    );
    assert!(owner.stack_preflight.get().is_none());
    prime(&mut owner, base, 2 * PAGE);
    owner.operations = 0;
    owner.gate = Some(1); // Detach the first view, then restore it on failure.
    assert!(owner.release_physical(0, 2 * PAGE, 1024 * 1024).is_err());
    assert!(!owner.poisoned);
    assert!(owner.stack_preflight.get().is_none());
    owner.gate = None;
    prime(&mut owner, base, 2 * PAGE);
    let mut actual = [0; 6];
    owner.pause().unwrap().read(base, &mut actual).unwrap();
    assert_eq!(&actual, b"shared");
    owner.release_physical(0, 2 * PAGE, 1024 * 1024).unwrap();
    assert_eq!(
        check(&mut owner, base, 2 * PAGE),
        Err(NativeMemoryError::Unmapped(base))
    );
    assert_eq!(
        check(&mut owner, base + 4 * PAGE, 2 * PAGE),
        Err(NativeMemoryError::Unmapped(base + 4 * PAGE))
    );
    assert_eq!((owner.mapped_bytes(), owner.backing_bytes()), (0, 0));
    owner
        .map_at(base, 2 * PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    prime(&mut owner, base, 2 * PAGE);
    assert!(
        owner
            .map_physical(
                base + 8 * PAGE,
                PAGE,
                4 * PAGE,
                NativePermissions::READ_WRITE
            )
            .is_err()
    );
    assert!(owner.stack_preflight.get().is_none());
    prime(&mut owner, base, 2 * PAGE);
}

#[test]
fn temporary_loader_protection_and_its_failure_retire_the_exact_proof() {
    let (mut owner, base) = space();
    owner
        .map_at(base, 3 * PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let at = base + PAGE - 3;
    owner
        .pause()
        .unwrap()
        .write(at, &37_u64.to_le_bytes())
        .unwrap();
    let replacement = [NativeWordReplacement {
        address: at,
        expected: 37,
        replacement: 51,
    }];
    prime(&mut owner, base, 3 * PAGE);
    owner.operations = 0;
    owner.gate = Some(1); // Late opening failure, before any replacement copy.
    assert!(
        owner
            .compare_replace_words(&replacement, &Default::default())
            .is_err()
    );
    assert!(!owner.poisoned);
    assert!(owner.stack_preflight.get().is_none());
    owner.gate = None;
    prime(&mut owner, base, 3 * PAGE);
    let mut actual = [0; 8];
    owner.pause().unwrap().read(at, &mut actual).unwrap();
    assert_eq!(u64::from_le_bytes(actual), 37);
    owner
        .compare_replace_words(&replacement, &Default::default())
        .unwrap();
    assert!(owner.stack_preflight.get().is_none());
    prime(&mut owner, base, 3 * PAGE);
    owner.pause().unwrap().read(at, &mut actual).unwrap();
    assert_eq!(u64::from_le_bytes(actual), 51);
}

#[test]
fn live_check_precedes_even_a_retained_exact_hit_and_zero_length_noop() {
    let (mut owner, base) = space();
    owner
        .map_at(base, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    prime(&mut owner, base, PAGE);
    assert_eq!(check(&mut owner, u64::MAX, 0), Ok(()));
    assert_eq!(
        owner.stack_preflight.get(),
        Some(NativeRange {
            address: base,
            size: PAGE
        })
    );
    let mut paused = owner.pause().unwrap();
    paused.write(base, b"x").unwrap();
    // Internal fail-stop paths may poison independently of a topology change.
    // Deliberately retain a positive proof to exercise live-before-hit itself.
    paused.owner.poisoned = true;
    assert_eq!(
        paused.preflight_read_write_non_executable(base, PAGE),
        Err(NativeMemoryError::Poisoned)
    );
    assert_eq!(
        paused.preflight_read_write_non_executable(u64::MAX, 0),
        Err(NativeMemoryError::Poisoned)
    );
    assert_eq!(paused.owner.stack_preflight_scans.get(), 1);
}
