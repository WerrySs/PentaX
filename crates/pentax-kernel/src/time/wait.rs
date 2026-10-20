// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: actual process-clock cooperative sleep admission.
// Source KernelUsleep: public KytyPS5@f186b35706ddbc3faf5513f326e2335d224802f7,
// src/kernel/pthread.cpp blob bdb125ab7e77969821928985da0dfaff4c929070.
// See NOTICE for the retained original Kyty MIT grant and profile boundaries.

use super::{Duration, process_microseconds};
use crate::{CallArgs, Failure, GuestAccess, Kernel, Service};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

/// Opaque retained sleep admission. This carries no code execution authority.
/// The native owner must attest its exact suspended syscall before completing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TimeWait {
    owner: u64,
    serial: u64,
    thread: u32,
    context: u64,
    deadline: u64,
}
impl fmt::Debug for TimeWait {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("TimeWait").finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeWaitAction {
    Returned(u64),
    Waiting(TimeWait),
}

impl Kernel {
    /// Admit one low32 unsigned-microsecond Usleep call without host sleeping.
    /// All duration, work, clock and identity checks precede the fixed-slot
    /// commit. Zero returns immediately without sampling or creating an epoch.
    pub fn begin_time_wait(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &impl GuestAccess,
    ) -> Result<TimeWaitAction, Failure> {
        self.budget.attempt()?;
        self.budget.work(32)?;
        guest.preflight_pthread()?;
        if service != Service::Usleep {
            return Err(Failure::Unsupported("cooperative time service"));
        }
        let active = self.threads.active();
        if active.waiting || active.time_wait.is_some() {
            return Err(Failure::Unsupported("thread already waiting"));
        }
        let micros = u64::from(args.0[0] as u32);
        if micros * 1_000 > self.budget.limits.max_sleep_nanoseconds {
            return Err(Failure::Limit("sleep duration"));
        }
        if micros == 0 {
            return Ok(TimeWaitAction::Returned(0));
        }
        let serial = self
            .time
            .wait_serial
            .checked_add(1)
            .ok_or(Failure::Limit("time wait serial"))?;
        let (initial, elapsed) = self.time.prepare_process_counter(guest)?;
        let now = process_microseconds(elapsed, initial.frequency)?;
        let deadline = now
            .checked_add(micros)
            .ok_or(Failure::Limit("time wait deadline"))?;
        let owner = if self.time.wait_owner == 0 {
            NEXT_OWNER
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .map_err(|_| Failure::Limit("time wait owners"))?
        } else {
            self.time.wait_owner
        };
        let active = self.threads.active();
        let ticket = TimeWait {
            owner,
            serial,
            thread: active.id,
            context: active.context(),
            deadline,
        };
        // No allocations, guest operations or fallible steps after this point.
        self.time.counter_epoch = Some(initial);
        self.time.wait_owner = owner;
        self.time.wait_serial = serial;
        let active = self.threads.active_mut();
        active.time_wait = Some(ticket);
        active.waiting = true;
        Ok(TimeWaitAction::Waiting(ticket))
    }

    fn validate_time_wait(&mut self, ticket: &TimeWait) -> Result<(), Failure> {
        self.budget.work(8)?;
        if ticket.owner == 0
            || ticket.owner != self.time.wait_owner
            || !self.threads.has_time_wait(ticket, &mut self.budget)?
        {
            return Err(Failure::Unsupported("time wait ticket"));
        }
        Ok(())
    }

    fn time_wait_now(&self, guest: &impl GuestAccess) -> Result<u64, Failure> {
        let (initial, elapsed) = self.time.prepare_process_counter(guest)?;
        process_microseconds(elapsed, initial.frequency)
    }

    /// Observe an exact live ticket before sampling its actual process clock.
    /// Another guest thread may be selected; this never consumes the ticket.
    pub fn time_wait_ready(
        &mut self,
        ticket: &TimeWait,
        guest: &impl GuestAccess,
    ) -> Result<bool, Failure> {
        self.validate_time_wait(ticket)?;
        Ok(self.time_wait_now(guest)? >= ticket.deadline)
    }

    pub fn time_wait_delay(
        &mut self,
        ticket: &TimeWait,
        guest: &impl GuestAccess,
    ) -> Result<Option<Duration>, Failure> {
        self.validate_time_wait(ticket)?;
        Ok(Some(Duration::from_micros(
            ticket.deadline.saturating_sub(self.time_wait_now(guest)?),
        )))
    }

    /// Called only after the runtime validates the current real native token,
    /// thread/TLS and source. A premature call stays parked with its same token.
    pub fn complete_time_wait(
        &mut self,
        ticket: &TimeWait,
        guest: &impl GuestAccess,
    ) -> Result<Option<u64>, Failure> {
        guest.preflight_pthread()?;
        self.validate_time_wait(ticket)?;
        let active = self.threads.active();
        if active.id != ticket.thread
            || active.context() != ticket.context
            || active.time_wait != Some(*ticket)
        {
            return Err(Failure::Unsupported("time wait thread"));
        }
        if self.time_wait_now(guest)? < ticket.deadline {
            return Ok(None);
        }
        let active = self.threads.active_mut();
        active.time_wait = None;
        active.waiting = false;
        Ok(Some(0))
    }
}

#[cfg(test)]
mod tests;
