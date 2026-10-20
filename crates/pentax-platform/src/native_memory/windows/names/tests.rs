// SPDX-License-Identifier: GPL-2.0-only
use super::*;

fn owner() -> (NativeAddressSpace, u64) {
    let owner = NativeAddressSpace::reserve_any(Default::default(), 4 * NATIVE_GUEST_PAGE).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    (owner, base)
}

fn query(
    owner: &mut NativeAddressSpace,
    point: u64,
    start: u64,
    size: u64,
    default: &[u8],
) -> NativeRangeName {
    owner
        .pause()
        .unwrap()
        .virtual_range_name(
            point,
            NativeRange {
                address: start,
                size,
            },
            default,
            &Default::default(),
        )
        .unwrap()
}

fn rename(owner: &mut NativeAddressSpace, start: u64, size: u64, name: &[u8]) {
    owner
        .pause()
        .unwrap()
        .set_virtual_range_name(start, size, name, &Default::default())
        .unwrap();
}

#[test]
fn virtual_names_keep_byte_boundaries_defaults_payload_and_permissions() {
    let (mut owner, base) = owner();
    let page = NATIVE_GUEST_PAGE;
    owner
        .map_at(base, 3 * page, NativePermissions::READ_WRITE)
        .unwrap();
    owner.pause().unwrap().write(base + 8, b"original").unwrap();
    rename(&mut owner, base + 17, page + 9, b"blue");
    let view = query(&mut owner, base + 20, base, 3 * page, b"initial");
    assert_eq!(view.name(), b"blue");
    assert_eq!(
        view.range,
        NativeRange {
            address: base + 17,
            size: page + 9
        }
    );
    let before = query(&mut owner, base + 16, base, 3 * page, b"initial");
    assert_eq!(before.name(), b"initial");
    assert_eq!(
        before.range,
        NativeRange {
            address: base,
            size: 17
        }
    );
    rename(&mut owner, base + 40, 40, b"");
    let cleared = query(&mut owner, base + 50, base, 3 * page, b"initial");
    assert!(cleared.name().is_empty());
    assert_eq!(
        cleared.range,
        NativeRange {
            address: base + 40,
            size: 40
        }
    );
    rename(&mut owner, base + 40, 40, b"blue");
    assert_eq!(owner.names.len(), 1);
    // Naming does not require payload permissions, nor does it restore them.
    owner
        .protect(
            base + page,
            page,
            NativePermissions::new(false, false, false),
        )
        .unwrap();
    let operations = owner.operations;
    rename(&mut owner, base + page + 5, 7, &[0xff, 0x80]);
    assert_eq!(owner.operations, operations);
    assert!(matches!(
        owner.pause().unwrap().preflight_read(base + page, 1),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    assert_eq!(
        query(&mut owner, base + page + 6, base + page, page, b"initial").name(),
        &[0xff, 0x80]
    );
    let mut actual = [0; 8];
    owner.pause().unwrap().read(base + 8, &mut actual).unwrap();
    assert_eq!(&actual, b"original");
    rename(&mut owner, base, 3 * page, b"initial");
    assert_eq!(
        query(&mut owner, base + 50, base, 3 * page, b"initial").range,
        NativeRange {
            address: base,
            size: 3 * page
        }
    );
}

#[test]
fn virtual_names_ignore_holes_and_do_not_resurrect_after_remapping() {
    let (mut owner, base) = owner();
    let page = NATIVE_GUEST_PAGE;
    owner
        .map_at(base, 3 * page, NativePermissions::READ_WRITE)
        .unwrap();
    rename(&mut owner, base, 3 * page, b"old");
    owner.unmap(base + page, page).unwrap();
    assert!(matches!(
        owner.pause().unwrap().virtual_range_name(
            base + page,
            NativeRange {
                address: base + page,
                size: page
            },
            b"new",
            &Default::default()
        ),
        Err(NativeMemoryError::Unmapped(_))
    ));
    // A rename through a hole labels only actual mapped intersections.
    rename(&mut owner, base, 3 * page, b"edges");
    owner
        .map_at(base + page, page, NativePermissions::READ_WRITE)
        .unwrap();
    assert_eq!(
        query(&mut owner, base + page, base + page, page, b"new").name(),
        b"new"
    );
    assert_eq!(query(&mut owner, base, base, page, b"new").name(), b"edges");
    assert_eq!(
        query(&mut owner, base + 2 * page, base + 2 * page, page, b"new").name(),
        b"edges"
    );
    rename(&mut owner, base + page, page, b"center");
    assert_eq!(owner.names.len(), 3);
    owner.unmap(base, 3 * page).unwrap();
    owner
        .map_at(base, 3 * page, NativePermissions::READ_WRITE)
        .unwrap();
    assert_eq!(
        query(&mut owner, base, base, 3 * page, b"fresh").name(),
        b"fresh"
    );
    rename(&mut owner, base, 3 * page, b"replacement");
    assert_eq!(owner.names.len(), 1); // Stale lifetime records are collected.
    let before = owner.mapped_bytes();
    owner.next_mapping_lifetime = u64::MAX;
    assert_eq!(
        owner.map_at(base + 3 * page, page, NativePermissions::READ_WRITE),
        Err(NativeMemoryError::BudgetExceeded)
    );
    assert_eq!(owner.mapped_bytes(), before);
    assert_eq!(
        query(&mut owner, base, base, 3 * page, b"fresh").name(),
        b"replacement"
    );
}

#[test]
fn virtual_name_admission_failures_leave_all_labels_and_payload_unchanged() {
    let (mut owner, base) = owner();
    let page = NATIVE_GUEST_PAGE;
    owner
        .map_at(base, page, NativePermissions::READ_WRITE)
        .unwrap();
    owner.pause().unwrap().write(base, b"original").unwrap();
    rename(&mut owner, base, page, b"old");
    for limits in [
        NativeNameLimits {
            max_records: 1,
            ..Default::default()
        },
        NativeNameLimits {
            max_work: 0,
            ..Default::default()
        },
        NativeNameLimits {
            max_scratch_bytes: 0,
            ..Default::default()
        },
    ] {
        assert_eq!(
            owner
                .pause()
                .unwrap()
                .set_virtual_range_name(base + 10, 10, b"new", &limits),
            Err(NativeMemoryError::BudgetExceeded)
        );
        assert_eq!(owner.names.len(), 1);
        assert_eq!(query(&mut owner, base + 12, base, page, b"").name(), b"old");
    }
    for name in [&[0x61; 32][..], &b"a\0b"[..]] {
        assert_eq!(
            owner
                .pause()
                .unwrap()
                .set_virtual_range_name(base, page, name, &Default::default()),
            Err(NativeMemoryError::InvalidRange)
        );
    }
    rename(&mut owner, u64::MAX, 0, b"empty");
    rename(&mut owner, u64::MAX, 1, b"unmapped");
    assert_eq!(owner.names.len(), 1);
    // Overflow saturates to MAX instead of wrapping back into low addresses.
    rename(&mut owner, base + 100, u64::MAX, b"tail");
    assert_eq!(query(&mut owner, base + 99, base, page, b"").name(), b"old");
    assert_eq!(
        query(&mut owner, base + 100, base, page, b"").name(),
        b"tail"
    );
    let mut actual = [0; 8];
    owner.pause().unwrap().read(base, &mut actual).unwrap();
    assert_eq!(&actual, b"original");
}

#[test]
fn virtual_names_are_independent_for_shared_physical_aliases_and_release() {
    let (mut owner, base) = owner();
    let page = NATIVE_GUEST_PAGE;
    let apply = |owner: &mut NativeAddressSpace, effect| {
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
    };
    apply(
        &mut owner,
        NativeMemoryEffect::AllocatePhysical {
            physical: 0,
            size: page,
        },
    );
    for address in [base, base + 2 * page] {
        apply(
            &mut owner,
            NativeMemoryEffect::MapPhysical {
                address,
                size: page,
                physical: 0,
                permissions: NativePermissions::READ_WRITE,
            },
        );
    }
    owner.pause().unwrap().write(base, b"shared").unwrap();
    rename(&mut owner, base, page, b"left");
    assert_eq!(
        query(&mut owner, base + 2 * page, base + 2 * page, page, b"right").name(),
        b"right"
    );
    let mut actual = [0; 6];
    owner
        .pause()
        .unwrap()
        .read(base + 2 * page, &mut actual)
        .unwrap();
    assert_eq!(&actual, b"shared");
    apply(
        &mut owner,
        NativeMemoryEffect::ReleasePhysical {
            physical: 0,
            size: page,
        },
    );
    apply(
        &mut owner,
        NativeMemoryEffect::AllocatePhysical {
            physical: 0,
            size: page,
        },
    );
    apply(
        &mut owner,
        NativeMemoryEffect::MapPhysical {
            address: base,
            size: page,
            physical: 0,
            permissions: NativePermissions::READ_WRITE,
        },
    );
    assert_eq!(
        query(&mut owner, base, base, page, b"fresh").name(),
        b"fresh"
    );
    owner.pause().unwrap().read(base, &mut actual).unwrap();
    assert_eq!(actual, [0; 6]);
}
