// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 from pinned KytyPS5 agc.cpp1561-1627; see NOTICE.
//! Bounded source-ordered ReserveDW and fragmented context command emission.

use super::*;

pub(super) struct ContextBuildState {
    operation: u32,
    phase: usize,
    first: u64,
    committed: bool,
}
impl ContextBuildState {
    // Zero denotes a standalone ReserveDW(22), never an allocation.
    fn steps(&self) -> &'static [u32] {
        match self.operation {
            0 => &[5],
            1 => &[0, 5, 8, 9, 3, 2],
            2 => &[3, 0, 5, 8, 9, 2],
            3 => &[0, 5, 8, 9, 3, 2, 5],
            _ => unreachable!("admitted context operation"),
        }
    }
}

impl Driver {
    pub(super) fn begin_context_operation(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        args: [u64; 6],
    ) -> Result<CommandBuilderAction, Failure> {
        let operation = args[1] as u32;
        // Source checks both before touching the descriptor or invoking guest code.
        if args[0] == 0 || operation > 3 {
            return Ok(CommandBuilderAction::Returned(0));
        }
        let state = ContextBuildState {
            operation,
            phase: 0,
            first: 0,
            committed: false,
        };
        let allocated: u64 = state.steps().iter().map(|n| u64::from(*n)).sum();
        let segments = state.steps().iter().filter(|n| **n != 0).count() as u64;
        let command_bytes = allocated * 4;
        // Each segment: cursor8, zero every DWORD, header4; first also op4.
        let output = command_bytes + segments * 12 + 4;
        // <=7 phases, <=9 descriptor reads/phase, <=4 transactions/segment.
        // Each <=36-byte output touches <=2 pages (<=13 transaction steps).
        // 8192 includes all zero/copy bytes, descriptor checks and fixed state.
        let work = 8192;
        if command_bytes > self.limits.commands.max_command_bytes as u64
            || work > self.limits.commands.max_work
            || std::mem::size_of::<PendingCommandBuilder>() > self.limits.commands.max_host_bytes
        {
            return Err(Failure::Limit);
        }
        let written = add(
            self.usage.memory_written_bytes,
            output,
            self.limits.max_memory_write_bytes,
        )?;
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            work,
            self.limits.max_preparation_work,
        )?;
        self.usage.memory_written_bytes = written;
        let pending = PendingCommandBuilder {
            owner: memory.owner_id(),
            function: Function::AgcDcbContextStateOp,
            buffer: args[0],
            packet: Packet::fixed(&[]),
            callback: 0,
            callback_args: [0; 3],
            payload: Vec::new(),
            preflight_work: work,
            registers: None,
            context: Some(state),
        };
        self.resume_context_operation(memory, pending, None)
    }

    pub(super) fn resume_context_operation(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        mut pending: PendingCommandBuilder,
        mut returned: Option<u64>,
    ) -> Result<CommandBuilderAction, Failure> {
        let mut state = pending.context.take().ok_or(Failure::UnsupportedQueue)?;
        // Retain earlier publication on any later descriptor/output failure.
        // A genuine allocator callback remains allowed to reenter the driver.
        self.poisoned = state.committed;
        while let Some(&words) = state.steps().get(state.phase) {
            let reserve = if words == 0 { 22 } else { words };
            let resumed = returned.take();
            if resumed.is_some_and(|value| value as u8 == 0) {
                self.poisoned = false;
                return Ok(CommandBuilderAction::Returned(0));
            }
            let (up, count) = available(memory, pending.buffer)?;
            if u64::from(reserve) > count {
                if resumed.is_some() {
                    self.poisoned = false;
                    return Ok(CommandBuilderAction::Returned(0));
                }
                pending.callback = pointer(memory, pending.buffer, 32)?;
                if pending.callback == 0 {
                    self.poisoned = false;
                    return Ok(CommandBuilderAction::Returned(0));
                }
                let reserved = word(memory, pending.buffer, 12)?;
                pending.callback_args = [
                    pending.buffer,
                    u64::from(reserve.wrapping_add(reserved)),
                    pointer(memory, pending.buffer, 40)?,
                ];
                pending.context = Some(state);
                self.poisoned = false;
                return Ok(CommandBuilderAction::Grow(pending));
            }
            if words != 0 {
                let next = up
                    .checked_add(u64::from(words) * 4)
                    .ok_or(Failure::Memory)?;
                self.command_write(
                    memory,
                    pending.buffer.checked_add(16).ok_or(Failure::Memory)?,
                    &next.to_le_bytes(),
                    pending.preflight_work,
                )?;
                state.committed = true;
                if up == 0 {
                    self.poisoned = false;
                    return Ok(CommandBuilderAction::Returned(0));
                }
                let zero = [0u8; 36];
                self.command_write(
                    memory,
                    up,
                    &zero[..words as usize * 4],
                    pending.preflight_work,
                )?;
                let first = state.first == 0;
                let header = pm4(words, 0x10, if first { 0x1a } else { 0 });
                self.command_write(memory, up, &header.to_le_bytes(), pending.preflight_work)?;
                if first {
                    state.first = up;
                    self.command_write(
                        memory,
                        up.checked_add(4).ok_or(Failure::Memory)?,
                        &state.operation.to_le_bytes(),
                        pending.preflight_work,
                    )?;
                }
            }
            state.phase += 1;
        }
        self.poisoned = false;
        Ok(CommandBuilderAction::Returned(state.first))
    }
}
