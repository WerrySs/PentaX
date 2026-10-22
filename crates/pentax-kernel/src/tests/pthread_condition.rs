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
fn condition_lifecycle_copies_attributes_and_publishes_real_zeroed_prefixes() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let errno = library(&mut owner, &mut guest, ErrnoLocation, &[]).unwrap();
    guest.copy(errno, &37_u32.to_le_bytes());
    let baseline = owner.usage();
    assert_eq!(
        library(&mut owner, &mut guest, PosixCondAttrInit, &[ARG]),
        Ok(0)
    );
    let attr = guest.u64(0);
    assert_eq!(guest.get(attr, 64), [0; 64]);
    for clock in [
        0,
        4,
        5,
        7,
        8,
        9,
        10,
        11,
        12,
        13,
        16,
        17,
        18,
        19,
        0x1_0000_0004,
    ] {
        assert_eq!(
            library(&mut owner, &mut guest, PosixCondAttrSetClock, &[ARG, clock]),
            Ok(0)
        );
    }
    for clock in [1, 2, 3, 6, 14, 15, 20, u64::MAX] {
        assert_eq!(
            library(&mut owner, &mut guest, PosixCondAttrSetClock, &[ARG, clock]),
            Ok(22)
        );
    }
    assert_eq!(
        library(
            &mut owner,
            &mut guest,
            PosixCondInit,
            &[ARG + 8, ARG, u64::MAX]
        ),
        Ok(0)
    );
    let condition = guest.u64(8);
    assert_eq!(guest.get(condition, 256), [0; 256]);
    assert_ne!(condition, attr);
    assert_eq!(guest.region(condition, 256, true).unwrap().protection, 3);
    guest.copy(condition + 248, &[0xa5; 8]);
    guest.put(16, &condition.to_le_bytes());
    assert_eq!(
        library(&mut owner, &mut guest, PosixCondAttrDestroy, &[ARG]),
        Ok(0)
    );
    for service in [
        CondSignal,
        CondBroadcast,
        PosixCondSignal,
        PosixCondBroadcast,
    ] {
        assert_eq!(library(&mut owner, &mut guest, service, &[ARG + 16]), Ok(0));
    }
    assert_eq!(guest.get(condition + 248, 8), [0xa5; 8]);
    assert_eq!(guest.get(errno, 4), 37_u32.to_le_bytes());
    assert_eq!(
        library(&mut owner, &mut guest, CondDestroy, &[ARG + 8]),
        Ok(0)
    );
    assert_eq!(guest.u64(8), 0);
    assert_eq!(guest.read(condition, &mut [0]), Err(GuestError::Unmapped));
    assert!(matches!(
        library(&mut owner, &mut guest, CondBroadcast, &[ARG + 16]),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(owner.usage().handles_live, baseline.handles_live);
    assert_eq!(owner.usage().mapped_bytes, baseline.mapped_bytes);
    assert_eq!(
        library(&mut owner, &mut guest, PosixCondBroadcast, &[0]),
        Ok(22)
    );
    assert_eq!(
        library(&mut owner, &mut guest, CondBroadcast, &[0]),
        Ok(0x8002_0016)
    );
}

#[test]
fn condition_lazy_publication_destruction_and_cross_kind_failures_are_transactional() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.regions[0].protection = 1;
    assert!(matches!(
        library(&mut owner, &mut guest, CondBroadcast, &[ARG]),
        Err(Failure::Guest(_))
    ));
    assert_eq!(
        (owner.usage().handles_live, owner.usage().mapped_bytes),
        (0, 0)
    );
    assert_eq!(guest.regions.len(), 1);
    guest.regions[0].protection = 3;
    assert_eq!(
        library(&mut owner, &mut guest, PosixCondBroadcast, &[ARG]),
        Ok(0)
    );
    let id = guest.u64(0);
    assert_eq!(id, Limits::default().virtual_base);
    for service in [MutexInit, CondInit] {
        assert!(matches!(
            library(&mut owner, &mut guest, service, &[ARG, 0, 0]),
            Err(Failure::Unsupported(_))
        ));
    }
    assert!(matches!(
        library(&mut owner, &mut guest, MutexLock, &[ARG]),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(
        library(&mut owner, &mut guest, MutexInit, &[ARG + 8, 0, 0]),
        Ok(0)
    );
    assert!(matches!(
        library(&mut owner, &mut guest, CondBroadcast, &[ARG + 8]),
        Err(Failure::Unsupported(_))
    ));
    assert!(matches!(
        library(&mut owner, &mut guest, CondInit, &[ARG + 8, 0, 0]),
        Err(Failure::Unsupported(_))
    ));
    guest.copy(id, &id.to_le_bytes());
    assert!(matches!(
        library(&mut owner, &mut guest, CondDestroy, &[id]),
        Err(Failure::Unsupported(_))
    ));
    guest.regions[0].protection = 1;
    assert!(matches!(
        library(&mut owner, &mut guest, CondDestroy, &[ARG]),
        Err(Failure::Guest(_))
    ));
    assert_eq!(guest.u64(0), id);
    assert_eq!(owner.usage().handles_live, 2);
    guest.regions[0].protection = 3;
    assert_eq!(library(&mut owner, &mut guest, CondDestroy, &[ARG]), Ok(0));
    assert_eq!(library(&mut owner, &mut guest, CondSignal, &[ARG]), Ok(0));
    assert_ne!(guest.u64(0), id);
    guest.put(16, &1_u64.to_le_bytes());
    assert!(matches!(
        library(&mut owner, &mut guest, CondBroadcast, &[ARG + 16]),
        Err(Failure::Unsupported(_))
    ));
}

#[test]
fn condition_bound_context_limits_and_exact_scopes_precede_effects() {
    struct Unbound;
    impl GuestAccess for Unbound {
        fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
            panic!("unexpected read")
        }
        fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
            panic!("unexpected write")
        }
    }
    let mut owner = kernel(Default::default());
    assert!(matches!(
        library(&mut owner, &mut Unbound, CondBroadcast, &[ARG]),
        Err(Failure::Unsupported(_))
    ));
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
            library(&mut owner, &mut guest, CondSignal, &[ARG]),
            Err(Failure::Limit(_))
        ));
        assert_eq!(guest.u64(0), 0);
        assert_eq!(owner.usage().handles_live, 0);
        assert_eq!(guest.regions.len(), 1);
    }
    assert_eq!(
        resolve("libkernel", (1, 1), "libkernel", 1, "2MOy+rUfuhQ"),
        Some(CondSignal)
    );
    assert_eq!(
        resolve("libkernel", (1, 1), "Posix", 1, "2MOy+rUfuhQ"),
        Some(PosixCondSignal)
    );
    assert_eq!(
        resolve("libkernel", (1, 1), "libkernel", 1, "0TyVk4MSLt0"),
        None
    );
    assert_eq!(
        resolve("libkernel", (1, 2), "Posix", 1, "0TyVk4MSLt0"),
        None
    );
}
