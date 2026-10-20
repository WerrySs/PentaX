// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX: bounded retained queues and transactional delivery.
// Pinned kernel/eventQueue.cpp/h; see NOTICE.

use crate::budget::{Budget, Limits};
use crate::{CallArgs, Errno, Failure, GuestAccess, GuestError, GuestWrite, Service};
use std::time::{Duration, Instant};

mod wait;
use wait::Waiter;

/// One admitted interrupt bound by an exclusive borrow to its real queue owner.
/// Only a successfully retired graphics command may publish this notification.
/// No guest pointer, caller-constructed target list or execution authority.
pub struct GraphicsInterrupt<'a> {
    queues: &'a mut [Queue],
    targets: Vec<(usize, usize)>,
    context: u32,
}
impl GraphicsInterrupt<'_> {
    /// Infallible publication after successful prior GPU work and label stores.
    pub fn complete(self) {
        for (queue, event) in self.targets {
            let queue = &mut self.queues[queue];
            let event = &mut queue.events[event];
            if event.active == 0 {
                event.active = queue.next_active;
                queue.next_active += 1;
            }
            event.fflags = event.fflags.saturating_add(1);
            event.data = u64::from(self.context);
        }
    }
}

impl crate::Kernel {
    /// Internal source graphics registration, not another public kernel call.
    /// Queue membership is the retained interrupt subscription: closed queues
    /// cannot keep stale live subscriptions or revive a reused handle.
    pub fn add_graphics_event(
        &mut self,
        queue: u64,
        event_id: i32,
        user_data: u64,
    ) -> Result<u64, Failure> {
        let at = self.event_queues.index(queue, &mut self.budget)?;
        let now = self.event_queues.nanos(Instant::now());
        self.event_queues.add(
            at,
            Event {
                ident: i64::from(event_id) as u64,
                filter: GRAPHICS,
                flags: 0,
                fflags: 0,
                data: i64::from(event_id) as u64,
                udata: user_data,
                deadline: 0,
                interval: 0,
                active: 0,
            },
            &mut self.budget,
            now,
        )
    }
    pub fn delete_graphics_event(&mut self, queue: u64, event_id: i32) -> Result<u64, Failure> {
        let at = self.event_queues.index(queue, &mut self.budget)?;
        let queue = &mut self.event_queues.queues[at];
        let event = EventQueues::event_index(
            queue,
            i64::from(event_id) as u64,
            GRAPHICS,
            &mut self.budget,
        )?;
        self.budget.work(queue.events.len() as u64)?;
        queue.events.remove(event);
        self.budget.removed_handle();
        Ok(0)
    }
    /// Admit every matching delivery, counter and metadata allocation BEFORE
    /// a release's label store. The borrow prevents queue/thread mutations
    /// until completion or abandonment; abandonment publishes no notification.
    pub fn prepare_graphics_interrupt(
        &mut self,
        event_id: i32,
        context: u32,
    ) -> Result<GraphicsInterrupt<'_>, Failure> {
        let id = i64::from(event_id) as u64;
        self.budget.work(self.usage().handles_live as u64 * 3 + 1)?;
        let count = self
            .event_queues
            .queues
            .iter()
            .filter(|q| {
                q.events
                    .iter()
                    .any(|e| e.ident == id && e.filter == GRAPHICS)
            })
            .count();
        let bytes = count
            .checked_mul(std::mem::size_of::<(usize, usize)>())
            .ok_or(Failure::Limit("graphics interrupt staging"))?;
        self.budget.scratch(bytes)?;
        let mut targets = Vec::new();
        targets
            .try_reserve_exact(count)
            .map_err(|_| Failure::Allocation)?;
        self.budget.record_scratch(bytes);
        for (index, queue) in self.event_queues.queues.iter().enumerate() {
            if let Some(at) = queue
                .events
                .iter()
                .position(|e| e.ident == id && e.filter == GRAPHICS)
            {
                if queue.events[at].active == 0 {
                    queue
                        .next_active
                        .checked_add(1)
                        .ok_or(Failure::Limit("event ordering"))?;
                }
                targets.push((index, at));
            }
        }
        Ok(GraphicsInterrupt {
            queues: &mut self.event_queues.queues,
            targets,
            context,
        })
    }
}

#[cfg(test)]
#[path = "tests/event_queue.rs"]
mod tests;
pub use wait::{EventQueueWait, EventQueueWaitAction};

const USER: i16 = -11;
const TIMER: i16 = -7;
const HR_TIMER: i16 = -15;
const GRAPHICS: i16 = -14;
const ADD: u16 = 1;
const ONESHOT: u16 = 0x10;
const CLEAR: u16 = 0x20;
const EVENT_BYTES: usize = 32;

#[derive(Clone, Copy)]
struct Event {
    ident: u64,
    filter: i16,
    flags: u16,
    fflags: u32,
    data: u64,
    udata: u64,
    deadline: u64,
    interval: u64,
    active: u64, // zero means not pending
}
impl Event {
    fn encode(self, output: &mut [u8]) {
        output[..8].copy_from_slice(&self.ident.to_le_bytes());
        output[8..10].copy_from_slice(&self.filter.to_le_bytes());
        output[10..12].copy_from_slice(&self.flags.to_le_bytes());
        output[12..16].copy_from_slice(&self.fflags.to_le_bytes());
        output[16..24].copy_from_slice(&self.data.to_le_bytes());
        output[24..32].copy_from_slice(&self.udata.to_le_bytes());
    }
}
struct Queue {
    id: u64,
    _name: Vec<u8>,
    events: Vec<Event>,
    next_active: u64,
}
pub(crate) struct EventQueues {
    queues: Vec<Queue>,
    next: u64,
    epoch: Instant,
    waiters: Vec<Waiter>,
    max_waiters: usize,
}

fn code(error: Errno) -> u64 {
    u64::from(error.kernel_code() as u32)
}
fn ident(value: u64) -> u64 {
    i64::from(value as i32) as u64
}

impl EventQueues {
    pub(crate) fn retained_handle_count(&self) -> usize {
        self.queues.len()
            + self
                .queues
                .iter()
                .map(|queue| queue.events.len())
                .sum::<usize>()
    }

    pub(crate) fn new(limits: Limits) -> Result<Self, Failure> {
        let mut waiters = Vec::new();
        waiters
            .try_reserve_exact(limits.max_thread_contexts)
            .map_err(|_| Failure::Allocation)?;
        Ok(Self {
            queues: Vec::new(),
            next: 1,
            epoch: Instant::now(),
            waiters,
            max_waiters: limits.max_thread_contexts,
        })
    }
    fn nanos(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.epoch)
            .as_nanos()
            .min(u128::from(u64::MAX - 1)) as u64
            + 1
    }
    fn index(&self, id: u64, budget: &mut Budget) -> Result<usize, Failure> {
        budget.work(self.queues.len() as u64 + 1)?;
        self.queues
            .iter()
            .position(|q| q.id == id)
            .ok_or(Errno::BadFd.into())
    }
    fn create(
        &mut self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let [destination, name, ..] = args.0;
        if destination == 0 || name == 0 {
            return Err(Errno::Invalid.into());
        }
        budget.can_add_handle()?;
        if self.next > i64::MAX as u64 {
            return Err(Failure::Limit("event queue identities"));
        }
        let next = self.next + 1;
        self.queues
            .try_reserve(1)
            .map_err(|_| Failure::Allocation)?;
        let mut bytes = budget.buffer(budget.limits.max_name_bytes)?;
        let mut terminated = false;
        for i in 0..bytes.len() {
            // Source stores the handle before copying its name. Read through
            // that staged store, so a same-VA overlapping name observes it.
            // Adapter work is bounded by eight one-byte backing-alias checks.
            budget.work(144)?;
            budget.precharge_read(1)?;
            bytes[i] = guest.read_after_u64_write_byte(
                name.checked_add(i as u64)
                    .ok_or(GuestError::AddressOverflow)?,
                destination,
                self.next,
            )?;
            if bytes[i] == 0 {
                bytes.truncate(i);
                terminated = true;
                break;
            }
        }
        if !terminated {
            return Err(Failure::Limit("name bytes"));
        }
        budget.write(
            guest,
            &[GuestWrite {
                address: destination,
                bytes: &self.next.to_le_bytes(),
            }],
        )?;
        self.queues.push(Queue {
            id: self.next,
            _name: bytes,
            events: Vec::new(),
            next_active: 1,
        });
        self.next = next;
        budget.added_handle();
        Ok(0)
    }
    fn event_index(
        queue: &Queue,
        id: u64,
        filter: i16,
        budget: &mut Budget,
    ) -> Result<usize, Failure> {
        budget.work(queue.events.len() as u64 + 1)?;
        queue
            .events
            .iter()
            .position(|e| e.ident == id && e.filter == filter)
            .ok_or(Errno::NoEntry.into())
    }
    fn add(
        &mut self,
        index: usize,
        event: Event,
        budget: &mut Budget,
        now: u64,
    ) -> Result<u64, Failure> {
        let queue = &mut self.queues[index];
        budget.work(queue.events.len() as u64 + 1)?;
        if let Some(at) = queue
            .events
            .iter()
            .position(|e| e.ident == event.ident && e.filter == event.filter)
        {
            // Source re-add changes only deadlines/udata, not flags, pending
            // data or queue position, even between user level/edge spellings.
            Self::tick(queue, now, budget)?;
            queue.events[at].deadline = event.deadline;
            queue.events[at].interval = event.interval;
            queue.events[at].udata = event.udata;
        } else {
            budget.can_add_handle()?;
            queue
                .events
                .try_reserve(1)
                .map_err(|_| Failure::Allocation)?;
            queue.events.push(event);
            budget.added_handle();
        }
        Ok(0)
    }
    fn tick(queue: &mut Queue, now: u64, budget: &mut Budget) -> Result<(), Failure> {
        let count = queue.events.len() as u64;
        budget.work(
            (count + 1)
                .checked_mul(count + 1)
                .ok_or(Failure::Limit("event queue work"))?,
        )?;
        queue
            .next_active
            .checked_add(count)
            .ok_or(Failure::Limit("event ordering"))?;
        for event in &mut queue.events {
            if event.active != 0 && event.deadline != 0 && event.deadline <= now {
                Self::timer_count(event, now);
            }
        }
        loop {
            let next = queue
                .events
                .iter()
                .enumerate()
                .filter(|(_, e)| e.active == 0 && e.deadline != 0 && e.deadline <= now)
                .min_by_key(|(_, e)| e.deadline)
                .map(|(i, _)| i);
            let Some(index) = next else {
                break;
            };
            let event = &mut queue.events[index];
            Self::timer_count(event, now);
            event.active = queue.next_active;
            queue.next_active += 1;
        }
        Ok(())
    }
    fn timer_count(event: &mut Event, now: u64) {
        if event.filter != TIMER {
            return;
        }
        let periods = (now - event.deadline)
            .checked_div(event.interval)
            .map_or_else(|| u64::from(event.active == 0), |periods| periods + 1);
        event.data = event.data.wrapping_add(periods);
        event.deadline = event
            .deadline
            .wrapping_add(periods.wrapping_mul(event.interval));
    }
    fn poll(
        &mut self,
        index: usize,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        now: Instant,
    ) -> Result<Option<u64>, Failure> {
        let time = self.nanos(now);
        let queue = &mut self.queues[index];
        Self::tick(queue, time, budget)?;
        let maximum = args.0[2] as i32 as usize;
        let count = queue
            .events
            .iter()
            .filter(|e| e.active != 0)
            .count()
            .min(maximum);
        if count == 0 {
            return Ok(None);
        }
        budget.work(
            (queue.events.len() as u64 + 1)
                .checked_mul(count as u64 + 3)
                .ok_or(Failure::Limit("event delivery work"))?,
        )?;
        queue
            .next_active
            .checked_add(count as u64)
            .ok_or(Failure::Limit("event ordering"))?;
        let mut selected = Vec::new();
        budget.scratch(
            count
                .checked_mul(std::mem::size_of::<usize>() + EVENT_BYTES)
                .ok_or(Failure::Limit("event staging"))?,
        )?;
        selected
            .try_reserve_exact(count)
            .map_err(|_| Failure::Allocation)?;
        let mut bytes = budget.buffer(count * EVENT_BYTES)?;
        budget.record_scratch(count * (std::mem::size_of::<usize>() + EVENT_BYTES));
        let mut prior = 0;
        for output in bytes.chunks_exact_mut(EVENT_BYTES) {
            let (at, event) = queue
                .events
                .iter()
                .enumerate()
                .filter(|(_, e)| e.active > prior)
                .min_by_key(|(_, e)| e.active)
                .unwrap();
            event.encode(output);
            prior = event.active;
            selected.push(at);
        }
        Self::publish(args.0[1], &bytes, args.0[3], count as u32, guest, budget)?;
        // All removals/reset/requeue follow successful publication. Indices are
        // stable until the final retain, and each pending event is scanned once.
        for index in &selected {
            let event = &mut queue.events[*index];
            if event.flags & ONESHOT != 0 {
                continue;
            }
            if event.flags & CLEAR != 0 || event.filter == GRAPHICS {
                event.active = 0;
                event.fflags = 0;
                event.data = 0;
            } else {
                event.active = queue.next_active;
                queue.next_active += 1;
            }
        }
        queue.events.retain(|event| {
            let remove = event.flags & ONESHOT != 0 && event.active > 0 && event.active <= prior;
            if remove {
                budget.removed_handle();
            }
            !remove
        });
        Ok(Some(0))
    }
    fn publish(
        address: u64,
        events: &[u8],
        count_address: u64,
        count: u32,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<(), Failure> {
        let end = address
            .checked_add(events.len() as u64)
            .ok_or(GuestError::AddressOverflow)?;
        let count_end = count_address
            .checked_add(4)
            .ok_or(GuestError::AddressOverflow)?;
        let output = count.to_le_bytes();
        if !events.is_empty() && address < count_end && count_address < end {
            // Source writes the count after the records. Split away bytes
            // overwritten by it; final descriptors never overlap virtually.
            let prefix = count_address
                .saturating_sub(address)
                .min(events.len() as u64) as usize;
            let suffix = count_end.saturating_sub(address).min(events.len() as u64) as usize;
            budget.write(
                guest,
                &[
                    GuestWrite {
                        address,
                        bytes: &events[..prefix],
                    },
                    GuestWrite {
                        address: address + suffix as u64,
                        bytes: &events[suffix..],
                    },
                    GuestWrite {
                        address: count_address,
                        bytes: &output,
                    },
                ],
            )
        } else if events.is_empty() {
            budget.write(
                guest,
                &[GuestWrite {
                    address: count_address,
                    bytes: &output,
                }],
            )
        } else {
            budget.write(
                guest,
                &[
                    GuestWrite {
                        address,
                        bytes: events,
                    },
                    GuestWrite {
                        address: count_address,
                        bytes: &output,
                    },
                ],
            )
        }
    }
    fn arguments(
        &self,
        args: CallArgs,
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<(usize, Option<u32>), Failure> {
        let index = self.index(args.0[0], budget)?;
        if args.0[1] == 0 {
            return Err(Errno::Fault.into());
        }
        if (args.0[2] as i32) < 1 {
            return Err(Errno::Invalid.into());
        }
        if args.0[3] == 0 {
            return Err(Failure::Unsupported("event wait missing count"));
        }
        let timeout = if args.0[4] == 0 {
            None
        } else {
            Some(budget.u32(guest, args.0[4])?)
        };
        Ok((index, timeout))
    }
    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        use Service::*;
        let now = Instant::now();
        match service {
            CreateEqueue => self.create(args, guest, budget),
            DeleteEqueue => {
                let at = self.index(args.0[0], budget)?;
                budget.work(self.queues.len() as u64 + self.queues[at].events.len() as u64 + 1)?;
                for _ in 0..=self.queues[at].events.len() {
                    budget.removed_handle();
                }
                self.queues.remove(at);
                Ok(0)
            }
            WaitEqueue => {
                let (at, timeout) = self.arguments(args, guest, budget)?;
                if let Some(value) = self.poll(at, args, guest, budget, now)? {
                    return Ok(value);
                }
                if timeout != Some(0) {
                    return Err(Failure::Unsupported("cooperative event wait required"));
                }
                Self::publish(0, &[], args.0[3], 0, guest, budget)?;
                Err(Errno::TimedOut.into())
            }
            AddUserEvent | AddUserEventEdge | AddTimerEvent | AddHRTimerEvent => {
                // HRTimer's pointer/time validation precedes source handle lookup.
                let delay = if service == AddHRTimerEvent {
                    if args.0[2] == 0 {
                        return Err(Errno::Fault.into());
                    }
                    let sec = budget.u64(guest, args.0[2])? as i64;
                    let ns = budget.u64(
                        guest,
                        args.0[2]
                            .checked_add(8)
                            .ok_or(GuestError::AddressOverflow)?,
                    )? as i64;
                    if sec < 0 || !(0..1_000_000_000).contains(&ns) {
                        return Err(Errno::Invalid.into());
                    }
                    (sec as u64)
                        .checked_mul(1_000_000_000)
                        .and_then(|n| n.checked_add(ns as u64))
                        .ok_or(Failure::Errno(Errno::Invalid))?
                } else {
                    u64::from(args.0[2] as u32) * 1000
                };
                let at = self.index(args.0[0], budget)?;
                let timer = matches!(service, AddTimerEvent | AddHRTimerEvent);
                self.add(
                    at,
                    Event {
                        ident: ident(args.0[1]),
                        filter: if !timer {
                            USER
                        } else if service == AddTimerEvent {
                            TIMER
                        } else {
                            HR_TIMER
                        },
                        flags: ADD
                            | if service == AddHRTimerEvent {
                                ONESHOT
                            } else if service != AddUserEvent {
                                CLEAR
                            } else {
                                0
                            },
                        fflags: 0,
                        data: 0,
                        udata: if timer { args.0[3] } else { 0 },
                        deadline: if timer {
                            self.nanos(now).saturating_add(delay)
                        } else {
                            0
                        },
                        interval: if service == AddTimerEvent { delay } else { 0 },
                        active: 0,
                    },
                    budget,
                    self.nanos(now),
                )
            }
            TriggerUserEvent | DeleteUserEvent | DeleteHRTimerEvent => {
                let at = self.index(args.0[0], budget)?;
                let queue = &mut self.queues[at];
                let index = Self::event_index(
                    queue,
                    ident(args.0[1]),
                    if service == DeleteHRTimerEvent {
                        HR_TIMER
                    } else {
                        USER
                    },
                    budget,
                )?;
                if service == TriggerUserEvent {
                    let event = &mut queue.events[index];
                    if event.active == 0 {
                        let next = queue
                            .next_active
                            .checked_add(1)
                            .ok_or(Failure::Limit("event ordering"))?;
                        event.active = queue.next_active;
                        queue.next_active = next;
                    }
                    event.data = args.0[2];
                    event.udata = args.0[2];
                } else {
                    budget.work(queue.events.len() as u64)?;
                    queue.events.remove(index);
                    budget.removed_handle();
                }
                Ok(0)
            }
            GetEventUserData | GetEventId | GetEventFilter | GetEventData | GetEventFflags
            | GetEventError => {
                if args.0[0] == 0 {
                    return Ok(0);
                }
                let field = |offset| {
                    args.0[0]
                        .checked_add(offset)
                        .ok_or(Failure::Guest(GuestError::AddressOverflow))
                };
                match service {
                    GetEventUserData => budget.u64(guest, field(24)?),
                    GetEventId => budget.u64(guest, field(0)?),
                    GetEventData => budget.u64(guest, field(16)?),
                    GetEventFflags => Ok(u64::from(budget.u32(guest, field(12)?)?)),
                    GetEventFilter | GetEventError => {
                        let mut bytes = [0; 2];
                        budget.read(
                            guest,
                            field(if service == GetEventFilter { 8 } else { 10 })?,
                            &mut bytes,
                        )?;
                        if service == GetEventFilter {
                            return Ok(u64::from(i16::from_le_bytes(bytes) as i32 as u32));
                        }
                        if u16::from_le_bytes(bytes) & 0x4000 == 0 {
                            return Ok(0);
                        }
                        Ok(u64::from(budget.u32(guest, field(16)?)?))
                    }
                    _ => unreachable!(),
                }
            }
            _ => Err(Failure::Unsupported("event queue service")),
        }
    }
}
