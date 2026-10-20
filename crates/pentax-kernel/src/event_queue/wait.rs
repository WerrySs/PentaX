// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX: retained event wait continuations, not host blocking.

use super::*;
use crate::Kernel;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_WAIT: AtomicU64 = AtomicU64::new(1);

pub struct EventQueueWait {
    serial: u64,
    thread: u32,
}
impl std::fmt::Debug for EventQueueWait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventQueueWait").finish_non_exhaustive()
    }
}
#[derive(Debug)]
pub enum EventQueueWaitAction {
    Returned(u64),
    Waiting(EventQueueWait),
}
pub(super) struct Waiter {
    serial: u64,
    thread: u32,
    args: CallArgs,
    deadline: Option<Instant>,
}
impl EventQueues {
    fn waiter_index(&self, ticket: &EventQueueWait, budget: &mut Budget) -> Result<usize, Failure> {
        budget.work(self.waiters.len() as u64 + 1)?;
        self.waiters
            .iter()
            .position(|w| w.serial == ticket.serial && w.thread == ticket.thread)
            .ok_or(Failure::Unsupported("unowned event wait"))
    }
}
impl Kernel {
    pub fn begin_event_queue_wait(
        &mut self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<EventQueueWaitAction, Failure> {
        self.begin_event_queue_wait_at(args, guest, Instant::now())
    }
    pub(crate) fn begin_event_queue_wait_at(
        &mut self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        now: Instant,
    ) -> Result<EventQueueWaitAction, Failure> {
        self.budget.attempt()?;
        guest.preflight_pthread()?;
        let result = self.enroll_event_queue_wait(args, guest, now);
        match result {
            Err(Failure::Errno(error)) => Ok(EventQueueWaitAction::Returned(code(error))),
            other => other,
        }
    }
    fn enroll_event_queue_wait(
        &mut self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        now: Instant,
    ) -> Result<EventQueueWaitAction, Failure> {
        if self.threads.active().waiting {
            return Err(Failure::Unsupported("thread already waiting"));
        }
        let (at, timeout) = self.event_queues.arguments(args, guest, &mut self.budget)?;
        if let Some(value) = self
            .event_queues
            .poll(at, args, guest, &mut self.budget, now)?
        {
            return Ok(EventQueueWaitAction::Returned(value));
        }
        if timeout == Some(0) {
            EventQueues::publish(0, &[], args.0[3], 0, guest, &mut self.budget)?;
            return Err(Errno::TimedOut.into());
        }
        let deadline = timeout
            .map(|micros| {
                now.checked_add(Duration::from_micros(u64::from(micros)))
                    .ok_or(Failure::Limit("event wait deadline"))
            })
            .transpose()?;
        if self.event_queues.waiters.len() >= self.event_queues.max_waiters {
            return Err(Failure::Limit("event waiters"));
        }
        let serial = NEXT_WAIT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Failure::Limit("event wait identities"))?;
        let thread = self.threads.active().id;
        self.event_queues.waiters.push(Waiter {
            serial,
            thread,
            args,
            deadline,
        });
        self.threads.active_mut().waiting = true;
        Ok(EventQueueWaitAction::Waiting(EventQueueWait {
            serial,
            thread,
        }))
    }
    pub fn event_queue_wait_ready(&mut self, ticket: &EventQueueWait) -> Result<bool, Failure> {
        self.event_queue_wait_ready_at(ticket, Instant::now())
    }
    pub(crate) fn event_queue_wait_ready_at(
        &mut self,
        ticket: &EventQueueWait,
        now: Instant,
    ) -> Result<bool, Failure> {
        let index = self.event_queues.waiter_index(ticket, &mut self.budget)?;
        let waiter = &self.event_queues.waiters[index];
        let id = waiter.args.0[0];
        let expired = waiter.deadline.is_some_and(|deadline| now >= deadline);
        let at = match self.event_queues.index(id, &mut self.budget) {
            Ok(index) => index,
            Err(Failure::Errno(Errno::BadFd)) => return Ok(true),
            Err(error) => return Err(error),
        };
        let time = self.event_queues.nanos(now);
        let queue = &mut self.event_queues.queues[at];
        EventQueues::tick(queue, time, &mut self.budget)?;
        Ok(expired || queue.events.iter().any(|e| e.active != 0))
    }
    pub fn event_queue_wait_delay(
        &mut self,
        ticket: &EventQueueWait,
    ) -> Result<Option<Duration>, Failure> {
        let now = Instant::now();
        if self.event_queue_wait_ready_at(ticket, now)? {
            return Ok(Some(Duration::ZERO));
        }
        let index = self.event_queues.waiter_index(ticket, &mut self.budget)?;
        let waiter = &self.event_queues.waiters[index];
        let at = self
            .event_queues
            .index(waiter.args.0[0], &mut self.budget)?;
        let queue = &self.event_queues.queues[at];
        self.budget.work(queue.events.len() as u64 + 1)?;
        let time = self.event_queues.nanos(now);
        let timer = queue
            .events
            .iter()
            .filter(|e| e.active == 0 && e.deadline != 0)
            .map(|e| Duration::from_nanos(e.deadline.saturating_sub(time)))
            .min();
        let deadline = waiter
            .deadline
            .map(|deadline| deadline.saturating_duration_since(now));
        Ok(match (timer, deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        })
    }
    pub fn complete_event_queue_wait(
        &mut self,
        ticket: &EventQueueWait,
        guest: &mut impl GuestAccess,
    ) -> Result<Option<u64>, Failure> {
        self.complete_event_queue_wait_at(ticket, guest, Instant::now())
    }
    pub(crate) fn complete_event_queue_wait_at(
        &mut self,
        ticket: &EventQueueWait,
        guest: &mut impl GuestAccess,
        now: Instant,
    ) -> Result<Option<u64>, Failure> {
        guest.preflight_pthread()?;
        if self.threads.active().id != ticket.thread {
            return Err(Failure::Unsupported("event wait thread"));
        }
        let index = self.event_queues.waiter_index(ticket, &mut self.budget)?;
        let waiter = &self.event_queues.waiters[index];
        let args = waiter.args;
        let expired = waiter.deadline.is_some_and(|deadline| now >= deadline);
        self.budget
            .work(self.event_queues.waiters.len() as u64 + 1)?;
        let result = match self.event_queues.index(args.0[0], &mut self.budget) {
            Ok(at) => {
                if let Some(result) =
                    self.event_queues
                        .poll(at, args, guest, &mut self.budget, now)?
                {
                    result
                } else if expired {
                    EventQueues::publish(0, &[], args.0[3], 0, guest, &mut self.budget)?;
                    code(Errno::TimedOut)
                } else {
                    return Ok(None);
                }
            }
            Err(Failure::Errno(Errno::BadFd)) => {
                EventQueues::publish(
                    0,
                    &[],
                    args.0[3],
                    code(Errno::BadFd) as u32,
                    guest,
                    &mut self.budget,
                )?;
                code(Errno::BadFd)
            }
            Err(error) => return Err(error),
        };
        self.event_queues.waiters.remove(index);
        self.threads.active_mut().waiting = false;
        Ok(Some(result))
    }
}
