// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use Service::*;

fn library(
    owner: &mut Kernel,
    guest: &mut impl GuestAccess,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    owner.invoke_library(service, CallArgs(args), guest)
}

#[test]
fn thread_attributes_preserve_defaults_full_width_values_and_exact_error_domains() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let errno = library(&mut owner, &mut guest, ErrnoLocation, &[]).unwrap();
    guest.copy(errno, &37_u32.to_le_bytes());
    assert_eq!(
        library(&mut owner, &mut guest, PosixThreadAttrInit, &[ARG]),
        Ok(0)
    );
    let id = guest.u64(0);
    assert_eq!(guest.get(id, 64), [0; 64]);
    for (service, expected, width) in [
        (ThreadAttrGetAffinity, 0x7f, 8),
        (ThreadAttrGetGuardSize, 0x1000, 8),
        (ThreadAttrGetStackSize, 0x100000, 8),
        (ThreadAttrGetStackAddr, 0, 8),
        (ThreadAttrGetSchedParam, 700, 4),
        (PosixThreadAttrGetSchedPolicy, 1, 4),
        (ThreadAttrGetSoloSched, 0, 4),
        (PosixThreadAttrGetDetachState, 0, 4),
    ] {
        guest.put(32, &[0xa5; 8]);
        assert_eq!(
            library(&mut owner, &mut guest, service, &[ARG, ARG + 32]),
            Ok(0)
        );
        assert_eq!(
            guest.u64(32),
            expected | if width == 4 { 0xa5a5_a5a5_0000_0000 } else { 0 }
        );
    }
    for (set, get, value) in [
        (
            ThreadAttrSetAffinity,
            ThreadAttrGetAffinity,
            0x1234_5678_9abc_def0,
        ),
        (ThreadAttrSetGuardSize, ThreadAttrGetGuardSize, u64::MAX),
        (
            ThreadAttrSetStackAddr,
            ThreadAttrGetStackAddr,
            0x2345_6789_abcd,
        ),
        (
            ThreadAttrSetStackSize,
            ThreadAttrGetStackSize,
            0x1_0000_4000,
        ),
    ] {
        assert_eq!(library(&mut owner, &mut guest, set, &[ARG, value]), Ok(0));
        assert_eq!(
            library(&mut owner, &mut guest, get, &[ARG, ARG + 32]),
            Ok(0)
        );
        assert_eq!(guest.u64(32), value);
    }
    guest.put(40, &(-731_i32).to_le_bytes());
    assert_eq!(
        library(
            &mut owner,
            &mut guest,
            PosixThreadAttrSetSchedParam,
            &[ARG, ARG + 40]
        ),
        Ok(0)
    );
    guest.put(32, &[0xa5; 8]);
    assert_eq!(
        library(
            &mut owner,
            &mut guest,
            ThreadAttrGetSchedParam,
            &[ARG, ARG + 32]
        ),
        Ok(0)
    );
    assert_eq!(
        guest.u64(32),
        0xa5a5_a5a5_0000_0000 | u64::from((-731_i32) as u32)
    );
    assert_eq!(
        library(
            &mut owner,
            &mut guest,
            ThreadAttrSetStackSize,
            &[ARG, 0x3fff]
        ),
        Ok(0x8002_0016)
    );
    assert_eq!(
        library(
            &mut owner,
            &mut guest,
            PosixThreadAttrSetStackSize,
            &[ARG, 0x3fff]
        ),
        Ok(22)
    );
    assert_eq!(
        library(
            &mut owner,
            &mut guest,
            ThreadAttrSetStack,
            &[ARG, 0, 0x4000]
        ),
        Ok(0x8002_0016)
    );
    assert_eq!(
        library(
            &mut owner,
            &mut guest,
            ThreadAttrGetStack,
            &[ARG, ARG + 48, ARG + 56]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(48), 0x2345_6789_abcd);
    assert_eq!(guest.u64(56), 0x1_0000_4000);
    for service in [ThreadAttrSetDetachState, ThreadAttrSetInheritSched] {
        assert!(matches!(
            library(&mut owner, &mut guest, service, &[ARG, 2]),
            Err(Failure::Unsupported(_))
        ));
    }
    assert_eq!(guest.get(errno, 4), 37_u32.to_le_bytes());
    assert_eq!(
        resolve("libkernel", (1, 1), "Posix", 1, "2Q0z6rnBrTE"),
        Some(PosixThreadAttrSetStackSize)
    );
    assert_eq!(
        resolve("libkernel", (1, 1), "libkernel", 1, "2Q0z6rnBrTE"),
        Some(ThreadAttrSetStackSize)
    );
    assert_eq!(
        resolve("libkernel", (1, 2), "libkernel", 1, "nsYoNRywwNg"),
        None
    );
}

#[test]
fn thread_attribute_outputs_and_lifetime_publish_atomically_and_reject_stale_or_cross_kind_slots() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.regions[0].protection = 1;
    assert!(matches!(
        library(&mut owner, &mut guest, ThreadAttrInit, &[ARG]),
        Err(Failure::Guest(_))
    ));
    assert_eq!(owner.usage().handles_live, 0);
    assert_eq!(guest.regions.len(), 1);
    guest.regions[0].protection = 3;
    assert_eq!(
        library(&mut owner, &mut guest, ThreadAttrInit, &[ARG]),
        Ok(0)
    );
    let id = guest.u64(0);
    guest.put(8, &id.to_le_bytes());
    guest.put(32, &[0x55; 16]);
    for second in [ARG + PAGE, ARG + 36, u64::MAX] {
        assert!(
            library(
                &mut owner,
                &mut guest,
                ThreadAttrGetStack,
                &[ARG, ARG + 32, second]
            )
            .is_err()
        );
        assert_eq!(guest.get(ARG + 32, 16), [0x55; 16]);
    }
    assert!(matches!(
        library(&mut owner, &mut guest, MutexInit, &[ARG]),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(
        library(&mut owner, &mut guest, MutexAttrInit, &[ARG + 16]),
        Ok(0)
    );
    assert!(matches!(
        library(&mut owner, &mut guest, ThreadAttrDestroy, &[ARG + 16]),
        Err(Failure::Unsupported(_))
    ));
    guest.regions[0].protection = 1;
    assert!(matches!(
        library(&mut owner, &mut guest, ThreadAttrDestroy, &[ARG]),
        Err(Failure::Guest(_))
    ));
    assert_eq!(guest.u64(0), id);
    guest.regions[0].protection = 3;
    assert_eq!(
        library(&mut owner, &mut guest, ThreadAttrDestroy, &[ARG]),
        Ok(0)
    );
    assert_eq!(guest.u64(0), 0);
    assert_eq!(guest.read(id, &mut [0]), Err(GuestError::Unmapped));
    assert!(matches!(
        library(
            &mut owner,
            &mut guest,
            ThreadAttrGetStackSize,
            &[ARG + 8, ARG + 32]
        ),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(
        library(&mut owner, &mut guest, ThreadAttrInit, &[ARG]),
        Ok(0)
    );
    assert_ne!(guest.u64(0), id);
    guest.put(24, &1_u64.to_le_bytes());
    assert_eq!(
        library(
            &mut owner,
            &mut guest,
            ThreadAttrSetStackSize,
            &[ARG + 24, 0x4000]
        ),
        Ok(0x8002_0016)
    );
    assert!(matches!(
        library(
            &mut owner,
            &mut guest,
            ThreadAttrGetStackSize,
            &[ARG + 24, ARG + 32]
        ),
        Err(Failure::Unsupported(_))
    ));
}

#[test]
fn thread_attribute_init_overwrites_arbitrary_write_only_outputs_without_reading_them() {
    for service in [ThreadAttrInit, PosixThreadAttrInit] {
        let mut owner = kernel(Default::default());
        let mut guest = TestGuest::new();
        guest.regions[0].bytes.fill(0xa5);
        guest.put(9, &u64::MAX.to_le_bytes());
        guest.regions[0].protection = 2; // Writable, deliberately unreadable.
        let bytes_read = owner.usage().bytes_read;
        assert_eq!(library(&mut owner, &mut guest, service, &[ARG + 9]), Ok(0));
        assert_eq!(owner.usage().bytes_read, bytes_read);
        guest.regions[0].protection = 3;
        let id = guest.u64(9);
        let mut expected = vec![0xa5; PAGE as usize];
        expected[9..17].copy_from_slice(&id.to_le_bytes());
        assert_eq!(guest.regions[0].bytes, expected);
        assert_eq!(guest.get(id, 64), [0; 64]);
        assert_eq!(
            library(
                &mut owner,
                &mut guest,
                ThreadAttrGetStackSize,
                &[ARG + 9, ARG + 32]
            ),
            Ok(0)
        );
        assert_eq!(guest.u64(32), 0x100000);
        assert_eq!(
            library(&mut owner, &mut guest, ThreadAttrDestroy, &[ARG + 9]),
            Ok(0)
        );
        assert_eq!(owner.usage().handles_live, 0);
    }
}

#[test]
fn thread_attribute_init_preserves_old_attribute_and_other_kind_owners_through_copied_slots() {
    for service in [ThreadAttrInit, PosixThreadAttrInit] {
        let mut owner = kernel(Default::default());
        let mut guest = TestGuest::new();
        assert_eq!(library(&mut owner, &mut guest, service, &[ARG]), Ok(0));
        let old = guest.u64(0);
        guest.put(8, &old.to_le_bytes());
        assert_eq!(
            library(
                &mut owner,
                &mut guest,
                ThreadAttrSetStackSize,
                &[ARG, 2 * PAGE]
            ),
            Ok(0)
        );
        assert_eq!(library(&mut owner, &mut guest, service, &[ARG]), Ok(0));
        assert_ne!(guest.u64(0), old);
        assert_eq!(owner.usage().handles_live, 2);
        for (slot, expected) in [(ARG, 0x100000), (ARG + 8, 2 * PAGE)] {
            assert_eq!(
                library(
                    &mut owner,
                    &mut guest,
                    ThreadAttrGetStackSize,
                    &[slot, ARG + 32]
                ),
                Ok(0)
            );
            assert_eq!(guest.u64(32), expected);
        }
        assert_eq!(
            library(&mut owner, &mut guest, MutexAttrInit, &[ARG + 16]),
            Ok(0)
        );
        let other = guest.u64(16);
        guest.put(24, &other.to_le_bytes());
        assert_eq!(library(&mut owner, &mut guest, service, &[ARG + 16]), Ok(0));
        assert_ne!(guest.u64(16), other);
        assert_eq!(owner.usage().handles_live, 4);
        assert_eq!(guest.get(other, 64), [0; 64]);
        assert_eq!(
            library(&mut owner, &mut guest, MutexAttrDestroy, &[ARG + 24]),
            Ok(0)
        );
        assert_eq!(guest.read(other, &mut [0]), Err(GuestError::Unmapped));
        for slot in [ARG, ARG + 8, ARG + 16] {
            assert_eq!(
                library(&mut owner, &mut guest, ThreadAttrDestroy, &[slot]),
                Ok(0)
            );
        }
        assert_eq!(
            (owner.usage().handles_live, owner.usage().mapped_bytes),
            (0, 0)
        );
    }
}

#[test]
fn thread_attribute_overwrite_refusals_preserve_the_old_slot_metadata_and_backing() {
    for service in [ThreadAttrInit, PosixThreadAttrInit] {
        for refusal in [0, 1, 2] {
            let mut owner = kernel(Default::default());
            let mut guest = TestGuest::new();
            assert_eq!(library(&mut owner, &mut guest, service, &[ARG]), Ok(0));
            let old = guest.u64(0);
            assert_eq!(
                library(
                    &mut owner,
                    &mut guest,
                    ThreadAttrSetStackSize,
                    &[ARG, 2 * PAGE]
                ),
                Ok(0)
            );
            let before = (
                owner.usage().handles_live,
                owner.usage().mapped_bytes,
                guest.regions.len(),
                guest.effects,
            );
            match refusal {
                0 => guest.regions[0].protection = 1,
                1 => guest.refuse = true,
                _ => owner.budget.limits.max_handles = owner.usage().handles_live,
            }
            let expected = if refusal == 2 {
                Failure::Limit("handles")
            } else {
                Failure::Guest(GuestError::WriteDenied)
            };
            assert_eq!(
                library(&mut owner, &mut guest, service, &[ARG]),
                Err(expected)
            );
            assert_eq!(
                (
                    owner.usage().handles_live,
                    owner.usage().mapped_bytes,
                    guest.regions.len(),
                    guest.effects
                ),
                before
            );
            guest.regions[0].protection = 3;
            guest.refuse = false;
            assert_eq!(guest.u64(0), old);
            assert_eq!(guest.get(old, 64), [0; 64]);
            assert_eq!(
                library(
                    &mut owner,
                    &mut guest,
                    ThreadAttrGetStackSize,
                    &[ARG, ARG + 32]
                ),
                Ok(0)
            );
            assert_eq!(guest.u64(32), 2 * PAGE);
            assert_eq!(
                library(&mut owner, &mut guest, ThreadAttrDestroy, &[ARG]),
                Ok(0)
            );
            assert_eq!(owner.usage().handles_live, 0);
        }
    }
}

#[test]
fn thread_attribute_query_requires_actual_initial_thread_stack_and_shared_resource_budget() {
    for limits in [
        Limits {
            max_handles: 0,
            ..Default::default()
        },
        Limits {
            max_mapped_bytes: 0,
            ..Default::default()
        },
    ] {
        let mut owner = kernel(limits);
        let mut guest = TestGuest::new();
        assert!(matches!(
            library(&mut owner, &mut guest, ThreadAttrInit, &[ARG]),
            Err(Failure::Limit(_))
        ));
        assert_eq!(guest.u64(0), 0);
        assert_eq!(guest.regions.len(), 1);
    }
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    assert_eq!(
        library(&mut owner, &mut guest, ThreadAttrInit, &[ARG]),
        Ok(0)
    );
    let thread = library(&mut owner, &mut guest, PthreadSelf, &[]).unwrap();
    for candidate in [thread, thread + 1] {
        assert!(matches!(
            library(&mut owner, &mut guest, ThreadAttrGet, &[candidate, ARG]),
            Err(Failure::Unsupported(_))
        ));
    }
    assert_eq!(
        library(
            &mut owner,
            &mut guest,
            ThreadAttrGetStackSize,
            &[ARG, ARG + 32]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(32), 0x100000);
}
