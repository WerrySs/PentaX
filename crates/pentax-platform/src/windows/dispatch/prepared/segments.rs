// SPDX-License-Identifier: GPL-2.0-only

//! Checked modeled segment integer operations. Host FS/GS stays untouched.

use super::*;
use pentax_codegen::{Instruction, Mnemonic, OpKind, Register};

#[path = "segments/integer.rs"]
mod integer;

pub(super) fn comparison_flags(
    left: u64,
    right: u64,
    size: usize,
    old: u64,
) -> Result<u64, NativeDispatchError> {
    integer::calculate(Mnemonic::Cmp, left, right, size, old).map(|(_, flags)| flags)
}

impl NativeBlockStepper<'_, '_, '_> {
    pub(super) fn emulate_segment(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, '_, '_>,
    ) -> Result<NativeBlockStop, NativeDispatchError> {
        let instruction = block
            .exit()
            .instruction()
            .ok_or(NativeDispatchError::UnsupportedSegmentInstruction)?;
        charge(
            &mut self.machine.usage.work_units,
            self.machine
                .target_visits
                .checked_add(128)
                .ok_or(NativeDispatchError::Budget("segment work"))?,
            self.machine.limits.max_work_units,
            "segment work",
        )?;
        if instruction.has_rep_prefix()
            || instruction.has_repne_prefix()
            || instruction.has_lock_prefix()
            || instruction.has_xacquire_prefix()
            || instruction.has_xrelease_prefix()
        {
            return Err(NativeDispatchError::UnsupportedSegmentInstruction);
        }
        if integer::supported(instruction.mnemonic()) {
            return integer::emulate(self, memory, block, instruction);
        }
        if !matches!(
            instruction.mnemonic(),
            Mnemonic::Mov | Mnemonic::Movsx | Mnemonic::Movsxd | Mnemonic::Movzx
        ) || instruction.op_count() != 2
        {
            return Err(NativeDispatchError::UnsupportedSegmentInstruction);
        }
        let load =
            instruction.op1_kind() == OpKind::Memory && instruction.op0_kind() == OpKind::Register;
        let store = instruction.op0_kind() == OpKind::Memory;
        if !load && !store {
            return Err(NativeDispatchError::UnsupportedSegmentInstruction);
        }
        if store && instruction.mnemonic() != Mnemonic::Mov {
            return Err(NativeDispatchError::UnsupportedSegmentInstruction);
        }
        let size = instruction.memory_size().size();
        if !matches!(size, 1 | 2 | 4 | 8) {
            return Err(NativeDispatchError::UnsupportedSegmentInstruction);
        }
        let address = effective_address(instruction, u32::from(load), &self.machine.state)?;
        let next_pc = block.exit().next_guest_pc();
        check_target(self.source, memory, next_pc)?;
        charge(
            &mut self.machine.usage.work_units,
            size as u64 * 2 + 8,
            self.machine.limits.max_work_units,
            "segment bytes",
        )?;
        let operations = self
            .machine
            .usage
            .segment_operations
            .checked_add(1)
            .ok_or(NativeDispatchError::Budget("segment operations"))?;
        let bytes = self
            .machine
            .usage
            .segment_bytes
            .checked_add(size as u64)
            .ok_or(NativeDispatchError::Budget("segment bytes"))?;
        let mut next = self.machine.state;
        if load {
            let target = instruction.op0_register();
            let target_size = target.size();
            let (slot, shift) = register_part(target, target_size)?;
            if (instruction.mnemonic() == Mnemonic::Mov && target_size != size)
                || (instruction.mnemonic() != Mnemonic::Mov && target_size <= size)
            {
                return Err(NativeDispatchError::UnsupportedSegmentInstruction);
            }
            let mut data = [0; 8];
            memory.read(address, &mut data[..size])?;
            let mut value = u64::from_le_bytes(data);
            if matches!(instruction.mnemonic(), Mnemonic::Movsx | Mnemonic::Movsxd) {
                let shift = 64 - size * 8;
                value = ((value << shift) as i64 >> shift) as u64;
            }
            let mask = width_mask(target_size);
            value &= mask;
            next.gpr[slot] = if target_size == 8 || target_size == 4 {
                value
            } else {
                (next.gpr[slot] & !(mask << shift)) | (value << shift)
            };
            check_rsp(&self.machine.stack, next.gpr[RSP])?;
        } else {
            let value = match instruction.op1_kind() {
                OpKind::Register => {
                    let (slot, shift) = register_part(instruction.op1_register(), size)?;
                    (next.gpr[slot] >> shift) & width_mask(size)
                }
                OpKind::Immediate8 if size == 1 => instruction.immediate8() as u64,
                OpKind::Immediate16 if size == 2 => instruction.immediate16() as u64,
                OpKind::Immediate32 if size == 4 => instruction.immediate32() as u64,
                OpKind::Immediate32to64 if size == 8 => instruction.immediate32to64() as u64,
                _ => return Err(NativeDispatchError::UnsupportedSegmentInstruction),
            };
            memory.write(address, &value.to_le_bytes()[..size])?;
        }
        next.guest_pc = next_pc;
        self.machine.state = next;
        self.machine.usage.segment_operations = operations;
        self.machine.usage.segment_bytes = bytes;
        self.machine.phase = Phase::Ready;
        Ok(NativeBlockStop::Resume { guest_pc: next_pc })
    }
}

fn effective_address(
    instruction: &Instruction,
    operand: u32,
    state: &NativeCpuState,
) -> Result<u64, NativeDispatchError> {
    let base = match instruction.segment_prefix() {
        Register::FS => state.fs_base,
        Register::GS => state.gs_base,
        _ => return Err(NativeDispatchError::UnsupportedSegmentInstruction),
    }
    .ok_or(NativeDispatchError::UnconfiguredSegment)?;
    // Iced applies original RIP, signed displacement, SIB and address-size
    // truncation before adding the modeled segment base to current GPR state.
    instruction
        .virtual_address(operand, 0, |register, _, _| {
            if register == instruction.segment_prefix() {
                Some(base)
            } else if register == Register::None {
                Some(0)
            } else if register.is_gpr64() {
                Some(state.gpr[gpr_slot(register)?])
            } else if register.is_gpr32() {
                Some(state.gpr[gpr_slot(register.full_register())?] as u32 as u64)
            } else {
                None
            }
        })
        .ok_or(NativeDispatchError::UnsupportedSegmentInstruction)
}

fn width_mask(size: usize) -> u64 {
    if size == 8 {
        u64::MAX
    } else {
        (1_u64 << (size * 8)) - 1
    }
}
fn register_part(register: Register, size: usize) -> Result<(usize, u32), NativeDispatchError> {
    if !register.is_gpr() || register.size() != size {
        return Err(NativeDispatchError::UnsupportedSegmentInstruction);
    }
    let slot = gpr_slot(register.full_register())
        .ok_or(NativeDispatchError::UnsupportedSegmentInstruction)?;
    let shift = if matches!(
        register,
        Register::AH | Register::BH | Register::CH | Register::DH
    ) {
        8
    } else {
        0
    };
    Ok((slot, shift))
}
