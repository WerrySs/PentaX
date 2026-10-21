// SPDX-License-Identifier: GPL-2.0-only

//! Closed diagnostics, independent of mutex ownership and scheduling policy.

use super::{Mutex, MutexState};

// Independent optional-observer lifetime allowance, never guest work or
// execution authority. Exhaustion makes diagnostics partial, not execution.
pub(super) const LOOKUP_VISITS: u64 = 65_536;

pub(super) fn eligible(mutex: &Mutex) -> bool {
    mutex.mutex_waits != 0 || mutex.notified_condition_waits != 0
}

pub(super) fn increment(counter: &mut u64, overflowed: &mut bool) {
    if *overflowed {
        return;
    }
    match counter.checked_add(1) {
        Some(next) => *counter = next,
        None => *overflowed = true,
    }
}

impl MutexState {
    // Notification already visits the bounded condition queue. Added mutex
    // visits have an independent fixed lifetime bound. The last actual index
    // can be reused only while it still matches this mutex identity; swap
    // removal or later publication cannot turn a stale index into a match.
    // Storage is the existing reserved mutex capacity * size_of::<Mutex>(),
    // plus inline state. No operation adds an allocation or a guest refusal.
    pub(crate) fn observe_condition_notification(&mut self, id: u64) {
        if self.observations_overflowed {
            return;
        }
        if let Some(index) = self.observation_mutex_index {
            if !self.observe_lookup_visit() {
                return;
            }
            if self.mutexes.get(index).is_some_and(|mutex| mutex.id == id) {
                increment(
                    &mut self.mutexes[index].notified_condition_waits,
                    &mut self.observations_overflowed,
                );
                return;
            }
        }
        for index in 0..self.mutexes.len() {
            if !self.observe_lookup_visit() {
                return;
            }
            if self.mutexes[index].id == id {
                self.observation_mutex_index = Some(index);
                increment(
                    &mut self.mutexes[index].notified_condition_waits,
                    &mut self.observations_overflowed,
                );
                return;
            }
        }
    }

    fn observe_lookup_visit(&mut self) -> bool {
        if self.observation_lookup_visits == 0 {
            self.observations_overflowed = true;
            return false;
        }
        self.observation_lookup_visits -= 1;
        true
    }

    pub(crate) fn wait_observations(&self) -> (u64, u64, u64, bool) {
        (
            self.waiters.len() as u64,
            self.eligible_releases,
            self.barging_acquisitions,
            self.observations_overflowed,
        )
    }
}

#[cfg(test)]
mod tests;
