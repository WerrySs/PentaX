// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded cooperative condition waits; see NOTICE.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

// Global, never-reused identities also reject tickets from another Kernel.
static NEXT_WAIT: AtomicU64 = AtomicU64::new(1);

/// Opaque queue membership, not authority to execute or complete a syscall.
pub struct ConditionWait {
    serial: u64,
    thread: u32,
}
impl std::fmt::Debug for ConditionWait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConditionWait").finish_non_exhaustive()
    }
}
#[derive(Debug)]
pub enum ConditionWaitAction {
    Returned(u64),
    Waiting(ConditionWait),
}
pub(super) struct Waiter {
    serial: u64,
    condition: u64,
    mutex: u64,
    thread: u32,
    count: u32,
    notified: bool,
}

impl ConditionState {
    pub(super) fn notify(
        &mut self,
        condition: u64,
        all: bool,
        mutexes: &mut crate::pthread_mutex::MutexState,
    ) {
        for waiter in &mut self.waiters {
            if waiter.condition == condition && !waiter.notified {
                waiter.notified = true;
                mutexes.observe_condition_notification(waiter.mutex);
                if !all {
                    break;
                }
            }
        }
    }
    pub(super) fn has_waiters(&self, condition: u64) -> bool {
        self.waiters
            .iter()
            .any(|waiter| waiter.condition == condition && !waiter.notified)
    }
    pub(crate) fn wait_observations(&self) -> (u64, u64) {
        (
            self.waiters.len() as u64,
            self.waiters.iter().filter(|waiter| waiter.notified).count() as u64,
        )
    }
    fn wait_index(&self, ticket: &ConditionWait, budget: &mut Budget) -> Result<usize, Failure> {
        budget.work(self.waiters.len() as u64 + 1)?;
        self.waiters
            .iter()
            .position(|waiter| waiter.serial == ticket.serial && waiter.thread == ticket.thread)
            .ok_or(Failure::Unsupported("unowned condition wait"))
    }
}

impl crate::Kernel {
    /// Atomically enroll the active thread and release the complete recursive
    /// mutex ownership. The caller must park its pending syscall, never return
    /// success until complete_condition_wait reacquires that mutex.
    pub fn begin_condition_wait(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<ConditionWaitAction, Failure> {
        if !matches!(service, Service::CondWait | Service::PosixCondWait) {
            return Err(Failure::Unsupported("condition wait service"));
        }
        self.budget.attempt()?;
        guest.preflight_pthread()?;
        let result = self.enroll_condition_wait(args, guest);
        match result {
            Ok(ticket) => Ok(ConditionWaitAction::Waiting(ticket)),
            Err(Failure::Errno(errno)) => Ok(ConditionWaitAction::Returned(
                if service == Service::PosixCondWait {
                    errno as u64
                } else {
                    u64::from(errno.kernel_code() as u32)
                },
            )),
            Err(error) => Err(error),
        }
    }

    fn enroll_condition_wait(
        &mut self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<ConditionWait, Failure> {
        let state = &mut self.pthread_condition;
        self.budget
            .work((state.waiters.len() + state.conditions.len() + 1) as u64)?;
        let thread = self.threads.active().id;
        if self.threads.active().waiting {
            return Err(Failure::Unsupported("thread already waiting"));
        }
        if state.waiters.len() >= state.max_waiters {
            return Err(Failure::Limit("condition waiters"));
        }
        if state.waiters.iter().any(|waiter| waiter.thread == thread) {
            return Err(Failure::Unsupported("duplicate condition wait"));
        }
        let serial = NEXT_WAIT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| Failure::Limit("condition wait identities"))?;
        // Source order lazily initializes the condition and mutex slots before
        // ownership validation. No enrollment/release occurs on a later error.
        let index =
            state.condition_index(args.0[0], true, &mut self.memory, guest, &mut self.budget)?;
        let prepared = self.pthread_mutex.prepare_condition_release(
            args.0[1],
            thread,
            &mut self.memory,
            guest,
            &mut self.budget,
        )?;
        state.waiters.push(Waiter {
            serial,
            condition: state.conditions[index].id,
            mutex: prepared.id,
            thread,
            count: prepared.count,
            notified: false,
        });
        self.pthread_mutex.commit_condition_release(&prepared);
        self.threads.active_mut().waiting = true;
        Ok(ConditionWait { serial, thread })
    }

    /// Read-only readiness, usable while a different guest context is selected.
    /// A notification alone is insufficient while another thread owns the mutex.
    pub fn condition_wait_ready(&mut self, ticket: &ConditionWait) -> Result<bool, Failure> {
        let index = self
            .pthread_condition
            .wait_index(ticket, &mut self.budget)?;
        let waiter = &self.pthread_condition.waiters[index];
        if !waiter.notified {
            return Ok(false);
        }
        self.pthread_mutex
            .condition_available(waiter.mutex, &mut self.budget)
    }

    /// Select the matching TLS/kernel context first. On success the complete
    /// recursive mutex ownership is restored and this ticket is consumed once.
    /// No host wait, guest output or synthetic syscall completion occurs here.
    pub fn complete_condition_wait(&mut self, ticket: &ConditionWait) -> Result<bool, Failure> {
        if self.threads.active().id != ticket.thread {
            return Err(Failure::Unsupported("condition wait thread"));
        }
        let index = self
            .pthread_condition
            .wait_index(ticket, &mut self.budget)?;
        let waiter = &self.pthread_condition.waiters[index];
        if !waiter.notified {
            return Ok(false);
        }
        // Account for removal and the second mutex lookup before ownership changes.
        self.budget
            .work(self.pthread_condition.waiters.len() as u64)?;
        if !self.pthread_mutex.reacquire_condition(
            waiter.mutex,
            waiter.thread,
            waiter.count,
            &mut self.budget,
        )? {
            return Ok(false);
        }
        self.pthread_condition.waiters.remove(index);
        self.threads.active_mut().waiting = false;
        Ok(true)
    }
}
