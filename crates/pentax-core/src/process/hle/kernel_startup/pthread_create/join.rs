// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owner-bound cooperative join of actual thread exits.
// Adapted from pinned kernel/pthread.cpp PthreadJoin; see NOTICE.
use super::*;
use pentax_kernel::{Errno, Failure};
#[derive(Clone, Copy)]
pub(in super::super) struct JoinTicket {
    index: usize,
    serial: u64,
    waiter: u64,
    output: u64,
    pointer: u64,
    waiter_index: Option<usize>,
}
pub(in super::super) enum JoinAction {
    Returned(u64),
    Wait(JoinTicket),
}
fn code(error: Errno) -> JoinAction {
    JoinAction::Returned(error.kernel_code() as u32 as u64)
}
impl ThreadRuntime {
    pub(in super::super) fn join_scan_work(&self) -> u64 {
        self.records.len() as u64 * 3 + 16
    }

    pub(in super::super) fn begin_join(
        &mut self,
        kernel: &mut Kernel,
        args: CallArgs,
        waiter: u64,
    ) -> Result<JoinAction, Failure> {
        let Some(index) = self
            .records
            .iter()
            .position(|r| r.published && r.layout.pthread_pointer == args.0[0])
        else {
            return Ok(code(Errno::NoSuchResource));
        };
        let row = &self.records[index];
        if row.joined {
            return Ok(code(Errno::NoSuchResource));
        }
        if row.thread_pointer == waiter {
            return Ok(code(Errno::Deadlock));
        }
        if row.attributes.detached || row.join_claim.is_some() {
            return Ok(code(Errno::Invalid));
        }
        let waiter_index = self
            .records
            .iter()
            .position(|row| row.thread_pointer == waiter);
        let mut cursor = Some(index);
        for step in 0..=self.records.len() {
            let Some(target) = cursor else {
                break;
            };
            if Some(target) == waiter_index || step == self.records.len() {
                return Ok(code(Errno::Deadlock));
            }
            cursor = self.records[target].waiting_for;
        }
        let serial = self.next_join_serial;
        let next = serial
            .checked_add(1)
            .ok_or(Failure::Limit("pthread join serial"))?;
        kernel.park_pthread_join()?;
        // All fallible admission precedes the single-owner reservation.
        self.records[index].join_claim = Some((serial, waiter));
        self.next_join_serial = next;
        if let Some(waiter) = waiter_index {
            self.records[waiter].waiting_for = Some(index);
        }
        Ok(JoinAction::Wait(JoinTicket {
            index,
            serial,
            waiter,
            output: args.0[1],
            pointer: args.0[0],
            waiter_index,
        }))
    }
    fn join_record(&self, ticket: &JoinTicket) -> Result<&Record, Failure> {
        self.records
            .get(ticket.index)
            .filter(|r| {
                r.published
                    && !r.joined
                    && !r.attributes.detached
                    && r.layout.pthread_pointer == ticket.pointer
                    && r.join_claim == Some((ticket.serial, ticket.waiter))
            })
            .ok_or(Failure::Unsupported("stale pthread join ownership"))
    }
    pub(in super::super) fn join_ready(&self, ticket: &JoinTicket) -> Result<bool, Failure> {
        Ok(self.join_record(ticket)?.returned.is_some())
    }
    pub(in super::super) fn complete_join(
        &mut self,
        kernel: &mut Kernel,
        ticket: &JoinTicket,
        waiter: u64,
        guest: &mut impl pentax_kernel::GuestAccess,
    ) -> Result<Option<u64>, Failure> {
        if ticket.waiter != waiter {
            return Err(Failure::Unsupported("pthread join waiter identity"));
        }
        let row = self.join_record(ticket)?;
        let Some(value) = row.returned else {
            return Ok(None);
        };
        kernel.publish_pthread_join(
            row.thread_pointer,
            ticket.pointer,
            ticket.output,
            value,
            true,
            guest,
        )?;
        // No allocation/fallible operation follows successful result publication.
        let row = &mut self.records[ticket.index];
        row.joined = true;
        row.join_claim = None;
        if let Some(waiter) = ticket.waiter_index {
            self.records[waiter].waiting_for = None;
        }
        Ok(Some(0))
    }
}
#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests;
