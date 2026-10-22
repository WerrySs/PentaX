// SPDX-License-Identifier: GPL-2.0-only
//! Original counted transitions; no host sleep or native execution participates.
use super::*;
use crate::MemoryEffect;
use crate::tests::{ARG, TestGuest, kernel};

fn call(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [u64::MAX; 6];
    args[..values.len()].copy_from_slice(values);
    owner.invoke_library(service, CallArgs(args), guest)
}
fn setup(initial: u64) -> (Kernel, TestGuest) {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    owner.activate_thread_context(1).unwrap();
    for context in [2, 3] {
        owner
            .register_thread_context(context, ARG + 0x200 + context * 8, 700, 0x7f, &mut guest)
            .unwrap();
    }
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PosixSemInit,
            &[ARG, 0x1234_5678, initial]
        ),
        Ok(0)
    );
    (owner, guest)
}
fn wait(owner: &mut Kernel, guest: &mut TestGuest, service: Service) -> PthreadSemaphoreWait {
    let PthreadSemaphoreWaitAction::Waiting(ticket) = owner
        .begin_pthread_semaphore_wait(
            service,
            CallArgs([ARG, u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX]),
            guest,
        )
        .unwrap()
    else {
        panic!("zero count must retain an actual wait");
    };
    ticket
}
fn count(owner: &mut Kernel, guest: &mut TestGuest) -> u32 {
    assert_eq!(
        call(owner, guest, Service::PthreadSemGetValue, &[ARG, ARG + 64]),
        Ok(0)
    );
    guest.u64(64) as u32
}
fn errno(owner: &mut Kernel, guest: &mut TestGuest, value: u32) -> u64 {
    let address = call(owner, guest, Service::ErrnoLocation, &[]).unwrap();
    guest
        .apply(
            MemoryEffect::None,
            &[GuestWrite {
                address,
                bytes: &value.to_le_bytes(),
            }],
        )
        .unwrap();
    address
}

#[test]
fn indefinite_wait_fast_path_and_invalid_returns_keep_exact_errno_domains() {
    for service in [Service::PosixSemWait, Service::PthreadSemWait] {
        let (mut owner, mut guest) = setup(1);
        let cell = errno(&mut owner, &mut guest, 37);
        let before = owner.usage().attempts;
        assert!(matches!(
            owner.begin_pthread_semaphore_wait(
                service,
                CallArgs([ARG, u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX]),
                &mut guest
            ),
            Ok(PthreadSemaphoreWaitAction::Returned(0))
        ));
        assert_eq!(owner.usage().attempts, before + 1);
        assert_eq!(
            guest.get(ARG, 16),
            [
                0xfa, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x78, 0x56, 0x34, 0x12
            ]
        );
        assert_eq!(guest.get(cell, 4), 37_u32.to_le_bytes());
        assert!(
            matches!(owner.begin_pthread_semaphore_wait(service, CallArgs([0; 6]), &mut guest), Ok(PthreadSemaphoreWaitAction::Returned(value)) if value == if service == Service::PosixSemWait { u64::from(u32::MAX) } else { 0x8002_0016 })
        );
        assert_eq!(
            guest.get(cell, 4),
            (if service == Service::PosixSemWait {
                22_u32
            } else {
                37_u32
            })
            .to_le_bytes()
        );
        assert!(owner.pthread_semaphores.waiters.is_empty());
        assert!(!owner.threads.active().waiting);
    }
}

#[test]
fn two_indefinite_waiters_consume_only_actual_posted_counts_and_keep_waiter_prefix() {
    let (mut owner, mut guest) = setup(0);
    let cell = errno(&mut owner, &mut guest, 37);
    let first = wait(&mut owner, &mut guest, Service::PosixSemWait);
    assert_eq!(guest.u64(0), 0x0000_0001_0000_09fa);
    owner.activate_thread_context(2).unwrap();
    let second = wait(&mut owner, &mut guest, Service::PthreadSemWait);
    owner.activate_thread_context(3).unwrap();
    assert_eq!(
        call(&mut owner, &mut guest, Service::PosixSemPost, &[ARG]),
        Ok(0)
    );
    let attempts = owner.usage().attempts;
    assert!(owner.pthread_semaphore_wait_ready(&first).unwrap());
    assert!(owner.pthread_semaphore_wait_ready(&second).unwrap());
    assert!(
        owner
            .complete_pthread_semaphore_wait(&first, &mut guest)
            .is_err()
    );
    owner.activate_thread_context(1).unwrap();
    assert_eq!(
        owner.complete_pthread_semaphore_wait(&first, &mut guest),
        Ok(Some(0))
    );
    assert_eq!(owner.usage().attempts, attempts);
    assert_eq!(guest.get(cell, 4), 37_u32.to_le_bytes());
    assert_eq!(guest.u64(0), 0x0000_0001_0000_09fa);
    assert!(!owner.pthread_semaphore_wait_ready(&second).unwrap());
    owner.activate_thread_context(2).unwrap();
    assert_eq!(
        owner.complete_pthread_semaphore_wait(&second, &mut guest),
        Ok(None)
    );
    owner.activate_thread_context(3).unwrap();
    assert_eq!(
        call(&mut owner, &mut guest, Service::PthreadSemPost, &[ARG]),
        Ok(0)
    );
    owner.activate_thread_context(2).unwrap();
    assert_eq!(
        owner.complete_pthread_semaphore_wait(&second, &mut guest),
        Ok(Some(0))
    );
    assert_eq!(count(&mut owner, &mut guest), 0);
    assert_eq!(guest.u64(0), 0x09fa);
    assert!(owner.pthread_semaphores.waiters.is_empty());
    assert!(owner.pthread_semaphore_wait_ready(&first).is_err());
}

#[test]
fn readiness_does_not_reserve_a_count_and_foreign_or_consumed_tickets_cannot_resume() {
    let (mut owner, mut guest) = setup(0);
    let ticket = wait(&mut owner, &mut guest, Service::PthreadSemWait);
    owner.activate_thread_context(2).unwrap();
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PthreadSemTimedWait,
            &[ARG, 0]
        ),
        Ok(0x8002_003c)
    );
    assert_eq!(guest.u64(0), 0x0000_0001_0000_09fa);
    assert_eq!(
        call(&mut owner, &mut guest, Service::PosixSemTryWait, &[ARG]),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(guest.u64(0), 0x0000_0001_0000_09fa);
    assert_eq!(
        call(&mut owner, &mut guest, Service::PthreadSemPost, &[ARG]),
        Ok(0)
    );
    assert!(owner.pthread_semaphore_wait_ready(&ticket).unwrap());
    assert_eq!(
        call(&mut owner, &mut guest, Service::PthreadSemTryWait, &[ARG]),
        Ok(0)
    );
    assert_eq!(guest.u64(0), 0x0000_0001_0000_09fa);
    owner.activate_thread_context(1).unwrap();
    assert_eq!(
        owner.complete_pthread_semaphore_wait(&ticket, &mut guest),
        Ok(None)
    );
    assert!(owner.threads.active().waiting);
    let (mut foreign, mut foreign_guest) = setup(0);
    let foreign_ticket = wait(&mut foreign, &mut foreign_guest, Service::PosixSemWait);
    assert!(owner.pthread_semaphore_wait_ready(&foreign_ticket).is_err());
    assert!(foreign.pthread_semaphore_wait_ready(&ticket).is_err());
    owner.activate_thread_context(2).unwrap();
    assert_eq!(
        call(&mut owner, &mut guest, Service::PosixSemPost, &[ARG]),
        Ok(0)
    );
    owner.activate_thread_context(1).unwrap();
    assert_eq!(
        owner.complete_pthread_semaphore_wait(&ticket, &mut guest),
        Ok(Some(0))
    );
    assert!(
        owner
            .complete_pthread_semaphore_wait(&ticket, &mut guest)
            .is_err()
    );
    assert_eq!(count(&mut owner, &mut guest), 0);
}

#[test]
fn failed_enrollment_and_resume_are_atomic_and_retained_waiters_block_lifetime_replacement() {
    let (mut owner, mut guest) = setup(0);
    let before = guest.get(ARG, 16);
    guest.refuse = true;
    assert!(matches!(
        owner.begin_pthread_semaphore_wait(
            Service::PosixSemWait,
            CallArgs([ARG, 0, 0, 0, 0, 0]),
            &mut guest
        ),
        Err(Failure::Guest(_))
    ));
    assert!(owner.pthread_semaphores.waiters.is_empty());
    assert!(!owner.threads.active().waiting);
    assert_eq!(guest.get(ARG, 16), before);
    guest.refuse = false;
    let ticket = wait(&mut owner, &mut guest, Service::PosixSemWait);
    owner.activate_thread_context(2).unwrap();
    assert_eq!(
        call(&mut owner, &mut guest, Service::PosixSemPost, &[ARG]),
        Ok(0)
    );
    let posted = guest.get(ARG, 16);
    for service in [
        Service::PosixSemInit,
        Service::PthreadSemInit,
        Service::PosixSemDestroy,
        Service::PthreadSemDestroy,
    ] {
        assert!(matches!(
            call(&mut owner, &mut guest, service, &[ARG, 0, 0]),
            Err(Failure::Unsupported(_))
        ));
        assert_eq!(guest.get(ARG, 16), posted);
    }
    owner.activate_thread_context(1).unwrap();
    guest.refuse = true;
    assert!(matches!(
        owner.complete_pthread_semaphore_wait(&ticket, &mut guest),
        Err(Failure::Guest(_))
    ));
    assert_eq!(guest.get(ARG, 16), posted);
    assert_eq!(owner.pthread_semaphores.waiters.len(), 1);
    assert!(owner.threads.active().waiting);
    guest.refuse = false;
    assert_eq!(count(&mut owner, &mut guest), 1);
    assert_eq!(
        owner.complete_pthread_semaphore_wait(&ticket, &mut guest),
        Ok(Some(0))
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::PthreadSemDestroy, &[ARG]),
        Ok(0)
    );
    assert!(owner.pthread_semaphore_wait_ready(&ticket).is_err());
    assert_eq!(
        call(&mut owner, &mut guest, Service::PosixSemInit, &[ARG, 0, 1]),
        Ok(0)
    );
    assert!(owner.pthread_semaphore_wait_ready(&ticket).is_err());
    assert_eq!(count(&mut owner, &mut guest), 1);
}

#[test]
fn waiter_and_attempt_limits_do_not_publish_membership_and_timed_apis_stay_explicit() {
    let (mut owner, mut guest) = setup(0);
    let before = guest.get(ARG, 16);
    owner.pthread_semaphores.max_waiters = 0;
    assert!(matches!(
        owner.begin_pthread_semaphore_wait(
            Service::PthreadSemWait,
            CallArgs([ARG, 0, 0, 0, 0, 0]),
            &mut guest
        ),
        Err(Failure::Limit("pthread semaphore waiters"))
    ));
    assert_eq!(guest.get(ARG, 16), before);
    assert!(!owner.threads.active().waiting);
    owner.pthread_semaphores.max_waiters = 1;
    owner.budget.limits.max_attempts = owner.usage().attempts;
    assert!(matches!(
        owner.begin_pthread_semaphore_wait(
            Service::PthreadSemWait,
            CallArgs([ARG, 0, 0, 0, 0, 0]),
            &mut guest
        ),
        Err(Failure::Limit("attempts"))
    ));
    assert_eq!(guest.get(ARG, 16), before);
    assert!(owner.pthread_semaphores.waiters.is_empty());
    owner.budget.limits.max_attempts += 4;
    assert!(matches!(
        call(
            &mut owner,
            &mut guest,
            Service::PthreadSemTimedWait,
            &[ARG, 1]
        ),
        Err(Failure::Unsupported("POSIX semaphore blocking wait"))
    ));
    assert!(matches!(
        owner.begin_pthread_semaphore_wait(
            Service::PosixSemTryWait,
            CallArgs([ARG, 0, 0, 0, 0, 0]),
            &mut guest
        ),
        Err(Failure::Unsupported("pthread semaphore wait service"))
    ));
}
