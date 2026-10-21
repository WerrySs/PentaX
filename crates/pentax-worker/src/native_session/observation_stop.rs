// SPDX-License-Identifier: GPL-2.0-only
//! Explicit cooperative observation duration, independent of execution budgets.

use super::NativeSessionError;
use std::time::{Duration, Instant};

const CHECK_BLOCKS: u64 = 1024;

pub(super) struct ObservationStop {
    seconds: Option<u32>,
    deadline: Option<Instant>,
    last_checked: Option<u64>,
}

impl ObservationStop {
    pub(super) fn new(seconds: Option<u32>, interactive: bool) -> Result<Self, NativeSessionError> {
        if seconds.is_some_and(|seconds| !interactive || !(1..=3600).contains(&seconds)) {
            return Err(NativeSessionError::FixtureInvariant(
                "observation duration profile",
            ));
        }
        Ok(Self {
            seconds,
            deadline: None,
            last_checked: None,
        })
    }

    /// Called with a real prepared executor, once per epoch. Rebinding a source
    /// never restarts the lifetime. The disabled path never evaluates the clock.
    pub(super) fn start(
        &mut self,
        now: impl FnOnce() -> Instant,
    ) -> Result<(), NativeSessionError> {
        if self.deadline.is_none()
            && let Some(seconds) = self.seconds
        {
            self.deadline = Some(
                now()
                    .checked_add(Duration::from_secs(u64::from(seconds)))
                    .ok_or(NativeSessionError::FixtureInvariant("observation deadline"))?,
            );
        }
        Ok(())
    }

    /// Check before another bounded operation, not after a completed return or
    /// error. Trace admission also respects the remaining logical-block cadence.
    pub(super) fn check(
        &mut self,
        blocks: u64,
        before_idle: bool,
        now: impl FnOnce() -> Instant,
    ) -> Result<(), NativeSessionError> {
        let Some(deadline) = self.deadline else {
            return Ok(());
        };
        if !before_idle
            && self.last_checked.is_some_and(|last| {
                blocks
                    .checked_sub(last)
                    .is_some_and(|elapsed| elapsed < CHECK_BLOCKS)
            })
        {
            return Ok(());
        }
        self.last_checked = Some(blocks);
        if now() >= deadline {
            return Err(NativeSessionError::ObservationStopped);
        }
        Ok(())
    }

    pub(super) fn maximum_span(&self, blocks: u64) -> u64 {
        if self.deadline.is_none() {
            return 2;
        }
        let remaining = self
            .last_checked
            .and_then(|last| blocks.checked_sub(last))
            .and_then(|elapsed| CHECK_BLOCKS.checked_sub(elapsed));
        if remaining.is_some_and(|remaining| remaining >= 2) {
            2
        } else {
            1
        }
    }
}

#[cfg(test)]
mod tests;
