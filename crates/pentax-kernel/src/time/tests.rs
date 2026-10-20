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
