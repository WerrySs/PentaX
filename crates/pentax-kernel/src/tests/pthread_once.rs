// SPDX-License-Identifier: GPL-2.0-only
//! Pure admission/backing transitions; these are not callback execution proofs.
use super::*;
fn args(control: u64, entry: u64) -> CallArgs {
    CallArgs([control, entry, 0, 0, 0, 0])
}
#[test]
fn once_source_validation_has_positive_results_and_no_unbound_callback_success() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let errno = owner
        .invoke_library(Service::ErrnoLocation, CallArgs([0; 6]), &mut guest)
        .unwrap();
    guest.copy(errno, &37_u32.to_le_bytes());
    for (control, entry, value) in [(0, ARG, 22), (ARG, 0, 22)] {
        assert_eq!(
            owner.invoke_library(Service::PthreadOnce, args(control, entry), &mut guest),
            Ok(value)
        );
    }
    guest.put(0, &0x1122_3344_0000_007f_u64.to_le_bytes());
    assert_eq!(
        owner.invoke_library(Service::PthreadOnce, args(ARG, u64::MAX), &mut guest),
        Ok(22)
    );
    guest.put(0, &0x1122_3344_0000_0001_u64.to_le_bytes());
    assert_eq!(
        owner.invoke_library(Service::PthreadOnce, args(ARG, u64::MAX), &mut guest),
        Ok(0)
    );
    for state in [2_u32, 3] {
        guest.put(0, &state.to_le_bytes());
        assert!(matches!(
            owner.invoke_library(Service::PthreadOnce, args(ARG, ARG), &mut guest),
            Err(Failure::Unsupported(_))
        ));
        assert_eq!(guest.u64(0), 0x1122_3344_0000_0000 | u64::from(state));
    }
    guest.put(0, &0_u32.to_le_bytes());
    assert!(matches!(
        owner.invoke_library(Service::PthreadOnce, args(ARG, ARG), &mut guest),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(guest.u64(0), 0x1122_3344_0000_0000);
    assert_eq!(guest.get(errno, 4), 37_u32.to_le_bytes());
    assert_eq!(
        resolve("libkernel", (1, 1), "libkernel", 1, "Z4QosVuAsA0"),
        None
    );
    assert_eq!(
        resolve("libkernel", (1, 1), "Posix", 1, "Z4QosVuAsA0"),
        Some(Service::PthreadOnce)
    );
}

#[test]
fn once_checked_word_publication_is_atomic_and_requires_the_actual_thread_binding() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(0, &0x1122_3344_0000_0000_u64.to_le_bytes());
    let PthreadOnceAction::Initialize(request) = owner
        .admit_pthread_once(args(ARG, ARG + 128), &guest)
        .unwrap()
    else {
        panic!("missing initializer admission")
    };
    assert_eq!((request.control(), request.entry()), (ARG, ARG + 128));
    assert_eq!(guest.u64(0), 0x1122_3344_0000_0000);
    guest.refuse = true;
    assert_eq!(
        owner.begin_pthread_once(&request, &mut guest),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.u64(0), 0x1122_3344_0000_0000);
    guest.refuse = false;
    owner.begin_pthread_once(&request, &mut guest).unwrap();
    assert_eq!(guest.u64(0), 0x1122_3344_0000_0002);
    assert!(matches!(
        owner.begin_pthread_once(&request, &mut guest),
        Err(Failure::Unsupported(_))
    ));
    guest.refuse = true;
    assert_eq!(
        owner.complete_pthread_once(&request, &mut guest),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.u64(0), 0x1122_3344_0000_0002);
    guest.refuse = false;
    owner.complete_pthread_once(&request, &mut guest).unwrap();
    assert_eq!(guest.u64(0), 0x1122_3344_0000_0001);
    assert_eq!(owner.usage().attempts, 1); // State writes are not second calls.
    struct Unbound;
    impl GuestAccess for Unbound {
        fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
            panic!("unbound once read")
        }
        fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
            panic!("unbound once write")
        }
    }
    assert!(matches!(
        owner.admit_pthread_once(args(ARG, ARG), &Unbound),
        Err(Failure::Unsupported(_))
    ));
    assert!(matches!(
        owner.begin_pthread_once(&request, &mut Unbound),
        Err(Failure::Unsupported(_))
    ));
    assert!(matches!(
        owner.complete_pthread_once(&request, &mut Unbound),
        Err(Failure::Unsupported(_))
    ));
}
