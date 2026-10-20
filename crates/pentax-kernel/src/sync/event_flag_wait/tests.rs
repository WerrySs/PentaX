// SPDX-License-Identifier: GPL-2.0-only
//! Original cooperative predicates, cohorts, deadlines and atomic refusals.

use super::*;
use crate::tests::{ARG, PAGE, TestGuest, kernel};

fn call(
    k: &mut Kernel,
    g: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    k.invoke_library(service, CallArgs(args), g)
}
fn setup(attr: u64, bits: u64) -> (Kernel, TestGuest, u64) {
    let mut k = kernel(Default::default());
    let mut g = TestGuest::new();
    k.activate_thread_context(1).unwrap();
    for context in 2..=4 {
        k.register_thread_context(
            context,
            ARG + 0x300 + context * 8,
            700 - context as i32,
            0x7f,
            &mut g,
        )
        .unwrap();
    }
    g.put(0x80, b"original-cooperative-flag\0");
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::CreateEventFlag,
            &[ARG, ARG + 0x80, attr, bits, 0]
        ),
        Ok(0)
    );
    let flag = g.u64(0);
    (k, g, flag)
}
fn begin(
    k: &mut Kernel,
    g: &mut TestGuest,
    service: Service,
    values: &[u64],
    now: Instant,
) -> Result<EventFlagWaitAction, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    k.begin_event_flag_operation_at(service, CallArgs(args), g, now)
}
fn waiting(action: Result<EventFlagWaitAction, Failure>) -> EventFlagWait {
    let EventFlagWaitAction::Waiting(ticket) = action.unwrap() else {
        panic!("operation must genuinely park")
    };
    ticket
}
fn returned(action: Result<EventFlagWaitAction, Failure>) -> u64 {
    let EventFlagWaitAction::Returned(value) = action.unwrap() else {
        panic!("operation must return immediately")
    };
    value
}

#[test]
fn actual_predicates_clear_only_after_outputs_and_source_ignores_fifo_priority() {
    for attr in [0x21, 0x22] {
        let (mut k, mut g, flag) = setup(attr, 0);
        let now = Instant::now();
        let a = waiting(begin(
            &mut k,
            &mut g,
            Service::WaitEventFlag,
            &[flag, 3, 0x21, ARG + 0x100, 0],
            now,
        ));
        k.activate_thread_context(2).unwrap();
        let b = waiting(begin(
            &mut k,
            &mut g,
            Service::WaitEventFlag,
            &[flag, 2, 0x12, ARG + 0x108, 0],
            now,
        ));
        k.activate_thread_context(3).unwrap();
        assert_eq!(
            call(&mut k, &mut g, Service::SetEventFlag, &[flag, 0x83]),
            Ok(0)
        );
        assert!(k.event_flag_wait_ready_at(&a, now).unwrap());
        assert!(k.event_flag_wait_ready_at(&b, now).unwrap());
        // No reserved grant: whichever real selected waiter completes first
        // applies its clear. The source constructor ignores FIFO/priority.
        k.activate_thread_context(1).unwrap();
        g.refuse = true;
        assert_eq!(
            k.complete_event_flag_wait_at(&a, &mut g, now),
            Err(Failure::Guest(crate::GuestError::WriteDenied))
        );
        assert_eq!(k.sync.flags[0].bits, 0x83);
        assert_eq!(k.sync.flag_waiters.len(), 2);
        g.refuse = false;
        assert_eq!(k.complete_event_flag_wait_at(&a, &mut g, now), Ok(Some(0)));
        assert_eq!(g.u64(0x100), 0x83);
        assert_eq!(k.sync.flags[0].bits, 0x80);
        assert!(!k.event_flag_wait_ready_at(&b, now).unwrap());
        k.activate_thread_context(3).unwrap();
        call(&mut k, &mut g, Service::SetEventFlag, &[flag, 2]).unwrap();
        k.activate_thread_context(2).unwrap();
        assert_eq!(k.complete_event_flag_wait_at(&b, &mut g, now), Ok(Some(0)));
        assert_eq!(g.u64(0x108), 0x82);
        assert_eq!(k.sync.flags[0].bits, 0);
        assert!(k.complete_event_flag_wait_at(&b, &mut g, now).is_err());
    }
}

#[test]
fn deterministic_timeout_and_late_predicate_preserve_source_optional_outputs() {
    let (mut k, mut g, flag) = setup(0x20, 2);
    let now = Instant::now();
    g.put(0x120, &10_000u32.to_le_bytes());
    let a = waiting(begin(
        &mut k,
        &mut g,
        Service::WaitEventFlag,
        &[flag, 1, 0x21, ARG + 0x100, ARG + 0x120],
        now,
    ));
    assert!(
        !k.event_flag_wait_ready_at(&a, now + Duration::from_micros(9999))
            .unwrap()
    );
    assert!(
        k.event_flag_wait_ready_at(&a, now + Duration::from_micros(10_000))
            .unwrap()
    );
    let attempts = k.usage().attempts;
    assert_eq!(
        k.complete_event_flag_wait_at(&a, &mut g, now + Duration::from_micros(10_000)),
        Ok(Some(code(Errno::TimedOut)))
    );
    assert_eq!(g.u64(0x100), 2);
    assert_eq!(g.get(ARG + 0x120, 4), [0; 4]);
    assert_eq!(k.sync.flags[0].bits, 2);
    assert_eq!(k.usage().attempts, attempts);
    g.put(0x120, &10_000u32.to_le_bytes());
    let b = waiting(begin(
        &mut k,
        &mut g,
        Service::WaitEventFlag,
        &[flag, 1, 1, 0, ARG + 0x120],
        now,
    ));
    k.activate_thread_context(2).unwrap();
    call(&mut k, &mut g, Service::SetEventFlag, &[flag, 1]).unwrap();
    k.activate_thread_context(1).unwrap();
    assert_eq!(
        k.complete_event_flag_wait_at(&b, &mut g, now + Duration::from_secs(1)),
        Ok(Some(0))
    );
    assert_eq!(g.get(ARG + 0x120, 4), [0; 4]);
    assert_eq!(k.sync.flags[0].bits, 3);
    assert_eq!(
        returned(begin(
            &mut k,
            &mut g,
            Service::WaitEventFlag,
            &[flag, 3, 1, 0, 0],
            now
        )),
        0
    );
}

#[test]
fn cancel_freezes_cohort_and_waits_for_faulted_outputs_before_deferred_set() {
    let (mut k, mut g, flag) = setup(0x20, 0);
    let now = Instant::now();
    g.put(0x120, &10_000u32.to_le_bytes());
    g.put(0x140, &[0xa5; 8]);
    let a = waiting(begin(
        &mut k,
        &mut g,
        Service::WaitEventFlag,
        &[flag, 1, 0x11, ARG + 0x100, ARG + 0x120],
        now,
    ));
    k.activate_thread_context(2).unwrap();
    let b = waiting(begin(
        &mut k,
        &mut g,
        Service::WaitEventFlag,
        &[flag, 2, 0x21, ARG + 0x108, 0],
        now,
    ));
    k.activate_thread_context(3).unwrap();
    let cancel = waiting(begin(
        &mut k,
        &mut g,
        Service::CancelEventFlag,
        &[flag, 0xff, ARG + 0x140],
        now,
    ));
    assert_eq!(g.get(ARG + 0x140, 8), [2, 0, 0, 0, 0xa5, 0xa5, 0xa5, 0xa5]);
    assert!(!k.event_flag_wait_ready_at(&cancel, now).unwrap());
    assert_eq!(
        k.complete_event_flag_wait_at(&cancel, &mut g, now),
        Ok(None)
    );
    k.activate_thread_context(4).unwrap();
    let set = waiting(begin(
        &mut k,
        &mut g,
        Service::SetEventFlag,
        &[flag, 0x100],
        now,
    ));
    assert!(!k.event_flag_wait_ready_at(&set, now).unwrap());
    k.activate_thread_context(1).unwrap();
    g.refuse = true;
    assert!(k.complete_event_flag_wait_at(&a, &mut g, now).is_err());
    g.refuse = false;
    assert_eq!(k.sync.flag_waiters.len(), 2);
    assert_eq!(k.sync.flags[0].bits, 0xff);
    assert_eq!(
        k.complete_event_flag_wait_at(&a, &mut g, now + Duration::from_micros(2000)),
        Ok(Some(code(Errno::Canceled)))
    );
    assert_eq!(g.u64(0x100), 0xff);
    assert_eq!(g.get(ARG + 0x120, 4), 8000u32.to_le_bytes());
    k.activate_thread_context(2).unwrap();
    assert_eq!(
        k.complete_event_flag_wait_at(&b, &mut g, now),
        Ok(Some(code(Errno::Canceled)))
    );
    assert_eq!(g.u64(0x108), 0xff);
    assert_eq!(k.sync.flags[0].bits, 0xff);
    k.activate_thread_context(3).unwrap();
    assert!(k.event_flag_wait_ready_at(&cancel, now).unwrap());
    assert_eq!(
        k.complete_event_flag_wait_at(&cancel, &mut g, now),
        Ok(Some(0))
    );
    assert!(k.complete_event_flag_wait_at(&cancel, &mut g, now).is_err());
    k.activate_thread_context(4).unwrap();
    assert_eq!(
        k.complete_event_flag_wait_at(&set, &mut g, now),
        Ok(Some(0))
    );
    assert_eq!(k.sync.flags[0].bits, 0x1ff);
    assert!(k.sync.flag_operations.is_empty());
}

#[test]
fn deletion_drains_access_results_and_never_retargets_deferred_mutations_after_recreation() {
    let (mut k, mut g, flag) = setup(0x20, 0x80);
    let now = Instant::now();
    let a = waiting(begin(
        &mut k,
        &mut g,
        Service::WaitEventFlag,
        &[flag, 1, 0x11, ARG + 0x100, 0],
        now,
    ));
    k.activate_thread_context(2).unwrap();
    let delete = waiting(begin(
        &mut k,
        &mut g,
        Service::DeleteEventFlag,
        &[flag],
        now,
    ));
    assert_eq!(k.handle_usage().synchronization, 1);
    k.activate_thread_context(3).unwrap();
    let set = waiting(begin(
        &mut k,
        &mut g,
        Service::SetEventFlag,
        &[flag, 1],
        now,
    ));
    k.activate_thread_context(1).unwrap();
    assert_eq!(
        k.complete_event_flag_wait_at(&a, &mut g, now),
        Ok(Some(code(Errno::Access)))
    );
    assert_eq!(g.u64(0x100), 0x80);
    k.activate_thread_context(2).unwrap();
    assert_eq!(
        k.complete_event_flag_wait_at(&delete, &mut g, now),
        Ok(Some(0))
    );
    assert_eq!(k.handle_usage().synchronization, 0);
    call(
        &mut k,
        &mut g,
        Service::CreateEventFlag,
        &[ARG, ARG + 0x80, 0x20, 0, 0],
    )
    .unwrap();
    assert_ne!(g.u64(0), flag);
    k.activate_thread_context(3).unwrap();
    assert_eq!(
        k.complete_event_flag_wait_at(&set, &mut g, now),
        Ok(Some(code(Errno::NoSuchResource)))
    );
    assert_eq!(k.sync.flags[0].bits, 0);
    assert_eq!(k.handle_usage().synchronization, 1);
}

#[test]
fn single_waiter_precedence_raw_refusals_and_foreign_or_wrong_thread_tickets_are_explicit() {
    let (mut k, mut g, flag) = setup(0x10, 0);
    let now = Instant::now();
    let a = waiting(begin(
        &mut k,
        &mut g,
        Service::WaitEventFlag,
        &[flag, 1, 1, ARG + 0x100, 0],
        now,
    ));
    k.activate_thread_context(2).unwrap();
    call(&mut k, &mut g, Service::SetEventFlag, &[flag, 1]).unwrap();
    g.put(0x108, &[0xaa; 8]);
    assert_eq!(
        returned(begin(
            &mut k,
            &mut g,
            Service::WaitEventFlag,
            &[flag, 1, 1, ARG + 0x108, 0],
            now
        )),
        code(Errno::Perm)
    );
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::PollEventFlag,
            &[flag, 1, 1, ARG + 0x108]
        ),
        Ok(code(Errno::Perm))
    );
    assert_eq!(g.u64(0x108), 0xaaaa_aaaa_aaaa_aaaa);
    assert!(matches!(
        call(
            &mut k,
            &mut g,
            Service::CancelEventFlag,
            &[flag, 2, ARG + 0x108]
        ),
        Err(Failure::Unsupported(_))
    ));
    assert!(matches!(
        call(&mut k, &mut g, Service::DeleteEventFlag, &[flag]),
        Err(Failure::Unsupported(_))
    ));
    assert!(k.complete_event_flag_wait_at(&a, &mut g, now).is_err());
    let mut foreign = kernel(Default::default());
    assert!(foreign.event_flag_wait_ready_at(&a, now).is_err());
    assert_eq!(k.sync.flags[0].bits, 1);
    assert!(k.sync.flags[0].draining.is_none());
    k.activate_thread_context(1).unwrap();
    assert_eq!(k.complete_event_flag_wait_at(&a, &mut g, now), Ok(Some(0)));
    assert!(k.event_flag_wait_ready_at(&a, now).is_err());
}

#[test]
fn admission_limits_and_failed_cancel_count_or_late_output_preserve_every_membership() {
    let (mut k, mut g, flag) = setup(0x20, 0);
    let now = Instant::now();
    k.sync.max_waiters = 0;
    assert!(matches!(
        begin(
            &mut k,
            &mut g,
            Service::WaitEventFlag,
            &[flag, 1, 1, ARG + 0x100, 0],
            now
        ),
        Err(Failure::Limit(_))
    ));
    assert!(!k.threads.active().waiting);
    assert!(k.sync.flag_waiters.is_empty());
    k.sync.max_waiters = 4;
    let a = waiting(begin(
        &mut k,
        &mut g,
        Service::WaitEventFlag,
        &[flag, 1, 0x21, ARG + 0x100, 0],
        now,
    ));
    k.activate_thread_context(2).unwrap();
    assert!(matches!(
        begin(
            &mut k,
            &mut g,
            Service::CancelEventFlag,
            &[flag, 7, ARG + PAGE - 3],
            now
        ),
        Err(Failure::Guest(_))
    ));
    assert!(k.sync.flags[0].draining.is_none());
    assert!(k.sync.flag_waiters[0].outcome.is_none());
    assert!(!k.threads.active().waiting);
    call(&mut k, &mut g, Service::SetEventFlag, &[flag, 1]).unwrap();
    k.activate_thread_context(1).unwrap();
    let prior_attempts = k.usage().attempts;
    let prior_bits = k.sync.flags[0].bits;
    k.budget.limits.max_bytes = k.usage().bytes_read + k.usage().bytes_written + 7;
    assert!(matches!(
        k.complete_event_flag_wait_at(&a, &mut g, now),
        Err(Failure::Limit(_))
    ));
    assert_eq!(k.sync.flags[0].bits, prior_bits);
    assert_eq!(k.sync.flag_waiters.len(), 1);
    assert!(k.threads.active().waiting);
    k.budget.limits.max_bytes = Limits::default().max_bytes;
    let work = k.usage().work;
    k.budget.limits.max_work = work;
    assert!(matches!(
        k.complete_event_flag_wait_at(&a, &mut g, now),
        Err(Failure::Limit(_))
    ));
    assert_eq!(k.sync.flags[0].bits, prior_bits);
    k.budget.limits.max_work = Limits::default().max_work;
    assert_eq!(k.complete_event_flag_wait_at(&a, &mut g, now), Ok(Some(0)));
    assert_eq!(k.usage().attempts, prior_attempts);
    assert_eq!(k.sync.flags[0].bits, 0);
}

#[test]
fn overlapping_pattern_and_timeout_outputs_follow_source_store_order_atomically() {
    let (mut k, mut g, flag) = setup(0x20, 0);
    let now = Instant::now();
    g.put(0xf8, &[0x5a; 24]);
    g.put(0x104, &10_000u32.to_le_bytes());
    let a = waiting(begin(
        &mut k,
        &mut g,
        Service::WaitEventFlag,
        &[flag, 1, 0x21, ARG + 0x100, ARG + 0x104],
        now,
    ));
    k.activate_thread_context(2).unwrap();
    call(
        &mut k,
        &mut g,
        Service::SetEventFlag,
        &[flag, 0x1122_3344_0000_0001],
    )
    .unwrap();
    k.activate_thread_context(1).unwrap();
    assert_eq!(
        k.complete_event_flag_wait_at(&a, &mut g, now + Duration::from_micros(2000)),
        Ok(Some(0))
    );
    assert_eq!(g.u64(0x100), (8000u64 << 32) | 1);
    assert_eq!(g.get(ARG + 0xf8, 8), [0x5a; 8]);
    assert_eq!(g.get(ARG + 0x108, 8), [0x5a; 8]);
    assert_eq!(k.sync.flags[0].bits, 0x1122_3344_0000_0000);
}

#[test]
fn deferred_wait_and_second_cancel_transition_without_return_or_repeated_admission() {
    let (mut k, mut g, flag) = setup(0x20, 0);
    let now = Instant::now();
    let old = waiting(begin(
        &mut k,
        &mut g,
        Service::WaitEventFlag,
        &[flag, 1, 1, ARG + 0x100, 0],
        now,
    ));
    k.activate_thread_context(2).unwrap();
    let first = waiting(begin(
        &mut k,
        &mut g,
        Service::CancelEventFlag,
        &[flag, 0x80, 0],
        now,
    ));
    k.activate_thread_context(3).unwrap();
    let next = waiting(begin(
        &mut k,
        &mut g,
        Service::WaitEventFlag,
        &[flag, 2, 1, ARG + 0x108, 0],
        now,
    ));
    k.activate_thread_context(4).unwrap();
    let second = waiting(begin(
        &mut k,
        &mut g,
        Service::CancelEventFlag,
        &[flag, 0x88, ARG + 0x140],
        now,
    ));
    let attempts = k.usage().attempts;
    k.activate_thread_context(1).unwrap();
    assert_eq!(
        k.complete_event_flag_wait_at(&old, &mut g, now),
        Ok(Some(code(Errno::Canceled)))
    );
    k.activate_thread_context(2).unwrap();
    assert_eq!(
        k.complete_event_flag_wait_at(&first, &mut g, now),
        Ok(Some(0))
    );
    k.activate_thread_context(3).unwrap();
    assert!(k.event_flag_wait_ready_at(&next, now).unwrap());
    // Barrier admission progresses internally, not a fabricated syscall return.
    assert_eq!(k.complete_event_flag_wait_at(&next, &mut g, now), Ok(None));
    assert!(k.threads.active().waiting);
    assert!(!k.event_flag_wait_ready_at(&next, now).unwrap());
    assert_eq!(k.sync.flag_waiters[0].serial, next.serial);
    k.activate_thread_context(4).unwrap();
    assert!(k.event_flag_wait_ready_at(&second, now).unwrap());
    assert_eq!(
        k.complete_event_flag_wait_at(&second, &mut g, now),
        Ok(None)
    );
    assert_eq!(g.get(ARG + 0x140, 4), 1i32.to_le_bytes());
    assert!(!k.event_flag_wait_ready_at(&second, now).unwrap());
    k.activate_thread_context(3).unwrap();
    assert_eq!(
        k.complete_event_flag_wait_at(&next, &mut g, now),
        Ok(Some(code(Errno::Canceled)))
    );
    assert_eq!(g.u64(0x108), 0x88);
    k.activate_thread_context(4).unwrap();
    assert_eq!(
        k.complete_event_flag_wait_at(&second, &mut g, now),
        Ok(Some(0))
    );
    assert_eq!(k.usage().attempts, attempts);
    assert!(k.sync.flag_waiters.is_empty());
    assert!(k.sync.flag_operations.is_empty());
}

#[test]
fn deferred_timeout_uses_captured_input_and_single_precedence_remains_before_barrier() {
    for attr in [0x10, 0x20] {
        let (mut k, mut g, flag) = setup(attr, 0);
        let now = Instant::now();
        let old = waiting(begin(
            &mut k,
            &mut g,
            Service::WaitEventFlag,
            &[flag, 1, 1, 0, 0],
            now,
        ));
        k.activate_thread_context(2).unwrap();
        let cancel = waiting(begin(
            &mut k,
            &mut g,
            Service::CancelEventFlag,
            &[flag, 0x80, 0],
            now,
        ));
        k.activate_thread_context(3).unwrap();
        g.put(0x120, &10_000u32.to_le_bytes());
        if attr == 0x10 {
            assert!(matches!(
                begin(
                    &mut k,
                    &mut g,
                    Service::WaitEventFlag,
                    &[flag, 0x80, 1, ARG + 0x108, ARG + PAGE - 3],
                    now
                ),
                Err(Failure::Guest(_))
            ));
            assert_eq!(
                returned(begin(
                    &mut k,
                    &mut g,
                    Service::WaitEventFlag,
                    &[flag, 0x80, 1, ARG + 0x108, ARG + 0x120],
                    now
                )),
                code(Errno::Perm)
            );
            assert_eq!(k.sync.flag_operations.len(), 1);
            continue;
        }
        let next = waiting(begin(
            &mut k,
            &mut g,
            Service::WaitEventFlag,
            &[flag, 2, 1, ARG + 0x108, ARG + 0x120],
            now,
        ));
        assert!(matches!(
            call(
                &mut k,
                &mut g,
                Service::PollEventFlag,
                &[flag, 0x80, 0x11, 0]
            ),
            Err(Failure::Unsupported(_))
        ));
        g.put(0x120, &50_000u32.to_le_bytes()); // Another thread cannot extend the admitted deadline.
        k.activate_thread_context(1).unwrap();
        assert_eq!(
            k.complete_event_flag_wait_at(&old, &mut g, now),
            Ok(Some(code(Errno::Canceled)))
        );
        k.activate_thread_context(2).unwrap();
        assert_eq!(
            k.complete_event_flag_wait_at(&cancel, &mut g, now),
            Ok(Some(0))
        );
        k.activate_thread_context(3).unwrap();
        assert_eq!(
            k.complete_event_flag_wait_at(&next, &mut g, now + Duration::from_micros(8000)),
            Ok(None)
        );
        assert!(
            !k.event_flag_wait_ready_at(&next, now + Duration::from_micros(9999))
                .unwrap()
        );
        assert_eq!(
            k.complete_event_flag_wait_at(&next, &mut g, now + Duration::from_micros(10_000)),
            Ok(Some(code(Errno::TimedOut)))
        );
        assert_eq!(g.u64(0x108), 0x80);
        assert_eq!(g.get(ARG + 0x120, 4), [0; 4]);
    }
}
