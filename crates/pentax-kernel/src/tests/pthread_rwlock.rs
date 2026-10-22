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
fn rwlock_recursion_writer_reading_and_unlock_order_match_source() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let errno = library(&mut owner, &mut guest, ErrnoLocation, &[]).unwrap();
    guest.copy(errno, &37_u32.to_le_bytes());
    let baseline = owner.usage();
    // POSIX init ignores the third register; the pinned implementation ignores attr.
    assert_eq!(
        library(
            &mut owner,
            &mut guest,
            PosixRwlockInit,
            &[ARG, u64::MAX, u64::MAX]
        ),
        Ok(0)
    );
    let id = guest.u64(0);
    assert_eq!(guest.get(id, 256), [0; 256]);
    guest.copy(id + 248, &[0xa5; 8]);
    guest.put(8, &id.to_le_bytes());
    for read in [RwlockReadLock, RwlockTryReadLock, RwlockTimedReadLock] {
        assert_eq!(library(&mut owner, &mut guest, read, &[ARG, 0]), Ok(0));
    }
    assert_eq!(
        library(&mut owner, &mut guest, PosixRwlockWriteLock, &[ARG]),
        Ok(11)
    );
    assert_eq!(
        library(&mut owner, &mut guest, RwlockTryWriteLock, &[ARG]),
        Ok(0x8002_000b)
    );
    assert_eq!(
        library(&mut owner, &mut guest, RwlockTimedWriteLock, &[ARG, 0]),
        Ok(0x8002_000b)
    );
    assert_eq!(
        library(&mut owner, &mut guest, RwlockDestroy, &[ARG]),
        Ok(0x8002_0010)
    );
    for _ in 0..3 {
        assert_eq!(
            library(&mut owner, &mut guest, RwlockUnlock, &[ARG + 8]),
            Ok(0)
        );
    }
    for write in [RwlockWriteLock, RwlockTryWriteLock, RwlockTimedWriteLock] {
        assert_eq!(library(&mut owner, &mut guest, write, &[ARG, 0]), Ok(0));
        assert_eq!(
            library(&mut owner, &mut guest, RwlockTryReadLock, &[ARG]),
            Ok(0)
        );
        assert_eq!(library(&mut owner, &mut guest, RwlockUnlock, &[ARG]), Ok(0));
        // The reader was released first; the writer remains held.
        assert_eq!(
            library(&mut owner, &mut guest, PosixRwlockDestroy, &[ARG]),
            Ok(16)
        );
        assert_eq!(library(&mut owner, &mut guest, RwlockUnlock, &[ARG]), Ok(0));
    }
    assert_eq!(
        library(&mut owner, &mut guest, RwlockUnlock, &[ARG]),
        Ok(0x8002_0001)
    );
    assert_eq!(guest.get(id + 248, 8), [0xa5; 8]);
    assert_eq!(guest.get(errno, 4), 37_u32.to_le_bytes());
    assert_eq!(
        library(&mut owner, &mut guest, PosixRwlockDestroy, &[ARG]),
        Ok(0)
    );
    assert_eq!(guest.u64(0), 0);
    assert_eq!(guest.read(id, &mut [0]), Err(GuestError::Unmapped));
    assert!(matches!(
        library(&mut owner, &mut guest, RwlockReadLock, &[ARG + 8]),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(owner.usage().handles_live, baseline.handles_live);
    assert_eq!(owner.usage().mapped_bytes, baseline.mapped_bytes);
}

#[test]
fn rwlock_lazy_slots_attributes_and_failed_lifetime_transactions_preserve_state() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    // Try-write alone does not initialize zero, unlike try-read and unlock.
    assert!(matches!(
        library(&mut owner, &mut guest, RwlockTryWriteLock, &[ARG]),
        Err(Failure::Unsupported(_))
    ));
    guest.regions[0].protection = 1;
    assert!(matches!(
        library(&mut owner, &mut guest, RwlockReadLock, &[ARG]),
        Err(Failure::Guest(_))
    ));
    assert_eq!(guest.regions.len(), 1);
    assert_eq!(owner.usage().handles_live, 0);
    guest.regions[0].protection = 3;
    assert_eq!(
        library(&mut owner, &mut guest, RwlockUnlock, &[ARG]),
        Ok(0x8002_0001)
    );
    let id = guest.u64(0);
    assert_ne!(id, 0);
    for init in [MutexInit, CondInit, RwlockInit] {
        assert!(matches!(
            library(&mut owner, &mut guest, init, &[ARG, 0, 0]),
            Err(Failure::Unsupported(_))
        ));
    }
    assert_eq!(
        library(&mut owner, &mut guest, RwlockAttrInit, &[ARG + 8]),
        Ok(0)
    );
    let attr = guest.u64(8);
    assert_eq!(guest.get(attr, 64), [0; 64]);
    for kind in [0, 1, 2, u64::MAX] {
        assert_eq!(
            library(&mut owner, &mut guest, RwlockAttrSetType, &[ARG + 8, kind]),
            Ok(0)
        );
    }
    assert!(matches!(
        library(&mut owner, &mut guest, RwlockReadLock, &[ARG + 8]),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(
        library(&mut owner, &mut guest, RwlockAttrDestroy, &[ARG + 8]),
        Ok(0)
    );
    guest.regions[0].protection = 1;
    assert!(matches!(
        library(&mut owner, &mut guest, RwlockDestroy, &[ARG]),
        Err(Failure::Guest(_))
    ));
    assert_eq!(guest.u64(0), id);
    guest.regions[0].protection = 3;
    assert_eq!(
        library(&mut owner, &mut guest, RwlockDestroy, &[ARG]),
        Ok(0)
    );
    assert_eq!(
        library(&mut owner, &mut guest, RwlockTryReadLock, &[ARG]),
        Ok(0)
    );
    assert_ne!(guest.u64(0), id);
    guest.put(16, &1_u64.to_le_bytes());
    assert!(matches!(
        library(&mut owner, &mut guest, RwlockReadLock, &[ARG + 16]),
        Err(Failure::Unsupported(_))
    ));
}

#[test]
fn rwlock_budget_and_exact_library_domains_precede_publication() {
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
            library(&mut owner, &mut guest, RwlockReadLock, &[ARG]),
            Err(Failure::Limit(_))
        ));
        assert_eq!(guest.u64(0), 0);
        assert_eq!(guest.regions.len(), 1);
    }
    assert_eq!(
        resolve("libkernel", (1, 1), "Posix", 1, "sIlRvQqsN2Y"),
        Some(PosixRwlockWriteLock)
    );
    assert_eq!(
        resolve("libkernel", (1, 1), "libkernel", 1, "sIlRvQqsN2Y"),
        Some(RwlockWriteLock)
    );
    assert_eq!(
        resolve("libkernel", (1, 1), "Posix", 1, "XhWHn6P5R7U"),
        Some(RwlockTryWriteLock)
    );
    assert_eq!(
        resolve("libkernel", (1, 2), "libkernel", 1, "6ULAa0fq4jA"),
        None
    );
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    assert_eq!(
        library(&mut owner, &mut guest, RwlockInit, &[0]),
        Ok(0x8002_0016)
    );
    assert_eq!(
        library(&mut owner, &mut guest, PosixRwlockInit, &[0]),
        Ok(22)
    );
}
