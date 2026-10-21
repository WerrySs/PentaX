// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted from pinned RwlockLockCooperative; see NOTICE. PentaX retains bounded
// scheduler membership instead of a host condition-variable wait.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_WAIT: AtomicU64 = AtomicU64::new(1);

fn next_serial(counter: &AtomicU64) -> Result<u64, Failure> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |serial| {
            serial.checked_add(1)
        })
        .map_err(|_| Failure::Limit("rwlock wait identities"))
}

pub struct RwlockWait {
    serial: u64,
    thread: u32,
}
impl std::fmt::Debug for RwlockWait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RwlockWait").finish_non_exhaustive()
    }
}
#[derive(Debug)]
pub enum RwlockWaitAction {
    Returned(u64),
    Waiting(RwlockWait),
}

pub(super) struct Waiter {
    serial: u64,
    thread: u32,
    lock: u64,
    write: bool,
    positive: bool,
    deadline: Option<Instant>,
}
fn result(error: Errno, positive: bool) -> u64 {
    if positive {
        error as u64
    } else {
        u64::from(error.kernel_code() as u32)
    }
}

impl RwlockState {
    fn wait_indices(&self, ticket: &RwlockWait) -> Result<(usize, usize), Failure> {
        let waiter = self
            .waiters
            .iter()
            .position(|waiter| waiter.serial == ticket.serial && waiter.thread == ticket.thread)
            .ok_or(Failure::Unsupported("unowned rwlock wait"))?;
        let lock = self
            .locks
            .iter()
            .position(|lock| lock.id == self.waiters[waiter].lock && lock.waits != 0)
            .ok_or(Failure::Unsupported("stale rwlock wait object"))?;
        Ok((waiter, lock))
    }
    fn remove_wait(&mut self, waiter: usize, lock: usize) {
        if self.waiters[waiter].write {
            self.locks[lock].waiting_writers -= 1;
        } else {
            self.reader_reservations -= 1;
        }
        self.locks[lock].waits -= 1;
        self.waiters.swap_remove(waiter);
    }
}

impl crate::Kernel {
    fn rwlock_wait_work(&mut self) -> Result<(), Failure> {
        self.budget.work(
            24 + 4
                * (self.pthread_rwlock.locks.len()
                    + self.pthread_rwlock.readers.len()
                    + self.pthread_rwlock.waiters.len()) as u64,
        )
    }

    /// Enroll one actual selected-thread call. SCE timed arguments are scalar
    /// low32 relative microseconds on the monotonic clock, never a timespec.
    /// The existing POSIX write alias returns positive errno without setting it.
    pub fn begin_rwlock_wait(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<RwlockWaitAction, Failure> {
        self.begin_rwlock_wait_with(service, args, guest, Instant::now)
    }
    fn begin_rwlock_wait_with(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        now: impl FnOnce() -> Instant,
    ) -> Result<RwlockWaitAction, Failure> {
        use Service::*;
        if !matches!(
            service,
            RwlockReadLock
                | RwlockWriteLock
                | RwlockTimedReadLock
                | RwlockTimedWriteLock
                | PosixRwlockWriteLock
        ) {
            return Err(Failure::Unsupported("rwlock wait service"));
        }
        self.budget.attempt()?;
        guest.preflight_pthread()?;
        self.rwlock_wait_work()?;
        if self.threads.active().waiting {
            return Err(Failure::Unsupported("thread already waiting"));
        }
        let positive = service == PosixRwlockWriteLock;
        let outcome = (|| {
            let thread = self.threads.active().id;
            let state = &mut self.pthread_rwlock;
            let write = matches!(
                service,
                RwlockWriteLock | RwlockTimedWriteLock | PosixRwlockWriteLock
            );
            let lock = state.lock_index(
                args.0[0],
                true,
                !write,
                &mut self.memory,
                guest,
                &mut self.budget,
            )?;
            if state.can_acquire(lock, thread, write)? {
                state.acquire(lock, thread, write, false)?;
                return Ok(RwlockWaitAction::Returned(0));
            }
            let micros = matches!(service, RwlockTimedReadLock | RwlockTimedWriteLock)
                .then_some(args.0[1] as u32);
            if micros == Some(0) {
                return Err(Errno::TimedOut.into());
            }
            if state.waiters.len() >= state.maximum {
                return Err(Failure::Limit("rwlock waiters"));
            }
            if !write && state.readers.len() + state.reader_reservations >= state.maximum {
                return Err(Failure::Limit("rwlock reader records"));
            }
            if state.waiters.iter().any(|waiter| waiter.thread == thread) {
                return Err(Failure::Unsupported("duplicate rwlock wait"));
            }
            let waits = state.locks[lock]
                .waits
                .checked_add(1)
                .ok_or(Failure::Limit("rwlock waiters"))?;
            let writers = state.locks[lock]
                .waiting_writers
                .checked_add(usize::from(write))
                .ok_or(Failure::Limit("rwlock writers"))?;
            let deadline = micros
                .map(|micros| {
                    now()
                        .checked_add(Duration::from_micros(u64::from(micros)))
                        .ok_or(Failure::Limit("rwlock deadline"))
                })
                .transpose()?;
            let serial = next_serial(&NEXT_WAIT)?;
            state.waiters.push(Waiter {
                serial,
                thread,
                lock: state.locks[lock].id,
                write,
                positive,
                deadline,
            });
            state.locks[lock].waits = waits;
            state.locks[lock].waiting_writers = writers;
            if !write {
                state.reader_reservations += 1;
            }
            self.threads.active_mut().waiting = true;
            Ok(RwlockWaitAction::Waiting(RwlockWait { serial, thread }))
        })();
        match outcome {
            Err(Failure::Errno(error)) => Ok(RwlockWaitAction::Returned(result(error, positive))),
            result => result,
        }
    }

    pub fn rwlock_wait_ready(&mut self, ticket: &RwlockWait) -> Result<bool, Failure> {
        self.rwlock_wait_ready_at(ticket, Instant::now())
    }
    fn rwlock_wait_ready_at(&mut self, ticket: &RwlockWait, now: Instant) -> Result<bool, Failure> {
        self.rwlock_wait_work()?;
        let state = &self.pthread_rwlock;
        let (waiter, lock) = state.wait_indices(ticket)?;
        let waiter = &state.waiters[waiter];
        Ok(state.can_acquire(lock, ticket.thread, waiter.write)?
            || waiter.deadline.is_some_and(|deadline| now >= deadline))
    }
    pub fn rwlock_wait_delay(&mut self, ticket: &RwlockWait) -> Result<Option<Duration>, Failure> {
        let now = Instant::now();
        self.rwlock_wait_work()?;
        let state = &self.pthread_rwlock;
        let (waiter, lock) = state.wait_indices(ticket)?;
        let waiter = &state.waiters[waiter];
        if state.can_acquire(lock, ticket.thread, waiter.write)? {
            return Ok(Some(Duration::ZERO));
        }
        Ok(waiter
            .deadline
            .map(|deadline| deadline.saturating_duration_since(now)))
    }

    /// Complete only after the scheduler selects this ticket's real thread and
    /// validates the exact pending executor continuation. Eligibility precedes
    /// expiry, matching the source even after a delayed scheduling turn.
    pub fn complete_rwlock_wait(&mut self, ticket: &RwlockWait) -> Result<Option<u64>, Failure> {
        self.complete_rwlock_wait_at(ticket, Instant::now())
    }
    fn complete_rwlock_wait_at(
        &mut self,
        ticket: &RwlockWait,
        now: Instant,
    ) -> Result<Option<u64>, Failure> {
        if self.threads.active().id != ticket.thread || !self.threads.active().waiting {
            return Err(Failure::Unsupported("rwlock wait thread"));
        }
        self.rwlock_wait_work()?;
        let state = &mut self.pthread_rwlock;
        let (waiter, lock) = state.wait_indices(ticket)?;
        let value = &state.waiters[waiter];
        let write = value.write;
        let positive = value.positive;
        let acquire = state.can_acquire(lock, ticket.thread, write)?;
        let mut returned = if acquire {
            0
        } else if value.deadline.is_some_and(|deadline| now >= deadline) {
            result(Errno::TimedOut, value.positive)
        } else {
            return Ok(None);
        };
        if acquire {
            match state.acquire(lock, ticket.thread, write, !write) {
                Ok(()) => {}
                Err(Failure::Errno(error)) => returned = result(error, positive),
                Err(error) => return Err(error),
            }
        }
        state.remove_wait(waiter, lock);
        self.threads.active_mut().waiting = false;
        Ok(Some(returned))
    }

    /// Internal scheduler cancellation cleanup, not a guest pthread_cancel ABI.
    /// It consumes membership once and preserves all existing read/write owners.
    pub fn cancel_rwlock_wait(&mut self, ticket: &RwlockWait) -> Result<(), Failure> {
        if self.threads.active().id != ticket.thread || !self.threads.active().waiting {
            return Err(Failure::Unsupported("rwlock wait thread"));
        }
        self.rwlock_wait_work()?;
        let (waiter, lock) = self.pthread_rwlock.wait_indices(ticket)?;
        self.pthread_rwlock.remove_wait(waiter, lock);
        self.threads.active_mut().waiting = false;
        Ok(())
    }

    #[cfg(test)]
    fn begin_rwlock_wait_at(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        now: Instant,
    ) -> Result<RwlockWaitAction, Failure> {
        self.begin_rwlock_wait_with(service, args, guest, || now)
    }
}

#[cfg(test)]
mod tests;
