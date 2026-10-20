// SPDX-License-Identifier: GPL-2.0-only
use super::*;
use windows_sys::Win32::System::Memory::{
    MEM_RESERVE, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
    PAGE_EXECUTE_WRITECOPY, VirtualQuery,
};

fn space() -> NativeAddressSpace {
    NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x30000).unwrap()
}
fn base(space: &NativeAddressSpace) -> u64 {
    space.reserved_ranges().next().unwrap().address
}

#[test]
fn ordered_span_checks_keep_holes_permissions_and_logical_nx_distinct() {
    let mut owner = space();
    let b = base(&owner);
    let page = NATIVE_GUEST_PAGE;
    owner
        .map_at(b, page, NativePermissions::READ_WRITE)
        .unwrap();
    owner
        .map_at(b + 2 * page, page, NativePermissions::READ_WRITE)
        .unwrap();
    {
        let paused = owner.pause().unwrap();
        for result in [
            paused.preflight_read(b + 7, 2 * page),
            paused.preflight_write(b + 7, 2 * page),
            paused.preflight_non_executable(b + 7, 2 * page),
        ] {
            assert_eq!(result, Err(NativeMemoryError::Unmapped(b + page)));
        }
        assert_eq!(paused.preflight_non_executable(b + 2 * page, page), Ok(()));
    }
    owner
        .map_at(b + page, page, NativePermissions::READ_WRITE)
        .unwrap();
    owner
        .protect(b + 2 * page, page, NativePermissions::new(true, true, true))
        .unwrap();
    {
        let paused = owner.pause().unwrap();
        assert_eq!(paused.preflight_read(b + 7, 3 * page - 14), Ok(()));
        assert_eq!(paused.preflight_write(b + 7, 3 * page - 14), Ok(()));
        assert_eq!(
            paused.preflight_non_executable(b + 7, 3 * page - 14),
            Err(NativeMemoryError::PermissionDenied(b + 2 * page))
        );
    }
    owner
        .protect(b + page, page, NativePermissions::new(false, false, false))
        .unwrap();
    {
        let paused = owner.pause().unwrap();
        assert_eq!(
            paused.preflight_read(b + 7, 2 * page),
            Err(NativeMemoryError::PermissionDenied(b + page))
        );
        // NX alone grants neither read nor write permission.
        assert_eq!(paused.preflight_non_executable(b + page, page), Ok(()));
        assert_eq!(paused.preflight_non_executable(u64::MAX, 0), Ok(()));
        assert_eq!(
            paused.preflight_non_executable(u64::MAX, 1),
            Err(NativeMemoryError::InvalidRange)
        );
    }
    owner.abandon();
    assert_eq!(
        owner.check_permissions(0, 0, None, 4),
        Err(NativeMemoryError::Poisoned)
    );
}
#[test]
fn combined_stack_preflight_checks_live_full_spans_cpu_rights_holes_and_poison() {
    let mut owner = space();
    let b = base(&owner);
    let page = NATIVE_GUEST_PAGE;
    let address = b + 7;
    let length = 3 * page - 14;
    owner
        .map_at(b, page, NativePermissions::READ_WRITE)
        .unwrap();
    owner
        .map_at(b + 2 * page, page, NativePermissions::READ_WRITE)
        .unwrap();
    owner.pause().unwrap().write(b + 7, b"keep").unwrap();
    assert_eq!(
        owner
            .pause()
            .unwrap()
            .preflight_read_write_non_executable(address, length),
        Err(NativeMemoryError::Unmapped(b + page))
    );
    owner
        .map_at(b + page, page, NativePermissions::READ_WRITE)
        .unwrap();
    // GPU RW makes physical backing writable even when logical CPU rights fail.
    // It must neither grant CPU access nor disqualify an otherwise valid stack.
    for (permissions, accepted) in [
        (NativePermissions::READ_WRITE, true),
        (NativePermissions::READ_WRITE.with_gpu(true, true), true),
        (NativePermissions::READ.with_gpu(false, true), false),
        (
            NativePermissions::new(false, true, false).with_gpu(true, false),
            false,
        ),
        (
            NativePermissions::new(false, false, false).with_gpu(true, true),
            false,
        ),
        (NativePermissions::new(false, false, false), false),
        (
            NativePermissions::new(true, true, true).with_gpu(true, true),
            false,
        ),
    ] {
        owner.protect(b + page, page, permissions).unwrap();
        let operations = owner.operations;
        {
            let paused = owner.pause().unwrap();
            let expected = if accepted {
                Ok(())
            } else {
                Err(NativeMemoryError::PermissionDenied(b + page))
            };
            assert_eq!(
                paused.preflight_read_write_non_executable(address, length),
                expected
            );
            assert_eq!(
                paused.preflight_read_write_non_executable(b + page - 1, 2),
                expected
            );
            assert_eq!(
                paused.preflight_read_write_non_executable(b + 7, page - 14),
                Ok(())
            );
        }
        assert_eq!(owner.operations, operations);
    }
    owner
        .protect(b + page, page, NativePermissions::READ_WRITE)
        .unwrap();
    owner
        .protect(b + 2 * page, page, NativePermissions::READ)
        .unwrap();
    assert_eq!(
        owner
            .pause()
            .unwrap()
            .preflight_read_write_non_executable(address, length),
        Err(NativeMemoryError::PermissionDenied(b + 2 * page))
    );
    owner
        .protect(b + 2 * page, page, NativePermissions::READ_WRITE)
        .unwrap();
    owner.unmap(b + page, page).unwrap();
    {
        let paused = owner.pause().unwrap();
        assert_eq!(
            paused.preflight_read_write_non_executable(address, length),
            Err(NativeMemoryError::Unmapped(b + page))
        );
        assert_eq!(
            paused.preflight_read_write_non_executable(u64::MAX, 0),
            Ok(())
        );
        assert_eq!(
            paused.preflight_read_write_non_executable(u64::MAX, 2),
            Err(NativeMemoryError::InvalidRange)
        );
        let mut original = [0; 4];
        paused.read(b + 7, &mut original).unwrap();
        assert_eq!(&original, b"keep");
    }
    owner.abandon();
    // Private liveness seam: the public pause API already refuses this owner.
    let paused = PausedNativeMemory { owner: &mut owner };
    for (address, size) in [(b, page), (u64::MAX, 0), (u64::MAX, 2)] {
        assert_eq!(
            paused.preflight_read_write_non_executable(address, size),
            Err(NativeMemoryError::Poisoned)
        );
    }
}

fn query(address: u64) -> MEMORY_BASIC_INFORMATION {
    let mut info = MEMORY_BASIC_INFORMATION::default();
    // SAFETY: Valid writable output and exact structure size. VirtualQuery
    // inspects the numeric address without dereferencing guest memory.
    let written = unsafe {
        VirtualQuery(
            address as usize as *const c_void,
            &mut info,
            std::mem::size_of_val(&info),
        )
    };
    assert_ne!(written, 0);
    info
}

#[test]
fn gpu_permissions_are_independent_of_cpu_permissions_and_all_host_backing_stays_nx() {
    let mut owner = space();
    let b = base(&owner);
    owner
        .map_at(b, NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    for cpu in 0..8 {
        for gpu in 0..4 {
            let permissions = NativePermissions::new(cpu & 1 != 0, cpu & 2 != 0, cpu & 4 != 0)
                .with_gpu(gpu & 1 != 0, gpu & 2 != 0);
            owner.protect(b, NATIVE_GUEST_PAGE, permissions).unwrap();
            let expected = if cpu & 2 != 0 || gpu & 2 != 0 {
                PAGE_READWRITE
            } else if cpu & 5 != 0 || gpu & 1 != 0 {
                PAGE_READONLY
            } else {
                PAGE_NOACCESS
            };
            assert_eq!(query(b).Protect, expected);
            let mut paused = owner.pause().unwrap();
            assert_eq!(paused.read(b, &mut [0]).is_ok(), cpu & 1 != 0);
            assert_eq!(paused.write(b, &[13]).is_ok(), cpu & 2 != 0);
            assert_eq!(paused.fetch_source(b, &mut [0]).is_ok(), cpu & 4 != 0);
            assert_eq!(paused.read_gpu(b, &mut [0]).is_ok(), gpu & 1 != 0);
            assert_eq!(
                paused
                    .apply_gpu_writes(
                        &[NativeMemoryWrite {
                            address: b,
                            bytes: &[27]
                        }],
                        &Default::default()
                    )
                    .is_ok(),
                gpu & 2 != 0
            );
            assert_eq!(
                paused.snapshot_gpu_buffer(b, &mut [0], true, true).is_ok(),
                gpu == 3
            );
            assert_eq!(
                paused.snapshot_gpu_buffer(b, &mut [0], false, true).is_ok(),
                gpu & 2 != 0
            );
        }
    }
    owner.abandon();
    assert!(matches!(owner.pause(), Err(NativeMemoryError::Poisoned)));
}

#[test]
fn final_owner_drop_observes_actual_release_without_querying_reused_addresses() {
    let s = space();
    let address = base(&s);
    LAST_RELEASE.with(|slot| slot.set(None));
    drop(s);
    assert_eq!(
        LAST_RELEASE.with(std::cell::Cell::get),
        Some((address, true))
    );
}
#[test]
fn actual_os_pages_share_checked_cross_page_reads_and_writes() {
    let mut s = space();
    let b = base(&s) + NATIVE_GUEST_PAGE;
    s.map_at(b, 2 * NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let mut paused = s.pause().unwrap();
    let at = b + NATIVE_GUEST_PAGE - 3;
    paused.write(at, b"native backing").unwrap();
    let mut got = [0; 14];
    paused.read(at, &mut got).unwrap();
    assert_eq!(&got, b"native backing");
    assert!(matches!(
        paused.read(b - 1, &mut [0]),
        Err(NativeMemoryError::Unmapped(_))
    ));
    assert_eq!(query(b - NATIVE_GUEST_PAGE).State, MEM_RESERVE);
    assert_eq!(s.committed_bytes(), 2 * NATIVE_GUEST_PAGE);
}

#[test]
fn decommit_retains_address_owner_and_neighbors_and_recommit_supplies_zero_backing() {
    let mut s = space();
    let b = base(&s);
    let owner = s.owner_id();
    let reserved = s.reserved_bytes();
    s.map_at(b, 3 * NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    s.pause().unwrap().write(b, &[11]).unwrap();
    s.pause()
        .unwrap()
        .write(b + NATIVE_GUEST_PAGE, &[77; 8])
        .unwrap();
    s.pause()
        .unwrap()
        .write(b + 2 * NATIVE_GUEST_PAGE, &[13])
        .unwrap();
    s.unmap(b + NATIVE_GUEST_PAGE, NATIVE_GUEST_PAGE).unwrap();
    assert_eq!(s.owner_id(), owner);
    assert_eq!(s.reserved_bytes(), reserved);
    assert_eq!(s.mapped_bytes(), 2 * NATIVE_GUEST_PAGE);
    assert_eq!(query(b + NATIVE_GUEST_PAGE).State, MEM_RESERVE);
    let mut bytes = [0];
    s.pause().unwrap().read(b, &mut bytes).unwrap();
    assert_eq!(bytes, [11]);
    s.pause()
        .unwrap()
        .read(b + 2 * NATIVE_GUEST_PAGE, &mut bytes)
        .unwrap();
    assert_eq!(bytes, [13]);
    s.pause()
        .unwrap()
        .preflight_reserved_unmapped(b + NATIVE_GUEST_PAGE, NATIVE_GUEST_PAGE)
        .unwrap();
    s.map_at(
        b + NATIVE_GUEST_PAGE,
        NATIVE_GUEST_PAGE,
        NativePermissions::READ_WRITE,
    )
    .unwrap();
    let mut fresh = [77; 8];
    s.pause()
        .unwrap()
        .read(b + NATIVE_GUEST_PAGE, &mut fresh)
        .unwrap();
    assert_eq!(fresh, [0; 8]);
}

#[test]
fn decommit_preflight_and_first_failure_preserve_backing_but_partial_failure_poison_is_fatal() {
    let mut s = space();
    let b = base(&s);
    s.map_at(b, 2 * NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    s.pause().unwrap().write(b, &[31]).unwrap();
    assert!(matches!(
        s.unmap(b, 3 * NATIVE_GUEST_PAGE),
        Err(NativeMemoryError::Unmapped(_))
    ));
    s.operations = 0;
    s.gate = Some(0);
    assert!(matches!(
        s.unmap(b, 2 * NATIVE_GUEST_PAGE),
        Err(NativeMemoryError::OperatingSystem { .. })
    ));
    let mut retained = [0];
    s.pause().unwrap().read(b, &mut retained).unwrap();
    assert_eq!(retained, [31]);
    s.operations = 0;
    s.gate = Some(1);
    assert_eq!(
        s.unmap(b, 2 * NATIVE_GUEST_PAGE),
        Err(NativeMemoryError::Poisoned)
    );
    assert_eq!(query(b).State, MEM_RESERVE);
    assert!(matches!(s.pause(), Err(NativeMemoryError::Poisoned)));
    assert_eq!(
        s.map_at(b, NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE),
        Err(NativeMemoryError::Poisoned)
    );
    assert_eq!(
        s.unmap(b + NATIVE_GUEST_PAGE, NATIVE_GUEST_PAGE),
        Err(NativeMemoryError::Poisoned)
    );
}
#[test]
fn all_backing_stays_os_nx_including_logical_executable_source() {
    let mut s = space();
    let b = base(&s);
    s.map_at(b, NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    s.pause().unwrap().write(b, &[0xc3]).unwrap();
    s.protect(b, NATIVE_GUEST_PAGE, NativePermissions::READ_EXECUTE)
        .unwrap();
    let mut p = s.pause().unwrap();
    let mut byte = [0];
    p.fetch_source(b, &mut byte).unwrap();
    assert_eq!(byte, [0xc3]);
    assert!(matches!(
        p.write(b, &[0x90]),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    let info = query(b);
    assert_eq!(info.Protect, PAGE_READONLY);
    assert_eq!(
        info.Protect
            & (PAGE_EXECUTE | PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY),
        0
    );
}
#[test]
fn complete_span_permission_or_hole_fault_preserves_bytes_and_read_output() {
    let mut s = space();
    let b = base(&s);
    s.map_at(b, 2 * NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let at = b + NATIVE_GUEST_PAGE - 1;
    s.pause().unwrap().write(at, b"AB").unwrap();
    s.protect(
        b + NATIVE_GUEST_PAGE,
        NATIVE_GUEST_PAGE,
        NativePermissions::READ,
    )
    .unwrap();
    let mut p = s.pause().unwrap();
    assert!(matches!(
        p.write(at, b"xy"),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    let mut got = [0; 2];
    p.read(at, &mut got).unwrap();
    assert_eq!(got, *b"AB");
    let mut untouched = [7; 2];
    assert!(
        p.read(b + 2 * NATIVE_GUEST_PAGE - 1, &mut untouched)
            .is_err()
    );
    assert_eq!(untouched, [7; 2]);
}
#[test]
fn failed_late_commit_rolls_back_only_new_pages_and_preserves_existing_data() {
    let mut s = space();
    let b = base(&s);
    s.map_at(b, NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    s.pause().unwrap().write(b, b"keep").unwrap();
    s.operations = 0;
    s.gate = Some(1);
    assert!(
        s.map_at(
            b + NATIVE_GUEST_PAGE,
            2 * NATIVE_GUEST_PAGE,
            NativePermissions::READ_WRITE
        )
        .is_err()
    );
    assert_eq!(s.mapped_bytes(), NATIVE_GUEST_PAGE);
    assert_eq!(query(b + NATIVE_GUEST_PAGE).State, MEM_RESERVE);
    let mut got = [0; 4];
    s.pause().unwrap().read(b, &mut got).unwrap();
    assert_eq!(&got, b"keep");
    s.gate = None;
    s.map_at(
        b + NATIVE_GUEST_PAGE,
        2 * NATIVE_GUEST_PAGE,
        NativePermissions::READ_WRITE,
    )
    .unwrap();
    s.pause()
        .unwrap()
        .read(b + NATIVE_GUEST_PAGE, &mut got)
        .unwrap();
    assert_eq!(got, [0; 4]);
}
#[test]
fn failed_late_protect_restores_actual_os_and_logical_permissions() {
    let mut s = space();
    let b = base(&s);
    s.map_at(b, 2 * NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    s.operations = 0;
    s.gate = Some(1);
    assert!(
        s.protect(b, 2 * NATIVE_GUEST_PAGE, NativePermissions::READ)
            .is_err()
    );
    assert_eq!(query(b).Protect, PAGE_READWRITE);
    s.pause()
        .unwrap()
        .write(b + NATIVE_GUEST_PAGE - 1, b"ok")
        .unwrap();
}
#[test]
fn fixed_guest_mapping_cannot_replace_another_owned_host_reservation() {
    let mut original = space();
    let b = base(&original);
    original
        .map_at(b, NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    original.pause().unwrap().write(b, b"keep").unwrap();
    let mut other = NativeAddressSpace::new(NativeMemoryLimits::default()).unwrap();
    assert!(
        other
            .map_at(b, NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
            .is_err()
    );
    assert_eq!(other.mapped_bytes(), 0);
    assert_eq!(other.reserved_bytes(), 0);
    let mut got = [0; 4];
    original.pause().unwrap().read(b, &mut got).unwrap();
    assert_eq!(&got, b"keep");
}
#[test]
fn bounds_budgets_overlap_and_empty_accesses_are_checked_before_os_mutation() {
    let mut s = space();
    let b = base(&s);
    s.map_at(b, NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    assert!(matches!(
        s.map_at(b, NATIVE_GUEST_PAGE, NativePermissions::READ),
        Err(NativeMemoryError::Overlap)
    ));
    assert!(matches!(
        s.map_at(b + 1, NATIVE_GUEST_PAGE, NativePermissions::READ),
        Err(NativeMemoryError::Unaligned)
    ));
    assert!(matches!(
        s.map_at(u64::MAX, NATIVE_GUEST_PAGE, NativePermissions::READ),
        Err(NativeMemoryError::InvalidRange)
    ));
    s.limits.mapped_bytes = NATIVE_GUEST_PAGE;
    assert!(matches!(
        s.map_at(
            b + NATIVE_GUEST_PAGE,
            NATIVE_GUEST_PAGE,
            NativePermissions::READ
        ),
        Err(NativeMemoryError::BudgetExceeded)
    ));
    let mut p = s.pause().unwrap();
    p.write(u64::MAX, &[]).unwrap();
    p.read(u64::MAX, &mut []).unwrap();
}
