// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use crate::{GuestError, Limits, MemoryEffect};
use std::cell::Cell;

struct Clock {
    ticks: Cell<u64>,
    frequency: Cell<u64>,
}
impl GuestAccess for Clock {
    fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
        panic!("counter service must not read guest memory");
    }
    fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        panic!("counter service must not write guest memory");
    }
    fn time_counter(&self) -> Result<TimeCounterSample, Failure> {
        let ticks = self.ticks.get();
        self.ticks.set(ticks + 50);
        Ok(TimeCounterSample {
            ticks,
            frequency: self.frequency.get(),
        })
    }
}

#[test]
fn process_counters_share_one_epoch_and_preserve_the_native_frequency() {
    let mut clock = Clock {
        ticks: Cell::new(100),
        frequency: Cell::new(2_000_000),
    };
    let mut state = TimeState::new();
    let mut budget = Budget::new(Limits::default()).unwrap();
    let invoke = |state: &mut TimeState, clock: &mut Clock, budget: &mut Budget, service| {
        state.invoke(service, CallArgs([0; 6]), clock, budget)
    };
    assert_eq!(
        invoke(
            &mut state,
            &mut clock,
            &mut budget,
            Service::ProcessTimeCounter
        ),
        Ok(50)
    );
    assert_eq!(
        invoke(&mut state, &mut clock, &mut budget, Service::ProcessTime),
        Ok(50)
    );
    assert_eq!(
        invoke(&mut state, &mut clock, &mut budget, Service::ReadTsc),
        Ok(250)
    );
    for service in [Service::TscFrequency, Service::ProcessTimeCounterFrequency] {
        assert_eq!(
            invoke(&mut state, &mut clock, &mut budget, service),
            Ok(2_000_000)
        );
    }
    let mut other = TimeState::new();
    assert_eq!(
        invoke(
            &mut other,
            &mut clock,
            &mut budget,
            Service::ProcessTimeCounter
        ),
        Ok(50)
    );
    assert_eq!(state.counter_epoch.unwrap().ticks, 100);
    assert_eq!(other.counter_epoch.unwrap().ticks, 400);
    clock.ticks.set(1);
    assert_eq!(
        invoke(
            &mut state,
            &mut clock,
            &mut budget,
            Service::ProcessTimeCounter
        ),
        Ok(0)
    );
    clock.frequency.set(3_000_000);
    assert!(
        invoke(
            &mut state,
            &mut clock,
            &mut budget,
            Service::ProcessTimeCounter
        )
        .is_err()
    );
    assert_eq!(state.counter_epoch.unwrap().frequency, 2_000_000);
    clock.frequency.set(0);
    assert!(invoke(&mut state, &mut clock, &mut budget, Service::ProcessTime).is_err());
}

#[test]
fn internal_process_counter_shares_service_epoch_without_synthetic_attempts_or_io() {
    let mut kernel = crate::Kernel::new(
        Limits::default(),
        crate::ProcessIdentity {
            process_id: 100,
            initial_thread_id: 7,
        },
        Vec::new(),
    )
    .unwrap();
    let mut clock = Clock {
        ticks: Cell::new(100),
        frequency: Cell::new(0),
    };
    let initial_usage = kernel.usage();
    assert!(matches!(
        kernel.process_time_counter(&clock),
        Err(Failure::Unsupported("zero native TSC frequency"))
    ));
    assert!(kernel.time.counter_epoch.is_none());
    assert_eq!(kernel.usage(), initial_usage);

    // Refused observation did not set an epoch. The first successful accessor
    // takes the same two samples as ProcessTimeCounter, then shares its epoch.
    clock.ticks.set(100);
    clock.frequency.set(2_000_000);
    assert_eq!(kernel.process_time_counter(&clock), Ok(50));
    assert_eq!(clock.ticks.get(), 200);
    assert_eq!(kernel.usage(), initial_usage);
    assert_eq!(
        kernel.invoke(Service::ProcessTime, CallArgs([0; 6]), &mut clock),
        Ok(50)
    );
    let service_usage = kernel.usage();
    assert_eq!(service_usage.attempts, initial_usage.attempts + 1);
    assert_eq!(kernel.process_time_counter(&clock), Ok(150));
    assert_eq!(kernel.usage(), service_usage);
    assert_eq!(
        kernel.invoke(Service::ProcessTimeCounter, CallArgs([0; 6]), &mut clock),
        Ok(200)
    );

    let before_failure = kernel.usage();
    clock.frequency.set(3_000_000);
    assert!(matches!(
        kernel.process_time_counter(&clock),
        Err(Failure::Unsupported("changed native TSC frequency"))
    ));
    assert_eq!(kernel.usage(), before_failure);
    assert_eq!(kernel.time.counter_epoch.unwrap().ticks, 100);
    clock.frequency.set(2_000_000);
    clock.ticks.set(400);
    assert_eq!(kernel.process_time_counter(&clock), Ok(300));
    assert_eq!(kernel.usage(), before_failure);
    assert_eq!(
        kernel.process_clock(&clock),
        Ok(ProcessClock {
            counter: 350,
            microseconds: 175
        })
    );
    assert_eq!(clock.ticks.get(), 500); // Paired values used exactly one sample.
    assert_eq!(kernel.usage(), before_failure);
    clock.frequency.set(3_000_000);
    assert!(kernel.process_clock(&clock).is_err());
    assert_eq!(kernel.time.counter_epoch.unwrap().ticks, 100);
    assert_eq!(kernel.usage(), before_failure);
}

#[test]
fn paired_process_clock_checks_both_samples_and_conversion_before_epoch_publication() {
    struct Samples {
        values: [(u64, u64); 2],
        calls: Cell<usize>,
    }
    impl GuestAccess for Samples {
        fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
            panic!("paired process clock must not read guest memory");
        }
        fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
            panic!("paired process clock must not write guest memory");
        }
        fn time_counter(&self) -> Result<TimeCounterSample, Failure> {
            let index = self.calls.get();
            self.calls.set(index + 1);
            let (ticks, frequency) = self.values[index];
            Ok(TimeCounterSample { ticks, frequency })
        }
    }
    for values in [
        [(100, 2_000_000), (150, 0)],
        [(100, 2_000_000), (150, 3_000_000)],
        [(0, 1), (u64::MAX, 1)],
    ] {
        let mut kernel = crate::Kernel::new(
            Limits::default(),
            crate::ProcessIdentity {
                process_id: 100,
                initial_thread_id: 7,
            },
            Vec::new(),
        )
        .unwrap();
        let clock = Samples {
            values,
            calls: Cell::new(0),
        };
        let usage = kernel.usage();
        assert!(kernel.process_clock(&clock).is_err());
        assert_eq!(clock.calls.get(), 2);
        assert!(kernel.time.counter_epoch.is_none());
        assert_eq!(kernel.usage(), usage);
        let retry = Samples {
            values: [(100, 2_000_000), (150, 2_000_000)],
            calls: Cell::new(0),
        };
        assert_eq!(
            kernel.process_clock(&retry),
            Ok(ProcessClock {
                counter: 50,
                microseconds: 25
            })
        );
        assert_eq!(retry.calls.get(), 2);
        assert_eq!(kernel.time.counter_epoch.unwrap().ticks, 100);
        assert_eq!(kernel.usage(), usage);
    }
}
