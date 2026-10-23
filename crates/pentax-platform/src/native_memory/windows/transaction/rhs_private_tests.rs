// SPDX-License-Identifier: GPL-2.0-only
//! Original consumers for shared LEFT and exact private or mixed RHS spans.
use super::*;
const PAGE: u64 = NATIVE_GUEST_PAGE;

fn shared_left_fixture() -> (NativeAddressSpace, NativeRange, NativeRange, NativeRange) {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 340 * PAGE).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner.allocate_physical(0, PAGE).unwrap();
    owner
        .map_physical(base, PAGE, 0, NativePermissions::READ_WRITE)
        .unwrap();
    let a = NativeRange {
        address: base + 4 * PAGE,
        size: 64 * PAGE,
    };
    let b = NativeRange {
        address: base + 72 * PAGE,
        size: 256 * PAGE,
    };
    owner
        .map_at(a.address, a.size, NativePermissions::READ_WRITE)
        .unwrap();
    owner
        .map_at(b.address, b.size, NativePermissions::READ_EXECUTE)
        .unwrap();
    (
        owner,
        NativeRange {
            address: base + 64,
            size: 32,
        },
        a,
        b,
    )
}

#[test]
fn shared_left_and_two_private_rhs_spans_keep_work_fresh_bytes_and_current_permissions() {
    let (mut owner, left, a, b) = shared_left_fixture();
    for _ in 0..3 {
        let mut paused = owner.pause().unwrap();
        assert_eq!(
            paused.preflight_disjoint_backing(left, a, 260).unwrap(),
            260
        );
        assert_eq!(
            paused.preflight_disjoint_backing(left, b, 1028).unwrap(),
            1028
        );
        let transient = NativeRange {
            address: left.address + 128,
            size: 8,
        };
        assert_eq!(
            paused
                .preflight_disjoint_backing(left, transient, 8)
                .unwrap(),
            8
        );
        paused.write(a.address + a.size - 8, b"current!").unwrap();
        let mut bytes = [0; 8];
        paused.read(a.address + a.size - 8, &mut bytes).unwrap();
        assert_eq!(&bytes, b"current!");
        assert!(matches!(
            paused.preflight_fetch_source(a.address, 1),
            Err(NativeMemoryError::PermissionDenied(_))
        ));
        assert!(matches!(
            paused.preflight_disjoint_backing(left, a, 259),
            Err(NativeMemoryError::BudgetExceeded)
        ));
        let overlapping = NativeRange {
            address: a.address + PAGE,
            size: 32,
        };
        assert!(matches!(
            paused.preflight_disjoint_backing(overlapping, a, u64::MAX),
            Err(NativeMemoryError::Overlap)
        ));
    }
    owner
        .protect(
            a.address,
            a.size,
            NativePermissions::new(false, false, false),
        )
        .unwrap();
    let mut paused = owner.pause().unwrap();
    assert_eq!(
        paused.preflight_disjoint_backing(left, a, 260).unwrap(),
        260
    );
    assert!(matches!(
        paused.preflight_read(a.address, 1),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    assert!(matches!(
        paused.write(a.address, b"x"),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    paused.owner.poisoned = true;
    assert!(matches!(
        paused.preflight_disjoint_backing(left, a, 260),
        Err(NativeMemoryError::Poisoned)
    ));
    assert!(matches!(
        paused.preflight_disjoint_backing(
            NativeRange {
                address: 0,
                size: 0
            },
            a,
            0
        ),
        Err(NativeMemoryError::Poisoned)
    ));
}

#[test]
fn late_shared_rhs_page_revokes_private_proof_and_actual_aliases_remain_refused() {
    let (mut owner, left, large, _) = shared_left_fixture();
    let right = NativeRange {
        address: large.address,
        size: 2 * PAGE,
    };
    assert_eq!(
        owner
            .pause()
            .unwrap()
            .preflight_disjoint_backing(left, right, 12)
            .unwrap(),
        12
    );
    owner.unmap(right.address + PAGE, PAGE).unwrap();
    assert!(matches!(
        owner
            .pause()
            .unwrap()
            .preflight_disjoint_backing(left, right, 12),
        Err(NativeMemoryError::Unmapped(_))
    ));
    owner
        .map_physical(right.address + PAGE, PAGE, 0, NativePermissions::READ_WRITE)
        .unwrap();
    let mut paused = owner.pause().unwrap();
    paused.write(left.address, b"actual alias").unwrap();
    let mut bytes = [0; 12];
    paused.read(right.address + PAGE + 64, &mut bytes).unwrap();
    assert_eq!(&bytes, b"actual alias");
    for _ in 0..2 {
        assert!(matches!(
            paused.preflight_disjoint_backing(left, right, 11),
            Err(NativeMemoryError::BudgetExceeded)
        ));
        assert!(matches!(
            paused.preflight_disjoint_backing(left, right, 12),
            Err(NativeMemoryError::Overlap)
        ));
    }
    owner.unmap(left.address / PAGE * PAGE, PAGE).unwrap();
    assert!(matches!(
        owner
            .pause()
            .unwrap()
            .preflight_disjoint_backing(left, right, 12),
        Err(NativeMemoryError::Unmapped(_))
    ));
}
