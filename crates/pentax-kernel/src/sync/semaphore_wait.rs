// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded cooperative kernel semaphore continuations.
// Adapted from pinned kernel/semaphore.cpp; see NOTICE.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_WAIT: AtomicU64 = AtomicU64::new(1);

pub struct SemaphoreWait {
    serial: u64,
    thread: u32,
}
impl std::fmt::Debug for SemaphoreWait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SemaphoreWait").finish_non_exhaustive()
    }
}
#[derive(Debug)]
pub enum SemaphoreWaitAction {
    Returned(u64),
    Waiting(SemaphoreWait),
}
pub(super) struct Waiter {
    serial: u64,
    semaphore: u64,
    thread: u32,
    priority: i32,
    need: i32,
    // Some(0) is a real reserved grant, not an instruction to reacquire counts.
    result: Option<u64>,
    started: Instant,
    deadline: Option<Instant>,
    timeout: Option<(u64, u32)>,
}

impl SyncState {
    pub(super) fn waiting_count(&self, semaphore: u64) -> i32 {
        self.waiters
            .iter()
            .filter(|waiter| waiter.semaphore == semaphore && waiter.result.is_none())
            .count() as i32
    }
    pub(super) fn finish_waiters(&mut self, semaphore: u64, error: Errno) {
        for waiter in &mut self.waiters {
            if waiter.semaphore == semaphore && waiter.result.is_none() {
                waiter.result = Some(u64::from(error.kernel_code() as u32));
            }
        }
    }
    pub(super) fn grant_waiters(&mut self, index: usize) {
        let semaphore = &mut self.semaphores[index];
        // Source scans the entire ordered list: an unsatisfied large request
        // does not prevent a later smaller request from using available count.
        for waiter in &mut self.waiters {
            if waiter.semaphore == semaphore.id
                && waiter.result.is_none()
                && waiter.need <= semaphore.count
            {
                semaphore.count -= waiter.need;
                waiter.result = Some(0);
            }
        }
    }
    fn waiter_index(&self, ticket: &SemaphoreWait, budget: &mut Budget) -> Result<usize, Failure> {
        budget.work(self.waiters.len() as u64 + 1)?;
        self.waiters
            .iter()
            .position(|waiter| waiter.serial == ticket.serial && waiter.thread == ticket.thread)
            .ok_or(Failure::Unsupported("unowned semaphore wait"))
    }
}

impl crate::Kernel {
    pub fn begin_semaphore_wait(
        &mut self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<SemaphoreWaitAction, Failure> {
        self.begin_semaphore_wait_at(args, guest, Instant::now())
    }
    pub(crate) fn begin_semaphore_wait_at(
        &mut self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        now: Instant,
    ) -> Result<SemaphoreWaitAction, Failure> {
        self.budget.attempt()?;
        guest.preflight_pthread()?;
        let result = self.enroll_semaphore_wait(args, guest, now);
        match result {
            Err(Failure::Errno(error)) => Ok(SemaphoreWaitAction::Returned(u64::from(
                error.kernel_code() as u32,
            ))),
            result => result,
        }
    }
    fn enroll_semaphore_wait(
        &mut self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        now: Instant,
    ) -> Result<SemaphoreWaitAction, Failure> {
        if self.threads.active().waiting {
            return Err(Failure::Unsupported("thread already waiting"));
        }
        let state = &mut self.sync;
        let index = state.semaphore_index(args.0[0], &mut self.budget)?;
        let need = int(args.0[1]);
        if need < 1 || need > state.semaphores[index].max_count {
            return Err(Errno::Invalid.into());
        }
        let timeout = if args.0[2] == 0 {
            None
        } else {
            Some((args.0[2], self.budget.u32(guest, args.0[2])?))
        };
        if state.semaphores[index].count >= need {
            if let Some((address, micros)) = timeout {
                self.budget.write(
                    guest,
                    &[GuestWrite {
                        address,
                        bytes: &micros.to_le_bytes(),
                    }],
                )?;
            }
            state.semaphores[index].count -= need;
            return Ok(SemaphoreWaitAction::Returned(0));
        }
        if timeout.is_some_and(|(_, micros)| micros == 0) {
            return Err(Errno::TimedOut.into());
        }
        let deadline = if let Some((_, micros)) = timeout {
            let duration = Duration::from_micros(u64::from(micros));
            Some(
                now.checked_add(duration)
                    .ok_or(Failure::Limit("semaphore deadline"))?,
            )
        } else {
            None
        };
        self.budget.work(state.waiters.len() as u64 * 2 + 1)?;
        if state.waiters.len() >= state.max_waiters {
            return Err(Failure::Limit("semaphore waiters"));
        }
        let serial = NEXT_WAIT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| Failure::Limit("semaphore wait identities"))?;
        let thread = self.threads.active();
        let semaphore = &state.semaphores[index];
        let position = if semaphore.fifo {
            state.waiters.len()
        } else {
            state
                .waiters
                .iter()
                .position(|waiter| {
                    waiter.semaphore == semaphore.id && waiter.priority > thread.priority
                })
                .unwrap_or(state.waiters.len())
        };
        state.waiters.insert(
            position,
            Waiter {
                serial,
                semaphore: semaphore.id,
                thread: thread.id,
                priority: thread.priority,
                need,
                result: None,
                started: now,
                deadline,
                timeout,
            },
        );
        let ticket = SemaphoreWait {
            serial,
            thread: thread.id,
        };
        self.threads.active_mut().waiting = true;
        Ok(SemaphoreWaitAction::Waiting(ticket))
    }

    pub fn semaphore_wait_ready(&mut self, ticket: &SemaphoreWait) -> Result<bool, Failure> {
        self.semaphore_wait_ready_at(ticket, Instant::now())
    }
    pub(crate) fn semaphore_wait_ready_at(
        &mut self,
        ticket: &SemaphoreWait,
        now: Instant,
    ) -> Result<bool, Failure> {
        let index = self.sync.waiter_index(ticket, &mut self.budget)?;
        let waiter = &self.sync.waiters[index];
        Ok(waiter.result.is_some() || waiter.deadline.is_some_and(|deadline| now >= deadline))
    }
    /// Earliest finite delay; None means an indefinite pending wait. A reserved
    /// grant remains ready even after semaphore deletion or its original deadline.
    pub fn semaphore_wait_delay(
        &mut self,
        ticket: &SemaphoreWait,
    ) -> Result<Option<Duration>, Failure> {
        let index = self.sync.waiter_index(ticket, &mut self.budget)?;
        let waiter = &self.sync.waiters[index];
        if waiter.result.is_some() {
            return Ok(Some(Duration::ZERO));
        }
        Ok(waiter
            .deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now())))
    }
    pub fn complete_semaphore_wait(
        &mut self,
        ticket: &SemaphoreWait,
        guest: &mut impl GuestAccess,
    ) -> Result<Option<u64>, Failure> {
        self.complete_semaphore_wait_at(ticket, guest, Instant::now())
    }
    pub(crate) fn complete_semaphore_wait_at(
        &mut self,
        ticket: &SemaphoreWait,
        guest: &mut impl GuestAccess,
        now: Instant,
    ) -> Result<Option<u64>, Failure> {
        guest.preflight_pthread()?;
        if self.threads.active().id != ticket.thread {
            return Err(Failure::Unsupported("semaphore wait thread"));
        }
        let index = self.sync.waiter_index(ticket, &mut self.budget)?;
        let waiter = &self.sync.waiters[index];
        let result = waiter.result.or_else(|| {
            waiter
                .deadline
                .filter(|deadline| now >= *deadline)
                .map(|_| u64::from(Errno::TimedOut.kernel_code() as u32))
        });
        let Some(result) = result else {
            return Ok(None);
        };
        self.budget.work(self.sync.waiters.len() as u64)?;
        if let Some((address, micros)) = waiter.timeout {
            let elapsed = now
                .saturating_duration_since(waiter.started)
                .as_micros()
                .min(u128::from(u32::MAX)) as u32;
            let remaining = micros.saturating_sub(elapsed);
            self.budget.write(
                guest,
                &[GuestWrite {
                    address,
                    bytes: &remaining.to_le_bytes(),
                }],
            )?;
        }
        // A fault retains the exact grant/result and ticket. Count was reserved
        // once by signal, not charged again on resume, cancellation or deletion.
        self.sync.waiters.remove(index);
        self.threads.active_mut().waiting = false;
        Ok(Some(result))
    }
}
