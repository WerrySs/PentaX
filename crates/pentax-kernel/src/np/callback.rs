// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX from KytyPS5@209dffe network.cpp, NpManager.
//! Owned offline callback rows; immutable append-only rows permit exact snapshot
//! ranges without copying payloads. No host callback or network state is used.
use crate::{CallArgs, Failure, Kernel, Service, budget::Budget};
use std::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};
const INVALID_ARGUMENT: u64 = 0x8055_0003;
const DUPLICATE: u64 = 0x8055_0008;
// Reuse the native callback-frame bound. More simultaneous checks explicitly
// refuse; successful empty recursive checks need no additional frame.
const FRAMES: usize = 32;
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AdmittedNpCallback {
    sequence: u64,
    batch_sequence: u64,
    batch: u8,
    index: usize,
    end: usize,
    entry: u64,
    userdata: u64,
    user: i32,
}
impl AdmittedNpCallback {
    pub const fn entry(&self) -> u64 {
        self.entry
    }
    pub const fn args(&self) -> [u64; 3] {
        [self.user as u32 as u64, 1, self.userdata]
    }
}
impl fmt::Debug for AdmittedNpCallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdmittedNpCallback").finish_non_exhaustive()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpCallbackAction {
    Returned(u64),
    Call(AdmittedNpCallback),
}
#[derive(Clone, Copy)]
struct Row {
    entry: u64,
    userdata: u64,
}
pub(super) struct Callbacks {
    rows: Vec<Row>,
    snapshot_end: usize,
    active: [Option<AdmittedNpCallback>; FRAMES],
    maximum: usize,
    failed: bool,
}
impl Callbacks {
    pub(super) fn new(maximum: usize) -> Self {
        Self {
            rows: Vec::new(),
            snapshot_end: 0,
            active: [None; FRAMES],
            maximum,
            failed: false,
        }
    }
    pub(super) fn retained(&self) -> usize {
        self.rows.len()
    }
    fn sequence() -> Result<u64, Failure> {
        NEXT_SEQUENCE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |x| x.checked_add(1))
            .map_err(|_| Failure::Limit("NP callback serials"))
    }
    pub(super) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        if self.failed {
            return Err(Failure::Unsupported("failed NP callback batch"));
        }
        match service {
            Service::NpRegisterStateCallbackA => {
                if args.0[0] == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                budget.work(self.rows.len() as u64 * 2 + 1)?;
                if self.rows.iter().any(|x| x.entry == args.0[0]) {
                    return Ok(DUPLICATE);
                }
                if self.rows.len() >= self.maximum {
                    return Err(Failure::Limit("NP callback rows"));
                }
                budget.can_add_handle()?;
                let count = self.rows.len().checked_add(1).ok_or(Failure::Allocation)?;
                let held = self
                    .rows
                    .capacity()
                    .checked_mul(size_of::<Row>())
                    .ok_or(Failure::Allocation)?;
                budget.scratch(
                    count
                        .checked_mul(size_of::<Row>())
                        .and_then(|x| x.checked_add(held))
                        .ok_or(Failure::Allocation)?,
                )?;
                let mut grown = Vec::new();
                grown
                    .try_reserve_exact(count)
                    .map_err(|_| Failure::Allocation)?;
                let peak = grown
                    .capacity()
                    .checked_mul(size_of::<Row>())
                    .and_then(|x| x.checked_add(held))
                    .ok_or(Failure::Allocation)?;
                budget.scratch(peak)?;
                budget.record_scratch(peak);
                grown.extend_from_slice(&self.rows);
                grown.push(Row {
                    entry: args.0[0],
                    userdata: args.0[1],
                });
                self.rows = grown;
                budget.added_handle();
                Ok(count as u64)
            }
            Service::NpCheckCallback => {
                budget.work(1)?;
                if self.snapshot_end != self.rows.len() {
                    Err(Failure::Unsupported("prepared NP callback"))
                } else {
                    Ok(0)
                }
            }
            _ => Err(Failure::Unsupported("NP callback service")),
        }
    }
    fn admit(&mut self, user: i32, budget: &mut Budget) -> Result<NpCallbackAction, Failure> {
        budget.work(FRAMES as u64 + 1)?;
        if self.failed {
            return Err(Failure::Unsupported("failed NP callback batch"));
        }
        let end = self.rows.len();
        let index = self.snapshot_end;
        if index == end {
            return Ok(NpCallbackAction::Returned(0));
        }
        let batch = self
            .active
            .iter()
            .position(Option::is_none)
            .ok_or(Failure::Limit("NP callback frames"))?;
        let sequence = Self::sequence()?;
        let row = self.rows[index];
        let request = AdmittedNpCallback {
            sequence,
            batch_sequence: sequence,
            batch: batch as u8,
            index,
            end,
            entry: row.entry,
            userdata: row.userdata,
            user,
        };
        // Equivalent to clearing every pending flag before any invocation: rows are
        // immutable, never removed/reused, and new registrations append after end.
        self.snapshot_end = end;
        self.active[batch] = Some(request);
        Ok(NpCallbackAction::Call(request))
    }
    fn complete(
        &mut self,
        request: &AdmittedNpCallback,
        budget: &mut Budget,
    ) -> Result<NpCallbackAction, Failure> {
        budget.work(2)?;
        let batch = usize::from(request.batch);
        if self.failed || self.active.get(batch) != Some(&Some(*request)) {
            return Err(Failure::Unsupported("stale NP callback admission"));
        }
        let index = request.index + 1;
        if index == request.end {
            self.active[batch] = None;
            return Ok(NpCallbackAction::Returned(0));
        }
        let sequence = Self::sequence()?;
        let row = self.rows[index];
        let next = AdmittedNpCallback {
            sequence,
            index,
            entry: row.entry,
            userdata: row.userdata,
            ..*request
        };
        self.active[batch] = Some(next);
        Ok(NpCallbackAction::Call(next))
    }
    fn fail(&mut self, request: &AdmittedNpCallback) {
        if self
            .active
            .get(usize::from(request.batch))
            .is_some_and(|row| {
                row.is_some_and(|current| current.batch_sequence == request.batch_sequence)
            })
        {
            self.failed = true;
        }
    }
}
impl Kernel {
    /// Admit a complete pending snapshot. This data ticket proves no execution.
    pub fn admit_np_callbacks(&mut self) -> Result<NpCallbackAction, Failure> {
        self.budget.attempt()?;
        let user = if self.np.callbacks.snapshot_end != self.np.callbacks.rows.len() {
            self.user.observed_id()
        } else {
            0
        };
        self.np.callbacks.admit(user, &mut self.budget)
    }
    /// Trusted native owner continuation after its exact actual return evidence.
    pub fn complete_np_callback(
        &mut self,
        request: &AdmittedNpCallback,
    ) -> Result<NpCallbackAction, Failure> {
        self.np.callbacks.complete(request, &mut self.budget)
    }
    /// Revoke a failed admitted batch permanently; never reinterpret it as empty.
    pub fn fail_np_callback(&mut self, request: &AdmittedNpCallback) {
        self.np.callbacks.fail(request);
    }
}
#[cfg(test)]
mod tests;
