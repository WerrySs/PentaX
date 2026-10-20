// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: retained read records and ordered guest transactions.
// Adapted from pinned libAmpr.cpp; see NOTICE.

use super::{AmprState, Binding, valid_header};
use crate::budget::Budget;
use crate::{CallArgs, Errno, Failure, GuestAccess, GuestWrite};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ReadCommand {
    pub(super) record_offset: u64,
    pub(super) file_id: u32,
    pub(super) destination: u64,
    pub(super) size: u64,
    pub(super) file_offset: u64,
}

fn valid_read(destination: u64, size: u64, offset: u64) -> bool {
    size != 0
        && size <= 0x1_0000_0000
        && offset < 0x100_0000_0000
        && destination
            .checked_add(size)
            .is_some_and(|end| end <= 0x0000_f000_0000_0000)
}
fn record_size(offset: u64) -> u32 {
    if offset >> 32 == 0 { 20 } else { 24 }
}

// At most 24 record bytes and three four-byte header stores. Stage source store
// order, including reads after earlier stores, without touching disjoint gaps.
struct Edits {
    cells: [(u64, u8); 36],
    length: usize,
}
impl Edits {
    fn new() -> Self {
        Self {
            cells: [(0, 0); 36],
            length: 0,
        }
    }
    fn write(&mut self, address: u64, bytes: &[u8]) {
        for (index, byte) in bytes.iter().copied().enumerate() {
            let at = address + index as u64;
            if let Some(cell) = self.cells[..self.length]
                .iter_mut()
                .find(|cell| cell.0 == at)
            {
                cell.1 = byte;
            } else {
                self.cells[self.length] = (at, byte);
                self.length += 1;
            }
        }
    }
    fn read_u32(
        &self,
        address: u64,
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u32, Failure> {
        let mut bytes = [0; 4];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let at = address + index as u64;
            if let Some(cell) = self.cells[..self.length].iter().find(|cell| cell.0 == at) {
                *byte = cell.1;
            } else {
                budget.read(guest, at, std::slice::from_mut(byte))?;
            }
        }
        Ok(u32::from_le_bytes(bytes))
    }
    fn commit(mut self, guest: &mut impl GuestAccess, budget: &mut Budget) -> Result<(), Failure> {
        self.cells[..self.length].sort_unstable_by_key(|cell| cell.0);
        let mut bytes = [0; 36];
        for (target, (_, byte)) in bytes.iter_mut().zip(&self.cells[..self.length]) {
            *target = *byte;
        }
        let mut writes = [GuestWrite {
            address: 0,
            bytes: &[],
        }; 4];
        let mut count = 0;
        let mut start = 0;
        while start < self.length {
            let mut end = start + 1;
            while end < self.length && self.cells[end].0 == self.cells[end - 1].0 + 1 {
                end += 1;
            }
            writes[count] = GuestWrite {
                address: self.cells[start].0,
                bytes: &bytes[start..end],
            };
            count += 1;
            start = end;
        }
        budget.write(guest, &writes[..count])
    }
}

impl AmprState {
    pub(crate) fn measure_read(args: CallArgs) -> Result<u64, Failure> {
        let [_, destination, size, offset, ..] = args.0;
        if !valid_read(destination, size, offset) {
            return Err(Errno::Invalid.into());
        }
        Ok(u64::from(record_size(offset)))
    }

    pub(crate) fn append_read(
        &mut self,
        args: CallArgs,
        file_offset: u64,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let [object, _, _, file_id, destination, size] = args.0;
        if object == 0 || !valid_read(destination, size, file_offset) {
            return Err(Errno::Invalid.into());
        }
        if !valid_header(object) {
            return Err(Errno::Fault.into());
        }
        budget.work(self.bindings.len() as u64 * 6 + 2048)?;
        let index = if let Some(index) = self
            .bindings
            .iter()
            .position(|entry| entry.object == object)
        {
            index
        } else {
            let snapshot = Self::load(object, guest, budget)?;
            let index = self.admit(object, budget)?;
            // Source first-use header caching precedes space validation. It is
            // an exact-object snapshot; loading alone does not register aliases.
            self.bindings.push(Binding {
                object,
                snapshot,
                alias: None,
                reads: Vec::new(),
                gather: None,
            });
            budget.added_handle();
            index
        };
        let snapshot = self.bindings[index].snapshot;
        let length = record_size(file_offset);
        if snapshot.buffer == 0
            || length > snapshot.size
            || snapshot.offset > snapshot.size - length
            || snapshot
                .buffer
                .checked_add(u64::from(snapshot.size))
                .is_none()
        {
            return Err(Errno::Fault.into());
        }
        budget.can_add_ampr_command()?;
        budget.scratch(36)?;
        budget.record_scratch(36);
        let reads = &mut self.bindings[index].reads;
        if reads.len() == reads.capacity() {
            budget.work(reads.len() as u64 + 1)?;
        }
        reads.try_reserve(1).map_err(|_| Failure::Allocation)?;
        let command = ReadCommand {
            record_offset: u64::from(snapshot.offset),
            file_id: file_id as u32,
            destination,
            size,
            file_offset,
        };
        let gather = (
            command.file_id,
            command.destination + command.size,
            command.file_offset + command.size,
        );
        let mut record = [0; 24];
        record[0] = 0x17;
        let mut edits = Edits::new();
        edits.write(
            snapshot.buffer + command.record_offset,
            &record[..length as usize],
        );
        let next_offset = snapshot.offset + length;
        edits.write(object + 4, &next_offset.to_le_bytes());
        let count = edits.read_u32(object + 8, guest, budget)?.wrapping_add(1);
        edits.write(object + 8, &count.to_le_bytes());
        let kind = edits.read_u32(object, guest, budget)? | 0x1_0000;
        edits.write(object, &kind.to_le_bytes());
        // Distinct physical aliases are refused by the adapter. Only after
        // this atomic write may retained offset, queue and continuation advance.
        edits.commit(guest, budget)?;
        let binding = &mut self.bindings[index];
        binding.snapshot.offset = next_offset;
        binding.reads.push(command);
        binding.gather = Some(gather);
        budget.added_ampr_command();
        Ok(0)
    }
}

#[cfg(test)]
#[path = "../tests/ampr_read.rs"]
mod tests;
