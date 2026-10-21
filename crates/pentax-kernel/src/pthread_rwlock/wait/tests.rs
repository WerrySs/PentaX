// SPDX-License-Identifier: GPL-2.0-only
//! Original deterministic ownership and wait scenarios; no sleeps or execution.
use super::*;
use crate::Kernel;
use crate::tests::{ARG, TestGuest, kernel};
use Service::*;

fn owner(limits: Limits) -> Kernel {
    let mut owner = kernel(limits);
    owner.activate_thread_context(1).unwrap();
    owner
}
fn args(values: &[u64]) -> CallArgs {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    CallArgs(args)
}
fn call(owner: &mut Kernel, guest: &mut TestGuest, service: Service, values: &[u64]) -> u64 {
    owner.invoke_library(service, args(values), guest).unwrap()
}
fn waiting(action: Result<RwlockWaitAction, Failure>) -> RwlockWait {
    match action.unwrap() {
        RwlockWaitAction::Waiting(ticket) => ticket,
        other => panic!("expected real wait: {other:?}"),
    }
}
fn returned(action: Result<RwlockWaitAction, Failure>) -> u64 {
    match action.unwrap() {
        RwlockWaitAction::Returned(value) => value,
        other => panic!("expected immediate result: {other:?}"),
    }
}

#[test]
fn multiple_readers_recursive_readers_and_writer_preference_transfer_real_ownership() {
    let mut owner = owner(Default::default());
    let mut guest = TestGuest::new();
    assert_eq!(call(&mut owner, &mut guest, RwlockReadLock, &[ARG]), 0);
    let id = guest.u64(0);
    guest.put(8, &id.to_le_bytes()); // Actual owned prefix alias.
    owner.activate_thread_context(2).unwrap();
    assert_eq!(call(&mut owner, &mut guest, RwlockReadLock, &[ARG + 8]), 0);
    assert_eq!(owner.pthread_rwlock.readers.len(), 2);
    owner.activate_thread_context(3).unwrap();
    let attempts = owner.usage().attempts;
    let writer = waiting(owner.begin_rwlock_wait(PosixRwlockWriteLock, args(&[ARG]), &mut guest));
    assert_eq!(owner.usage().attempts, attempts + 1);
    assert_eq!(owner.pthread_rwlock.locks[0].waiting_writers, 1);
    assert!(!owner.rwlock_wait_ready(&writer).unwrap());
    assert_eq!(owner.rwlock_wait_delay(&writer).unwrap(), None);
    owner.activate_thread_context(1).unwrap();
    assert_eq!(call(&mut owner, &mut guest, RwlockTryReadLock, &[ARG]), 0);
    assert_eq!(
        call(&mut owner, &mut guest, PosixRwlockWriteLock, &[ARG]),
        11
    );
    assert_eq!(call(&mut owner, &mut guest, RwlockUnlock, &[ARG]), 0);
    owner.activate_thread_context(4).unwrap();
    assert_eq!(
        call(&mut owner, &mut guest, RwlockTryReadLock, &[ARG]),
        0x8002_0010
    );
    let reader = waiting(owner.begin_rwlock_wait(RwlockReadLock, args(&[ARG]), &mut guest));
    assert!(!owner.rwlock_wait_ready(&reader).unwrap());
    owner.activate_thread_context(1).unwrap();
    assert_eq!(
        call(&mut owner, &mut guest, RwlockDestroy, &[ARG]),
        0x8002_0010
    );
    assert_eq!(call(&mut owner, &mut guest, RwlockUnlock, &[ARG]), 0);
    assert!(!owner.rwlock_wait_ready(&writer).unwrap()); // The other real reader remains.
    owner.activate_thread_context(2).unwrap();
    assert_eq!(call(&mut owner, &mut guest, RwlockUnlock, &[ARG + 8]), 0);
    assert!(owner.rwlock_wait_ready(&writer).unwrap());
    assert!(!owner.rwlock_wait_ready(&reader).unwrap());
    owner.activate_thread_context(3).unwrap();
    let attempts = owner.usage().attempts;
    assert_eq!(owner.complete_rwlock_wait(&writer).unwrap(), Some(0));
    assert_eq!(owner.usage().attempts, attempts);
    assert!(!owner.rwlock_wait_ready(&reader).unwrap());
    assert_eq!(call(&mut owner, &mut guest, RwlockReadLock, &[ARG]), 0);
    assert_eq!(call(&mut owner, &mut guest, RwlockUnlock, &[ARG]), 0); // Own reader first.
    assert!(owner.pthread_rwlock.locks[0].writer.is_some());
    assert_eq!(call(&mut owner, &mut guest, RwlockUnlock, &[ARG]), 0);
    owner.activate_thread_context(4).unwrap();
    assert_eq!(owner.complete_rwlock_wait(&reader).unwrap(), Some(0));
    assert_eq!(call(&mut owner, &mut guest, RwlockUnlock, &[ARG]), 0);
    assert_eq!(call(&mut owner, &mut guest, RwlockDestroy, &[ARG]), 0);
    assert!(owner.pthread_rwlock.readers.is_empty() && owner.pthread_rwlock.waiters.is_empty());
    assert_eq!(owner.pthread_rwlock.reader_reservations, 0);
}

#[test]
fn relative_low32_deadlines_preserve_predicate_before_timeout_and_error_domains() {
    for write in [false, true] {
        let mut owner = owner(Limits {
            max_sleep_nanoseconds: 0,
            ..Default::default()
        });
        let mut guest = TestGuest::new();
        let now = Instant::now();
        let held = if write {
            RwlockReadLock
        } else {
            RwlockWriteLock
        };
        let timed = if write {
            RwlockTimedWriteLock
        } else {
            RwlockTimedReadLock
        };
        assert_eq!(call(&mut owner, &mut guest, held, &[ARG]), 0);
        owner.activate_thread_context(2).unwrap();
        assert_eq!(
            returned(owner.begin_rwlock_wait_at(timed, args(&[ARG, 0]), &mut guest, now)),
            0x8002_003c
        );
        let ticket = waiting(owner.begin_rwlock_wait_at(
            timed,
            args(&[ARG, 0xabcd_0000_0000_0005]),
            &mut guest,
            now,
        ));
        assert_eq!(
            owner.pthread_rwlock.waiters[0].deadline,
            Some(now + Duration::from_micros(5))
        );
        assert!(
            !owner
                .rwlock_wait_ready_at(&ticket, now + Duration::from_micros(4))
                .unwrap()
        );
        assert_eq!(
            owner
                .complete_rwlock_wait_at(&ticket, now + Duration::from_micros(4))
                .unwrap(),
            None
        );
        assert_eq!(
            owner
                .complete_rwlock_wait_at(&ticket, now + Duration::from_micros(5))
                .unwrap(),
            Some(0x8002_003c)
        );
        assert_eq!(owner.pthread_rwlock.locks[0].waiting_writers, 0);
        assert_eq!(owner.pthread_rwlock.reader_reservations, 0);
        let late = waiting(owner.begin_rwlock_wait_at(timed, args(&[ARG, 5]), &mut guest, now));
        owner.activate_thread_context(1).unwrap();
        assert_eq!(call(&mut owner, &mut guest, RwlockUnlock, &[ARG]), 0);
        owner.activate_thread_context(2).unwrap();
        assert_eq!(
            owner
                .complete_rwlock_wait_at(&late, now + Duration::from_secs(1))
                .unwrap(),
            Some(0)
        );
        assert_eq!(call(&mut owner, &mut guest, RwlockUnlock, &[ARG]), 0);
        assert_eq!(
            returned(owner.begin_rwlock_wait_at(timed, args(&[ARG, 0]), &mut guest, now)),
            0
        );
        assert_eq!(call(&mut owner, &mut guest, RwlockUnlock, &[ARG]), 0);
    }
}

#[test]
fn wait_identity_cancellation_and_original_object_alias_survive_without_replay_or_refunds() {
    let mut first = owner(Default::default());
    let mut second = owner(Default::default());
    let mut a = TestGuest::new();
    let mut b = TestGuest::new();
    for (owner, guest) in [(&mut first, &mut a), (&mut second, &mut b)] {
        assert_eq!(call(owner, guest, RwlockReadLock, &[ARG]), 0);
        owner.activate_thread_context(2).unwrap();
    }
    let one = waiting(first.begin_rwlock_wait(RwlockWriteLock, args(&[ARG]), &mut a));
    let two = waiting(second.begin_rwlock_wait(RwlockWriteLock, args(&[ARG]), &mut b));
    assert!(first.rwlock_wait_ready(&two).is_err());
    assert!(second.cancel_rwlock_wait(&one).is_err());
    assert_eq!(first.pthread_rwlock.waiters.len(), 1);
    first.activate_thread_context(1).unwrap();
    assert!(first.cancel_rwlock_wait(&one).is_err());
    assert!(first.complete_rwlock_wait(&one).is_err());
    assert_eq!(call(&mut first, &mut a, RwlockTryReadLock, &[ARG]), 0);
    first.activate_thread_context(3).unwrap();
    assert_eq!(
        call(&mut first, &mut a, RwlockTryReadLock, &[ARG]),
        0x8002_0010
    );
    first.activate_thread_context(2).unwrap();
    let attempts = first.usage().attempts;
    first.cancel_rwlock_wait(&one).unwrap();
    assert_eq!(first.usage().attempts, attempts);
    assert_eq!(first.pthread_rwlock.locks[0].waiting_writers, 0);
    assert!(first.cancel_rwlock_wait(&one).is_err());
    first.activate_thread_context(3).unwrap();
    assert_eq!(call(&mut first, &mut a, RwlockReadLock, &[ARG]), 0); // No stale writer preference.
    assert_eq!(call(&mut first, &mut a, RwlockUnlock, &[ARG]), 0);
    first.activate_thread_context(2).unwrap();
    let later = waiting(first.begin_rwlock_wait(RwlockWriteLock, args(&[ARG]), &mut a));
    assert!(first.complete_rwlock_wait(&one).is_err());
    let id = a.u64(0);
    a.put(8, &id.to_le_bytes());
    a.put(0, &u64::MAX.to_le_bytes());
    first.activate_thread_context(1).unwrap();
    assert_eq!(call(&mut first, &mut a, RwlockUnlock, &[ARG + 8]), 0);
    assert_eq!(call(&mut first, &mut a, RwlockUnlock, &[ARG + 8]), 0);
    first.activate_thread_context(2).unwrap();
    assert_eq!(first.complete_rwlock_wait(&later).unwrap(), Some(0));
    assert_eq!(call(&mut first, &mut a, RwlockUnlock, &[ARG + 8]), 0);
    assert!(first.rwlock_wait_ready(&later).is_err());
}

#[test]
fn reader_records_are_preadmitted_for_waiters_and_quota_failures_preserve_membership() {
    let exhausted = AtomicU64::new(u64::MAX);
    assert!(matches!(
        next_serial(&exhausted),
        Err(Failure::Limit("rwlock wait identities"))
    ));
    assert_eq!(exhausted.load(Ordering::Relaxed), u64::MAX);
    let mut faulted = self::owner(Default::default());
    let mut denied = TestGuest::new();
    denied.regions[0].protection = 1;
    assert!(matches!(
        faulted.begin_rwlock_wait(RwlockReadLock, args(&[ARG]), &mut denied),
        Err(Failure::Guest(_))
    ));
    assert!(faulted.pthread_rwlock.locks.is_empty());
    assert!(faulted.pthread_rwlock.readers.is_empty() && faulted.pthread_rwlock.waiters.is_empty());
    assert!(!faulted.threads.active().waiting);
    assert_eq!(faulted.usage().handles_live, 0);
    let mut owner = owner(Limits {
        max_thread_contexts: 2,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    assert_eq!(call(&mut owner, &mut guest, RwlockInit, &[ARG, 0, 0]), 0);
    assert_eq!(
        call(&mut owner, &mut guest, RwlockInit, &[ARG + 8, 0, 0]),
        0
    );
    assert_eq!(call(&mut owner, &mut guest, RwlockWriteLock, &[ARG]), 0);
    owner.activate_thread_context(2).unwrap();
    let ticket = waiting(owner.begin_rwlock_wait(RwlockReadLock, args(&[ARG]), &mut guest));
    assert_eq!(owner.pthread_rwlock.reader_reservations, 1);
    owner.activate_thread_context(1).unwrap();
    assert_eq!(call(&mut owner, &mut guest, RwlockReadLock, &[ARG]), 0);
    let resources = (owner.usage().handles_live, owner.usage().mapped_bytes);
    for adapter in [false, true] {
        if adapter {
            assert!(matches!(
                owner.begin_rwlock_wait(RwlockReadLock, args(&[ARG + 16]), &mut guest),
                Err(Failure::Limit("rwlock reader records"))
            ));
        } else {
            assert!(matches!(
                owner.invoke_library(RwlockTryReadLock, args(&[ARG + 16]), &mut guest),
                Err(Failure::Limit("rwlock reader records"))
            ));
        }
        assert_eq!(guest.u64(16), 0);
        assert_eq!(
            (owner.usage().handles_live, owner.usage().mapped_bytes),
            resources
        );
        assert!(!owner.threads.active().waiting);
    }
    let before = (
        owner.pthread_rwlock.readers.len(),
        owner.pthread_rwlock.waiters.len(),
        guest.u64(8),
    );
    assert!(matches!(
        owner.invoke_library(RwlockReadLock, args(&[ARG + 8]), &mut guest),
        Err(Failure::Limit("rwlock reader records"))
    ));
    assert_eq!(
        (
            owner.pthread_rwlock.readers.len(),
            owner.pthread_rwlock.waiters.len(),
            guest.u64(8)
        ),
        before
    );
    assert_eq!(call(&mut owner, &mut guest, RwlockUnlock, &[ARG]), 0);
    assert_eq!(call(&mut owner, &mut guest, RwlockUnlock, &[ARG]), 0);
    owner.activate_thread_context(2).unwrap();
    assert_eq!(owner.complete_rwlock_wait(&ticket).unwrap(), Some(0));
    assert_eq!(owner.pthread_rwlock.reader_reservations, 0);
    assert_eq!(call(&mut owner, &mut guest, RwlockUnlock, &[ARG]), 0);
    owner.activate_thread_context(1).unwrap();
    assert_eq!(call(&mut owner, &mut guest, RwlockReadLock, &[ARG]), 0);
    owner.activate_thread_context(2).unwrap();
    let ticket = waiting(owner.begin_rwlock_wait(RwlockWriteLock, args(&[ARG]), &mut guest));
    let usage = owner.usage();
    owner.budget.limits.max_work = usage.work;
    assert!(matches!(
        owner.complete_rwlock_wait(&ticket),
        Err(Failure::Limit("work"))
    ));
    assert!(matches!(
        owner.cancel_rwlock_wait(&ticket),
        Err(Failure::Limit("work"))
    ));
    assert_eq!(owner.pthread_rwlock.waiters.len(), 1);
    assert_eq!(owner.pthread_rwlock.locks[0].waiting_writers, 1);
    assert!(owner.threads.active().waiting);
    assert_eq!(owner.usage(), usage);
}
