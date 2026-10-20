// SPDX-License-Identifier: GPL-2.0-only
//! Original real placeholder/backing proof, without GPU or guest execution.
use super::*;

#[test]
fn constrained_reservation_keeps_full_extent_owner_zero_commit_and_real_mapping_rights() {
    let size = 2 * 65536;
    let window = NativeRange {
        address: 65536,
        size: (1 << 40) - 65536,
    };
    let limits = NativeMemoryLimits {
        mapped_bytes: NATIVE_GUEST_PAGE,
        reserved_bytes: size,
    };
    let mut owner = NativeAddressSpace::reserve_within(limits, size, window).unwrap();
    let range = owner.reserved_ranges().next().unwrap();
    assert_eq!(owner.reserved_ranges().count(), 1);
    assert_eq!(range.size, size);
    assert!(range.address >= window.address);
    assert!(range.address + range.size <= window.address + window.size);
    assert!(range.address.is_multiple_of(owner.allocation_granularity()));
    assert_eq!(
        (
            owner.reserved_bytes(),
            owner.mapped_bytes(),
            owner.committed_bytes(),
            owner.backing_bytes()
        ),
        (size, 0, 0, 0)
    );
    let identity = owner.owner_id();
    let other = NativeAddressSpace::reserve_within(limits, size, window).unwrap();
    assert_ne!(other.owner_id(), identity);
    let other_range = other.reserved_ranges().next().unwrap();
    assert!(
        range.address + size <= other_range.address || other_range.address + size <= range.address
    );
    let at = range.address + NATIVE_GUEST_PAGE;
    assert!(matches!(
        owner.pause().unwrap().preflight_gpu_read(at, 8),
        Err(NativeMemoryError::Unmapped(_))
    ));
    owner
        .map_at(
            at,
            NATIVE_GUEST_PAGE,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    assert_eq!(owner.owner_id(), identity);
    {
        let mut memory = owner.pause().unwrap();
        memory.write(at, b"owned-frame").unwrap();
        let mut bytes = [0; 11];
        memory.read_gpu(at, &mut bytes).unwrap();
        assert_eq!(&bytes, b"owned-frame");
        assert!(matches!(
            memory.preflight_fetch_source(at, 8),
            Err(NativeMemoryError::PermissionDenied(_))
        ));
    }
    assert_eq!(
        (
            owner.reserved_bytes(),
            owner.mapped_bytes(),
            owner.committed_bytes()
        ),
        (size, NATIVE_GUEST_PAGE, NATIVE_GUEST_PAGE)
    );
    owner
        .protect(
            at,
            NATIVE_GUEST_PAGE,
            NativePermissions::READ.with_gpu(true, false),
        )
        .unwrap();
    assert!(matches!(
        owner.pause().unwrap().preflight_gpu_write(at, 8),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    owner.unmap(at, NATIVE_GUEST_PAGE).unwrap();
    assert_eq!(
        (
            owner.reserved_bytes(),
            owner.mapped_bytes(),
            owner.committed_bytes()
        ),
        (size, 0, 0)
    );
    drop(owner);
    LAST_RELEASE.with(|last| assert_eq!(last.get(), Some((range.address, true))));
    assert_eq!(other.reserved_bytes(), size); // Distinct live owner unchanged.
}
