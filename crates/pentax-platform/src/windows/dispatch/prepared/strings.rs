// SPDX-License-Identifier: GPL-2.0-only

//! Bounded string elements over the actual paused owner. No host string routine,
//! segment mutation, scratch allocation or unbounded native REP body is used.
//! Intel SDM Vol.2, MOVS/LODS/STOS/CMPS/SCAS and REP define the transitions.

use super::*;
use pentax_codegen::{Instruction, Mnemonic, OpKind, Register};

const QUANTUM: u64 = 256;
const DF: u64 = 1 << 10;
const ZF: u64 = 1 << 6;

#[derive(Clone, Copy)]
pub(super) struct RepeatState {
    pc: u64,
    flags: u64,
}

#[derive(Clone, Copy)]
enum Operation {
    Move,
    Store,
    Load,
    Compare,
    Scan,
}
impl Operation {
    fn reads_source(self) -> bool {
        matches!(self, Self::Move | Self::Load | Self::Compare)
    }
    fn uses_destination(self) -> bool {
        !matches!(self, Self::Load)
    }
    fn writes_destination(self) -> bool {
        matches!(self, Self::Move | Self::Store)
    }
    fn compares(self) -> bool {
        matches!(self, Self::Compare | Self::Scan)
    }
}

fn operation(instruction: &Instruction) -> Option<Operation> {
    // The implicit operand kinds distinguish MOVSD/CMPSD from their SSE
    // homonyms. Address-size32 is deliberately refused rather than truncated.
    let source = instruction
        .op_kinds()
        .any(|kind| kind == OpKind::MemorySegRSI);
    let destination = instruction
        .op_kinds()
        .any(|kind| kind == OpKind::MemoryESRDI);
    match instruction.mnemonic() {
        Mnemonic::Movsb | Mnemonic::Movsw | Mnemonic::Movsd | Mnemonic::Movsq
            if source && destination =>
        {
            Some(Operation::Move)
        }
        Mnemonic::Stosb | Mnemonic::Stosw | Mnemonic::Stosd | Mnemonic::Stosq if destination => {
            Some(Operation::Store)
        }
        Mnemonic::Lodsb | Mnemonic::Lodsw | Mnemonic::Lodsd | Mnemonic::Lodsq if source => {
            Some(Operation::Load)
        }
        Mnemonic::Cmpsb | Mnemonic::Cmpsw | Mnemonic::Cmpsd | Mnemonic::Cmpsq
            if source && destination =>
        {
            Some(Operation::Compare)
        }
        Mnemonic::Scasb | Mnemonic::Scasw | Mnemonic::Scasd | Mnemonic::Scasq if destination => {
            Some(Operation::Scan)
        }
        _ => None,
    }
}
pub(super) fn supported(instruction: &Instruction) -> bool {
    operation(instruction).is_some()
}

impl NativeBlockStepper<'_, '_, '_> {
    pub(super) fn emulate_string(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, '_, '_>,
    ) -> Result<NativeBlockStop, NativeDispatchError> {
        let instruction = block
            .exit()
            .instruction()
            .ok_or(NativeDispatchError::UnsupportedStringInstruction)?;
        let op = operation(instruction).ok_or(NativeDispatchError::UnsupportedStringInstruction)?;
        if instruction.has_lock_prefix()
            || instruction.has_xacquire_prefix()
            || instruction.has_xrelease_prefix()
            || (instruction.has_repne_prefix() && !op.compares())
        {
            return Err(NativeDispatchError::UnsupportedStringInstruction);
        }
        let size = instruction.memory_size().size();
        if !matches!(size, 1 | 2 | 4 | 8) {
            return Err(NativeDispatchError::UnsupportedStringInstruction);
        }
        let repeat = instruction.has_rep_prefix() || instruction.has_repne_prefix();
        let pc = block.exit().guest_pc();
        let next_pc = block.exit().next_guest_pc();
        let admission = self
            .machine
            .target_visits
            .checked_add(32)
            .ok_or(NativeDispatchError::Budget("string admission"))?;
        charge(
            &mut self.machine.usage.work_units,
            admission,
            self.machine.limits.max_work_units,
            "string admission",
        )?;
        check_target(self.source, memory, next_pc)?;
        if let Some(retained) = self.machine.string_repeat
            && (retained.pc != pc || !repeat || !op.compares())
        {
            return Err(NativeDispatchError::InvalidState);
        }
        // Count-zero performs no effective-address/segment resolution, loads,
        // stores, accumulator/flag changes or index updates.
        if repeat && self.machine.state.gpr[RCX] == 0 {
            self.machine.string_repeat = None;
            return Ok(self.string_resume(next_pc));
        }
        let segment_base = if op.reads_source() {
            match instruction.segment_prefix() {
                Register::FS => self
                    .machine
                    .state
                    .fs_base
                    .ok_or(NativeDispatchError::UnconfiguredSegment)?,
                Register::GS => self
                    .machine
                    .state
                    .gs_base
                    .ok_or(NativeDispatchError::UnconfiguredSegment)?,
                _ => 0,
            }
        } else {
            0
        };
        if repeat && op.compares() && self.machine.string_repeat.is_none() {
            self.machine.string_repeat = Some(RepeatState {
                pc,
                flags: self.machine.state.rflags,
            });
        }
        for _ in 0..if repeat { QUANTUM } else { 1 } {
            let effect_bytes =
                size as u64 * (u64::from(op.reads_source()) + u64::from(op.uses_destination()));
            charge(
                &mut self.machine.usage.work_units,
                effect_bytes + 32,
                self.machine.limits.max_work_units,
                "string element",
            )?;
            let elements = self
                .machine
                .usage
                .string_elements
                .checked_add(1)
                .ok_or(NativeDispatchError::Budget("string elements"))?;
            let bytes = self
                .machine
                .usage
                .string_bytes
                .checked_add(effect_bytes)
                .ok_or(NativeDispatchError::Budget("string bytes"))?;
            // Each element is one transaction. MOVS must read the current
            // source after earlier writes, even for destructive overlap.
            let result = self.string_element(memory, op, size, segment_base, repeat);
            if let Err(error) = result {
                if let Some(retained) = self.machine.string_repeat {
                    self.machine.state.rflags = retained.flags;
                }
                return Err(error);
            }
            self.machine.usage.string_elements = elements;
            self.machine.usage.string_bytes = bytes;
            let condition_ended = op.compares()
                && ((self.machine.state.rflags & ZF != 0) == instruction.has_repne_prefix());
            if !repeat || self.machine.state.gpr[RCX] == 0 || condition_ended {
                self.machine.string_repeat = None;
                return Ok(self.string_resume(next_pc));
            }
        }
        // Bounded cooperative continuation retains the original instruction PC.
        // No following guest instruction executes between its elements.
        Ok(self.string_resume(pc))
    }

    fn string_resume(&mut self, pc: u64) -> NativeBlockStop {
        self.machine.state.guest_pc = pc;
        self.machine.phase = Phase::Ready;
        NativeBlockStop::Resume { guest_pc: pc }
    }

    fn string_element(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        op: Operation,
        size: usize,
        base: u64,
        repeat: bool,
    ) -> Result<(), NativeDispatchError> {
        let mut next = self.machine.state;
        let mut source = [0; 8];
        let mut destination = [0; 8];
        let destination_address = next.gpr[RDI];
        if op.writes_destination() {
            memory.preflight_write(destination_address, size as u64)?;
        }
        if op.reads_source() {
            let address = base
                .checked_add(next.gpr[RSI])
                .ok_or(NativeDispatchError::InvalidState)?;
            memory.read(address, &mut source[..size])?;
        }
        if matches!(op, Operation::Compare | Operation::Scan) {
            memory.read(destination_address, &mut destination[..size])?;
        }
        let left = if op.reads_source() {
            u64::from_le_bytes(source)
        } else {
            next.gpr[RAX]
        };
        if op.compares() {
            next.rflags = segments::comparison_flags(
                left,
                u64::from_le_bytes(destination),
                size,
                next.rflags,
            )?;
        }
        if matches!(op, Operation::Load) {
            let value = u64::from_le_bytes(source);
            next.gpr[RAX] = if size == 4 || size == 8 {
                value
            } else {
                let mask = (1_u64 << (size * 8)) - 1;
                (next.gpr[RAX] & !mask) | value
            };
        }
        let advance = |index: u64| {
            if next.rflags & DF == 0 {
                index.wrapping_add(size as u64)
            } else {
                index.wrapping_sub(size as u64)
            }
        };
        if op.reads_source() {
            next.gpr[RSI] = advance(next.gpr[RSI]);
        }
        if op.uses_destination() {
            next.gpr[RDI] = advance(next.gpr[RDI]);
        }
        if repeat {
            next.gpr[RCX] -= 1;
        }
        if op.writes_destination() {
            memory.write(destination_address, &left.to_le_bytes()[..size])?;
        }
        // After the only effectful write, context publication is infallible.
        self.machine.state = next;
        Ok(())
    }
}
