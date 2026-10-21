// SPDX-License-Identifier: GPL-2.0-only
//! Original configured-pool outputs, policy and actual mapping admission.
use super::*;

fn query(owner: &mut Kernel, guest: &mut impl GuestAccess, output: u64) -> Result<u64, Failure> {
    owner.invoke_library(
        Service::ConfiguredFlexibleMemorySize,
        CallArgs([output, u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX]),
        guest,
    )
}

struct NoAccess;
impl GuestAccess for NoAccess {
    fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
        panic!("configured query unexpectedly read guest memory");
    }
    fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        panic!("configured query unexpectedly wrote guest memory");
    }
}

#[test]
fn configured_size_is_immutable_exact_eight_byte_output_and_null_is_normal_kernel_int() {
    for capacity in [PAGE, 3 * PAGE, 64 * 1024 * 1024, 1024 * 1024 * 1024] {
        let mut owner = kernel(Limits {
            flexible_memory_bytes: capacity,
            ..Limits::default()
        });
        assert_eq!(
            owner.invoke(
                Service::ConfiguredFlexibleMemorySize,
                CallArgs([0; 6]),
                &mut NoAccess
            ),
            Err(Failure::Errno(Errno::Invalid))
        );
        assert_eq!(query(&mut owner, &mut NoAccess, 0), Ok(0x8002_0016));
        for offset in [0, 1, PAGE - 8, PAGE - 3, PAGE] {
            let mut guest = TestGuest::new();
            guest.regions[0].length = 2 * PAGE;
            guest.regions[0].bytes = vec![0xa5; (2 * PAGE) as usize];
            assert_eq!(query(&mut owner, &mut guest, ARG + offset), Ok(0));
            let mut expected = vec![0xa5; (2 * PAGE) as usize];
            expected[offset as usize..offset as usize + 8].copy_from_slice(&capacity.to_le_bytes());
            assert_eq!(guest.regions[0].bytes, expected);
            assert_eq!(
                (owner.usage().handles_live, owner.usage().mapped_bytes),
                (0, 0)
            );
            assert!(!owner.user_service_initialized());
        }
        assert_eq!(
            (owner.usage().bytes_read, owner.usage().bytes_written),
            (0, 40)
        );
    }
}

#[test]
fn configured_size_refuses_backing_and_quota_failures_before_any_output_mutation() {
    // This raw fixture coalesces denied whole-span writes to WriteDenied.
    for (output, readonly, expected) in [
        (ARG + PAGE, false, GuestError::WriteDenied),
        (ARG + PAGE - 3, false, GuestError::WriteDenied),
        (ARG + 1, true, GuestError::WriteDenied),
        (u64::MAX - 3, false, GuestError::AddressOverflow),
    ] {
        let mut owner = kernel(Limits::default());
        let mut guest = TestGuest::new();
        guest.regions[0].bytes.fill(0xa5);
        if readonly {
            guest.regions[0].protection = 1;
        }
        assert_eq!(
            query(&mut owner, &mut guest, output),
            Err(Failure::Guest(expected))
        );
        assert!(guest.regions[0].bytes.iter().all(|byte| *byte == 0xa5));
        assert_eq!(
            (owner.usage().handles_live, owner.usage().mapped_bytes),
            (0, 0)
        );
    }
    for (limits, label, attempts, work) in [
        (
            Limits {
                max_attempts: 0,
                ..Limits::default()
            },
            "attempts",
            0,
            0,
        ),
        (
            Limits {
                max_work: 1,
                ..Limits::default()
            },
            "work",
            1,
            1,
        ),
        (
            Limits {
                max_work: 9,
                ..Limits::default()
            },
            "work",
            1,
            2,
        ),
        (
            Limits {
                max_bytes: 7,
                ..Limits::default()
            },
            "guest bytes",
            1,
            2,
        ),
    ] {
        let mut owner = kernel(limits);
        assert_eq!(
            query(&mut owner, &mut NoAccess, ARG),
            Err(Failure::Limit(label))
        );
        assert_eq!(
            (
                owner.usage().attempts,
                owner.usage().work,
                owner.usage().bytes_read,
                owner.usage().bytes_written
            ),
            (attempts, work, 0, 0)
        );
    }
}

#[test]
fn configured_pool_limits_flexible_maps_and_recovers_on_unmap_without_counting_direct_or_internal()
{
    let mut owner = kernel(Limits {
        flexible_memory_bytes: 2 * PAGE,
        ..Limits::default()
    });
    let mut guest = TestGuest::new();
    // Real kernel-owned errno backing shares aggregate mapping accounting, but
    // must not consume the caller's configured flexible pool.
    assert!(call(&mut owner, &mut guest, Service::ErrnoLocation, &[]).unwrap() != 0);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::MapFlexible,
            &[ARG, 2 * PAGE, 3, 0]
        ),
        Ok(0)
    );
    let first = guest.u64(0);
    guest.put(64, b"original-flexible-pool\0");
    guest.put(8, &0_u64.to_le_bytes());
    let effects = guest.effects;
    let resources = (owner.usage().handles_live, owner.usage().mapped_bytes);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::MapNamedFlexible,
            &[ARG + 8, PAGE, 3, 0, ARG + 64]
        ),
        Err(Failure::Errno(Errno::NoMemory))
    );
    assert_eq!(guest.u64(8), 0);
    assert_eq!(guest.effects, effects);
    assert_eq!(
        (owner.usage().handles_live, owner.usage().mapped_bytes),
        resources
    );
    assert_eq!(query(&mut owner, &mut guest, ARG + 128), Ok(0));
    assert_eq!(guest.u64(128), 2 * PAGE);
    assert_eq!(
        call(&mut owner, &mut guest, Service::Munmap, &[first, 2 * PAGE]),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::MapNamedFlexible,
            &[ARG + 8, PAGE, 3, 0, ARG + 64]
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::AllocateMainDirect,
            &[PAGE, PAGE, 0, ARG + 16]
        ),
        Ok(0)
    );
    let physical = guest.u64(16);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::MapDirect,
            &[ARG + 24, PAGE, 3, 0, physical, PAGE]
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::MapFlexible,
            &[ARG + 32, PAGE, 3, 0]
        ),
        Ok(0)
    );
    assert_eq!(query(&mut owner, &mut guest, ARG + 128), Ok(0));
    assert_eq!(guest.u64(128), 2 * PAGE);
    let effects = guest.effects;
    guest.put(40, &0_u64.to_le_bytes());
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::MapFlexible,
            &[ARG + 40, PAGE, 3, 0]
        ),
        Err(Failure::Errno(Errno::NoMemory))
    );
    assert_eq!(guest.effects, effects + 1); // Only the fixture's prepared hint.
    assert_eq!(guest.u64(40), 0);
}

#[test]
fn configured_pool_policy_refuses_zero_unaligned_or_out_of_profile_capacity() {
    for capacity in [0, 1, PAGE - 1, 13_824 * 1024 * 1024, u64::MAX & !(PAGE - 1)] {
        assert!(matches!(
            Kernel::new(
                Limits {
                    flexible_memory_bytes: capacity,
                    ..Limits::default()
                },
                ProcessIdentity {
                    process_id: 100,
                    initial_thread_id: 7
                },
                vec![]
            ),
            Err(Failure::Limit("invalid resource policy"))
        ));
    }
    assert_eq!(Limits::default().flexible_memory_bytes, 64 * 1024 * 1024);
    // Configuration is independent of backing admission: a valid configured
    // capacity can exceed a deliberately smaller aggregate mapping allowance.
    let mut owner = kernel(Limits {
        flexible_memory_bytes: 2 * PAGE,
        max_mapped_bytes: PAGE - 1,
        ..Limits::default()
    });
    let mut guest = TestGuest::new();
    assert_eq!(query(&mut owner, &mut guest, ARG + 8), Ok(0));
    assert_eq!(guest.u64(8), 2 * PAGE);
    assert!(matches!(
        call(
            &mut owner,
            &mut guest,
            Service::MapFlexible,
            &[ARG, PAGE, 3, 0]
        ),
        Err(Failure::Limit("mapped bytes"))
    ));
    assert_eq!(owner.usage().mapped_bytes, 0);
}
