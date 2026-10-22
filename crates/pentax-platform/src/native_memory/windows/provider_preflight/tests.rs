// SPDX-License-Identifier: GPL-2.0-only
//! Original real-owner permission regressions; no guest execution or title data.
use super::*;
const PAGE: u64 = NATIVE_GUEST_PAGE;
fn space() -> (NativeAddressSpace, u64) {
    let owner = NativeAddressSpace::reserve_any(Default::default(), 0x40000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    (owner, base)
}
fn mapped() -> (NativeAddressSpace, u64) {
    let (mut owner, base) = space();
    owner
        .map_at(base, 5 * PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    owner
        .pause()
        .unwrap()
        .write(base, &37_u64.to_le_bytes())
        .unwrap();
    owner
        .protect(base, 2 * PAGE, NativePermissions::READ_EXECUTE)
        .unwrap();
    owner
        .protect(base + 2 * PAGE, 2 * PAGE, NativePermissions::READ)
        .unwrap();
    (owner, base)
}
fn prime(owner: &mut NativeAddressSpace, base: u64) {
    let paused = owner.pause().unwrap();
    paused.preflight_provider_code(base, 2 * PAGE).unwrap();
    paused
        .preflight_provider_metadata(base + 2 * PAGE, 2 * PAGE)
        .unwrap();
}
fn empty(owner: &NativeAddressSpace) {
    assert_eq!(owner.provider_preflight.get(), [None; 2]);
}

#[test]
fn exact_positive_profiles_persist_across_pauses_without_caching_payload_or_foreign_pages() {
    let (mut owner, base) = mapped();
    prime(&mut owner, base);
    for value in 1..=8_u8 {
        let mut paused = owner.pause().unwrap();
        paused.write(base + 4 * PAGE, &[value]).unwrap();
        paused.preflight_provider_code(base, 2 * PAGE).unwrap();
        paused
            .preflight_provider_metadata(base + 2 * PAGE, 2 * PAGE)
            .unwrap();
        let mut actual = [0];
        paused.read(base + 4 * PAGE, &mut actual).unwrap();
        assert_eq!(actual, [value]);
    }
    assert_eq!(owner.provider_preflight_scans.get(), [1, 1]);
    owner
        .pause()
        .unwrap()
        .preflight_provider_code(base + 1, PAGE)
        .unwrap();
    prime(&mut owner, base);
    assert_eq!(owner.provider_preflight_scans.get(), [3, 1]);
    let (mut other, other_base) = mapped();
    assert_ne!(owner.owner_id(), other.owner_id());
    assert_eq!(
        other
            .pause()
            .unwrap()
            .preflight_provider_code(base, 2 * PAGE),
        Err(NativeMemoryError::Unmapped(base))
    );
    prime(&mut other, other_base);
    assert_eq!(other.provider_preflight_scans.get(), [2, 1]);
    assert_eq!(owner.provider_preflight_scans.get(), [3, 1]);
}

#[test]
fn changed_late_page_rights_and_same_address_remap_cannot_reuse_old_provider_permissions() {
    let (mut owner, base) = mapped();
    prime(&mut owner, base);
    for rights in [
        NativePermissions::READ,
        NativePermissions::READ_WRITE,
        NativePermissions::new(true, true, true),
    ] {
        owner.protect(base + PAGE, PAGE, rights).unwrap();
        empty(&owner);
        assert_eq!(
            owner
                .pause()
                .unwrap()
                .preflight_provider_code(base, 2 * PAGE),
            Err(NativeMemoryError::PermissionDenied(base + PAGE))
        );
        owner
            .protect(base + PAGE, PAGE, NativePermissions::READ_EXECUTE)
            .unwrap();
        prime(&mut owner, base);
    }
    for rights in [
        NativePermissions::READ_WRITE,
        NativePermissions::READ_EXECUTE,
        NativePermissions::READ.with_gpu(false, true),
    ] {
        owner.protect(base + 3 * PAGE, PAGE, rights).unwrap();
        empty(&owner);
        assert_eq!(
            owner
                .pause()
                .unwrap()
                .preflight_provider_metadata(base + 2 * PAGE, 2 * PAGE),
            Err(NativeMemoryError::PermissionDenied(base + 3 * PAGE))
        );
        owner
            .protect(base + 3 * PAGE, PAGE, NativePermissions::READ)
            .unwrap();
        prime(&mut owner, base);
    }
    owner.unmap(base + PAGE, PAGE).unwrap();
    empty(&owner);
    assert_eq!(
        owner
            .pause()
            .unwrap()
            .preflight_provider_code(base, 2 * PAGE),
        Err(NativeMemoryError::Unmapped(base + PAGE))
    );
    owner
        .map_at(base + PAGE, PAGE, NativePermissions::READ)
        .unwrap();
    empty(&owner);
    assert_eq!(
        owner
            .pause()
            .unwrap()
            .preflight_provider_code(base, 2 * PAGE),
        Err(NativeMemoryError::PermissionDenied(base + PAGE))
    );
    owner
        .protect(base + PAGE, PAGE, NativePermissions::READ_EXECUTE)
        .unwrap();
    prime(&mut owner, base);
}

#[test]
fn failed_direct_and_transaction_mutations_clear_proofs_before_preflight_or_rollback() {
    let (mut owner, base) = mapped();
    prime(&mut owner, base);
    assert_eq!(
        owner.protect(base + 1, PAGE, NativePermissions::READ),
        Err(NativeMemoryError::Unaligned)
    );
    empty(&owner);
    prime(&mut owner, base);
    assert!(owner.map_at(base, PAGE, NativePermissions::READ).is_err());
    empty(&owner);
    prime(&mut owner, base);
    assert_eq!(
        owner.pause().unwrap().apply_transaction(
            NativeMemoryTransaction {
                effect: NativeMemoryEffect::Protect {
                    address: base + 1,
                    size: PAGE,
                    permissions: NativePermissions::READ
                },
                writes: &[],
            },
            &Default::default()
        ),
        Err(NativeMemoryError::Unaligned)
    );
    empty(&owner);
    prime(&mut owner, base);
    owner.operations = 0;
    owner.gate = Some(1);
    assert!(matches!(
        owner.protect(base, 2 * PAGE, NativePermissions::READ),
        Err(NativeMemoryError::OperatingSystem { .. })
    ));
    assert!(!owner.poisoned);
    empty(&owner);
    owner.gate = None;
    prime(&mut owner, base);
    let mut marker = [0; 8];
    owner
        .pause()
        .unwrap()
        .fetch_source(base, &mut marker)
        .unwrap();
    assert_eq!(u64::from_le_bytes(marker), 37);
}

#[test]
fn loader_replacement_and_physical_view_release_retire_provider_proofs() {
    let (mut owner, base) = mapped();
    prime(&mut owner, base);
    owner
        .compare_replace_words(
            &[NativeWordReplacement {
                address: base,
                expected: 37,
                replacement: 51,
            }],
            &Default::default(),
        )
        .unwrap();
    empty(&owner);
    prime(&mut owner, base);
    let mut marker = [0; 8];
    owner
        .pause()
        .unwrap()
        .fetch_source(base, &mut marker)
        .unwrap();
    assert_eq!(u64::from_le_bytes(marker), 51);
    assert!(
        owner
            .compare_replace_words(
                &[NativeWordReplacement {
                    address: base,
                    expected: 37,
                    replacement: 99
                }],
                &Default::default()
            )
            .is_err()
    );
    empty(&owner);
    prime(&mut owner, base);
    owner.unmap(base + 2 * PAGE, 2 * PAGE).unwrap();
    owner.allocate_physical(0, 2 * PAGE).unwrap();
    owner
        .map_physical(base + 2 * PAGE, 2 * PAGE, 0, NativePermissions::READ)
        .unwrap();
    prime(&mut owner, base);
    owner.release_physical(0, 2 * PAGE, 1024 * 1024).unwrap();
    empty(&owner);
    assert_eq!(
        owner
            .pause()
            .unwrap()
            .preflight_provider_metadata(base + 2 * PAGE, 2 * PAGE),
        Err(NativeMemoryError::Unmapped(base + 2 * PAGE))
    );
}

#[test]
fn retained_hits_still_require_a_live_owner_and_revocation_never_resurrects_them() {
    let (mut owner, base) = mapped();
    prime(&mut owner, base);
    {
        let paused = owner.pause().unwrap();
        paused.owner.poisoned = true; // Exercise live-before-hit without clearing the positive slots.
        assert_eq!(
            paused.preflight_provider_code(base, 2 * PAGE),
            Err(NativeMemoryError::Poisoned)
        );
        assert_eq!(
            paused.preflight_provider_metadata(base + 2 * PAGE, 2 * PAGE),
            Err(NativeMemoryError::Poisoned)
        );
        assert_eq!(
            paused.preflight_provider_code(u64::MAX, 0),
            Err(NativeMemoryError::Poisoned)
        );
        assert_eq!(paused.owner.provider_preflight_scans.get(), [1, 1]);
    }
    owner.abandon();
    empty(&owner);
    assert!(matches!(owner.pause(), Err(NativeMemoryError::Poisoned)));
}
