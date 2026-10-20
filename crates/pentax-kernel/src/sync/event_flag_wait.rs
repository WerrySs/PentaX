// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted from public KytyPS5@f186b35706ddbc3faf5513f326e2335d224802f7,
// src/kernel/eventFlag.cpp, blob594e7e148bfa6294d246053ef37cda4ac09105a3.
// Modified for PentaX: bounded cooperative waits, atomic outputs and drain
// continuations instead of host condvars/spins. See NOTICE and Kyty-MIT.txt.

use super::*;
use crate::Kernel;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_WAIT: AtomicU64 = AtomicU64::new(1);

pub struct EventFlagWait {
    serial: u64,
    thread: u32,
}
impl std::fmt::Debug for EventFlagWait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventFlagWait").finish_non_exhaustive()
    }
}
#[derive(Debug)]
pub enum EventFlagWaitAction {
    Returned(u64),
    Waiting(EventFlagWait),
}

#[derive(Clone, Copy)]
pub(super) struct Waiter {
    serial: u64,
    thread: u32,
    flag: u64,
    pattern: u64,
    wait: WaitMode,
    clear: ClearMode,
    output: u64,
    timeout: Option<(u64, u32)>,
    started: Instant,
    deadline: Option<Instant>,
    // Cancel/Delete freeze only their original cohort, before other mutations.
    outcome: Option<(u64, Errno)>,
}
#[derive(Clone, Copy)]
enum OperationKind {
    Deferred {
        service: Service,
        args: CallArgs,
        timing: Option<WaitTiming>,
    },
    Drain {
        delete: bool,
    },
}
#[derive(Clone, Copy)]
struct WaitTiming {
    timeout: Option<(u64, u32)>,
    started: Instant,
}
#[derive(Clone, Copy)]
pub(super) struct Operation {
    serial: u64,
    thread: u32,
    flag: u64,
    kind: OperationKind,
}

#[derive(Clone, Copy)]
struct Arguments {
    index: usize,
    pattern: u64,
    wait: WaitMode,
    clear: ClearMode,
    output: u64,
    timeout: Option<(u64, u32)>,
}
const fn code(error: Errno) -> u64 {
    error.kernel_code() as u32 as u64
}
fn satisfied(bits: u64, pattern: u64, wait: WaitMode) -> bool {
    match wait {
        WaitMode::And => bits & pattern == pattern,
        WaitMode::Or => bits & pattern != 0,
    }
}
fn clear(bits: &mut u64, pattern: u64, mode: ClearMode) {
    match mode {
        ClearMode::None => {}
        ClearMode::All => *bits = 0,
        ClearMode::Bits => *bits &= !pattern,
    }
}

/// Source stores the full pattern first, then remaining microseconds. Normalize
/// virtual overlap in that order; the adapter must refuse ambiguous physical
/// aliases and validate every live byte before its single atomic transaction.
fn publish(
    bits: u64,
    output: u64,
    timeout: Option<(u64, u32)>,
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<(), Failure> {
    let bit_bytes = bits.to_le_bytes();
    let (time_address, micros) = timeout.unwrap_or((0, 0));
    let time_bytes = micros.to_le_bytes();
    let bit_end = output
        .checked_add(8)
        .ok_or(crate::GuestError::AddressOverflow)?;
    let time_end = time_address
        .checked_add(4)
        .ok_or(crate::GuestError::AddressOverflow)?;
    if output != 0 && time_address != 0 && output < time_end && time_address < bit_end {
        let start = output.min(time_address);
        let length = usize::try_from(bit_end.max(time_end) - start)
            .map_err(|_| Failure::Limit("event flag output"))?;
        let mut bytes = [0; 12];
        let bit_at = (output - start) as usize;
        let time_at = (time_address - start) as usize;
        bytes[bit_at..bit_at + 8].copy_from_slice(&bit_bytes);
        bytes[time_at..time_at + 4].copy_from_slice(&time_bytes);
        budget.work(12)?;
        return budget.write(
            guest,
            &[GuestWrite {
                address: start,
                bytes: &bytes[..length],
            }],
        );
    }
    let mut writes = [GuestWrite {
        address: 0,
        bytes: &[],
    }; 2];
    let mut used = 0;
    if output != 0 {
        writes[used] = GuestWrite {
            address: output,
            bytes: &bit_bytes,
        };
        used += 1;
    }
    if time_address != 0 {
        writes[used] = GuestWrite {
            address: time_address,
            bytes: &time_bytes,
        };
        used += 1;
    }
    if used != 0 {
        budget.write(guest, &writes[..used])?;
    }
    Ok(())
}

impl SyncState {
    pub(super) fn require_flag_live(&self, index: usize) -> Result<(), Failure> {
        if self.flags[index].draining.is_some() {
            return Err(Failure::Unsupported(
                "event flag drain requires continuation",
            ));
        }
        Ok(())
    }
    pub(super) fn flag_waiting_count(&self, flag: u64) -> usize {
        self.flag_waiters.iter().filter(|w| w.flag == flag).count()
    }
    fn flag_arguments(
        &self,
        args: CallArgs,
        guest: &impl GuestAccess,
        budget: &mut Budget,
        poll: bool,
        captured: Option<Option<(u64, u32)>>,
    ) -> Result<Arguments, Failure> {
        let index = self.flag_index(args.0[0], budget)?;
        if args.0[1] == 0 {
            return Err(Errno::Invalid.into());
        }
        let (wait, clear) = decode_wait_mode(args.0[2] as u32)?;
        let timeout = if let Some(timeout) = captured {
            timeout
        } else if poll || args.0[4] == 0 {
            None
        } else {
            Some((args.0[4], budget.u32(guest, args.0[4])?))
        };
        budget.work(self.flag_waiters.len() as u64 + 1)?;
        // Source checks Single BEFORE the predicate, even for an immediate poll.
        if self.flags[index].single_waiter && self.flag_waiting_count(args.0[0]) != 0 {
            return Err(Errno::Perm.into());
        }
        Ok(Arguments {
            index,
            pattern: args.0[1],
            wait,
            clear,
            output: args.0[3],
            timeout,
        })
    }
    pub(super) fn direct_event_flag_wait(
        &mut self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        poll: bool,
    ) -> Result<u64, Failure> {
        let a = self.flag_arguments(args, guest, budget, poll, None)?;
        self.require_flag_live(a.index)?;
        let bits = self.flags[a.index].bits;
        let matched = satisfied(bits, a.pattern, a.wait);
        if !matched && !poll && !a.timeout.is_some_and(|(_, us)| us == 0) {
            return Err(Failure::Unsupported("blocking event flag wait"));
        }
        publish(bits, a.output, a.timeout, guest, budget)?;
        if !matched {
            return Err(if poll { Errno::Busy } else { Errno::TimedOut }.into());
        }
        clear(&mut self.flags[a.index].bits, a.pattern, a.clear);
        Ok(0)
    }
    pub(super) fn direct_event_flag_cancel(
        &mut self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let index = self.flag_index(args.0[0], budget)?;
        self.require_flag_live(index)?;
        budget.work(self.flag_waiters.len() as u64 + 1)?;
        if self.flag_waiting_count(args.0[0]) != 0 {
            return Err(Failure::Unsupported("blocking event flag cancellation"));
        }
        if args.0[2] != 0 {
            budget.write(
                guest,
                &[GuestWrite {
                    address: args.0[2],
                    bytes: &0i32.to_le_bytes(),
                }],
            )?;
        }
        self.flags[index].bits = args.0[1];
        Ok(0)
    }
    fn ticket_serial(&self, existing: Option<usize>, budget: &mut Budget) -> Result<u64, Failure> {
        let count = self.flag_waiters.len() + self.flag_operations.len();
        budget.work(count as u64 * 2 + 1)?;
        if let Some(index) = existing {
            return Ok(self.flag_operations[index].serial);
        }
        if count >= self.max_waiters {
            return Err(Failure::Limit("event flag waiters"));
        }
        NEXT_WAIT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Failure::Limit("event flag wait identities"))
    }
    fn operation(&mut self, existing: Option<usize>, operation: Operation) {
        if let Some(index) = existing {
            self.flag_operations[index] = operation;
        } else {
            self.flag_operations.push(operation);
        }
    }
}

impl Kernel {
    pub fn begin_event_flag_wait(
        &mut self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<EventFlagWaitAction, Failure> {
        self.begin_event_flag_operation(Service::WaitEventFlag, args, guest)
    }
    pub fn begin_event_flag_operation(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<EventFlagWaitAction, Failure> {
        self.begin_event_flag_operation_at(service, args, guest, Instant::now())
    }
    fn begin_event_flag_operation_at(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        now: Instant,
    ) -> Result<EventFlagWaitAction, Failure> {
        self.budget.attempt()?;
        guest.preflight_pthread()?;
        if self.threads.active().waiting {
            return Err(Failure::Unsupported("thread already waiting"));
        }
        match self.start_event_flag_operation(service, args, guest, now, None, None) {
            Err(Failure::Errno(error)) => Ok(EventFlagWaitAction::Returned(code(error))),
            other => other,
        }
    }
    fn start_event_flag_operation(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        now: Instant,
        existing: Option<usize>,
        timing: Option<WaitTiming>,
    ) -> Result<EventFlagWaitAction, Failure> {
        if !matches!(
            service,
            Service::WaitEventFlag
                | Service::SetEventFlag
                | Service::ClearEventFlag
                | Service::CancelEventFlag
                | Service::DeleteEventFlag
        ) {
            return Err(Failure::Unsupported("event flag continuation service"));
        }
        let state = &mut self.sync;
        let index = state.flag_index(args.0[0], &mut self.budget)?;
        // Precharge all table motion before any output/bit/handle publication.
        self.budget
            .work((state.flag_waiters.len() + state.flag_operations.len()) as u64 + 1)?;
        let thread = self.threads.active().id;
        // Source reads timeout and enforces Single before predicate/status.
        // Preserve those inputs and its clock start across barrier deferral.
        let arguments = if service == Service::WaitEventFlag {
            Some(state.flag_arguments(
                args,
                guest,
                &mut self.budget,
                false,
                timing.map(|t| t.timeout),
            )?)
        } else {
            None
        };
        if state.flags[index].draining.is_some() {
            let serial = state.ticket_serial(existing, &mut self.budget)?;
            let timing = arguments.map(|a| {
                timing.unwrap_or(WaitTiming {
                    timeout: a.timeout,
                    started: now,
                })
            });
            state.operation(
                existing,
                Operation {
                    serial,
                    thread,
                    flag: args.0[0],
                    kind: OperationKind::Deferred {
                        service,
                        args,
                        timing,
                    },
                },
            );
            self.threads.active_mut().waiting = true;
            return Ok(EventFlagWaitAction::Waiting(EventFlagWait {
                serial,
                thread,
            }));
        }
        if let Some(a) = arguments {
            let started = timing.map_or(now, |t| t.started);
            let deadline = a
                .timeout
                .map(|(_, micros)| {
                    started
                        .checked_add(Duration::from_micros(u64::from(micros)))
                        .ok_or(Failure::Limit("event flag deadline"))
                })
                .transpose()?;
            let bits = state.flags[index].bits;
            let matched = satisfied(bits, a.pattern, a.wait);
            if matched || deadline.is_some_and(|end| now >= end) {
                let elapsed = now
                    .saturating_duration_since(started)
                    .as_micros()
                    .min(u128::from(u32::MAX)) as u32;
                let remaining = a
                    .timeout
                    .map(|(address, micros)| (address, micros.saturating_sub(elapsed)));
                publish(bits, a.output, remaining, guest, &mut self.budget)?;
                if !matched {
                    return Err(Errno::TimedOut.into());
                }
                clear(&mut state.flags[index].bits, a.pattern, a.clear);
                return Ok(EventFlagWaitAction::Returned(0));
            }
            let serial = state.ticket_serial(existing, &mut self.budget)?;
            if let Some(index) = existing {
                state.flag_operations.remove(index);
            }
            state.flag_waiters.push(Waiter {
                serial,
                thread,
                flag: args.0[0],
                pattern: a.pattern,
                wait: a.wait,
                clear: a.clear,
                output: a.output,
                timeout: a.timeout,
                started,
                deadline,
                outcome: None,
            });
            self.threads.active_mut().waiting = true;
            return Ok(EventFlagWaitAction::Waiting(EventFlagWait {
                serial,
                thread,
            }));
        }
        if matches!(service, Service::SetEventFlag | Service::ClearEventFlag) {
            return state
                .invoke(service, args, guest, &mut self.budget)
                .map(EventFlagWaitAction::Returned);
        }
        self.budget.work(state.flag_waiters.len() as u64 + 1)?;
        let count = state.flag_waiting_count(args.0[0]);
        if count == 0 {
            return state
                .invoke(service, args, guest, &mut self.budget)
                .map(EventFlagWaitAction::Returned);
        }
        let serial = state.ticket_serial(existing, &mut self.budget)?;
        let delete = service == Service::DeleteEventFlag;
        if !delete && args.0[2] != 0 {
            let count =
                i32::try_from(count).map_err(|_| Failure::Limit("event flag waiter count"))?;
            self.budget.write(
                guest,
                &[GuestWrite {
                    address: args.0[2],
                    bytes: &count.to_le_bytes(),
                }],
            )?;
        }
        let bits = if delete {
            state.flags[index].bits
        } else {
            args.0[1]
        };
        let error = if delete {
            Errno::Access
        } else {
            Errno::Canceled
        };
        for waiter in &mut state.flag_waiters {
            if waiter.flag == args.0[0] {
                waiter.outcome = Some((bits, error));
            }
        }
        state.flags[index].bits = bits;
        state.flags[index].draining = Some(serial);
        state.operation(
            existing,
            Operation {
                serial,
                thread,
                flag: args.0[0],
                kind: OperationKind::Drain { delete },
            },
        );
        self.threads.active_mut().waiting = true;
        Ok(EventFlagWaitAction::Waiting(EventFlagWait {
            serial,
            thread,
        }))
    }
    fn event_flag_ticket(&mut self, ticket: &EventFlagWait) -> Result<(bool, usize), Failure> {
        self.budget
            .work((self.sync.flag_waiters.len() + self.sync.flag_operations.len()) as u64 + 1)?;
        if let Some(index) = self
            .sync
            .flag_waiters
            .iter()
            .position(|w| w.serial == ticket.serial && w.thread == ticket.thread)
        {
            return Ok((true, index));
        }
        self.sync
            .flag_operations
            .iter()
            .position(|w| w.serial == ticket.serial && w.thread == ticket.thread)
            .map(|index| (false, index))
            .ok_or(Failure::Unsupported("unowned event flag wait"))
    }
    pub fn event_flag_wait_ready(&mut self, ticket: &EventFlagWait) -> Result<bool, Failure> {
        self.event_flag_wait_ready_at(ticket, Instant::now())
    }
    fn event_flag_wait_ready_at(
        &mut self,
        ticket: &EventFlagWait,
        now: Instant,
    ) -> Result<bool, Failure> {
        let (wait, index) = self.event_flag_ticket(ticket)?;
        if wait {
            let w = self.sync.flag_waiters[index];
            if w.outcome.is_some() {
                return Ok(true);
            }
            let flag = self.sync.flag_index(w.flag, &mut self.budget)?;
            return Ok(satisfied(self.sync.flags[flag].bits, w.pattern, w.wait)
                || w.deadline.is_some_and(|end| now >= end));
        }
        let operation = self.sync.flag_operations[index];
        match operation.kind {
            OperationKind::Drain { .. } => {
                self.budget.work(self.sync.flag_waiters.len() as u64 + 1)?;
                Ok(self.sync.flag_waiting_count(operation.flag) == 0)
            }
            OperationKind::Deferred { .. } => {
                match self.sync.flag_index(operation.flag, &mut self.budget) {
                    Ok(index) => Ok(self.sync.flags[index].draining.is_none()),
                    Err(Failure::Errno(Errno::NoSuchResource)) => Ok(true),
                    Err(error) => Err(error),
                }
            }
        }
    }
    pub fn event_flag_wait_delay(
        &mut self,
        ticket: &EventFlagWait,
    ) -> Result<Option<Duration>, Failure> {
        let now = Instant::now();
        if self.event_flag_wait_ready_at(ticket, now)? {
            return Ok(Some(Duration::ZERO));
        }
        let (wait, index) = self.event_flag_ticket(ticket)?;
        Ok(if wait {
            self.sync.flag_waiters[index]
                .deadline
                .map(|end| end.saturating_duration_since(now))
        } else {
            None
        })
    }
    pub fn complete_event_flag_wait(
        &mut self,
        ticket: &EventFlagWait,
        guest: &mut impl GuestAccess,
    ) -> Result<Option<u64>, Failure> {
        self.complete_event_flag_wait_at(ticket, guest, Instant::now())
    }
    fn complete_event_flag_wait_at(
        &mut self,
        ticket: &EventFlagWait,
        guest: &mut impl GuestAccess,
        now: Instant,
    ) -> Result<Option<u64>, Failure> {
        guest.preflight_pthread()?;
        if self.threads.active().id != ticket.thread || !self.threads.active().waiting {
            return Err(Failure::Unsupported("event flag wait thread"));
        }
        let (wait, index) = self.event_flag_ticket(ticket)?;
        let state = &mut self.sync;
        self.budget.work(
            (state.flag_waiters.len() + state.flag_operations.len() + state.flags.len()) as u64 + 1,
        )?;
        if wait {
            let w = state.flag_waiters[index];
            let (bits, error, flag) = if let Some((bits, error)) = w.outcome {
                (bits, Some(error), None)
            } else {
                let flag = state.flag_index(w.flag, &mut self.budget)?;
                let bits = state.flags[flag].bits;
                if satisfied(bits, w.pattern, w.wait) {
                    (bits, None, Some(flag))
                } else if w.deadline.is_some_and(|end| now >= end) {
                    (bits, Some(Errno::TimedOut), None)
                } else {
                    return Ok(None);
                }
            };
            let timeout = w.timeout.map(|(address, micros)| {
                let elapsed = now
                    .saturating_duration_since(w.started)
                    .as_micros()
                    .min(u128::from(u32::MAX)) as u32;
                (address, micros.saturating_sub(elapsed))
            });
            publish(bits, w.output, timeout, guest, &mut self.budget)?;
            if let Some(flag) = flag {
                clear(&mut state.flags[flag].bits, w.pattern, w.clear);
            }
            state.flag_waiters.remove(index);
            self.threads.active_mut().waiting = false;
            return Ok(Some(error.map_or(0, code)));
        }
        let operation = state.flag_operations[index];
        match operation.kind {
            OperationKind::Drain { delete } => {
                if state.flag_waiting_count(operation.flag) != 0 {
                    return Ok(None);
                }
                let flag = state.flag_index(operation.flag, &mut self.budget)?;
                if state.flags[flag].draining != Some(operation.serial) {
                    return Err(Failure::Unsupported("event flag drain identity"));
                }
                if delete {
                    state.flags.swap_remove(flag);
                    self.budget.removed_handle();
                } else {
                    state.flags[flag].draining = None;
                }
                state.flag_operations.remove(index);
                self.threads.active_mut().waiting = false;
                Ok(Some(0))
            }
            OperationKind::Deferred {
                service,
                args,
                timing,
            } => {
                match state.flag_index(operation.flag, &mut self.budget) {
                    Ok(flag) if state.flags[flag].draining.is_some() => return Ok(None),
                    Err(Failure::Errno(Errno::NoSuchResource)) => {
                        state.flag_operations.remove(index);
                        self.threads.active_mut().waiting = false;
                        return Ok(Some(code(Errno::NoSuchResource)));
                    }
                    Err(error) => return Err(error),
                    _ => {}
                }
                let result =
                    self.start_event_flag_operation(service, args, guest, now, Some(index), timing);
                let value = match result {
                    Ok(EventFlagWaitAction::Waiting(_)) => return Ok(None),
                    Ok(EventFlagWaitAction::Returned(value)) => value,
                    Err(Failure::Errno(error)) => code(error),
                    Err(error) => return Err(error),
                };
                self.sync.flag_operations.remove(index);
                self.threads.active_mut().waiting = false;
                Ok(Some(value))
            }
        }
    }
}

#[cfg(test)]
#[path = "event_flag_wait/tests.rs"]
mod tests;
