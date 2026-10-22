// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded cooperative POSIX/pthread semaphore waits.
// Adapted from pinned kernel/semaphore.cpp PosixSemPrivate::Wait; see NOTICE.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

// Opaque, globally unique membership rejects foreign and consumed tickets.
static NEXT_WAIT: AtomicU64 = AtomicU64::new(1);

pub struct PthreadSemaphoreWait {
    serial: u64,
    thread: u32,
}
impl std::fmt::Debug for PthreadSemaphoreWait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PthreadSemaphoreWait")
            .finish_non_exhaustive()
    }
}
#[derive(Debug)]
pub enum PthreadSemaphoreWaitAction {
    Returned(u64),
    Waiting(PthreadSemaphoreWait),
}
pub(super) struct Waiter {
    serial: u64,
    address: u64,
    thread: u32,
    posix: bool,
}

impl SemaphoreState {
    pub(super) fn has_waiters(&self, address: u64) -> bool {
        self.waiters.iter().any(|waiter| waiter.address == address)
    }
    fn wait_index(
        &self,
        ticket: &PthreadSemaphoreWait,
        budget: &mut Budget,
    ) -> Result<usize, Failure> {
        budget.work(self.waiters.len() as u64 + 1)?;
        self.waiters
            .iter()
            .position(|waiter| waiter.serial == ticket.serial && waiter.thread == ticket.thread)
            .ok_or(Failure::Unsupported("unowned pthread semaphore wait"))
    }
    fn wait_semaphore(&self, waiter: usize, budget: &mut Budget) -> Result<usize, Failure> {
        budget.work(self.entries.len() as u64 + 1)?;
        self.entries
            .iter()
            .position(|entry| entry.address == self.waiters[waiter].address)
            .ok_or(Failure::Unsupported("stale pthread semaphore wait object"))
    }
}

impl Kernel {
    /// Retain an indefinite wait only when no count is available. Fast-path
    /// writes and both return domains use the existing semaphore transaction.
    pub fn begin_pthread_semaphore_wait(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<PthreadSemaphoreWaitAction, Failure> {
        if !matches!(service, Service::PosixSemWait | Service::PthreadSemWait) {
            return Err(Failure::Unsupported("pthread semaphore wait service"));
        }
        self.budget.attempt()?;
        guest.preflight_pthread()?;
        let posix = service == Service::PosixSemWait;
        if posix {
            guest.preflight_posix_errno(None)?;
        }
        if self.threads.active().waiting {
            return Err(Failure::Unsupported("thread already waiting"));
        }
        self.budget.work(
            (self.pthread_semaphores.entries.len() + self.pthread_semaphores.waiters.len()) as u64
                + 16,
        )?;
        let index = self
            .pthread_semaphores
            .entries
            .iter()
            .position(|entry| entry.address == args.0[0]);
        if index.is_none_or(|index| self.pthread_semaphores.entries[index].count != 0) {
            return match self.pthread_semaphore(service, args, guest) {
                Ok(value) => Ok(PthreadSemaphoreWaitAction::Returned(value)),
                Err(Failure::Errno(error)) => Ok(PthreadSemaphoreWaitAction::Returned(u64::from(
                    error.kernel_code() as u32,
                ))),
                Err(error) => Err(error),
            };
        }
        let state = &mut self.pthread_semaphores;
        if state.waiters.len() >= state.max_waiters {
            return Err(Failure::Limit("pthread semaphore waiters"));
        }
        let thread = self.threads.active().id;
        if state.waiters.iter().any(|waiter| waiter.thread == thread) {
            return Err(Failure::Unsupported("duplicate pthread semaphore wait"));
        }
        let serial = NEXT_WAIT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| Failure::Limit("pthread semaphore wait identities"))?;
        let payload = prefix(args.0[0], 0, None, true);
        // All storage and membership work is admitted before output. A fault
        // publishes no queue entry or thread flag; push cannot allocate here.
        self.budget.write(
            guest,
            &[GuestWrite {
                address: payload.address,
                bytes: &payload.bytes[..payload.length],
            }],
        )?;
        state.waiters.push(Waiter {
            serial,
            address: args.0[0],
            thread,
            posix,
        });
        self.threads.active_mut().waiting = true;
        Ok(PthreadSemaphoreWaitAction::Waiting(PthreadSemaphoreWait {
            serial,
            thread,
        }))
    }

    /// Predicate only. A posted count is not reserved: source TryWait/new
    /// callers may consume it before this thread's actual scheduling turn.
    pub fn pthread_semaphore_wait_ready(
        &mut self,
        ticket: &PthreadSemaphoreWait,
    ) -> Result<bool, Failure> {
        let state = &self.pthread_semaphores;
        let waiter = state.wait_index(ticket, &mut self.budget)?;
        let semaphore = state.wait_semaphore(waiter, &mut self.budget)?;
        Ok(state.entries[semaphore].count > 0)
    }

    /// The runtime must first validate its exact saved syscall token and
    /// selected TLS/executor. Consume one count only after atomic guest output.
    pub fn complete_pthread_semaphore_wait(
        &mut self,
        ticket: &PthreadSemaphoreWait,
        guest: &mut impl GuestAccess,
    ) -> Result<Option<u64>, Failure> {
        guest.preflight_pthread()?;
        if self.threads.active().id != ticket.thread || !self.threads.active().waiting {
            return Err(Failure::Unsupported("pthread semaphore wait thread"));
        }
        let state = &mut self.pthread_semaphores;
        let waiter = state.wait_index(ticket, &mut self.budget)?;
        let semaphore = state.wait_semaphore(waiter, &mut self.budget)?;
        if state.waiters[waiter].posix {
            guest.preflight_posix_errno(None)?;
        }
        let entry = &state.entries[semaphore];
        if entry.count == 0 {
            return Ok(None);
        }
        self.budget.work(state.waiters.len() as u64 + 1)?;
        let waiting = state
            .waiters
            .iter()
            .enumerate()
            .any(|(index, other)| index != waiter && other.address == entry.address);
        let count = entry.count - 1;
        let payload = prefix(entry.address, count, None, waiting);
        self.budget.write(
            guest,
            &[GuestWrite {
                address: payload.address,
                bytes: &payload.bytes[..payload.length],
            }],
        )?;
        // Rejected writes retain count, membership and waiting flag together.
        state.entries[semaphore].count = count;
        state.waiters.swap_remove(waiter);
        self.threads.active_mut().waiting = false;
        Ok(Some(0))
    }
}

#[cfg(test)]
mod tests;
