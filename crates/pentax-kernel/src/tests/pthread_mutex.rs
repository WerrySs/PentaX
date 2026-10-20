// SPDX-License-Identifier: GPL-2.0-only
//! Original mutex state, ABI and publication-failure cases; no game inputs.

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
fn mutex_recursive_attributes_are_copied_and_positive_pthread_errors_leave_errno_unchanged() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let errno = library(&mut owner, &mut guest, ErrnoLocation, &[]).unwrap();
    guest.copy(errno, &37_u32.to_le_bytes());
    assert_eq!(
        library(&mut owner, &mut guest, PosixMutexAttrInit, &[ARG]),
        Ok(0)
    );
    assert_eq!(
        library(
            &mut owner,
            &mut guest,
            MutexAttrSetType,
            &[ARG, 0xffff_ffff_0000_0002]
        ),
        Ok(0)
    );
    assert_eq!(
        library(
            &mut owner,
            &mut guest,
            PosixMutexInit,
            &[ARG + 8, ARG, u64::MAX]
        ),
        Ok(0)
    );
    assert_eq!(
        library(&mut owner, &mut guest, PosixMutexAttrSetType, &[ARG, 1]),
        Ok(0)
    );
    assert_eq!(
        library(&mut owner, &mut guest, PosixMutexAttrDestroy, &[ARG]),
        Ok(0)
    );
    assert_eq!(guest.u64(0), 0);
    for service in [MutexLock, PosixMutexTryLock, PosixMutexLock] {
        assert_eq!(library(&mut owner, &mut guest, service, &[ARG + 8]), Ok(0));
    }
    for _ in 0..2 {
        assert_eq!(
            library(&mut owner, &mut guest, MutexDestroy, &[ARG + 8]),
            Ok(0x8002_0010)
        );
        assert_eq!(
            library(&mut owner, &mut guest, PosixMutexUnlock, &[ARG + 8]),
            Ok(0)
        );
    }
    assert_eq!(
        library(&mut owner, &mut guest, PosixMutexDestroy, &[ARG + 8]),
        Ok(16)
    );
    assert_eq!(
        library(&mut owner, &mut guest, MutexUnlock, &[ARG + 8]),
        Ok(0)
    );
    assert_eq!(
        library(&mut owner, &mut guest, MutexUnlock, &[ARG + 8]),
        Ok(0x8002_0001)
    );
    assert_eq!(
        library(&mut owner, &mut guest, PosixMutexUnlock, &[ARG + 8]),
        Ok(1)
    );
    assert_eq!(
        library(&mut owner, &mut guest, PosixMutexDestroy, &[ARG + 8]),
        Ok(0)
    );
    assert_eq!(guest.u64(8), 0);
    assert_eq!(owner.usage().handles_live, 1); // Only the real errno cell remains.
    assert_eq!(guest.get(errno, 4), 37_u32.to_le_bytes());
    assert_eq!(
        resolve("libkernel", (1, 1), "Posix", 1, "Io9+nTKXZtA"),
        None
    );
    assert_eq!(
        resolve("libkernel", (1, 1), "libkernel", 1, "Io9+nTKXZtA"),
        Some(PosixMutexTimedLock)
    );
}

#[test]
fn mutex_lazy_initializers_and_absolute_deadlines_preserve_direct_and_posix_domains() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    for initial in [0_u64, 1] {
        guest.put(0, &initial.to_le_bytes());
        assert_eq!(
            library(&mut owner, &mut guest, MutexUnlock, &[ARG]),
            Ok(0x8002_0001)
        );
        assert!(guest.u64(0) > 1); // Source lazy creation also precedes EPERM.
        assert_eq!(library(&mut owner, &mut guest, MutexLock, &[ARG]), Ok(0));
        assert_eq!(
            library(&mut owner, &mut guest, MutexLock, &[ARG]),
            Ok(0x8002_000b)
        );
        assert_eq!(
            library(&mut owner, &mut guest, PosixMutexLock, &[ARG]),
            Ok(11)
        );
        assert_eq!(
            library(&mut owner, &mut guest, PosixMutexTryLock, &[ARG]),
            Ok(16)
        );
        assert_eq!(
            library(
                &mut owner,
                &mut guest,
                MutexTimedLock,
                &[ARG, 0x100_0000_0000]
            ),
            Ok(0x8002_003c)
        );
        assert_eq!(
            library(
                &mut owner,
                &mut guest,
                PosixMutexTimedLock,
                &[ARG, ARG + 32]
            ),
            Ok(60)
        );
        assert_eq!(
            library(&mut owner, &mut guest, PosixMutexTimedLock, &[ARG, 0]),
            Ok(22)
        );
        guest.put(32, &(-1_i64).to_le_bytes());
        assert_eq!(
            library(
                &mut owner,
                &mut guest,
                PosixMutexTimedLock,
                &[ARG, ARG + 32]
            ),
            Ok(22)
        );
        guest.put(32, &0_u64.to_le_bytes());
        guest.put(40, &1_000_000_000_i64.to_le_bytes());
        assert_eq!(
            library(
                &mut owner,
                &mut guest,
                PosixMutexTimedLock,
                &[ARG, ARG + 32]
            ),
            Ok(22)
        );
        guest.put(40, &0_u64.to_le_bytes());
        assert!(matches!(
            library(&mut owner, &mut guest, MutexTimedLock, &[ARG, 100_001]),
            Err(Failure::Limit(_))
        ));
        assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), Ok(0));
        // A free mutex acquires immediately, independently of a long deadline.
        assert_eq!(
            library(
                &mut owner,
                &mut guest,
                MutexTimedLock,
                &[ARG, u32::MAX as u64]
            ),
            Ok(0)
        );
        assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), Ok(0));
        assert_eq!(library(&mut owner, &mut guest, MutexDestroy, &[ARG]), Ok(0));
    }
    assert_eq!(
        library(&mut owner, &mut guest, MutexLock, &[0]),
        Ok(0x8002_0016)
    );
    assert_eq!(
        library(&mut owner, &mut guest, PosixMutexDestroy, &[0]),
        Ok(22)
    );
    assert!(matches!(
        library(&mut owner, &mut guest, PosixMutexAttrInit, &[0]),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(owner.usage().handles_live, 0);
}

#[test]
fn mutex_protocol_metadata_survives_attribute_mutation_and_preserves_contention_refusals() {
    for protocol in [1, 2] {
        let mut owner = kernel(Default::default());
        let mut guest = TestGuest::new();
        owner.activate_thread_context(1).unwrap();
        assert_eq!(
            library(&mut owner, &mut guest, MutexAttrInit, &[ARG]),
            Ok(0)
        );
        assert_eq!(
            library(
                &mut owner,
                &mut guest,
                MutexAttrSetProtocol,
                &[ARG, 0xdead_beef_0000_0000 | protocol]
            ),
            Ok(0)
        );
        assert_eq!(
            library(&mut owner, &mut guest, MutexInit, &[ARG + 8, ARG, 0]),
            Ok(0)
        );
        assert_eq!(
            library(&mut owner, &mut guest, PosixMutexAttrSetProtocol, &[ARG, 0]),
            Ok(0)
        );
        assert_eq!(
            library(&mut owner, &mut guest, MutexAttrDestroy, &[ARG]),
            Ok(0)
        );
        assert_eq!(
            library(&mut owner, &mut guest, MutexLock, &[ARG + 8]),
            Ok(0)
        );
        owner.activate_thread_context(2).unwrap();
        assert_eq!(
            library(&mut owner, &mut guest, MutexTimedLock, &[ARG + 8, 0]),
            Ok(0x8002_003c)
        );
        assert_eq!(
            library(&mut owner, &mut guest, MutexTimedLock, &[ARG + 8, 1]),
            Err(Failure::Unsupported("contended pthread priority protocol"))
        );
        assert_eq!(
            library(&mut owner, &mut guest, PosixMutexTryLock, &[ARG + 8]),
            Ok(16)
        );
        assert_eq!(
            library(&mut owner, &mut guest, MutexLock, &[ARG + 8]),
            Err(Failure::Unsupported("contended pthread priority protocol"))
        );
        assert_eq!(
            library(&mut owner, &mut guest, MutexUnlock, &[ARG + 8]),
            Ok(0x8002_0001)
        );
        owner.activate_thread_context(1).unwrap();
        assert_eq!(
            library(&mut owner, &mut guest, MutexUnlock, &[ARG + 8]),
            Ok(0)
        );
        assert_eq!(
            library(&mut owner, &mut guest, MutexDestroy, &[ARG + 8]),
            Ok(0)
        );
        assert_eq!(owner.usage().handles_live, 0);
    }
}

#[test]
fn mutex_failed_publication_and_shared_resource_limits_keep_slots_state_and_backing_intact() {
    let mut owner = kernel(Limits {
        max_handles: 2,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    guest.refuse = true;
    assert_eq!(
        library(&mut owner, &mut guest, MutexAttrInit, &[ARG]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.u64(0), 0);
    assert_eq!(owner.usage().handles_live, 0);
    guest.refuse = false;
    assert_eq!(
        library(&mut owner, &mut guest, MutexAttrInit, &[ARG]),
        Ok(0)
    );
    let attr = guest.u64(0);
    // Failed publication consumed neither a mapping nor a fresh object address.
    assert_eq!(attr, Limits::default().virtual_base);
    assert_eq!(owner.usage().mapped_bytes, PAGE);
    for protocol in [3, u64::MAX] {
        assert!(matches!(
            library(
                &mut owner,
                &mut guest,
                PosixMutexAttrSetProtocol,
                &[ARG, protocol]
            ),
            Err(Failure::Unsupported(_))
        ));
    }
    for protocol in [1, 2, 0xdead_beef_0000_0001, 0xdead_beef_0000_0002] {
        assert_eq!(
            library(
                &mut owner,
                &mut guest,
                PosixMutexAttrSetProtocol,
                &[ARG, protocol]
            ),
            Ok(0)
        );
    }
    assert_eq!(
        library(&mut owner, &mut guest, MutexAttrSetProtocol, &[ARG, 0]),
        Ok(0)
    );
    assert_eq!(
        library(&mut owner, &mut guest, PosixMutexAttrSetType, &[ARG, 5]),
        Ok(22)
    );
    assert_eq!(
        library(&mut owner, &mut guest, MutexInit, &[ARG + 8, ARG, 0]),
        Ok(0)
    );
    let mutex = guest.u64(8);
    assert!(matches!(
        library(&mut owner, &mut guest, MutexInit, &[ARG + 16, 0, 0]),
        Err(Failure::Limit(_))
    ));
    assert_eq!(guest.u64(16), 0);
    assert!(matches!(
        library(&mut owner, &mut guest, MutexInit, &[ARG + 8, 0, 0]),
        Err(Failure::Unsupported(_))
    ));
    guest.put(16, &attr.to_le_bytes());
    assert!(matches!(
        library(&mut owner, &mut guest, MutexLock, &[ARG + 16]),
        Err(Failure::Unsupported(_))
    ));
    guest.refuse = true;
    assert_eq!(
        library(&mut owner, &mut guest, MutexAttrDestroy, &[ARG]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(
        library(&mut owner, &mut guest, MutexDestroy, &[ARG + 8]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(
        (guest.u64(0), guest.u64(8), owner.usage().handles_live),
        (attr, mutex, 2)
    );
    // Once initialized, locking reads the slot and changes only retained state.
    guest.regions[0].protection = 1;
    assert_eq!(
        library(&mut owner, &mut guest, MutexLock, &[ARG + 8]),
        Ok(0)
    );
    assert_eq!(
        library(&mut owner, &mut guest, MutexUnlock, &[ARG + 8]),
        Ok(0)
    );
    guest.regions[0].protection = 3;
    guest.refuse = false;
    assert_eq!(
        library(&mut owner, &mut guest, MutexDestroy, &[ARG + 8]),
        Ok(0)
    );
    guest.put(8, &mutex.to_le_bytes());
    assert!(matches!(
        library(&mut owner, &mut guest, MutexLock, &[ARG + 8]),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(
        library(&mut owner, &mut guest, MutexAttrDestroy, &[ARG]),
        Ok(0)
    );
    assert_eq!(owner.usage().handles_live, 0);
    guest.put(24, &0_u64.to_le_bytes());
    assert_eq!(owner.usage().mapped_bytes, 0);
    guest.refuse = true;
    assert_eq!(
        library(&mut owner, &mut guest, MutexLock, &[ARG + 24]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.u64(24), 0);
    assert_eq!(owner.usage().handles_live, 0);
}

#[test]
fn mutex_prefixes_are_zeroed_owned_writable_and_revoked_on_transactional_destroy() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    library(&mut owner, &mut guest, MutexAttrInit, &[ARG]).unwrap();
    library(&mut owner, &mut guest, MutexInit, &[ARG + 8, ARG, 0]).unwrap();
    let attr = guest.u64(0);
    let mutex = guest.u64(8);
    assert_ne!(attr, mutex);
    assert!(mutex + PAGE <= 1 << 47);
    assert_eq!(guest.get(attr, 64), [0; 64]);
    assert_eq!(guest.get(mutex, 256), [0; 256]);
    assert_eq!(guest.region(mutex, 256, true).unwrap().protection, 3);
    guest.copy(attr + 60, &[0x81; 4]);
    guest.copy(mutex + 252, &[0x42; 4]);
    guest.put(16, &mutex.to_le_bytes());
    library(&mut owner, &mut guest, MutexLock, &[ARG + 16]).unwrap();
    library(&mut owner, &mut guest, MutexUnlock, &[ARG + 8]).unwrap();
    assert_eq!(guest.get(attr + 60, 4), [0x81; 4]);
    assert_eq!(guest.get(mutex + 252, 4), [0x42; 4]);
    for (service, args) in [
        (Munmap, vec![mutex, PAGE]),
        (Mprotect, vec![mutex, PAGE, 1]),
    ] {
        assert!(matches!(
            library(&mut owner, &mut guest, service, &args),
            Err(Failure::Unsupported(_))
        ));
    }
    // Publishing zero into a cell within the very object being unmapped must
    // not release its backing or invalidate the retained identity on failure.
    guest.copy(mutex, &mutex.to_le_bytes());
    assert!(matches!(
        library(&mut owner, &mut guest, MutexDestroy, &[mutex]),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(owner.usage().mapped_bytes, 2 * PAGE);
    guest.regions[0].protection = 1;
    assert!(matches!(
        library(&mut owner, &mut guest, MutexDestroy, &[ARG + 8]),
        Err(Failure::Guest(_))
    ));
    assert_eq!(guest.get(mutex + 252, 4), [0x42; 4]);
    guest.regions[0].protection = 3;
    library(&mut owner, &mut guest, MutexDestroy, &[ARG + 8]).unwrap();
    assert_eq!(guest.u64(8), 0);
    assert_eq!(guest.read(mutex, &mut [0; 1]), Err(GuestError::Unmapped));
    library(&mut owner, &mut guest, MutexInit, &[ARG + 24, 0, 0]).unwrap();
    assert_ne!(guest.u64(24), mutex);
    assert!(matches!(
        library(&mut owner, &mut guest, MutexLock, &[ARG + 16]),
        Err(Failure::Unsupported(_))
    ));
    library(&mut owner, &mut guest, MutexAttrDestroy, &[ARG]).unwrap();
    assert_eq!(guest.read(attr, &mut [0; 1]), Err(GuestError::Unmapped));
    library(&mut owner, &mut guest, MutexDestroy, &[ARG + 24]).unwrap();
    assert_eq!(
        (owner.usage().mapped_bytes, owner.usage().handles_live),
        (0, 0)
    );
}

#[test]
fn mutex_mapping_work_and_virtual_lifetime_limits_precede_publication() {
    for limits in [
        Limits {
            max_mapped_bytes: 0,
            ..Default::default()
        },
        Limits {
            max_work: 100,
            ..Default::default()
        },
    ] {
        let mut owner = kernel(limits);
        let mut guest = TestGuest::new();
        assert!(matches!(
            library(&mut owner, &mut guest, MutexInit, &[ARG, 0, 0]),
            Err(Failure::Limit(_))
        ));
        assert_eq!(guest.u64(0), 0);
        assert_eq!(guest.regions.len(), 1);
        assert_eq!(
            (owner.usage().mapped_bytes, owner.usage().handles_live),
            (0, 0)
        );
    }
    let mut owner = kernel(Limits {
        virtual_bytes: PAGE,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    guest.regions[0].protection = 1;
    assert!(matches!(
        library(&mut owner, &mut guest, MutexInit, &[ARG, 0, 0]),
        Err(Failure::Guest(_))
    ));
    assert_eq!(guest.regions.len(), 1);
    guest.regions[0].protection = 3;
    library(&mut owner, &mut guest, MutexInit, &[ARG, 0, 0]).unwrap();
    assert_eq!(guest.u64(0), Limits::default().virtual_base);
    library(&mut owner, &mut guest, MutexDestroy, &[ARG]).unwrap();
    assert_eq!(
        library(&mut owner, &mut guest, MutexInit, &[ARG, 0, 0]),
        Ok(0x8002_000c)
    );
    assert_eq!(guest.u64(0), 0);
    assert_eq!(
        (owner.usage().mapped_bytes, owner.usage().handles_live),
        (0, 0)
    );
}

#[test]
fn unbound_mutex_calls_refuse_before_guest_io_or_errno_allocation() {
    struct Unbound;
    impl GuestAccess for Unbound {
        fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
            panic!("unbound pthread input")
        }
        fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
            panic!("unbound pthread output")
        }
    }
    let mut owner = kernel(Default::default());
    for service in [
        MutexInit,
        MutexLock,
        PosixMutexAttrInit,
        PosixMutexTimedLock,
    ] {
        assert!(matches!(
            library(&mut owner, &mut Unbound, service, &[ARG, ARG]),
            Err(Failure::Unsupported(_))
        ));
    }
    assert_eq!(owner.usage().bytes_read + owner.usage().bytes_written, 0);
    assert_eq!(owner.usage().handles_live, 0);
    assert_eq!(owner.usage().attempts, 4);
}
