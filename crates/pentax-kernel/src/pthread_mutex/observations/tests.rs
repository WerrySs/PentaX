// SPDX-License-Identifier: GPL-2.0-only
// Original deterministic queue transitions; no guest execution or host sleeps.

use crate::tests::{ARG, TestGuest, kernel};
use crate::{CallArgs, ConditionWait, ConditionWaitAction, Failure, Kernel, Limits};
use crate::{MutexWait, MutexWaitAction, Service};
use Service::*;

fn args(values: &[u64]) -> CallArgs {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    CallArgs(args)
}

fn library(owner: &mut Kernel, guest: &mut TestGuest, service: Service, values: &[u64]) -> u64 {
    owner.invoke_library(service, args(values), guest).unwrap()
}

fn condition(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    condition: u64,
    mutex: u64,
) -> ConditionWait {
    let ConditionWaitAction::Waiting(ticket) = owner
        .begin_condition_wait(CondWait, args(&[condition, mutex]), guest)
        .unwrap()
    else {
        panic!("expected retained condition wait");
    };
    ticket
}

fn mutex(owner: &mut Kernel, guest: &mut TestGuest, slot: u64) -> MutexWait {
    let MutexWaitAction::Waiting(ticket) = owner
        .begin_mutex_wait(MutexLock, args(&[slot]), guest)
        .unwrap()
    else {
        panic!("expected retained mutex wait");
    };
    ticket
}

fn counts(owner: &Kernel) -> (u64, u64, u64, u64, u64, bool) {
    let usage = owner.handle_usage();
    (
        usage.pthread_mutex_waits,
        usage.pthread_condition_waits,
        usage.pthread_notified_condition_waits,
        usage.pthread_eligible_mutex_releases,
        usage.pthread_barging_mutex_acquisitions,
        usage.pthread_wait_observations_overflowed,
    )
}

#[test]
fn condition_observations_match_notified_mutex_and_survive_destroy_until_real_reacquisition() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let (first_mutex, second_mutex, first_condition, second_condition) =
        (ARG, ARG + 8, ARG + 16, ARG + 24);
    owner.activate_thread_context(1).unwrap();
    assert_eq!(
        library(&mut owner, &mut guest, MutexLock, &[first_mutex]),
        0
    );
    let first = condition(&mut owner, &mut guest, first_condition, first_mutex);
    owner.activate_thread_context(2).unwrap();
    assert_eq!(
        library(&mut owner, &mut guest, MutexLock, &[second_mutex]),
        0
    );
    let second = condition(&mut owner, &mut guest, second_condition, second_mutex);
    assert_eq!(counts(&owner), (0, 2, 0, 0, 0, false));

    owner.activate_thread_context(3).unwrap();
    // Unnotified condition tickets do not make either mutex eligible.
    assert_eq!(
        library(&mut owner, &mut guest, MutexLock, &[first_mutex]),
        0
    );
    assert_eq!(
        library(&mut owner, &mut guest, MutexUnlock, &[first_mutex]),
        0
    );
    assert_eq!(
        library(&mut owner, &mut guest, CondSignal, &[first_condition]),
        0
    );
    // Repeated signal/broadcast cannot count the same notification twice.
    assert_eq!(
        library(&mut owner, &mut guest, CondSignal, &[first_condition]),
        0
    );
    assert_eq!(
        library(&mut owner, &mut guest, CondBroadcast, &[first_condition]),
        0
    );
    assert_eq!(counts(&owner), (0, 2, 1, 0, 0, false));
    // Notification on another mutex must not cause a false release or barge.
    assert_eq!(
        library(&mut owner, &mut guest, MutexTryLock, &[second_mutex]),
        0
    );
    assert_eq!(
        library(&mut owner, &mut guest, MutexUnlock, &[second_mutex]),
        0
    );
    assert_eq!(counts(&owner), (0, 2, 1, 0, 0, false));
    assert_eq!(
        library(&mut owner, &mut guest, MutexTryLock, &[first_mutex]),
        0
    );
    assert_eq!(counts(&owner), (0, 2, 1, 0, 1, false));
    assert_eq!(
        library(&mut owner, &mut guest, MutexUnlock, &[first_mutex]),
        0
    );
    assert_eq!(counts(&owner), (0, 2, 1, 1, 1, false));
    assert!(owner.complete_condition_wait(&first).is_err());
    assert_eq!(counts(&owner), (0, 2, 1, 1, 1, false));
    owner.activate_thread_context(1).unwrap();
    assert!(owner.complete_condition_wait(&first).unwrap());
    assert!(owner.complete_condition_wait(&first).is_err());
    assert_eq!(counts(&owner), (0, 1, 0, 1, 1, false));
    assert_eq!(
        library(&mut owner, &mut guest, MutexUnlock, &[first_mutex]),
        0
    );

    assert_eq!(
        library(&mut owner, &mut guest, CondSignal, &[second_condition]),
        0
    );
    assert_eq!(
        library(&mut owner, &mut guest, CondDestroy, &[second_condition]),
        0
    );
    // The destroyed condition's notified ticket still owns its mutex binding.
    assert_eq!(counts(&owner), (0, 1, 1, 1, 1, false));
    assert_eq!(
        library(&mut owner, &mut guest, MutexLock, &[second_mutex]),
        0
    );
    assert_eq!(
        library(&mut owner, &mut guest, MutexUnlock, &[second_mutex]),
        0
    );
    owner.activate_thread_context(2).unwrap();
    assert!(owner.complete_condition_wait(&second).unwrap());
    assert_eq!(counts(&owner), (0, 0, 0, 2, 2, false));
    assert_eq!(
        library(&mut owner, &mut guest, MutexUnlock, &[second_mutex]),
        0
    );
    for slot in [first_mutex, second_mutex] {
        assert_eq!(library(&mut owner, &mut guest, MutexDestroy, &[slot]), 0);
    }
    assert_eq!(
        library(&mut owner, &mut guest, CondDestroy, &[first_condition]),
        0
    );
    let before = owner.usage();
    let snapshot = owner.handle_usage();
    assert_eq!(snapshot.accounted(), 0);
    assert_eq!(owner.handle_usage(), snapshot);
    assert_eq!(owner.usage(), before);
    assert_eq!(counts(&owner), (0, 0, 0, 2, 2, false));
}

#[test]
fn mutex_observations_exclude_refusals_and_wait_completion_but_include_condition_release() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    owner.activate_thread_context(1).unwrap();
    assert_eq!(library(&mut owner, &mut guest, MutexLock, &[ARG]), 0);
    owner.activate_thread_context(2).unwrap();
    let pending = mutex(&mut owner, &mut guest, ARG);
    owner.activate_thread_context(3).unwrap();
    assert_eq!(library(&mut owner, &mut guest, PosixMutexUnlock, &[ARG]), 1);
    assert_eq!(
        library(&mut owner, &mut guest, PosixMutexTryLock, &[ARG]),
        16
    );
    assert_eq!(counts(&owner), (1, 0, 0, 0, 0, false));
    owner.activate_thread_context(1).unwrap();
    let maximum = owner.budget.limits.max_work;
    owner.budget.limits.max_work = owner.usage().work;
    assert!(matches!(
        owner.invoke_library(MutexUnlock, args(&[ARG]), &mut guest),
        Err(Failure::Limit(_))
    ));
    assert_eq!(counts(&owner), (1, 0, 0, 0, 0, false));
    owner.budget.limits.max_work = maximum;
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), 0);
    assert_eq!(library(&mut owner, &mut guest, MutexTryLock, &[ARG]), 0);
    assert_eq!(counts(&owner), (1, 0, 0, 1, 1, false));
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), 0);
    owner.activate_thread_context(3).unwrap();
    assert!(matches!(
        owner
            .begin_mutex_wait(MutexLock, args(&[ARG]), &mut guest)
            .unwrap(),
        MutexWaitAction::Returned(0)
    ));
    assert_eq!(counts(&owner), (1, 0, 0, 2, 2, false));
    let condition = condition(&mut owner, &mut guest, ARG + 8, ARG);
    // CondWait's admitted complete release also exposes the ordinary waiter.
    assert_eq!(counts(&owner), (1, 1, 0, 3, 2, false));
    owner.activate_thread_context(2).unwrap();
    assert_eq!(owner.complete_mutex_wait(&pending).unwrap(), Some(0));
    assert!(owner.complete_mutex_wait(&pending).is_err());
    assert_eq!(counts(&owner), (0, 1, 0, 3, 2, false));
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), 0);
    assert_eq!(library(&mut owner, &mut guest, CondSignal, &[ARG + 8]), 0);
    owner.activate_thread_context(3).unwrap();
    assert!(owner.complete_condition_wait(&condition).unwrap());
    assert_eq!(counts(&owner), (0, 0, 0, 3, 2, false));
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), 0);
    assert_eq!(counts(&owner), (0, 0, 0, 3, 2, false));
}

#[test]
fn recursive_decrements_do_not_count_and_diagnostic_saturation_never_refuses_real_transitions() {
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
    for _ in 0..2 {
        assert_eq!(library(&mut owner, &mut guest, MutexLock, &[ARG + 8]), 0);
    }
    owner.activate_thread_context(2).unwrap();
    let pending = mutex(&mut owner, &mut guest, ARG + 8);
    owner.activate_thread_context(1).unwrap();
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG + 8]), 0);
    assert_eq!(counts(&owner), (1, 0, 0, 0, 0, false));
    // Only the diagnostic history is placed at its boundary. Actual ownership,
    // queue membership, guest backing and all admission limits remain real.
    owner.pthread_mutex.eligible_releases = u64::MAX - 1;
    owner.pthread_mutex.barging_acquisitions = u64::MAX - 1;
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG + 8]), 0);
    assert_eq!(library(&mut owner, &mut guest, MutexTryLock, &[ARG + 8]), 0);
    assert_eq!(counts(&owner), (1, 0, 0, u64::MAX, u64::MAX, false));
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG + 8]), 0);
    assert_eq!(library(&mut owner, &mut guest, MutexTryLock, &[ARG + 8]), 0);
    assert_eq!(counts(&owner), (1, 0, 0, u64::MAX, u64::MAX, true));
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG + 8]), 0);
    owner.activate_thread_context(2).unwrap();
    assert_eq!(owner.complete_mutex_wait(&pending).unwrap(), Some(0));
    assert_eq!(counts(&owner), (0, 0, 0, u64::MAX, u64::MAX, true));
    let before = owner.usage();
    let snapshot = owner.handle_usage();
    assert_eq!(owner.handle_usage(), snapshot);
    assert_eq!(owner.usage(), before);
    assert_eq!(counts(&kernel(Limits::default())), (0, 0, 0, 0, 0, false));
}

#[test]
fn exhausted_notification_observer_preserves_real_notifications_ownership_and_guest_budget() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    owner.activate_thread_context(1).unwrap();
    assert_eq!(library(&mut owner, &mut guest, MutexLock, &[ARG]), 0);
    let first = condition(&mut owner, &mut guest, ARG + 16, ARG);
    owner.activate_thread_context(2).unwrap();
    assert_eq!(library(&mut owner, &mut guest, MutexLock, &[ARG + 8]), 0);
    let second = condition(&mut owner, &mut guest, ARG + 24, ARG + 8);
    // One remaining diagnostic visit can track the first actual notification.
    // A later notification must still commit even though its lookup is refused.
    owner.pthread_mutex.observation_lookup_visits = 1;
    owner.activate_thread_context(3).unwrap();
    assert_eq!(library(&mut owner, &mut guest, CondSignal, &[ARG + 16]), 0);
    assert_eq!(counts(&owner), (0, 2, 1, 0, 0, false));
    assert_eq!(owner.pthread_mutex.observation_lookup_visits, 0);
    assert_eq!(library(&mut owner, &mut guest, CondSignal, &[ARG + 24]), 0);
    assert_eq!(counts(&owner), (0, 2, 2, 0, 0, true));
    // History stays partial after disable; real acquisition/release still work.
    assert_eq!(library(&mut owner, &mut guest, MutexLock, &[ARG]), 0);
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), 0);
    assert_eq!(counts(&owner), (0, 2, 2, 0, 0, true));
    owner.activate_thread_context(1).unwrap();
    assert!(owner.complete_condition_wait(&first).unwrap());
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), 0);
    owner.activate_thread_context(2).unwrap();
    assert!(owner.complete_condition_wait(&second).unwrap());
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG + 8]), 0);
    assert_eq!(counts(&owner), (0, 0, 0, 0, 0, true));
    let before = owner.usage();
    let _ = counts(&owner);
    assert_eq!(owner.usage(), before);
    assert_eq!(owner.pthread_mutex.observation_lookup_visits, 0);
}
