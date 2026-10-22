// SPDX-License-Identifier: GPL-2.0-only
//! Original attribute-output matrix; lock and wait lifecycle stays independent.
use super::*;
use Service::*;

const CASES: [(Service, Service, Service, u64); 5] = [
    (MutexAttrInit, MutexAttrDestroy, MutexAttrSetType, 2),
    (
        PosixMutexAttrInit,
        PosixMutexAttrDestroy,
        PosixMutexAttrSetType,
        2,
    ),
    (CondAttrInit, CondAttrDestroy, CondAttrSetClock, 4),
    (
        PosixCondAttrInit,
        PosixCondAttrDestroy,
        PosixCondAttrSetClock,
        4,
    ),
    (RwlockAttrInit, RwlockAttrDestroy, RwlockAttrSetType, 2),
];

fn library(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    arguments: &[u64],
) -> Result<u64, Failure> {
    let mut args = [u64::MAX; 6];
    args[..arguments.len()].copy_from_slice(arguments);
    owner.invoke_library(service, CallArgs(args), guest)
}

#[test]
fn attribute_outputs_ignore_old_bytes_and_preserve_aliases_across_same_and_other_kind_overwrites() {
    for (init, destroy, set, value) in CASES {
        let mut owner = kernel(Default::default());
        let mut guest = TestGuest::new();
        guest.put(0, &u64::MAX.to_le_bytes());
        guest.regions[0].protection = 2;
        let read = owner.usage().bytes_read;
        assert_eq!(library(&mut owner, &mut guest, init, &[ARG]), Ok(0));
        assert_eq!(owner.usage().bytes_read, read);
        guest.regions[0].protection = 3;
        let old = guest.u64(0);
        guest.put(8, &old.to_le_bytes());
        assert_eq!(
            library(&mut owner, &mut guest, set, &[ARG + 8, value]),
            Ok(0)
        );
        guest.copy(old + 56, &[0x42; 8]);
        let before = (
            owner.usage().handles_live,
            owner.usage().mapped_bytes,
            guest.regions.len(),
            guest.effects,
        );
        guest.regions[0].protection = 1;
        assert_eq!(
            library(&mut owner, &mut guest, init, &[ARG]),
            Err(Failure::Guest(GuestError::WriteDenied))
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
        assert_eq!(guest.u64(0), old);
        guest.regions[0].protection = 3;
        assert_eq!(library(&mut owner, &mut guest, init, &[ARG]), Ok(0));
        let second = guest.u64(0);
        assert_ne!(second, old);
        assert_eq!(guest.get(second, 64), [0; 64]);
        assert_eq!(guest.get(old + 56, 8), [0x42; 8]);
        guest.put(24, &second.to_le_bytes());
        assert_eq!(
            library(&mut owner, &mut guest, ThreadAttrInit, &[ARG + 16]),
            Ok(0)
        );
        let other = guest.u64(16);
        guest.put(0, &other.to_le_bytes());
        assert_eq!(library(&mut owner, &mut guest, init, &[ARG]), Ok(0));
        assert_ne!(guest.u64(0), other);
        assert_eq!(owner.usage().handles_live, 4);
        assert_eq!(
            library(
                &mut owner,
                &mut guest,
                ThreadAttrGetStackSize,
                &[ARG + 16, ARG + 32]
            ),
            Ok(0)
        );
        assert_eq!(guest.u64(32), 0x100000);
        assert_eq!(
            library(&mut owner, &mut guest, set, &[ARG + 8, value]),
            Ok(0)
        );
        for slot in [ARG, ARG + 8, ARG + 24] {
            assert_eq!(library(&mut owner, &mut guest, destroy, &[slot]), Ok(0));
        }
        assert_eq!(
            library(&mut owner, &mut guest, ThreadAttrDestroy, &[ARG + 16]),
            Ok(0)
        );
        assert_eq!(
            (owner.usage().handles_live, owner.usage().mapped_bytes),
            (0, 0)
        );
    }
}

#[test]
fn overwritten_mutex_attributes_keep_old_recursive_metadata_and_restore_new_defaults() {
    for init in [MutexAttrInit, PosixMutexAttrInit] {
        let mut owner = kernel(Default::default());
        let mut guest = TestGuest::new();
        assert_eq!(library(&mut owner, &mut guest, init, &[ARG]), Ok(0));
        let old = guest.u64(0);
        guest.put(8, &old.to_le_bytes());
        assert_eq!(
            library(&mut owner, &mut guest, MutexAttrSetType, &[ARG, 2]),
            Ok(0)
        );
        assert_eq!(library(&mut owner, &mut guest, init, &[ARG]), Ok(0));
        for (slot, attribute, second_lock) in [(ARG + 16, ARG + 8, 0), (ARG + 24, ARG, 0x8002_0010)]
        {
            assert_eq!(
                library(&mut owner, &mut guest, MutexInit, &[slot, attribute, 0]),
                Ok(0)
            );
            assert_eq!(library(&mut owner, &mut guest, MutexLock, &[slot]), Ok(0));
            assert_eq!(
                library(&mut owner, &mut guest, MutexTryLock, &[slot]),
                Ok(second_lock)
            );
            if second_lock == 0 {
                assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[slot]), Ok(0));
            }
            assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[slot]), Ok(0));
            assert_eq!(
                library(&mut owner, &mut guest, MutexDestroy, &[slot]),
                Ok(0)
            );
        }
        for slot in [ARG, ARG + 8] {
            assert_eq!(
                library(&mut owner, &mut guest, MutexAttrDestroy, &[slot]),
                Ok(0)
            );
        }
        assert_eq!(owner.usage().handles_live, 0);
    }
}
