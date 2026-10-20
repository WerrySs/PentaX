// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use Service::*;

fn library(owner: &mut Kernel, guest: &mut TestGuest, service: Service, values: &[u64]) -> u64 {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    owner
        .invoke_library(service, CallArgs(args), guest)
        .unwrap()
}
fn wait(owner: &mut Kernel, guest: &mut TestGuest, condition: u64, mutex: u64) -> ConditionWait {
    let ConditionWaitAction::Waiting(ticket) = owner
        .begin_condition_wait(CondWait, CallArgs([condition, mutex, 0, 0, 0, 0]), guest)
        .unwrap()
    else {
        panic!("expected pending wait");
    };
    ticket
}

#[test]
fn condition_wait_releases_all_recursive_ownership_and_reacquires_before_completion() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    owner.activate_thread_context(1).unwrap();
    assert_eq!(library(&mut owner, &mut guest, MutexAttrInit, &[ARG]), 0);
    assert_eq!(
        library(&mut owner, &mut guest, MutexAttrSetType, &[ARG, 2]),
        0
    );
    assert_eq!(
        library(&mut owner, &mut guest, MutexInit, &[ARG + 8, ARG, 0]),
        0
    );
    for _ in 0..3 {
        assert_eq!(library(&mut owner, &mut guest, MutexLock, &[ARG + 8]), 0);
    }
    // A notification with no waiter is not retained for a future wait.
    assert_eq!(library(&mut owner, &mut guest, CondSignal, &[ARG + 16]), 0);
    let ticket = wait(&mut owner, &mut guest, ARG + 16, ARG + 8);
    assert!(!owner.condition_wait_ready(&ticket).unwrap());
    assert!(!owner.complete_condition_wait(&ticket).unwrap());
    assert_eq!(
        library(&mut owner, &mut guest, MutexDestroy, &[ARG + 8]),
        0x8002_0010
    );
    assert_eq!(
        library(&mut owner, &mut guest, CondDestroy, &[ARG + 16]),
        0x8002_0010
    );
    owner.activate_thread_context(2).unwrap();
    assert_eq!(library(&mut owner, &mut guest, MutexTryLock, &[ARG + 8]), 0);
    assert_eq!(
        library(&mut owner, &mut guest, PosixCondSignal, &[ARG + 16]),
        0
    );
    assert!(!owner.condition_wait_ready(&ticket).unwrap());
    assert!(owner.complete_condition_wait(&ticket).is_err());
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG + 8]), 0);
    assert!(owner.condition_wait_ready(&ticket).unwrap());
    owner.activate_thread_context(1).unwrap();
    // A late resume-admission failure may not acquire the mutex or consume the
    // notification; the same saved wait remains resumable after restoring work.
    owner.budget.limits.max_work = owner.usage().work + 2;
    assert!(matches!(
        owner.complete_condition_wait(&ticket),
        Err(Failure::Limit(_))
    ));
    owner.budget.limits.max_work = Limits::default().max_work;
    assert!(owner.condition_wait_ready(&ticket).unwrap());
    assert!(owner.complete_condition_wait(&ticket).unwrap());
    assert!(owner.complete_condition_wait(&ticket).is_err());
    // All three recursive acquisitions must be restored, not just one.
    for _ in 0..2 {
        assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG + 8]), 0);
        owner.activate_thread_context(2).unwrap();
        assert_eq!(
            library(&mut owner, &mut guest, PosixMutexTryLock, &[ARG + 8]),
            16
        );
        owner.activate_thread_context(1).unwrap();
    }
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG + 8]), 0);
    owner.activate_thread_context(2).unwrap();
    assert_eq!(library(&mut owner, &mut guest, MutexTryLock, &[ARG + 8]), 0);
}

#[test]
fn condition_signal_is_fifo_broadcast_wakes_remaining_waiters_and_tickets_are_owner_bound() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    owner.activate_thread_context(1).unwrap();
    assert_eq!(library(&mut owner, &mut guest, MutexLock, &[ARG]), 0);
    let first = wait(&mut owner, &mut guest, ARG + 8, ARG);
    owner.activate_thread_context(2).unwrap();
    assert_eq!(library(&mut owner, &mut guest, MutexLock, &[ARG]), 0);
    let second = wait(&mut owner, &mut guest, ARG + 8, ARG);
    owner.activate_thread_context(3).unwrap();
    assert_eq!(library(&mut owner, &mut guest, CondSignal, &[ARG + 8]), 0);
    assert!(owner.condition_wait_ready(&first).unwrap());
    assert!(!owner.condition_wait_ready(&second).unwrap());
    owner.activate_thread_context(1).unwrap();
    assert!(owner.complete_condition_wait(&first).unwrap());
    assert_eq!(
        library(&mut owner, &mut guest, CondBroadcast, &[ARG + 8]),
        0
    );
    assert!(!owner.condition_wait_ready(&second).unwrap());
    // A notified waiter no longer uses the condition object, but retains its
    // mutex reservation until it actually reacquires and consumes its ticket.
    assert_eq!(library(&mut owner, &mut guest, CondDestroy, &[ARG + 8]), 0);
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), 0);
    assert_eq!(
        library(&mut owner, &mut guest, MutexDestroy, &[ARG]),
        0x8002_0010
    );
    assert!(owner.condition_wait_ready(&second).unwrap());
    let mut foreign = kernel(Default::default());
    let mut other_guest = TestGuest::new();
    foreign.activate_thread_context(1).unwrap();
    assert_eq!(
        library(&mut foreign, &mut other_guest, MutexLock, &[ARG]),
        0
    );
    let unrelated = wait(&mut foreign, &mut other_guest, ARG + 8, ARG);
    assert!(foreign.condition_wait_ready(&second).is_err());
    assert!(owner.condition_wait_ready(&unrelated).is_err());
    owner.activate_thread_context(2).unwrap();
    assert!(owner.complete_condition_wait(&second).unwrap());
}

#[test]
fn condition_wait_errors_and_admission_failure_preserve_mutex_ownership() {
    for service in [CondWait, PosixCondWait] {
        let mut owner = kernel(Default::default());
        let mut guest = TestGuest::new();
        owner.activate_thread_context(1).unwrap();
        assert_eq!(library(&mut owner, &mut guest, MutexLock, &[ARG]), 0);
        owner.activate_thread_context(2).unwrap();
        let before = owner.usage().attempts;
        let result = owner
            .begin_condition_wait(service, CallArgs([ARG + 8, ARG, 0, 0, 0, 0]), &mut guest)
            .unwrap();
        assert!(
            matches!(result, ConditionWaitAction::Returned(value) if value ==
            if service == CondWait { 0x8002_0001 } else { 1 })
        );
        assert_eq!(owner.usage().attempts, before + 1);
        assert_eq!(
            library(&mut owner, &mut guest, PosixMutexTryLock, &[ARG]),
            16
        );
        owner.activate_thread_context(1).unwrap();
        let work = owner.usage().work;
        owner.budget.limits.max_work = work;
        assert!(matches!(
            owner.begin_condition_wait(service, CallArgs([ARG + 8, ARG, 0, 0, 0, 0]), &mut guest),
            Err(Failure::Limit(_))
        ));
        owner.budget.limits.max_work = Limits::default().max_work;
        owner.activate_thread_context(2).unwrap();
        assert_eq!(
            library(&mut owner, &mut guest, PosixMutexTryLock, &[ARG]),
            16
        );
        let result = owner
            .begin_condition_wait(service, CallArgs([0; 6]), &mut guest)
            .unwrap();
        assert!(
            matches!(result, ConditionWaitAction::Returned(value) if value ==
            if service == CondWait { 0x8002_0016 } else { 22 })
        );
    }
}
