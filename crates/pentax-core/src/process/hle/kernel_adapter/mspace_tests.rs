// SPDX-License-Identifier: GPL-2.0-only
// Original PentaX tests over actual native backing; no guest code is run.

use super::*;
use pentax_platform::native_memory::NativeAddressSpace;

const PAGE: u64 = 0x4000;

fn adapter<'borrow, 'owner>(
    paused: &'borrow mut PausedNativeMemory<'owner>,
) -> KernelMemory<'borrow, 'owner, 'static> {
    KernelMemory {
        paused,
        source: None,
        supplied_modules: 0,
        lookup_limits: None,
        lookup_work: None,
        loaded_modules: None,
        thread_tls: None,
        initial_entry: None,
    }
}

#[test]
fn heap_arena_proof_checks_whole_unaligned_span_without_changing_bytes_or_backing() {
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 4 * PAGE).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base, 3 * PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let marker = [0xb7; 96];
    {
        let mut paused = space.pause().unwrap();
        paused.write(base + 16, &marker).unwrap();
        paused.write(base + 3 * PAGE - 96, &marker).unwrap();
    }
    let before = space.statistics();
    {
        let mut paused = space.pause().unwrap();
        let guest = adapter(&mut paused);
        // The size covers three pages although its start is not page aligned.
        assert_eq!(guest.preflight_heap_arena(base + 16, 3 * PAGE - 32), Ok(()));
        for address in [base + 16, base + 3 * PAGE - 96] {
            let mut actual = [0; 96];
            guest.read(address, &mut actual).unwrap();
            assert_eq!(actual, marker);
        }
        assert_eq!(guest.preflight_heap_arena(u64::MAX, 0), Ok(()));
        assert_eq!(
            guest.preflight_heap_arena(u64::MAX - 15, 96),
            Err(GuestError::AddressOverflow)
        );
    }
    assert_eq!(space.statistics(), before);
}

#[test]
fn heap_arena_proof_refuses_each_missing_permission_including_late_executable_page() {
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 4 * PAGE).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base, 2 * PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    for (permissions, expected) in [
        (NativePermissions::READ, GuestError::WriteDenied),
        (
            NativePermissions::new(false, true, false),
            GuestError::ReadDenied,
        ),
        (
            NativePermissions::new(true, true, true),
            GuestError::ReadDenied,
        ),
    ] {
        space.protect(base + PAGE, PAGE, permissions).unwrap();
        let before = space.statistics();
        {
            let mut paused = space.pause().unwrap();
            let guest = adapter(&mut paused);
            assert_eq!(
                guest.preflight_heap_arena(base + 16, 2 * PAGE - 32),
                Err(expected)
            );
            assert_eq!(guest.preflight_heap_arena(base + 16, 96), Ok(()));
        }
        assert_eq!(space.statistics(), before);
    }
    space
        .protect(base + PAGE, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let mut paused = space.pause().unwrap();
    assert_eq!(
        adapter(&mut paused).preflight_heap_arena(base + 16, 2 * PAGE - 32),
        Ok(())
    );
}

#[test]
fn heap_arena_proof_never_fills_sparse_holes_or_accepts_another_owner() {
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 4 * PAGE).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    for address in [base, base + 2 * PAGE] {
        space
            .map_at(address, PAGE, NativePermissions::READ_WRITE)
            .unwrap();
    }
    let before = space.statistics();
    {
        let mut paused = space.pause().unwrap();
        let guest = adapter(&mut paused);
        assert_eq!(
            guest.preflight_heap_arena(base + 16, 3 * PAGE - 32),
            Err(GuestError::Unmapped)
        );
        assert_eq!(
            guest.preflight_heap_arena(base + PAGE - 16, 96),
            Err(GuestError::Unmapped)
        );
        assert_eq!(guest.preflight_heap_arena(base + 2 * PAGE, PAGE), Ok(()));
        assert_eq!(
            guest.preflight_heap_arena(base + 3 * PAGE, 96),
            Err(GuestError::Unmapped)
        );
    }
    assert_eq!(space.statistics(), before);
    let mut other = NativeAddressSpace::reserve_any(Default::default(), 4 * PAGE).unwrap();
    let other_base = other.reserved_ranges().next().unwrap().address;
    other
        .map_at(other_base, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let other_before = other.statistics();
    {
        let mut paused = other.pause().unwrap();
        let guest = adapter(&mut paused);
        assert_eq!(
            guest.preflight_heap_arena(base, 96),
            Err(GuestError::Unmapped)
        );
        assert_eq!(guest.preflight_heap_arena(other_base, 96), Ok(()));
    }
    assert_eq!(other.statistics(), other_before);
    assert_eq!(space.statistics(), before);
}
