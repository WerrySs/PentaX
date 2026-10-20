// SPDX-License-Identifier: GPL-2.0-only
//! Original modeled process clocks; no real sleeps or guest execution.

use super::*;
use crate::{GuestError, GuestWrite, Limits, MemoryEffect, ProcessIdentity, TimeCounterSample};
use std::cell::Cell;

struct Clock {
    ticks: Cell<u64>,
    frequency: Cell<u64>,
    samples: Cell<u64>,
    permitted: bool,
    fail: bool,
}
impl Clock {
    fn new() -> Self {
        Self {
            ticks: Cell::new(100),
            frequency: Cell::new(1_000_000),
            samples: Cell::new(0),
            permitted: true,
            fail: false,
        }
    }
}
impl GuestAccess for Clock {
    fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
        panic!("time continuation must not read payload");
    }
    fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        panic!("time continuation must not write payload or errno");
    }
    fn preflight_pthread(&self) -> Result<(), Failure> {
        if self.permitted {
            Ok(())
        } else {
            Err(Failure::Unsupported("original clock admission"))
        }
    }
    fn time_counter(&self) -> Result<TimeCounterSample, Failure> {
        self.samples.set(self.samples.get() + 1);
        if self.fail {
            return Err(Failure::Unsupported("original clock failure"));
        }
        Ok(TimeCounterSample {
            ticks: self.ticks.get(),
            frequency: self.frequency.get(),
        })
    }
}
fn owner(limits: Limits) -> Kernel {
    Kernel::new(
        limits,
        ProcessIdentity {
            process_id: 42,
            initial_thread_id: 7,
        },
        Vec::new(),
    )
    .unwrap()
}
fn waiting(kernel: &mut Kernel, clock: &Clock, micros: u64) -> TimeWait {
    match kernel
        .begin_time_wait(Service::Usleep, CallArgs([micros, 0, 0, 0, 0, 0]), clock)
        .unwrap()
    {
        TimeWaitAction::Waiting(ticket) => ticket,
        TimeWaitAction::Returned(_) => panic!("positive wait was not retained"),
    }
}

#[test]
fn real_process_deadline_low32_one_attempt_and_exactly_once_completion() {
    let mut kernel = owner(Limits::default());
    let clock = Clock::new();
    kernel.threads.active_mut().errno = Some(0x10040);
    let ticket = waiting(&mut kernel, &clock, (0xa5a5_a5a5 << 32) | 10);
    assert_eq!(ticket.deadline, 10);
    assert_eq!(clock.samples.get(), 2); // The existing lazy process epoch.
    assert!(kernel.threads.active().waiting);
    clock.ticks.set(109);
    assert!(!kernel.time_wait_ready(&ticket, &clock).unwrap());
    assert_eq!(
        kernel.time_wait_delay(&ticket, &clock).unwrap(),
        Some(Duration::from_micros(1))
    );
    assert_eq!(kernel.complete_time_wait(&ticket, &clock).unwrap(), None);
    assert_eq!(kernel.threads.active().time_wait, Some(ticket));
    clock.ticks.set(110);
    assert!(kernel.time_wait_ready(&ticket, &clock).unwrap());
    assert_eq!(
        kernel.time_wait_delay(&ticket, &clock).unwrap(),
        Some(Duration::ZERO)
    );
    assert_eq!(kernel.complete_time_wait(&ticket, &clock).unwrap(), Some(0));
    assert!(!kernel.threads.active().waiting);
    assert_eq!(kernel.threads.active().time_wait, None);
    assert_eq!(kernel.threads.active().errno, Some(0x10040));
    let samples = clock.samples.get();
    assert!(kernel.complete_time_wait(&ticket, &clock).is_err());
    assert!(kernel.time_wait_ready(&ticket, &clock).is_err());
    assert_eq!(clock.samples.get(), samples);
    assert_eq!(kernel.usage().attempts, 1);
    assert_eq!(
        (kernel.usage().bytes_read, kernel.usage().bytes_written),
        (0, 0)
    );
}

#[test]
fn observations_allow_another_thread_but_completion_rejects_wrong_context_and_owner() {
    let clock = Clock::new();
    let mut kernel = owner(Limits::default());
    let mut foreign = owner(Limits::default());
    // First selection binds the preexisting main record. Bind it before the
    // wait so selecting another context actually creates a distinct thread.
    kernel.activate_thread_context(1).unwrap();
    foreign.activate_thread_context(1).unwrap();
    let ticket = waiting(&mut kernel, &clock, 10);
    let other = waiting(&mut foreign, &clock, 10);
    assert_ne!(ticket, other);
    let samples = clock.samples.get();
    assert!(foreign.time_wait_ready(&ticket, &clock).is_err());
    assert_eq!(clock.samples.get(), samples);
    assert_eq!(foreign.threads.active().time_wait, Some(other));
    kernel.activate_thread_context(9).unwrap();
    clock.ticks.set(110);
    assert!(kernel.time_wait_ready(&ticket, &clock).unwrap());
    let samples = clock.samples.get();
    assert!(kernel.complete_time_wait(&ticket, &clock).is_err());
    assert_eq!(clock.samples.get(), samples);
    assert!(!kernel.threads.active().waiting);
    kernel.activate_thread_context(1).unwrap();
    assert_eq!(kernel.threads.active().time_wait, Some(ticket));
    assert_eq!(kernel.complete_time_wait(&ticket, &clock).unwrap(), Some(0));
    let next = waiting(&mut kernel, &clock, 1);
    assert_ne!(ticket, next);
    let samples = clock.samples.get();
    assert!(kernel.time_wait_delay(&ticket, &clock).is_err());
    assert_eq!(clock.samples.get(), samples);
    assert_eq!(kernel.threads.active().time_wait, Some(next));
}

#[test]
fn zero_duration_and_all_admission_refusals_precede_clock_or_pending_state() {
    let clock = Clock::new();
    let mut kernel = owner(Limits {
        max_sleep_nanoseconds: 5_000,
        ..Default::default()
    });
    assert_eq!(
        kernel
            .begin_time_wait(Service::Usleep, CallArgs([1 << 32, 0, 0, 0, 0, 0]), &clock)
            .unwrap(),
        TimeWaitAction::Returned(0)
    );
    assert!(matches!(
        kernel.begin_time_wait(Service::Usleep, CallArgs([6, 0, 0, 0, 0, 0]), &clock),
        Err(Failure::Limit("sleep duration"))
    ));
    assert!(matches!(
        kernel.begin_time_wait(Service::Nanosleep, CallArgs([0; 6]), &clock),
        Err(Failure::Unsupported("cooperative time service"))
    ));
    assert_eq!(clock.samples.get(), 0);
    assert_eq!(kernel.time.counter_epoch, None);
    assert_eq!(kernel.time.wait_owner, 0);
    assert!(!kernel.threads.active().waiting);
    for limits in [
        Limits {
            max_attempts: 0,
            ..Default::default()
        },
        Limits {
            max_work: 32,
            ..Default::default()
        },
    ] {
        let mut kernel = owner(limits);
        assert!(
            kernel
                .begin_time_wait(Service::Usleep, CallArgs([1, 0, 0, 0, 0, 0]), &clock)
                .is_err()
        );
        assert_eq!(kernel.time.counter_epoch, None);
        assert_eq!(kernel.threads.active().time_wait, None);
    }
    let mut kernel = owner(Limits::default());
    let denied = Clock {
        permitted: false,
        ..Clock::new()
    };
    assert!(
        kernel
            .begin_time_wait(Service::Usleep, CallArgs([1, 0, 0, 0, 0, 0]), &denied)
            .is_err()
    );
    assert_eq!(denied.samples.get(), 0);
    assert_eq!(kernel.time.wait_owner, 0);
}

#[test]
fn clock_failures_duplicate_begin_and_overflow_preserve_retryable_admission() {
    let mut kernel = owner(Limits::default());
    let mut clock = Clock {
        fail: true,
        ..Clock::new()
    };
    assert!(
        kernel
            .begin_time_wait(Service::Usleep, CallArgs([10, 0, 0, 0, 0, 0]), &clock)
            .is_err()
    );
    assert_eq!(kernel.time.counter_epoch, None);
    assert_eq!((kernel.time.wait_owner, kernel.time.wait_serial), (0, 0));
    clock.fail = false;
    let ticket = waiting(&mut kernel, &clock, 10);
    let samples = clock.samples.get();
    assert!(
        kernel
            .begin_time_wait(Service::Usleep, CallArgs([1, 0, 0, 0, 0, 0]), &clock)
            .is_err()
    );
    assert_eq!(clock.samples.get(), samples);
    clock.frequency.set(2_000_000);
    assert!(kernel.complete_time_wait(&ticket, &clock).is_err());
    assert_eq!(kernel.threads.active().time_wait, Some(ticket));
    clock.frequency.set(1_000_000);
    clock.ticks.set(110);
    assert_eq!(kernel.complete_time_wait(&ticket, &clock).unwrap(), Some(0));

    let mut overflow = owner(Limits::default());
    overflow.time.counter_epoch = Some(TimeCounterSample {
        ticks: 0,
        frequency: 1_000_000,
    });
    clock.ticks.set(u64::MAX - 4);
    assert!(matches!(
        overflow.begin_time_wait(Service::Usleep, CallArgs([10, 0, 0, 0, 0, 0]), &clock),
        Err(Failure::Limit("time wait deadline"))
    ));
    assert_eq!(
        (overflow.time.wait_owner, overflow.time.wait_serial),
        (0, 0)
    );
    assert_eq!(overflow.threads.active().time_wait, None);
}
