// SPDX-License-Identifier: GPL-2.0-only

//! Original actual-memory tests. Execution requires independent unsafe review.

use super::*;
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_MAPPED, MEM_RESERVE, MEMORY_BASIC_INFORMATION, VirtualQuery,
};

const PAGE: u64 = NATIVE_GUEST_PAGE;

#[test]
fn gpu_writes_share_physical_bytes_but_keep_per_alias_permissions_and_atomic_outputs() {
    let mut s = space();
    let a = base(&s);
    let b = a + 4 * PAGE;
    let gpu = NativePermissions::new(false, false, false).with_gpu(true, true);
    apply(&mut s, allocate(2 * PAGE), &[]).unwrap();
    apply(&mut s, direct(a, 2 * PAGE, 0, gpu), &[]).unwrap();
    apply(
        &mut s,
        direct(b, 2 * PAGE, 0, NativePermissions::READ_WRITE),
        &[],
    )
    .unwrap();
    s.pause().unwrap().write(b, b"seed").unwrap();
    let mut actual = [0; 4];
    let mut paused = s.pause().unwrap();
    paused.read_gpu(a, &mut actual).unwrap();
    assert_eq!(&actual, b"seed");
    assert!(matches!(
        paused.read(a, &mut actual),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    assert!(matches!(
        paused.read_gpu(b, &mut actual),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    paused
        .apply_gpu_writes(
            &[NativeMemoryWrite {
                address: a,
                bytes: b"done",
            }],
            &Default::default(),
        )
        .unwrap();
    paused.read(b, &mut actual).unwrap();
    assert_eq!(&actual, b"done");
    s.protect(a + PAGE, PAGE, gpu.with_gpu(true, false))
        .unwrap();
    let mut paused = s.pause().unwrap();
    let outputs = [
        NativeMemoryWrite {
            address: a,
            bytes: b"bad!",
        },
        NativeMemoryWrite {
            address: a + PAGE,
            bytes: b"no",
        },
    ];
    assert!(matches!(
        paused.apply_gpu_writes(&outputs, &Default::default()),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    paused.read(b, &mut actual).unwrap();
    assert_eq!(&actual, b"done");
    s.protect(b, PAGE, NativePermissions::READ.with_gpu(true, true))
        .unwrap();
    let outputs = [
        NativeMemoryWrite {
            address: a,
            bytes: b"bad!",
        },
        NativeMemoryWrite {
            address: b + 1,
            bytes: b"no",
        },
    ];
    assert_eq!(
        s.pause()
            .unwrap()
            .apply_gpu_writes(&outputs, &Default::default()),
        Err(NativeMemoryError::Overlap)
    );
    let mut paused = s.pause().unwrap();
    let limits = NativeTransactionLimits {
        max_output_bytes: 0,
        ..Default::default()
    };
    assert_eq!(
        paused.apply_gpu_writes(&outputs[..1], &limits),
        Err(NativeMemoryError::BudgetExceeded)
    );
    paused.read(b, &mut actual).unwrap();
    assert_eq!(&actual, b"done");
    s.unmap(a, 2 * PAGE).unwrap();
    assert!(matches!(
        s.pause().unwrap().read_gpu(a, &mut actual),
        Err(NativeMemoryError::Unmapped(_))
    ));
    apply(&mut s, release(2 * PAGE), &[]).unwrap();
    assert_eq!(s.committed_bytes(), 0);
}

fn direct(
    address: u64,
    size: u64,
    physical: u64,
    permissions: NativePermissions,
) -> NativeMemoryEffect {
    NativeMemoryEffect::MapPhysical {
        address,
        size,
        physical,
        permissions,
    }
}
fn allocate(size: u64) -> NativeMemoryEffect {
    NativeMemoryEffect::AllocatePhysical { physical: 0, size }
}
fn release(size: u64) -> NativeMemoryEffect {
    NativeMemoryEffect::ReleasePhysical { physical: 0, size }
}

fn space() -> NativeAddressSpace {
    NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x3_0000).unwrap()
}
fn base(space: &NativeAddressSpace) -> u64 {
    space.reserved_ranges().next().unwrap().address
}
fn query(address: u64) -> MEMORY_BASIC_INFORMATION {
    let mut info = MEMORY_BASIC_INFORMATION::default();
    // SAFETY: This only queries an owned numeric address. The output pointer
    // is writable for exactly the supplied ABI structure size.
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
fn map(address: u64, size: u64, permissions: NativePermissions) -> NativeMemoryEffect {
    NativeMemoryEffect::MapZeroed {
        address,
        size,
        permissions,
    }
}
fn protect(address: u64, size: u64, permissions: NativePermissions) -> NativeMemoryEffect {
    NativeMemoryEffect::Protect {
        address,
        size,
        permissions,
    }
}
fn apply(
    space: &mut NativeAddressSpace,
    effect: NativeMemoryEffect,
    writes: &[NativeMemoryWrite<'_>],
) -> Result<(), NativeMemoryError> {
    space.pause()?.apply_transaction(
        NativeMemoryTransaction { effect, writes },
        &Default::default(),
    )
}

#[test]
fn map_outputs_use_prospective_pages_and_preserve_actual_zero_tails_and_holes() {
    let mut s = space();
    let b = base(&s);
    s.map_at(b, PAGE, NativePermissions::READ_WRITE).unwrap();
    s.pause().unwrap().write(b, b"old").unwrap();
    let reserved = s.reserved_bytes();
    let outputs = [
        NativeMemoryWrite {
            address: b,
            bytes: b"new",
        },
        NativeMemoryWrite {
            address: b + PAGE + 3,
            bytes: b"thread-prefix",
        },
        NativeMemoryWrite {
            address: b + 2 * PAGE - 3,
            bytes: b"cross-page",
        },
    ];
    apply(
        &mut s,
        map(b + PAGE, 2 * PAGE, NativePermissions::READ_WRITE),
        &outputs,
    )
    .unwrap();
    let mut expected = vec![0; 3 * PAGE as usize];
    for output in outputs {
        let offset = (output.address - b) as usize;
        expected[offset..offset + output.bytes.len()].copy_from_slice(output.bytes);
    }
    let mut actual = vec![0x55; expected.len()];
    s.pause().unwrap().read(b, &mut actual).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        (s.mapped_bytes(), s.committed_bytes(), s.reserved_bytes()),
        (3 * PAGE, 3 * PAGE, reserved)
    );
    assert_eq!(query(b + 3 * PAGE).State, MEM_RESERVE);
    assert_eq!(query(b + PAGE).Protect, PAGE_READWRITE);
    assert!(matches!(
        s.pause().unwrap().fetch_source(b + PAGE, &mut [0]),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
}

#[test]
fn invalid_late_output_overlap_and_prospective_readonly_fail_before_os_mutation() {
    let mut s = space();
    let b = base(&s);
    s.map_at(b, PAGE, NativePermissions::READ_WRITE).unwrap();
    s.pause().unwrap().write(b, b"unchanged").unwrap();
    let before = s.operations;
    let outputs = [
        NativeMemoryWrite {
            address: b,
            bytes: b"overwrite",
        },
        NativeMemoryWrite {
            address: b + PAGE,
            bytes: b"new",
        },
        NativeMemoryWrite {
            address: b + 3 * PAGE,
            bytes: b"late",
        },
    ];
    assert!(matches!(
        apply(
            &mut s,
            map(b + PAGE, PAGE, NativePermissions::READ_WRITE),
            &outputs
        ),
        Err(NativeMemoryError::Unmapped(_))
    ));
    let outputs = [
        NativeMemoryWrite {
            address: b + PAGE,
            bytes: b"first",
        },
        NativeMemoryWrite {
            address: b + PAGE + 2,
            bytes: b"overlap",
        },
    ];
    assert_eq!(
        apply(
            &mut s,
            map(b + PAGE, PAGE, NativePermissions::READ_WRITE),
            &outputs
        ),
        Err(NativeMemoryError::Overlap)
    );
    assert!(matches!(
        apply(
            &mut s,
            map(b + PAGE, PAGE, NativePermissions::READ),
            &outputs[..1]
        ),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    assert_eq!(s.operations, before);
    assert_eq!(
        (s.mapped_bytes(), query(b + PAGE).State),
        (PAGE, MEM_RESERVE)
    );
    let mut actual = [0; 9];
    s.pause().unwrap().read(b, &mut actual).unwrap();
    assert_eq!(&actual, b"unchanged");
}

#[test]
fn protect_output_validation_uses_final_permissions_without_temporary_write_grants() {
    let mut s = space();
    let b = base(&s);
    s.map_at(b, 2 * PAGE, NativePermissions::READ).unwrap();
    let before = s.operations;
    let outputs = [
        NativeMemoryWrite {
            address: b,
            bytes: b"first",
        },
        NativeMemoryWrite {
            address: b + PAGE,
            bytes: b"denied",
        },
    ];
    assert!(matches!(
        apply(
            &mut s,
            protect(b, PAGE, NativePermissions::READ_WRITE),
            &outputs
        ),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    assert_eq!((s.operations, query(b).Protect), (before, PAGE_READONLY));
    apply(
        &mut s,
        protect(b, PAGE, NativePermissions::READ_WRITE),
        &outputs[..1],
    )
    .unwrap();
    assert_eq!(
        (query(b).Protect, query(b + PAGE).Protect),
        (PAGE_READWRITE, PAGE_READONLY)
    );
    assert!(matches!(
        apply(
            &mut s,
            protect(b, PAGE, NativePermissions::READ),
            &outputs[..1]
        ),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    assert_eq!(query(b).Protect, PAGE_READWRITE);
    let mut actual = [0; 5];
    s.pause().unwrap().read(b, &mut actual).unwrap();
    assert_eq!(&actual, b"first");
    // Logical execute remains OS READONLY, never an executable view.
    apply(
        &mut s,
        protect(b, PAGE, NativePermissions::READ_EXECUTE),
        &[],
    )
    .unwrap();
    assert_eq!(query(b).Protect, PAGE_READONLY);
    s.pause().unwrap().fetch_source(b, &mut actual).unwrap();
}

#[test]
fn late_map_failure_rolls_back_before_any_output_and_exact_retry_commits() {
    let mut s = space();
    let b = base(&s);
    s.map_at(b, PAGE, NativePermissions::READ_WRITE).unwrap();
    s.pause().unwrap().write(b, b"keep").unwrap();
    s.operations = 0;
    s.gate = Some(1);
    let outputs = [
        NativeMemoryWrite {
            address: b,
            bytes: b"edit",
        },
        NativeMemoryWrite {
            address: b + PAGE,
            bytes: b"real",
        },
    ];
    assert!(
        apply(
            &mut s,
            map(b + PAGE, 2 * PAGE, NativePermissions::READ_WRITE),
            &outputs
        )
        .is_err()
    );
    assert!(!s.poisoned);
    assert_eq!(s.mapped_bytes(), PAGE);
    assert_eq!(query(b + PAGE).State, MEM_RESERVE);
    let mut actual = [0; 4];
    s.pause().unwrap().read(b, &mut actual).unwrap();
    assert_eq!(&actual, b"keep");
    s.gate = None;
    apply(
        &mut s,
        map(b + PAGE, 2 * PAGE, NativePermissions::READ_WRITE),
        &outputs,
    )
    .unwrap();
    s.pause().unwrap().read(b, &mut actual).unwrap();
    assert_eq!(&actual, b"edit");
    s.pause().unwrap().read(b + PAGE, &mut actual).unwrap();
    assert_eq!(&actual, b"real");
}

#[test]
fn late_protect_failure_restores_both_permissions_and_leaves_outputs_untouched() {
    let mut s = space();
    let b = base(&s);
    s.map_at(b, 2 * PAGE, NativePermissions::READ).unwrap();
    s.operations = 0;
    s.gate = Some(1);
    assert!(
        apply(
            &mut s,
            protect(b, 2 * PAGE, NativePermissions::READ_WRITE),
            &[NativeMemoryWrite {
                address: b,
                bytes: b"never"
            }]
        )
        .is_err()
    );
    assert!(!s.poisoned);
    assert_eq!(
        (query(b).Protect, query(b + PAGE).Protect),
        (PAGE_READONLY, PAGE_READONLY)
    );
    let mut actual = [1; 5];
    s.pause().unwrap().read(b, &mut actual).unwrap();
    assert_eq!(actual, [0; 5]);
}

#[test]
fn failed_os_rollback_is_fatal_and_revokes_every_paused_memory_operation() {
    let mut s = space();
    let b = base(&s);
    s.map_at(b, 2 * PAGE, NativePermissions::READ).unwrap();
    s.operations = 0;
    s.gate = Some(1);
    s.rollback_gate = Some(0);
    {
        let mut p = s.pause().unwrap();
        assert_eq!(
            p.apply_transaction(
                NativeMemoryTransaction {
                    effect: protect(b, 2 * PAGE, NativePermissions::READ_WRITE),
                    writes: &[],
                },
                &Default::default()
            ),
            Err(NativeMemoryError::Poisoned)
        );
        assert_eq!(p.read(b, &mut [0]), Err(NativeMemoryError::Poisoned));
        assert_eq!(p.write(b, &[1]), Err(NativeMemoryError::Poisoned));
        assert_eq!(
            p.fetch_source(b, &mut [0]),
            Err(NativeMemoryError::Poisoned)
        );
        assert_eq!(p.preflight_read(b, 1), Err(NativeMemoryError::Poisoned));
        assert_eq!(p.preflight_write(b, 1), Err(NativeMemoryError::Poisoned));
        assert_eq!(
            p.apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::None,
                    writes: &[]
                },
                &Default::default()
            ),
            Err(NativeMemoryError::Poisoned)
        );
    }
    assert!(matches!(s.pause(), Err(NativeMemoryError::Poisoned)));
    assert_eq!(
        s.map_at(b + 2 * PAGE, PAGE, NativePermissions::READ),
        Err(NativeMemoryError::Poisoned)
    );
    assert_eq!(
        s.protect(b, PAGE, NativePermissions::READ),
        Err(NativeMemoryError::Poisoned)
    );
    assert_eq!(query(b).Protect, PAGE_READWRITE); // Rollback was deliberately prevented.
}

#[test]
fn failed_decommit_poison_preserves_ownership_of_unpublished_commitment() {
    let mut s = space();
    let b = base(&s);
    s.gate = Some(1);
    s.rollback_gate = Some(0);
    assert_eq!(
        apply(&mut s, map(b, 2 * PAGE, NativePermissions::READ_WRITE), &[]),
        Err(NativeMemoryError::Poisoned)
    );
    assert_eq!(s.mapped_bytes(), 0); // No partial logical publication.
    assert_eq!(query(b).State, MEM_COMMIT);
    assert!(matches!(s.pause(), Err(NativeMemoryError::Poisoned)));
}

#[test]
fn failed_pending_release_keeps_the_actual_reservation_in_the_poisoned_owner() {
    let mut s = NativeAddressSpace::new(NativeMemoryLimits::default()).unwrap();
    let reservation = Reservation::reserve(None, s.granularity).unwrap();
    let address = reservation.address();
    let size = reservation.size as u64;
    s.reservations.try_reserve_exact(1).unwrap();
    s.rollback_gate = Some(0);
    s.rollback_reservations(vec![reservation]);
    assert!(s.poisoned);
    assert_eq!(s.reserved_bytes(), size);
    assert_eq!(s.reserved_ranges().next().unwrap().address, address);
    assert_eq!(query(address).State, MEM_RESERVE);
    assert!(matches!(s.pause(), Err(NativeMemoryError::Poisoned)));
    LAST_RELEASE.with(|slot| slot.set(None));
    drop(s);
    assert_eq!(
        LAST_RELEASE.with(std::cell::Cell::get),
        Some((address, true))
    );
}

#[test]
fn zero_transaction_quotas_refuse_before_os_effects_or_output_copies() {
    for kind in 0..3 {
        let mut s = space();
        let b = base(&s);
        let mut limits = NativeTransactionLimits::default();
        match kind {
            0 => limits.max_outputs = 0,
            1 => limits.max_output_bytes = 0,
            _ => limits.max_preflight_steps = 0,
        }
        let before = s.operations;
        let outputs = [NativeMemoryWrite {
            address: b,
            bytes: &[1],
        }];
        assert_eq!(
            s.pause().unwrap().apply_transaction(
                NativeMemoryTransaction {
                    effect: map(b, PAGE, NativePermissions::READ_WRITE),
                    writes: &outputs,
                },
                &limits
            ),
            Err(NativeMemoryError::BudgetExceeded)
        );
        assert_eq!(
            (s.operations, s.mapped_bytes(), query(b).State),
            (before, 0, MEM_RESERVE)
        );
    }
}

#[test]
fn memory_range_unmap_preflights_outputs_and_holes_then_reuses_zeroed_backing() {
    let mut s = space();
    let b = base(&s);
    s.map_at(b, 3 * PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    s.pause().unwrap().write(b, b"left").unwrap();
    s.pause().unwrap().write(b + PAGE, b"old").unwrap();
    s.pause().unwrap().write(b + 2 * PAGE, b"right").unwrap();
    let reserved = s.reserved_bytes();
    let effect = NativeMemoryEffect::Unmap {
        address: b + PAGE,
        size: PAGE,
    };
    let before = s.operations;
    assert!(matches!(
        apply(
            &mut s,
            effect,
            &[NativeMemoryWrite {
                address: b + PAGE,
                bytes: b"invalid prospective output",
            }]
        ),
        Err(NativeMemoryError::Unmapped(_))
    ));
    assert_eq!(s.operations, before);
    s.gate = Some(before);
    assert!(apply(&mut s, effect, &[]).is_err());
    assert!(!s.poisoned);
    assert_eq!(query(b + PAGE).State, MEM_COMMIT);
    s.gate = None;
    apply(
        &mut s,
        effect,
        &[NativeMemoryWrite {
            address: b,
            bytes: b"kept",
        }],
    )
    .unwrap();
    assert_eq!(query(b + PAGE).State, MEM_RESERVE);
    assert_eq!((s.mapped_bytes(), s.reserved_bytes()), (2 * PAGE, reserved));
    let before = s.operations;
    assert!(matches!(
        apply(
            &mut s,
            NativeMemoryEffect::Unmap {
                address: b,
                size: 3 * PAGE,
            },
            &[]
        ),
        Err(NativeMemoryError::Unmapped(_))
    ));
    assert_eq!(s.operations, before);
    let mut actual = [0; 5];
    s.pause().unwrap().read(b + 2 * PAGE, &mut actual).unwrap();
    assert_eq!(&actual, b"right");
    s.pause().unwrap().read(b, &mut actual[..4]).unwrap();
    assert_eq!(&actual[..4], b"kept");
    apply(
        &mut s,
        map(b + PAGE, PAGE, NativePermissions::READ_WRITE),
        &[],
    )
    .unwrap();
    let mut zeroes = vec![1; PAGE as usize];
    s.pause().unwrap().read(b + PAGE, &mut zeroes).unwrap();
    assert!(zeroes.iter().all(|byte| *byte == 0));
    assert_eq!(s.reserved_bytes(), reserved);
}

#[test]
fn memory_range_partial_decommit_poison_revokes_access_and_never_copies_outputs() {
    let mut s = space();
    let b = base(&s);
    s.map_at(b, 3 * PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    s.pause().unwrap().write(b + 2 * PAGE, b"kept").unwrap();
    let effect = NativeMemoryEffect::Unmap {
        address: b,
        size: 2 * PAGE,
    };
    let before = s.operations;
    assert_eq!(
        s.pause().unwrap().apply_transaction(
            NativeMemoryTransaction {
                effect,
                writes: &[]
            },
            &NativeTransactionLimits {
                max_preflight_steps: 0,
                ..Default::default()
            }
        ),
        Err(NativeMemoryError::BudgetExceeded)
    );
    assert_eq!(s.operations, before);
    s.operations = 0;
    s.gate = Some(1);
    {
        let mut p = s.pause().unwrap();
        assert_eq!(
            p.apply_transaction(
                NativeMemoryTransaction {
                    effect,
                    writes: &[NativeMemoryWrite {
                        address: b + 2 * PAGE,
                        bytes: b"edit"
                    }],
                },
                &Default::default()
            ),
            Err(NativeMemoryError::Poisoned)
        );
        assert_eq!(
            p.read(b + 2 * PAGE, &mut [0; 4]),
            Err(NativeMemoryError::Poisoned)
        );
        assert_eq!(
            p.write(b + 2 * PAGE, b"edit"),
            Err(NativeMemoryError::Poisoned)
        );
        assert_eq!(
            p.fetch_source(b, &mut [0]),
            Err(NativeMemoryError::Poisoned)
        );
    }
    assert!(matches!(s.pause(), Err(NativeMemoryError::Poisoned)));
    assert_eq!(query(b).State, MEM_RESERVE);
    assert_eq!(query(b + PAGE).State, MEM_COMMIT);
    assert_eq!(s.mapped_bytes(), 3 * PAGE); // Partial metadata is never published.
}

#[test]
fn shared_physical_aliases_keep_real_bytes_across_partial_unmap_protection_and_remap() {
    let mut s = space();
    let b = base(&s);
    let a = b + PAGE;
    let c = b + 5 * PAGE;
    s.map_at(b, PAGE, NativePermissions::READ_WRITE).unwrap();
    s.pause().unwrap().write(b, b"neighbor").unwrap();
    apply(&mut s, allocate(3 * PAGE), &[]).unwrap();
    apply(
        &mut s,
        direct(a, 2 * PAGE, PAGE, NativePermissions::READ_WRITE),
        &[],
    )
    .unwrap();
    apply(
        &mut s,
        direct(c, 2 * PAGE, PAGE, NativePermissions::READ),
        &[],
    )
    .unwrap();
    assert_eq!(
        (s.mapped_bytes(), s.committed_bytes()),
        (5 * PAGE, 4 * PAGE)
    );
    assert_eq!(query(a).Type, MEM_MAPPED);
    assert_eq!(query(c).Protect, PAGE_READONLY);
    let at = PAGE - 3;
    s.pause().unwrap().write(a + at, b"shared-alias").unwrap();
    let mut bytes = [0; 12];
    s.pause().unwrap().read(c + at, &mut bytes).unwrap();
    assert_eq!(&bytes, b"shared-alias");
    assert!(matches!(
        s.pause().unwrap().write(c, &[1]),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    s.protect(c, 2 * PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    s.pause().unwrap().write(c, b"same").unwrap();
    s.pause().unwrap().read(a, &mut bytes[..4]).unwrap();
    assert_eq!(&bytes[..4], b"same");
    s.unmap(a + PAGE, PAGE).unwrap();
    assert_eq!(query(a + PAGE).State, MEM_RESERVE);
    s.pause().unwrap().read(c + PAGE, &mut bytes[..9]).unwrap();
    assert_eq!(&bytes[..9], b"red-alias");
    apply(
        &mut s,
        direct(
            a + PAGE,
            PAGE,
            2 * PAGE,
            NativePermissions::new(false, false, false),
        ),
        &[],
    )
    .unwrap();
    assert_eq!(query(a + PAGE).Protect, PAGE_NOACCESS);
    assert!(matches!(
        s.pause().unwrap().read(a + PAGE, &mut [0]),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    s.protect(a + PAGE, PAGE, NativePermissions::READ).unwrap();
    s.pause().unwrap().read(a + PAGE, &mut bytes[..9]).unwrap();
    assert_eq!(&bytes[..9], b"red-alias");
    assert_eq!(
        apply(&mut s, release(4 * PAGE), &[]),
        Err(NativeMemoryError::InvalidRange)
    );
    s.unmap(a, 2 * PAGE).unwrap();
    s.unmap(c, 2 * PAGE).unwrap();
    assert_eq!((s.mapped_bytes(), s.committed_bytes()), (PAGE, 4 * PAGE));
    apply(&mut s, direct(a, PAGE, PAGE, NativePermissions::READ), &[]).unwrap();
    s.pause().unwrap().read(a, &mut bytes[..4]).unwrap();
    assert_eq!(&bytes[..4], b"same");
    s.unmap(a, PAGE).unwrap();
    apply(&mut s, release(3 * PAGE), &[]).unwrap();
    assert_eq!(s.committed_bytes(), PAGE);
    apply(&mut s, allocate(3 * PAGE), &[]).unwrap();
    apply(&mut s, direct(a, PAGE, PAGE, NativePermissions::READ), &[]).unwrap();
    let mut page = vec![1; PAGE as usize];
    s.pause().unwrap().read(a, &mut page).unwrap();
    assert!(page.iter().all(|v| *v == 0));
    s.unmap(a, PAGE).unwrap();
    s.map_at(a, PAGE, NativePermissions::READ_WRITE).unwrap();
    s.pause().unwrap().write(a, b"private").unwrap();
    apply(&mut s, direct(c, PAGE, PAGE, NativePermissions::READ), &[]).unwrap();
    s.pause().unwrap().read(c, &mut bytes[..7]).unwrap();
    assert_eq!(&bytes[..7], &[0; 7]);
    s.pause().unwrap().read(b, &mut bytes[..8]).unwrap();
    assert_eq!(&bytes[..8], b"neighbor");
    // Observe actual cleanup of both private replacements and shared views;
    // querying their addresses after Drop could race unrelated OS address reuse.
    LAST_RELEASE.with(|slot| slot.set(None));
    drop(s);
    assert_eq!(LAST_RELEASE.with(std::cell::Cell::get), Some((b, true)));
}

#[test]
fn shared_physical_transactions_refuse_alias_outputs_and_restore_failed_new_views() {
    let mut s = space();
    let b = base(&s);
    let a = b + PAGE;
    let c = b + 5 * PAGE;
    s.map_at(b, PAGE, NativePermissions::READ_WRITE).unwrap();
    let before = s.operations;
    assert!(matches!(
        apply(
            &mut s,
            allocate(3 * PAGE),
            &[NativeMemoryWrite {
                address: b + PAGE,
                bytes: b"invalid",
            }]
        ),
        Err(NativeMemoryError::Unmapped(_))
    ));
    assert_eq!((s.operations, s.physical_allocations.len()), (before, 0));
    apply(&mut s, allocate(3 * PAGE), &[]).unwrap();
    apply(
        &mut s,
        direct(c, PAGE, PAGE, NativePermissions::READ_WRITE),
        &[],
    )
    .unwrap();
    s.pause().unwrap().write(c, b"keep").unwrap();
    let before = s.operations;
    let alias_outputs = [
        NativeMemoryWrite {
            address: a,
            bytes: b"first",
        },
        NativeMemoryWrite {
            address: c,
            bytes: b"second",
        },
    ];
    assert_eq!(
        apply(
            &mut s,
            direct(a, 2 * PAGE, PAGE, NativePermissions::READ_WRITE),
            &alias_outputs
        ),
        Err(NativeMemoryError::Overlap)
    );
    assert_eq!(s.operations, before);
    s.operations = 0;
    s.gate = Some(1);
    assert!(
        apply(
            &mut s,
            direct(a, 2 * PAGE, PAGE, NativePermissions::READ_WRITE),
            &[]
        )
        .is_err()
    );
    assert!(!s.poisoned);
    assert_eq!(query(a).State, MEM_RESERVE);
    assert_eq!(query(a + PAGE).State, MEM_RESERVE);
    let mut bytes = [0; 4];
    s.pause().unwrap().read(c, &mut bytes).unwrap();
    assert_eq!(&bytes, b"keep");
    s.gate = None;
    apply(
        &mut s,
        direct(a, PAGE, PAGE, NativePermissions::READ_WRITE),
        &[],
    )
    .unwrap();
    apply(
        &mut s,
        direct(a + PAGE, PAGE, PAGE, NativePermissions::READ_WRITE),
        &[],
    )
    .unwrap();
    let repeated = vec![7; PAGE as usize + 1];
    let span = |address, size| NativeRange { address, size };
    assert_eq!(
        s.pause()
            .unwrap()
            .preflight_disjoint_backing(span(a + 7, 4), span(c + 7, 4), 100),
        Err(NativeMemoryError::Overlap)
    );
    assert_eq!(
        s.pause()
            .unwrap()
            .preflight_disjoint_backing(span(a + 7, 4), span(c + 11, 4), 100),
        Ok(8)
    );
    assert_eq!(
        s.pause()
            .unwrap()
            .preflight_disjoint_backing(span(a, 4), span(b, 4), 0),
        Err(NativeMemoryError::BudgetExceeded)
    );
    assert_eq!(
        s.pause()
            .unwrap()
            .preflight_disjoint_backing(span(a, 4), span(b, 4), 100),
        Ok(8)
    );
    assert_eq!(
        s.pause()
            .unwrap()
            .preflight_distinct_backing(a, PAGE + 1, 100),
        Err(NativeMemoryError::Overlap)
    );
    assert_eq!(
        s.pause().unwrap().preflight_distinct_backing(a, PAGE, 0),
        Err(NativeMemoryError::BudgetExceeded)
    );
    assert_eq!(
        s.pause()
            .unwrap()
            .preflight_distinct_backing(a, PAGE, 100)
            .unwrap(),
        6
    );
    assert_eq!(
        apply(
            &mut s,
            NativeMemoryEffect::None,
            &[NativeMemoryWrite {
                address: a,
                bytes: &repeated,
            }]
        ),
        Err(NativeMemoryError::Overlap)
    );
    s.pause().unwrap().read(c, &mut bytes).unwrap();
    assert_eq!(&bytes, b"keep");
    let cap = s.committed_bytes();
    s.restrict_limits(NativeMemoryLimits {
        mapped_bytes: cap,
        reserved_bytes: s.reserved_bytes(),
    })
    .unwrap();
    assert_eq!(
        apply(
            &mut s,
            NativeMemoryEffect::AllocatePhysical {
                physical: 3 * PAGE,
                size: PAGE
            },
            &[]
        ),
        Err(NativeMemoryError::BudgetExceeded)
    );
    assert_eq!(
        s.map_at(b + 3 * PAGE, PAGE, NativePermissions::READ),
        Err(NativeMemoryError::BudgetExceeded)
    );
}

#[test]
fn shared_physical_failed_view_rollback_poison_keeps_section_and_reservation_ownership() {
    let mut s = space();
    let a = base(&s) + PAGE;
    apply(&mut s, allocate(2 * PAGE), &[]).unwrap();
    let reserved = s.reserved_bytes();
    s.operations = 0;
    s.gate = Some(1);
    s.rollback_gate = Some(0);
    assert_eq!(
        apply(
            &mut s,
            direct(a, 2 * PAGE, 0, NativePermissions::READ_WRITE),
            &[]
        ),
        Err(NativeMemoryError::Poisoned)
    );
    assert_eq!(query(a).Type, MEM_MAPPED);
    assert_eq!(s.mapped_bytes(), 0);
    assert_eq!(s.reserved_bytes(), reserved);
    assert_eq!(s.physical_allocations.len(), 1);
    assert!(matches!(s.pause(), Err(NativeMemoryError::Poisoned)));
}

#[test]
fn physical_release_middle_withdraws_all_aliases_and_preserves_offsets_across_sections() {
    let mut s = space();
    let b = base(&s);
    let a = b + PAGE;
    let c = b + 5 * PAGE;
    s.map_at(b, PAGE, NativePermissions::READ_WRITE).unwrap();
    apply(&mut s, allocate(3 * PAGE), &[]).unwrap();
    for at in [a, c] {
        apply(
            &mut s,
            direct(at, 3 * PAGE, 0, NativePermissions::READ_WRITE),
            &[],
        )
        .unwrap();
    }
    for (offset, bytes) in [
        (0, b"left".as_slice()),
        (PAGE, b"old".as_slice()),
        (2 * PAGE, b"right".as_slice()),
    ] {
        s.pause().unwrap().write(a + offset, bytes).unwrap();
    }
    let middle = NativeMemoryEffect::ReleasePhysical {
        physical: PAGE,
        size: PAGE,
    };
    apply(&mut s, middle, &[]).unwrap();
    assert_eq!(s.physical_allocations.len(), 2);
    // The partially retained OS section stays fully charged until its last
    // surviving piece is freed. Logical physical holes can be allocated anew.
    assert_eq!(s.committed_bytes(), 4 * PAGE);
    let mut bytes = [0; 5];
    for at in [a, c] {
        assert_eq!(query(at + PAGE).State, MEM_RESERVE);
        s.pause().unwrap().read(at, &mut bytes[..4]).unwrap();
        assert_eq!(&bytes[..4], b"left");
        s.pause().unwrap().read(at + 2 * PAGE, &mut bytes).unwrap();
        assert_eq!(&bytes, b"right");
    }
    let before = s.operations;
    assert_eq!(
        apply(&mut s, release(3 * PAGE), &[]),
        Err(NativeMemoryError::InvalidRange)
    );
    assert_eq!(s.operations, before); // Late physical hole: detach nothing.
    apply(
        &mut s,
        NativeMemoryEffect::AllocatePhysical {
            physical: PAGE,
            size: PAGE,
        },
        &[],
    )
    .unwrap();
    for at in [a, c] {
        apply(
            &mut s,
            direct(at + PAGE, PAGE, PAGE, NativePermissions::READ_WRITE),
            &[],
        )
        .unwrap();
    }
    s.pause().unwrap().read(a + PAGE, &mut bytes).unwrap();
    assert_eq!(bytes, [0; 5]);
    s.pause().unwrap().write(a + PAGE, b"fresh").unwrap();
    s.pause().unwrap().read(c + PAGE, &mut bytes).unwrap();
    assert_eq!(&bytes, b"fresh");
    s.unmap(a, 3 * PAGE).unwrap();
    // One map traverses three logical pieces belonging to two OS sections.
    apply(
        &mut s,
        direct(a, 3 * PAGE, 0, NativePermissions::READ_WRITE),
        &[],
    )
    .unwrap();
    for (offset, expected) in [
        (0, b"left".as_slice()),
        (PAGE, b"fresh".as_slice()),
        (2 * PAGE, b"right".as_slice()),
    ] {
        s.pause()
            .unwrap()
            .read(a + offset, &mut bytes[..expected.len()])
            .unwrap();
        assert_eq!(&bytes[..expected.len()], expected);
    }
    apply(&mut s, release(3 * PAGE), &[]).unwrap();
    assert!(s.physical_allocations.is_empty());
    assert!(s.physical_sections.iter().all(Option::is_none));
    assert_eq!((s.mapped_bytes(), s.committed_bytes()), (PAGE, PAGE));
    for at in [a, c] {
        assert_eq!(query(at).State, MEM_RESERVE);
    }
    apply(&mut s, allocate(3 * PAGE), &[]).unwrap();
    assert_eq!(s.physical_sections.len(), 2); // Reuse empty handle slots.
    apply(&mut s, direct(a, 3 * PAGE, 0, NativePermissions::READ), &[]).unwrap();
    let mut all = vec![1; 3 * PAGE as usize];
    s.pause().unwrap().read(a, &mut all).unwrap();
    assert!(all.iter().all(|v| *v == 0));
}

#[test]
fn physical_release_preflights_disjoint_outputs_and_metadata_then_rolls_back_failed_detach() {
    let mut s = space();
    let b = base(&s);
    let a = b + PAGE;
    let c = b + 5 * PAGE;
    s.map_at(b, PAGE, NativePermissions::READ_WRITE).unwrap();
    apply(&mut s, allocate(3 * PAGE), &[]).unwrap();
    for at in [a, c] {
        apply(
            &mut s,
            direct(at, 3 * PAGE, 0, NativePermissions::READ_WRITE),
            &[],
        )
        .unwrap();
    }
    s.pause().unwrap().write(a + PAGE, b"kept").unwrap();
    let effect = NativeMemoryEffect::ReleasePhysical {
        physical: PAGE,
        size: PAGE,
    };
    let before = s.operations;
    assert!(matches!(
        apply(
            &mut s,
            effect,
            &[NativeMemoryWrite {
                address: c + PAGE,
                bytes: b"denied"
            }]
        ),
        Err(NativeMemoryError::Unmapped(_))
    ));
    assert_eq!(s.operations, before);
    assert_eq!(
        s.pause().unwrap().apply_transaction(
            NativeMemoryTransaction {
                effect,
                writes: &[]
            },
            &NativeTransactionLimits {
                max_scratch_bytes: 0,
                ..Default::default()
            }
        ),
        Err(NativeMemoryError::BudgetExceeded)
    );
    assert_eq!(s.operations, before);
    // Failure before closing the first whole section restores all detached views.
    s.operations = 0;
    s.gate = Some(6);
    assert!(apply(&mut s, release(3 * PAGE), &[]).is_err());
    assert!(!s.poisoned);
    let mut bytes = [0; 4];
    s.pause().unwrap().read(c + PAGE, &mut bytes).unwrap();
    assert_eq!(&bytes, b"kept");
    // A late detach failure restores an earlier alias with its original bytes.
    s.operations = 0;
    s.gate = Some(1);
    assert!(apply(&mut s, effect, &[]).is_err());
    assert!(!s.poisoned);
    s.pause().unwrap().read(a + PAGE, &mut bytes).unwrap();
    assert_eq!(&bytes, b"kept");
    assert_eq!(s.physical_allocations.len(), 1);
    s.operations = 0;
    s.gate = Some(1);
    s.rollback_operations = 0;
    s.rollback_gate = Some(0);
    assert_eq!(apply(&mut s, effect, &[]), Err(NativeMemoryError::Poisoned));
    assert!(matches!(s.pause(), Err(NativeMemoryError::Poisoned)));
    assert_eq!(query(a + PAGE).State, MEM_RESERVE);
    assert_eq!(query(c + PAGE).State, MEM_COMMIT);
}

#[test]
fn physical_release_failure_after_one_section_close_is_fatal_and_never_reuses_closed_handles() {
    let mut s = space();
    let a = base(&s) + PAGE;
    for physical in [0, PAGE] {
        apply(
            &mut s,
            NativeMemoryEffect::AllocatePhysical {
                physical,
                size: PAGE,
            },
            &[],
        )
        .unwrap();
    }
    apply(
        &mut s,
        direct(a, 2 * PAGE, 0, NativePermissions::READ_WRITE),
        &[],
    )
    .unwrap();
    s.operations = 0;
    s.gate = Some(3); // Two detachments, first close, second close refuses.
    assert_eq!(
        apply(&mut s, release(2 * PAGE), &[]),
        Err(NativeMemoryError::Poisoned)
    );
    assert_eq!(s.operations, 4);
    assert!(matches!(s.pause(), Err(NativeMemoryError::Poisoned)));
    for at in [a, a + PAGE] {
        assert_eq!(query(at).State, MEM_RESERVE);
    }
}
