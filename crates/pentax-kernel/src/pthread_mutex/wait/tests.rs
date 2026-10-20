// SPDX-License-Identifier: GPL-2.0-only
// Original deterministic cooperative mutex scenarios; no guest execution or sleeps.

use super::*;
use crate::Kernel;
use crate::tests::{ARG, TestGuest, kernel};
use Service::*;

fn owner(limits: Limits) -> Kernel {
    let mut owner = kernel(limits);
    owner.activate_thread_context(1).unwrap();
    owner
}
fn arguments(values: &[u64]) -> CallArgs {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    CallArgs(args)
}
fn library(owner: &mut Kernel, guest: &mut TestGuest, service: Service, values: &[u64]) -> u64 {
    owner
        .invoke_library(service, arguments(values), guest)
        .unwrap()
}
fn returned(action: Result<MutexWaitAction, Failure>) -> u64 {
    match action.unwrap() {
        MutexWaitAction::Returned(value) => value,
        action => panic!("expected immediate result: {action:?}"),
    }
}
fn waiting(action: Result<MutexWaitAction, Failure>) -> MutexWait {
    match action.unwrap() {
        MutexWaitAction::Waiting(ticket) => ticket,
        action => panic!("expected pending wait: {action:?}"),
    }
}
fn begin(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
    now: Instant,
) -> Result<MutexWaitAction, Failure> {
    owner.begin_mutex_wait_at(service, arguments(values), guest, now)
}

#[test]
fn mutex_wait_protocols_preserve_real_exclusion_one_attempt_and_owner_thread_completion() {
    for protocol in 0..=2 {
        let mut owner = owner(Default::default());
        let mut guest = TestGuest::new();
        assert_eq!(library(&mut owner, &mut guest, MutexAttrInit, &[ARG]), 0);
        assert_eq!(
            library(
                &mut owner,
                &mut guest,
                MutexAttrSetProtocol,
                &[ARG, protocol]
            ),
            0
        );
        assert_eq!(
            library(&mut owner, &mut guest, MutexInit, &[ARG + 8, ARG, 0]),
            0
        );
        assert_eq!(library(&mut owner, &mut guest, MutexAttrDestroy, &[ARG]), 0);
        assert_eq!(
            returned(owner.begin_mutex_wait(MutexLock, arguments(&[ARG + 8]), &mut guest)),
            0
        );
        let parent = owner.threads.active().id;
        owner.activate_thread_context(2).unwrap();
        let child = owner.threads.active().id;
        let attempts = owner.usage().attempts;
        let ticket =
            waiting(owner.begin_mutex_wait(PosixMutexLock, arguments(&[ARG + 8]), &mut guest));
        assert_eq!(owner.usage().attempts, attempts + 1);
        assert_eq!(
            (
                owner.pthread_mutex.mutexes[0].owner,
                owner.pthread_mutex.mutexes[0].count
            ),
            (Some(parent), 1)
        );
        assert!(!owner.mutex_wait_ready(&ticket).unwrap());
        assert_eq!(owner.mutex_wait_delay(&ticket).unwrap(), None);
        assert_eq!(owner.complete_mutex_wait(&ticket).unwrap(), None);
        owner.activate_thread_context(1).unwrap();
        assert!(owner.complete_mutex_wait(&ticket).is_err());
        assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG + 8]), 0);
        assert!(owner.mutex_wait_ready(&ticket).unwrap());
        assert_eq!(
            owner.mutex_wait_delay(&ticket).unwrap(),
            Some(Duration::ZERO)
        );
        assert_eq!(
            library(&mut owner, &mut guest, MutexDestroy, &[ARG + 8]),
            0x8002_0010
        );
        owner.activate_thread_context(2).unwrap();
        let attempts = owner.usage().attempts;
        assert_eq!(owner.complete_mutex_wait(&ticket).unwrap(), Some(0));
        assert_eq!(owner.usage().attempts, attempts);
        assert_eq!(
            (
                owner.pthread_mutex.mutexes[0].owner,
                owner.pthread_mutex.mutexes[0].count
            ),
            (Some(child), 1)
        );
        assert!(owner.mutex_wait_ready(&ticket).is_err());
        assert!(owner.complete_mutex_wait(&ticket).is_err());
        assert_eq!(
            library(&mut owner, &mut guest, PosixMutexUnlock, &[ARG + 8]),
            0
        );
        assert_eq!(library(&mut owner, &mut guest, MutexDestroy, &[ARG + 8]), 0);
        assert_eq!(
            (owner.usage().handles_live, owner.usage().mapped_bytes),
            (0, 0)
        );
    }
}

#[test]
fn mutex_wait_tickets_are_process_bound_consumed_once_and_keep_original_resolved_slot_identity() {
    let mut first = owner(Default::default());
    let mut second = owner(Default::default());
    let mut a = TestGuest::new();
    let mut b = TestGuest::new();
    for (owner, guest) in [(&mut first, &mut a), (&mut second, &mut b)] {
        assert_eq!(library(owner, guest, MutexLock, &[ARG]), 0);
        owner.activate_thread_context(2).unwrap();
    }
    let one = waiting(first.begin_mutex_wait(MutexLock, arguments(&[ARG]), &mut a));
    let two = waiting(second.begin_mutex_wait(MutexLock, arguments(&[ARG]), &mut b));
    assert!(first.mutex_wait_ready(&two).is_err());
    assert!(second.complete_mutex_wait(&one).is_err());
    assert_eq!(first.pthread_mutex.waiters.len(), 1);
    assert_eq!(second.pthread_mutex.waiters.len(), 1);
    let original = a.u64(0);
    a.put(8, &original.to_le_bytes());
    a.put(0, &u64::MAX.to_le_bytes());
    first.activate_thread_context(1).unwrap();
    assert_eq!(library(&mut first, &mut a, MutexUnlock, &[ARG + 8]), 0);
    first.activate_thread_context(2).unwrap();
    assert_eq!(first.complete_mutex_wait(&one).unwrap(), Some(0));
    assert_eq!(library(&mut first, &mut a, MutexUnlock, &[ARG + 8]), 0);
    first.activate_thread_context(1).unwrap();
    assert_eq!(library(&mut first, &mut a, MutexLock, &[ARG + 8]), 0);
    first.activate_thread_context(2).unwrap();
    let later = waiting(first.begin_mutex_wait(MutexLock, arguments(&[ARG + 8]), &mut a));
    assert!(first.complete_mutex_wait(&one).is_err());
    assert_eq!(first.pthread_mutex.waiters.len(), 1);
    assert!(first.threads.active().waiting);
    first.activate_thread_context(1).unwrap();
    assert_eq!(library(&mut first, &mut a, MutexUnlock, &[ARG + 8]), 0);
    first.activate_thread_context(2).unwrap();
    assert_eq!(first.complete_mutex_wait(&later).unwrap(), Some(0));
}

#[test]
fn mutex_wait_relative_width_absolute_validation_and_deadline_predicate_preserve_source_domains() {
    let mut owner = owner(Limits {
        max_sleep_nanoseconds: 0,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    let now = Instant::now();
    assert_eq!(library(&mut owner, &mut guest, MutexLock, &[ARG]), 0);
    let parent = owner.threads.active().id;
    owner.activate_thread_context(2).unwrap();
    let ticket = waiting(begin(
        &mut owner,
        &mut guest,
        MutexTimedLock,
        &[ARG, 0xabcd_0000_0000_0005],
        now,
    ));
    assert_eq!(
        owner.pthread_mutex.waiters[0].deadline,
        Some(now + Duration::from_micros(5))
    );
    assert!(
        !owner
            .mutex_wait_ready_at(&ticket, now + Duration::from_micros(4))
            .unwrap()
    );
    assert_eq!(
        owner
            .complete_mutex_wait_at(&ticket, now + Duration::from_micros(4))
            .unwrap(),
        None
    );
    assert!(
        owner
            .mutex_wait_ready_at(&ticket, now + Duration::from_micros(5))
            .unwrap()
    );
    assert_eq!(
        owner
            .complete_mutex_wait_at(&ticket, now + Duration::from_micros(5))
            .unwrap(),
        Some(0x8002_003c)
    );
    assert_eq!(owner.pthread_mutex.mutexes[0].owner, Some(parent));
    assert_eq!(
        returned(begin(
            &mut owner,
            &mut guest,
            PosixMutexTimedLock,
            &[u64::MAX, 0],
            now
        )),
        22
    );
    guest.put(32, &[0; 16]);
    guest.put(40, &1_000_000_000i64.to_le_bytes());
    assert_eq!(
        returned(begin(
            &mut owner,
            &mut guest,
            PosixMutexTimedLock,
            &[u64::MAX, ARG + 32],
            now
        )),
        22
    );
    guest.put(32, &[0; 16]);
    assert_eq!(
        returned(begin(
            &mut owner,
            &mut guest,
            PosixMutexTimedLock,
            &[ARG, ARG + 32],
            now
        )),
        60
    );
    let future = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        + Duration::from_secs(2);
    guest.put(32, &(future.as_secs() as i64).to_le_bytes());
    guest.put(40, &i64::from(future.subsec_nanos()).to_le_bytes());
    let positive = waiting(begin(
        &mut owner,
        &mut guest,
        PosixMutexTimedLock,
        &[ARG, ARG + 32],
        now,
    ));
    let deadline = owner.pthread_mutex.waiters[0].deadline.unwrap();
    assert_eq!(
        owner.complete_mutex_wait_at(&positive, deadline).unwrap(),
        Some(60)
    );
    let full = waiting(begin(
        &mut owner,
        &mut guest,
        MutexTimedLock,
        &[ARG, u64::MAX],
        now,
    ));
    assert_eq!(
        owner.pthread_mutex.waiters[0].deadline,
        Some(now + Duration::from_micros(u64::from(u32::MAX)))
    );
    owner.activate_thread_context(1).unwrap();
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), 0);
    owner.activate_thread_context(2).unwrap();
    // Source tests owner before its timeout loop: a free mutex wins even on a late wake.
    assert_eq!(
        owner
            .complete_mutex_wait_at(&full, now + Duration::from_secs(5000))
            .unwrap(),
        Some(0)
    );
    assert_eq!(owner.usage().attempts, 8); // Initial lock, six begin calls and the owner unlock.
}

#[test]
fn mutex_wait_self_deadlock_self_timeout_recursive_overflow_and_errno_are_independent() {
    let mut owner = owner(Default::default());
    let mut guest = TestGuest::new();
    let now = Instant::now();
    let errno = library(&mut owner, &mut guest, Service::ErrnoLocation, &[]);
    guest
        .apply(
            crate::MemoryEffect::None,
            &[crate::GuestWrite {
                address: errno,
                bytes: &37u32.to_le_bytes(),
            }],
        )
        .unwrap();
    assert_eq!(
        returned(begin(&mut owner, &mut guest, MutexLock, &[ARG], now)),
        0
    );
    let current = owner.threads.active().id;
    assert_eq!(
        returned(begin(&mut owner, &mut guest, MutexLock, &[ARG], now)),
        0x8002_000b
    );
    assert_eq!(
        returned(begin(&mut owner, &mut guest, PosixMutexLock, &[ARG], now)),
        11
    );
    assert_eq!(
        library(&mut owner, &mut guest, PosixMutexTryLock, &[ARG]),
        16
    );
    let self_timed = waiting(begin(
        &mut owner,
        &mut guest,
        MutexTimedLock,
        &[ARG, 5],
        now,
    ));
    assert!(
        !owner
            .mutex_wait_ready_at(&self_timed, now + Duration::from_micros(4))
            .unwrap()
    );
    assert_eq!(
        owner
            .complete_mutex_wait_at(&self_timed, now + Duration::from_micros(5))
            .unwrap(),
        Some(0x8002_003c)
    );
    assert_eq!(owner.pthread_mutex.mutexes[0].owner, Some(current));
    assert_eq!(owner.pthread_mutex.mutexes[0].count, 1);
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), 0);
    assert_eq!(library(&mut owner, &mut guest, MutexDestroy, &[ARG]), 0);
    assert_eq!(
        library(&mut owner, &mut guest, MutexAttrInit, &[ARG + 8]),
        0
    );
    assert_eq!(
        library(&mut owner, &mut guest, MutexAttrSetType, &[ARG + 8, 2]),
        0
    );
    assert_eq!(
        library(&mut owner, &mut guest, MutexInit, &[ARG, ARG + 8, 0]),
        0
    );
    assert_eq!(
        returned(begin(&mut owner, &mut guest, MutexLock, &[ARG], now)),
        0
    );
    // Recursive acquisition succeeds even for an expired/zero relative deadline.
    assert_eq!(
        returned(begin(
            &mut owner,
            &mut guest,
            PosixMutexTimedLock,
            &[ARG, ARG + 32],
            now
        )),
        0
    );
    assert_eq!(owner.pthread_mutex.mutexes[0].count, 2);
    owner.pthread_mutex.mutexes[0].count = u32::MAX;
    assert_eq!(
        returned(begin(&mut owner, &mut guest, PosixMutexLock, &[ARG], now)),
        35
    );
    assert_eq!(owner.pthread_mutex.mutexes[0].count, u32::MAX);
    assert!(owner.pthread_mutex.waiters.is_empty());
    assert!(!owner.threads.active().waiting);
    owner.pthread_mutex.mutexes[0].count = 1;
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), 0);
    assert_eq!(library(&mut owner, &mut guest, MutexDestroy, &[ARG]), 0);
    assert_eq!(
        library(&mut owner, &mut guest, MutexAttrDestroy, &[ARG + 8]),
        0
    );
    assert_eq!(guest.get(errno, 4), 37u32.to_le_bytes());
    assert_eq!(owner.usage().handles_live, 1);
}

#[test]
fn mutex_wait_quota_work_unbound_and_late_lazy_publication_refusals_preserve_ownership() {
    let mut owner = owner(Default::default());
    let mut guest = TestGuest::new();
    let now = Instant::now();
    guest.refuse = true;
    assert!(matches!(
        begin(&mut owner, &mut guest, MutexLock, &[ARG], now),
        Err(Failure::Guest(_))
    ));
    assert!(owner.pthread_mutex.mutexes.is_empty());
    assert_eq!(guest.u64(0), 0);
    guest.refuse = false;
    assert_eq!(
        returned(begin(&mut owner, &mut guest, MutexLock, &[ARG], now)),
        0
    );
    let parent = owner.threads.active().id;
    owner.activate_thread_context(2).unwrap();
    owner.pthread_mutex.max_waiters = 0;
    assert!(matches!(
        begin(&mut owner, &mut guest, MutexLock, &[ARG], now),
        Err(Failure::Limit(_))
    ));
    assert!(owner.pthread_mutex.waiters.is_empty());
    assert!(!owner.threads.active().waiting);
    assert_eq!(owner.pthread_mutex.mutexes[0].owner, Some(parent));
    owner.pthread_mutex.max_waiters = owner.budget.limits.max_thread_contexts;
    let maximum = owner.budget.limits.max_work;
    owner.budget.limits.max_work = owner.usage().work + 1;
    assert!(matches!(
        begin(&mut owner, &mut guest, MutexLock, &[ARG], now),
        Err(Failure::Limit(_))
    ));
    owner.budget.limits.max_work = maximum;
    let ticket = waiting(begin(&mut owner, &mut guest, MutexLock, &[ARG], now));
    owner.activate_thread_context(1).unwrap();
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), 0);
    owner.activate_thread_context(2).unwrap();
    owner.budget.limits.max_work = owner.usage().work + 4; // Both lookups pass; removal admission fails.
    assert!(matches!(
        owner.complete_mutex_wait(&ticket),
        Err(Failure::Limit(_))
    ));
    assert_eq!(owner.pthread_mutex.mutexes[0].owner, None);
    assert_eq!(owner.pthread_mutex.waiters.len(), 1);
    assert!(owner.threads.active().waiting);
    owner.budget.limits.max_work = maximum;
    assert_eq!(owner.complete_mutex_wait(&ticket).unwrap(), Some(0));
    struct Unbound;
    impl GuestAccess for Unbound {
        fn read(&self, _: u64, _: &mut [u8]) -> Result<(), crate::GuestError> {
            panic!("unbound read")
        }
        fn apply(
            &mut self,
            _: crate::MemoryEffect,
            _: &[crate::GuestWrite<'_>],
        ) -> Result<(), crate::GuestError> {
            panic!("unbound write")
        }
    }
    let before = owner.usage();
    assert!(matches!(
        owner.begin_mutex_wait(MutexLock, arguments(&[ARG]), &mut Unbound),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(owner.usage().bytes_read, before.bytes_read);
    assert_eq!(owner.usage().bytes_written, before.bytes_written);
}

#[test]
fn mutex_wait_competing_ready_tickets_never_double_grant_or_allow_destruction_between_owners() {
    let mut owner = owner(Default::default());
    let mut guest = TestGuest::new();
    assert_eq!(library(&mut owner, &mut guest, MutexLock, &[ARG]), 0);
    owner.activate_thread_context(2).unwrap();
    let first = waiting(owner.begin_mutex_wait(MutexLock, arguments(&[ARG]), &mut guest));
    owner.activate_thread_context(3).unwrap();
    let second = waiting(owner.begin_mutex_wait(PosixMutexLock, arguments(&[ARG]), &mut guest));
    owner.activate_thread_context(1).unwrap();
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), 0);
    assert!(owner.mutex_wait_ready(&first).unwrap());
    assert!(owner.mutex_wait_ready(&second).unwrap());
    assert_eq!(
        library(&mut owner, &mut guest, MutexDestroy, &[ARG]),
        0x8002_0010
    );
    owner.activate_thread_context(3).unwrap();
    assert_eq!(owner.complete_mutex_wait(&second).unwrap(), Some(0));
    assert!(!owner.mutex_wait_ready(&first).unwrap());
    owner.activate_thread_context(2).unwrap();
    assert_eq!(owner.complete_mutex_wait(&first).unwrap(), None);
    owner.activate_thread_context(3).unwrap();
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), 0);
    owner.activate_thread_context(2).unwrap();
    assert_eq!(owner.complete_mutex_wait(&first).unwrap(), Some(0));
    assert_eq!(owner.pthread_mutex.mutexes[0].mutex_waits, 0);
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), 0);
    assert_eq!(library(&mut owner, &mut guest, MutexDestroy, &[ARG]), 0);
}

#[test]
fn mutex_wait_and_condition_reacquisition_share_exclusion_and_one_wait_per_thread() {
    let mut owner = owner(Default::default());
    let mut guest = TestGuest::new();
    assert_eq!(library(&mut owner, &mut guest, MutexLock, &[ARG]), 0);
    owner.activate_thread_context(2).unwrap();
    let mutex = waiting(owner.begin_mutex_wait(MutexLock, arguments(&[ARG]), &mut guest));
    assert!(matches!(
        owner.begin_condition_wait(CondWait, arguments(&[ARG + 8, ARG]), &mut guest),
        Err(Failure::Unsupported(_))
    ));
    owner.activate_thread_context(1).unwrap();
    let crate::ConditionWaitAction::Waiting(condition) = owner
        .begin_condition_wait(CondWait, arguments(&[ARG + 8, ARG]), &mut guest)
        .unwrap()
    else {
        panic!("expected condition membership")
    };
    assert!(matches!(
        owner.begin_mutex_wait(MutexLock, arguments(&[ARG]), &mut guest),
        Err(Failure::Unsupported(_))
    ));
    assert!(owner.mutex_wait_ready(&mutex).unwrap());
    owner.activate_thread_context(2).unwrap();
    assert_eq!(owner.complete_mutex_wait(&mutex).unwrap(), Some(0));
    assert_eq!(library(&mut owner, &mut guest, CondSignal, &[ARG + 8]), 0);
    assert!(!owner.condition_wait_ready(&condition).unwrap());
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), 0);
    assert_eq!(
        library(&mut owner, &mut guest, MutexDestroy, &[ARG]),
        0x8002_0010
    );
    owner.activate_thread_context(1).unwrap();
    assert!(owner.complete_condition_wait(&condition).unwrap());
    assert_eq!(
        (
            owner.pthread_mutex.mutexes[0].mutex_waits,
            owner.pthread_mutex.mutexes[0].condition_waits
        ),
        (0, 0)
    );
    assert_eq!(library(&mut owner, &mut guest, MutexUnlock, &[ARG]), 0);
    assert_eq!(library(&mut owner, &mut guest, MutexDestroy, &[ARG]), 0);
    assert_eq!(library(&mut owner, &mut guest, CondDestroy, &[ARG + 8]), 0);
    assert_eq!(owner.usage().handles_live, 0);
}
