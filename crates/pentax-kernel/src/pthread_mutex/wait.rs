// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded cooperative mutex continuations.
// NativeMutexLock/Unlock from pinned kernel/pthread.cpp; see NOTICE/provenance.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

// Never reused across Kernel instances; a numeric thread/object match alone
// cannot replay a consumed wait or admit a ticket from another process owner.
static NEXT_WAIT: AtomicU64 = AtomicU64::new(1);

/// Private queue membership. This is neither native execution authority nor a
/// completed guest syscall; the executor must retain its actual pending token.
pub struct MutexWait {
    serial: u64,
    thread: u32,
}
impl std::fmt::Debug for MutexWait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MutexWait").finish_non_exhaustive()
    }
}
#[derive(Debug)]
pub enum MutexWaitAction {
    Returned(u64),
    Waiting(MutexWait),
}

pub(super) struct Waiter {
    serial: u64,
    mutex: u64,
    thread: u32,
    positive: bool,
    deadline: Option<Instant>,
    // Source has a distinct self-timed branch: timeout never recursively
    // reacquires a nonrecursive mutex or releases its original ownership.
    self_wait: bool,
}

fn error_value(error: Errno, positive: bool) -> u64 {
    if positive {
        error as u64
    } else {
        u64::from(error.kernel_code() as u32)
    }
}

impl MutexState {
    fn wait_index(&self, ticket: &MutexWait, budget: &mut Budget) -> Result<usize, Failure> {
        budget.work(self.waiters.len() as u64 + 1)?;
        self.waiters
            .iter()
            .position(|waiter| waiter.serial == ticket.serial && waiter.thread == ticket.thread)
            .ok_or(Failure::Unsupported("unowned mutex wait"))
    }
    fn wait_mutex(&self, waiter: usize, budget: &mut Budget) -> Result<usize, Failure> {
        budget.work(self.mutexes.len() as u64 + 1)?;
        self.mutexes
            .iter()
            .position(|mutex| mutex.id == self.waiters[waiter].mutex && mutex.mutex_waits != 0)
            .ok_or(Failure::Unsupported("stale mutex wait object"))
    }
}

impl crate::Kernel {
    /// Admit exactly one guest call. Contention enrolls the selected thread;
    /// it does not sleep this host or return success before real acquisition.
    pub fn begin_mutex_wait(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<MutexWaitAction, Failure> {
        self.begin_mutex_wait_with(service, args, guest, Instant::now)
    }

    fn begin_mutex_wait_with(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        now: impl FnOnce() -> Instant,
    ) -> Result<MutexWaitAction, Failure> {
        use Service::*;
        if !matches!(
            service,
            MutexLock | PosixMutexLock | MutexTimedLock | PosixMutexTimedLock
        ) {
            return Err(Failure::Unsupported("mutex wait service"));
        }
        self.budget.attempt()?;
        guest.preflight_pthread()?;
        self.budget.work(
            24 + self.pthread_mutex.waiters.len() as u64 * 3
                + self.pthread_mutex.mutexes.len() as u64 * 3,
        )?;
        if self.threads.active().waiting {
            return Err(Failure::Unsupported("thread already waiting"));
        }
        let positive = matches!(service, PosixMutexLock | PosixMutexTimedLock);
        let result = (|| {
            // POSIX validates and reads its absolute timespec before the source
            // wrapper resolves or lazily initializes the mutex slot.
            let micros = match service {
                PosixMutexTimedLock => Some(absolute_micros(args.0[1], guest, &mut self.budget)?),
                MutexTimedLock => Some(args.0[1] as u32),
                _ => None,
            };
            self.enroll_mutex_wait(args.0[0], micros, positive, guest, now)
        })();
        match result {
            Err(Failure::Errno(error)) => {
                Ok(MutexWaitAction::Returned(error_value(error, positive)))
            }
            result => result,
        }
    }

    fn enroll_mutex_wait(
        &mut self,
        slot: u64,
        micros: Option<u32>,
        positive: bool,
        guest: &mut impl GuestAccess,
        now: impl FnOnce() -> Instant,
    ) -> Result<MutexWaitAction, Failure> {
        let thread = self.threads.active().id;
        let state = &mut self.pthread_mutex;
        let index = state.mutex_index(slot, true, &mut self.memory, guest, &mut self.budget)?;
        let mutex = &mut state.mutexes[index];
        if mutex.owner.is_none() {
            mutex.owner = Some(thread);
            mutex.count = 1;
            return Ok(MutexWaitAction::Returned(0));
        }
        let self_wait = mutex.owner == Some(thread);
        if self_wait && mutex.kind == 2 {
            let count = mutex.count.checked_add(1).ok_or(Errno::Again)?;
            mutex.count = count;
            return Ok(MutexWaitAction::Returned(0));
        }
        if micros == Some(0) {
            return Err(Errno::TimedOut.into());
        }
        if self_wait && micros.is_none() {
            return Err(Errno::Deadlock.into());
        }
        if state.waiters.len() >= state.max_waiters {
            return Err(Failure::Limit("mutex waiters"));
        }
        if state.waiters.iter().any(|waiter| waiter.thread == thread) {
            return Err(Failure::Unsupported("duplicate mutex wait"));
        }
        let next_waiters = mutex
            .mutex_waits
            .checked_add(1)
            .ok_or(Failure::Limit("mutex waiters"))?;
        let deadline = micros
            .map(|micros| {
                now()
                    .checked_add(Duration::from_micros(u64::from(micros)))
                    .ok_or(Failure::Limit("mutex deadline"))
            })
            .transpose()?;
        let serial = NEXT_WAIT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| Failure::Limit("mutex wait identities"))?;
        // Source stores protocol0/1/2 but NativeMutexLock ignores it. Preserve
        // real exclusion for all three without claiming priority inheritance.
        state.waiters.push(Waiter {
            serial,
            mutex: mutex.id,
            thread,
            positive,
            deadline,
            self_wait,
        });
        mutex.mutex_waits = next_waiters;
        self.threads.active_mut().waiting = true;
        Ok(MutexWaitAction::Waiting(MutexWait { serial, thread }))
    }

    /// Observational predicate, usable while another guest TLS is selected.
    pub fn mutex_wait_ready(&mut self, ticket: &MutexWait) -> Result<bool, Failure> {
        self.mutex_wait_ready_at(ticket, Instant::now())
    }
    fn mutex_wait_ready_at(&mut self, ticket: &MutexWait, now: Instant) -> Result<bool, Failure> {
        let index = self.pthread_mutex.wait_index(ticket, &mut self.budget)?;
        let mutex = self.pthread_mutex.wait_mutex(index, &mut self.budget)?;
        let waiter = &self.pthread_mutex.waiters[index];
        Ok(
            (!waiter.self_wait && self.pthread_mutex.mutexes[mutex].owner.is_none())
                || waiter.deadline.is_some_and(|deadline| now >= deadline),
        )
    }

    pub fn mutex_wait_delay(&mut self, ticket: &MutexWait) -> Result<Option<Duration>, Failure> {
        let now = Instant::now();
        let index = self.pthread_mutex.wait_index(ticket, &mut self.budget)?;
        let mutex = self.pthread_mutex.wait_mutex(index, &mut self.budget)?;
        let waiter = &self.pthread_mutex.waiters[index];
        if !waiter.self_wait && self.pthread_mutex.mutexes[mutex].owner.is_none() {
            return Ok(Some(Duration::ZERO));
        }
        Ok(waiter
            .deadline
            .map(|deadline| deadline.saturating_duration_since(now)))
    }

    /// Select the enrolled thread and validate its actual pending native token
    /// before calling this method. A successful acquisition transfers ownership
    /// once; a timeout consumes only membership and preserves the current owner.
    pub fn complete_mutex_wait(&mut self, ticket: &MutexWait) -> Result<Option<u64>, Failure> {
        self.complete_mutex_wait_at(ticket, Instant::now())
    }
    fn complete_mutex_wait_at(
        &mut self,
        ticket: &MutexWait,
        now: Instant,
    ) -> Result<Option<u64>, Failure> {
        if self.threads.active().id != ticket.thread || !self.threads.active().waiting {
            return Err(Failure::Unsupported("mutex wait thread"));
        }
        let state = &mut self.pthread_mutex;
        let index = state.wait_index(ticket, &mut self.budget)?;
        let mutex_index = state.wait_mutex(index, &mut self.budget)?;
        let waiter = &state.waiters[index];
        // NativeMutexLock tests the owner predicate before its deadline loop.
        // A free mutex can therefore be acquired even after a late scheduling
        // wake; a self-timed wait follows its separate unconditional timeout.
        let acquire = !waiter.self_wait && state.mutexes[mutex_index].owner.is_none();
        let value = if acquire {
            0
        } else if waiter.deadline.is_some_and(|deadline| now >= deadline) {
            error_value(Errno::TimedOut, waiter.positive)
        } else {
            return Ok(None);
        };
        self.budget.work(state.waiters.len() as u64 + 1)?;
        let mutex = &mut state.mutexes[mutex_index];
        if acquire {
            mutex.owner = Some(ticket.thread);
            mutex.count = 1;
        }
        mutex.mutex_waits -= 1;
        state.waiters.remove(index);
        self.threads.active_mut().waiting = false;
        Ok(Some(value))
    }

    #[cfg(test)]
    fn begin_mutex_wait_at(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        now: Instant,
    ) -> Result<MutexWaitAction, Failure> {
        self.begin_mutex_wait_with(service, args, guest, || now)
    }
}

#[cfg(test)]
mod tests;
