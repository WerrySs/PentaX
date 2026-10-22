// SPDX-License-Identifier: GPL-2.0-only
//! Original backing/errno tests. Runtime completion is independently required.
use super::*;
use crate::tests::{ARG, TestGuest, kernel};
#[test]
fn join_admission_keeps_null_and_self_errno_and_consumes_one_attempt() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    assert_eq!(
        owner.admit_pthread_join(CallArgs([0; 6]), &guest),
        Ok(Some(0x8002_0016))
    );
    owner.activate_thread_context(ARG + 512).unwrap();
    owner
        .register_thread_context(ARG + 1024, ARG + 256, 700, 0x7f, &mut guest)
        .unwrap();
    owner.activate_thread_context(ARG + 1024).unwrap();
    let attempts = owner.usage().attempts;
    assert_eq!(
        owner.admit_pthread_join(CallArgs([ARG + 256, 0, 0, 0, 0, 0]), &guest),
        Ok(Some(0x8002_000b))
    );
    assert_eq!(owner.usage().attempts, attempts + 1);
}
#[test]
fn join_result_requires_registered_handle_and_failed_output_preserves_wait_state() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    owner.activate_thread_context(ARG + 512).unwrap();
    owner
        .register_thread_context(ARG + 1024, ARG + 256, 700, 0x7f, &mut guest)
        .unwrap();
    owner.park_pthread_join().unwrap();
    guest.put(0, &[0xa5; 8]);
    assert!(
        owner
            .publish_pthread_join(ARG + 1024, ARG + 264, ARG, 77, true, &mut guest)
            .is_err()
    );
    guest.refuse = true;
    assert_eq!(
        owner.publish_pthread_join(ARG + 1024, ARG + 256, ARG, 77, true, &mut guest),
        Err(Failure::Guest(crate::GuestError::WriteDenied))
    );
    assert_eq!(guest.get(ARG, 8), [0xa5; 8]);
    assert!(owner.threads.active().waiting);
    guest.refuse = false;
    owner
        .publish_pthread_join(ARG + 1024, ARG + 256, ARG, u64::MAX - 1, true, &mut guest)
        .unwrap();
    assert_eq!(guest.u64(0), u64::MAX - 1);
    assert!(!owner.threads.active().waiting);
    assert!(
        owner
            .publish_pthread_join(ARG + 1024, ARG + 256, ARG, 77, true, &mut guest)
            .is_err()
    );
}
