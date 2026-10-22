// SPDX-License-Identifier: GPL-2.0-only
//! Original admission, ownership and no-output tests; native fixtures prove RET.
use super::*;
use crate::tests::{ARG, TestGuest, kernel};

#[test]
fn detach_keeps_source_null_domains_and_real_registered_state_without_outputs() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let before = guest.get(ARG, 64).to_vec();
    for (service, code) in [
        (Service::PthreadDetach, 0x8002_0016),
        (Service::PosixPthreadDetach, 22),
    ] {
        let attempts = owner.usage().attempts;
        assert_eq!(
            owner.admit_pthread_detach(service, CallArgs([0; 6]), &guest),
            Ok(Some(code))
        );
        assert_eq!(owner.usage().attempts, attempts + 1);
    }
    owner.activate_thread_context(ARG + 512).unwrap();
    owner
        .register_thread_context(ARG + 1024, ARG + 256, 700, 0x7f, &mut guest)
        .unwrap();
    for (service, code) in [
        (Service::PthreadDetach, 0x8002_0003),
        (Service::PosixPthreadDetach, 3),
    ] {
        assert_eq!(
            owner.complete_pthread_detach(service, ARG + 264, &guest),
            Ok(code)
        );
    }
    owner.activate_thread_context(ARG + 1024).unwrap();
    let usage = owner.usage();
    for service in [
        Service::PthreadDetach,
        Service::PosixPthreadDetach,
        Service::PosixPthreadDetach,
    ] {
        assert_eq!(
            owner.admit_pthread_detach(
                service,
                CallArgs([ARG + 256, u64::MAX, 0, 0, 0, 0]),
                &guest
            ),
            Ok(None)
        );
        assert_eq!(
            owner.complete_pthread_detach(service, ARG + 256, &guest),
            Ok(0)
        );
        assert!(owner.threads.active().detached);
        assert_eq!(owner.threads.active().context(), ARG + 1024);
    }
    assert_eq!(owner.usage().bytes_read, usage.bytes_read);
    assert_eq!(owner.usage().bytes_written, usage.bytes_written);
    assert_eq!(guest.get(ARG, 64), before);
}

#[test]
fn detach_work_limit_refuses_before_changing_registered_lifecycle() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    owner.activate_thread_context(ARG + 512).unwrap();
    owner
        .register_thread_context(ARG + 1024, ARG + 256, 700, 0x7f, &mut guest)
        .unwrap();
    owner.activate_thread_context(ARG + 1024).unwrap();
    owner.budget.limits.max_work = owner.usage().work;
    assert!(matches!(
        owner.complete_pthread_detach(Service::PosixPthreadDetach, ARG + 256, &guest),
        Err(Failure::Limit(_))
    ));
    assert!(!owner.threads.active().detached);
}
