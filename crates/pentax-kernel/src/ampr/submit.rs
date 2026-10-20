// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded owned receipts and capability-backed APR IO.
// Adapted from pinned libAmpr.cpp; see NOTICE.

use std::collections::HashMap;

use super::{AmprState, Binding};
use crate::budget::Budget;
use crate::filesystem::FileState;
use crate::{CallArgs, Errno, Failure, GuestAccess, GuestWrite, Service};

#[derive(Clone, Copy, Default)]
struct Execution {
    error: i32,
    offset: u32,
}
struct Receipt {
    _object: u64,
    result: u64,
    execution: Execution,
}
#[derive(Default)]
pub(super) struct Submissions {
    receipts: HashMap<u32, Receipt>,
    last_id: u32,
}
impl Submissions {
    fn allocate(&mut self, object: u64, result: u64, budget: &mut Budget) -> Result<u32, Failure> {
        let id = self.last_id.wrapping_add(1).max(1);
        let new = !self.receipts.contains_key(&id);
        if new {
            budget.can_add_apr_submission()?;
            if self.receipts.len() == self.receipts.capacity() {
                budget.work(self.receipts.len() as u64 + 1)?;
            }
            self.receipts
                .try_reserve(1)
                .map_err(|_| Failure::Allocation)?;
        }
        self.receipts.insert(
            id,
            Receipt {
                _object: object,
                result,
                execution: Execution::default(),
            },
        );
        self.last_id = id;
        if new {
            budget.added_apr_submission();
        }
        Ok(id)
    }
}

// The source publishes these outputs separately, after actual IO. In particular
// an invalid result cannot undo an earlier ID store or a completed file read.
fn output(
    at: u64,
    bytes: &[u8],
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<(), Failure> {
    if at.checked_add(bytes.len() as u64).is_none() {
        return Err(Errno::Fault.into());
    }
    super::write(guest, budget, &[GuestWrite { address: at, bytes }])
}
fn result(
    at: u64,
    execution: Execution,
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<(), Failure> {
    if at != 0 {
        let mut bytes = [0; 8];
        bytes[..4].copy_from_slice(&execution.error.to_le_bytes());
        bytes[4..].copy_from_slice(&execution.offset.to_le_bytes());
        output(at, &bytes, guest, budget)?;
    }
    Ok(())
}

impl AmprState {
    fn execute(
        &mut self,
        object: u64,
        files: &FileState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<Execution, Failure> {
        budget.work(self.bindings.len() as u64 * 3 + 1)?;
        let index = match self.resolve_index(object) {
            Some(index) => index,
            None => {
                let snapshot = Self::load(object, guest, budget)?;
                let index = self.admit(object, budget)?;
                // A cold source lookup retains metadata without registering a
                // backing alias. Empty queues need no append-space validation.
                self.bindings.push(Binding {
                    object,
                    snapshot,
                    alias: None,
                    reads: Vec::new(),
                    gather: None,
                });
                budget.added_handle();
                index
            }
        };
        let binding = &self.bindings[index];
        // Appends retain commands in increasing record-offset order. Neither
        // guest header/record overwrites nor reads mutate this owned snapshot.
        for command in &binding.reads {
            if command.record_offset >= u64::from(binding.snapshot.offset) {
                break;
            }
            budget.work(1)?;
            match files.read_apr_file(
                command.file_id,
                command.destination,
                command.size,
                command.file_offset,
                guest,
                budget,
            ) {
                Ok(()) => {}
                Err(Failure::Errno(error)) => {
                    return Ok(Execution {
                        error: error.kernel_code(),
                        offset: command.record_offset as u32,
                    });
                }
                Err(error) => return Err(error),
            }
        }
        Ok(Execution::default())
    }

    pub(crate) fn submit(
        &mut self,
        service: Service,
        args: CallArgs,
        files: &FileState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let [object, _argument, third, fourth, ..] = args.0;
        if service == Service::AprWaitCommandBuffer {
            let receipt = self
                .submissions
                .receipts
                .remove(&(object as u32))
                .ok_or(Errno::NoSuchResource)?;
            budget.removed_apr_submission();
            result(receipt.result, receipt.execution, guest, budget)?;
            return Ok(0);
        }
        if object == 0 || (service == Service::AprSubmitCommandBufferAndGetId && third == 0) {
            return Err(Errno::Invalid.into());
        }
        if service == Service::AprSubmitCommandBuffer {
            self.execute(object, files, guest, budget)?;
            return Ok(0);
        }
        let (result_address, id_address) = match service {
            Service::AprSubmitCommandBufferAndGetResult => (third, fourth),
            Service::AprSubmitCommandBufferAndGetId => (0, third),
            _ => return Err(Failure::Unsupported("APR submission service")),
        };
        let id = self.submissions.allocate(object, result_address, budget)?;
        let execution = self.execute(object, files, guest, budget)?;
        // Allocation precedes execution; an outer or typed execution failure
        // leaves the initial receipt retained, as in the pinned source.
        self.submissions.receipts.get_mut(&id).unwrap().execution = execution;
        if id_address != 0 {
            output(id_address, &id.to_le_bytes(), guest, budget)?;
        }
        result(result_address, execution, guest, budget)?;
        Ok(0)
    }
}
