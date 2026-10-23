// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 from public KytyPS5 7b7f9a70 agc.cpp; see NOTICE.
//! Source-ordered multi-run register builders, without guest execution authority.

use super::*;

pub(super) struct RegisterBuildState {
    source: u64,
    count: usize,
    sh: bool,
    staged: Vec<u8>,
    first: u64,
    start: usize,
    next: usize,
    offset: u32,
    previous: u32,
    committed: bool,
}
impl RegisterBuildState {
    pub(super) fn retained_payload_bytes(&self) -> usize {
        self.staged.capacity()
    }
    fn offset(&self, memory: &PausedNativeMemory<'_>, index: usize) -> Result<u32, Failure> {
        if self.sh {
            let at = index * 8;
            Ok(u32::from_le_bytes(
                self.staged[at..at + 4]
                    .try_into()
                    .map_err(|_| Failure::Memory)?,
            ))
        } else {
            word(memory, self.source, index as u64 * 2)
        }
    }
    fn value(&self, index: usize) -> &[u8] {
        let at = if self.sh { index * 8 + 4 } else { index * 4 };
        &self.staged[at..at + 4]
    }
    fn read_value(&mut self, memory: &PausedNativeMemory<'_>, index: usize) -> Result<(), Failure> {
        if !self.sh {
            let value = word(memory, self.source, index as u64 * 2 + 1)?;
            self.staged[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        Ok(())
    }
    fn packet(&mut self, memory: &PausedNativeMemory<'_>) -> Result<Packet, Failure> {
        while self.next < self.count {
            let offset = self.offset(memory, self.next)?;
            if offset != self.previous.wrapping_add(1) {
                break;
            }
            self.read_value(memory, self.next)?;
            self.previous = offset;
            self.next += 1;
        }
        let count = u32::try_from(self.next - self.start).map_err(|_| Failure::Limit)?;
        let mut packet = Packet::fixed(&[
            pm4(count + 2, if self.sh { 0x76 } else { 0x79 }, 0),
            self.offset & 0xffff,
        ]);
        packet.allocate = count + 2;
        Ok(packet)
    }
}

/// (native opcode, patch operation): address, set-count, add-count.
pub(super) fn patch_kind(function: Function) -> Option<(u32, u8)> {
    use Function::*;
    Some(match function {
        AgcSetCxRegIndirectPatchSetAddress => (0x9f, 0),
        AgcSetShRegIndirectPatchSetAddress => (0x63, 0),
        AgcSetUcRegIndirectPatchSetAddress => (0x64, 0),
        AgcSetCxRegIndirectPatchSetNumRegisters => (0x9f, 1),
        AgcSetShRegIndirectPatchSetNumRegisters => (0x63, 1),
        AgcSetUcRegIndirectPatchSetNumRegisters => (0x64, 1),
        AgcSetCxRegIndirectPatchAddRegisters => (0x9f, 2),
        AgcSetShRegIndirectPatchAddRegisters => (0x63, 2),
        AgcSetUcRegIndirectPatchAddRegisters => (0x64, 2),
        _ => return None,
    })
}

impl Driver {
    pub(super) fn patch_register_command(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        function: Function,
        args: [u64; 6],
    ) -> Result<u64, Failure> {
        let (opcode, operation) = patch_kind(function).ok_or(Failure::UnsupportedQueue)?;
        if args[0] == 0 || operation == 0 && args[1] == 0 {
            return Err(Failure::UnsupportedQueue);
        }
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            4096,
            self.limits.max_preparation_work,
        )?;
        if word(memory, args[0], 0)? >> 8 & 0xff != opcode {
            return Ok(INVALID_PACKET);
        }
        let writes = if operation == 0 { 8 } else { 4 };
        self.usage.memory_written_bytes = add(
            self.usage.memory_written_bytes,
            writes,
            self.limits.max_memory_write_bytes,
        )?;
        if operation == 0 {
            let low = (word(memory, args[0], 1)? & 3) | (args[1] as u32 & !3);
            self.command_write(
                memory,
                args[0].checked_add(4).ok_or(Failure::Memory)?,
                &low.to_le_bytes(),
                4096,
            )?;
            self.command_write(
                memory,
                args[0].checked_add(8).ok_or(Failure::Memory)?,
                &((args[1] >> 32) as u32).to_le_bytes(),
                4096,
            )?;
        } else {
            let old = word(memory, args[0], 4)?;
            let count = if operation == 1 {
                args[1] as u32
            } else {
                (old & 0x3fff).wrapping_add(args[1] as u32)
            };
            let value = (old & !0x3fff) | (count & 0x3fff);
            self.command_write(
                memory,
                args[0].checked_add(16).ok_or(Failure::Memory)?,
                &value.to_le_bytes(),
                4096,
            )?;
        }
        self.poisoned = false;
        Ok(0)
    }

    pub(super) fn begin_register_group(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        function: Function,
        args: [u64; 6],
    ) -> Result<CommandBuilderAction, Failure> {
        let count = usize::try_from(args[2] as u32).map_err(|_| Failure::Limit)?;
        if count == 0 {
            return Ok(CommandBuilderAction::Returned(0));
        }
        if args[0] == 0 || args[1] == 0 {
            return Err(Failure::UnsupportedQueue);
        }
        let sh = function == Function::AgcCbSetShRegistersDirect;
        let staged_bytes = count
            .checked_mul(if sh { 8 } else { 4 })
            .ok_or(Failure::Limit)?;
        // At most count runs, each with two header DWORDs, its values and one
        // eight-byte cursor store. This entire call, including every later
        // allocator callback, shares these monotonic admissions.
        let command_bytes = (count as u64).checked_mul(12).ok_or(Failure::Limit)?;
        let writes = (count as u64).checked_mul(20).ok_or(Failure::Limit)?;
        // For n pairs and r <= n runs, publication uses 3*r+n single-output
        // None-effect transactions. A 4/8-byte store spans p<=2 guest pages,
        // so each charges 1+4*p+p*p<=13: at most 52*n transaction steps.
        // UC reads n values and n+r-1 offsets; SH reads 2*n snapshot words.
        // Each run adds at most nine descriptor reads (available before/after
        // growth, callback/reserved/user), hence at most 12*n reads spanning
        // at most 24*n pages. Their combined 76*n fits the 128*n envelope.
        // staged_bytes*8 separately admits linear staging/copy work; 4096
        // covers fixed metadata. These are admission units, not CPU
        // instructions. Actual callback execution and source/owner/executor
        // checks retain their separate executor/provider work budgets.
        let work = (count as u64)
            .checked_mul(128)
            .and_then(|n| {
                (staged_bytes as u64)
                    .checked_mul(8)
                    .and_then(|m| n.checked_add(m))
            })
            .and_then(|n| n.checked_add(4096))
            .ok_or(Failure::Limit)?;
        let retained = staged_bytes
            .checked_add(std::mem::size_of::<PendingCommandBuilder>())
            .ok_or(Failure::Limit)?;
        if command_bytes > self.limits.commands.max_command_bytes as u64
            || retained > self.limits.commands.max_host_bytes
            || work > self.limits.commands.max_work
        {
            return Err(Failure::Limit);
        }
        let written = add(
            self.usage.memory_written_bytes,
            writes,
            self.limits.max_memory_write_bytes,
        )?;
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            work,
            self.limits.max_preparation_work,
        )?;
        let mut staged = Vec::new();
        staged
            .try_reserve_exact(staged_bytes)
            .map_err(|_| Failure::Allocation)?;
        if staged
            .capacity()
            .checked_add(std::mem::size_of::<PendingCommandBuilder>())
            .is_none_or(|bytes| bytes > self.limits.commands.max_host_bytes)
        {
            return Err(Failure::Limit);
        }
        // Actual allocator capacity remains charged, including any excess.
        let capacity_work = staged
            .capacity()
            .checked_sub(staged_bytes)
            .ok_or(Failure::Limit)? as u64;
        if work
            .checked_add(capacity_work)
            .is_none_or(|n| n > self.limits.commands.max_work)
        {
            return Err(Failure::Limit);
        }
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            capacity_work,
            self.limits.max_preparation_work,
        )?;
        self.usage.memory_written_bytes = written;
        staged.resize(staged_bytes, 0);
        if sh {
            // Volatile source order: snapshot every offset/value pair before
            // the first AllocateDW can invoke guest code or publish a cursor.
            for index in 0..count {
                for field in 0..2 {
                    let value = word(memory, args[1], index as u64 * 2 + field)?;
                    let at = index * 8 + field as usize * 4;
                    staged[at..at + 4].copy_from_slice(&value.to_le_bytes());
                }
            }
        }
        let mut state = RegisterBuildState {
            source: args[1],
            count,
            sh,
            staged,
            first: 0,
            start: 0,
            next: 1,
            offset: 0,
            previous: 0,
            committed: false,
        };
        // UC reads value[0] before offset[0], matching its temporary array.
        state.read_value(memory, 0)?;
        state.offset = state.offset(memory, 0)?;
        state.previous = state.offset;
        let packet = state.packet(memory)?;
        let pending = PendingCommandBuilder {
            owner: memory.owner_id(),
            function,
            buffer: args[0],
            packet,
            callback: 0,
            callback_args: [0; 3],
            payload: Vec::new(),
            preflight_work: work + capacity_work,
            registers: Some(state),
            context: None,
        };
        self.resume_register_group(memory, pending, None)
    }

    pub(super) fn resume_register_group(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        mut pending: PendingCommandBuilder,
        mut returned: Option<u64>,
    ) -> Result<CommandBuilderAction, Failure> {
        let mut state = pending.registers.take().ok_or(Failure::UnsupportedQueue)?;
        self.poisoned = state.committed;
        loop {
            let mut allocated = None;
            if let Some(value) = returned.take() {
                if value as u8 != 0 {
                    let (up, count) = available(memory, pending.buffer)?;
                    if u64::from(pending.packet.allocate) <= count {
                        allocated = Some(up);
                    }
                }
            } else {
                let (up, count) = available(memory, pending.buffer)?;
                if u64::from(pending.packet.allocate) <= count {
                    allocated = Some(up);
                } else {
                    pending.callback = pointer(memory, pending.buffer, 32)?;
                    if pending.callback != 0 {
                        let reserved = word(memory, pending.buffer, 12)?;
                        pending.callback_args = [
                            pending.buffer,
                            u64::from(pending.packet.allocate.wrapping_add(reserved)),
                            pointer(memory, pending.buffer, 40)?,
                        ];
                        pending.registers = Some(state);
                        // Nested real services remain possible while paused at
                        // the genuine callback; committed prefixes are retained.
                        self.poisoned = false;
                        return Ok(CommandBuilderAction::Grow(pending));
                    }
                }
            }
            if let Some(up) = allocated {
                let next = up
                    .checked_add(u64::from(pending.packet.allocate) * 4)
                    .ok_or(Failure::Memory)?;
                self.command_write(
                    memory,
                    pending.buffer.checked_add(16).ok_or(Failure::Memory)?,
                    &next.to_le_bytes(),
                    pending.preflight_work,
                )?;
                state.committed = true;
                // AllocateDW publishes its cursor before returning the old
                // pointer. A null old cursor is still the source's failed
                // allocation branch; never attempt header writes at zero.
                if up != 0 {
                    if state.first == 0 {
                        state.first = up;
                    }
                    for (index, value) in pending.packet.words[..2].iter().enumerate() {
                        self.command_write(
                            memory,
                            up.checked_add(index as u64 * 4).ok_or(Failure::Memory)?,
                            &value.to_le_bytes(),
                            pending.preflight_work,
                        )?;
                    }
                    for index in state.start..state.next {
                        let at = up
                            .checked_add((index - state.start + 2) as u64 * 4)
                            .ok_or(Failure::Memory)?;
                        self.command_write(memory, at, state.value(index), pending.preflight_work)?;
                    }
                } else if state.sh {
                    self.poisoned = false;
                    return Ok(CommandBuilderAction::Returned(state.first));
                }
            } else if state.sh {
                self.poisoned = false;
                return Ok(CommandBuilderAction::Returned(state.first));
            }
            if state.next == state.count {
                self.poisoned = false;
                return Ok(CommandBuilderAction::Returned(state.first));
            }
            // UC deliberately continues after allocation failure and rereads
            // the boundary pair after the real callback, not before it.
            state.start = state.next;
            state.offset = state.offset(memory, state.start)?;
            state.previous = state.offset;
            state.read_value(memory, state.start)?;
            state.next += 1;
            pending.packet = state.packet(memory)?;
        }
    }
}
