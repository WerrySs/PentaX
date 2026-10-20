// SPDX-License-Identifier: GPL-2.0-only

//! Exact integer count forms over retained guest state, without host feature
//! dependence. Intel SDM Vol.2 defines LZCNT/TZCNT CF/ZF and POPCNT arithmetic
//! flags. Other arithmetic flags are undefined for LZCNT/TZCNT; retain them.
use super::*;
use pentax_codegen::{Code, Instruction, Mnemonic, OpKind, Register};

fn width(code: Code) -> Option<usize> {
    use Code::*;
    match code {
        Lzcnt_r16_rm16 | Tzcnt_r16_rm16 | Popcnt_r16_rm16 => Some(2),
        Lzcnt_r32_rm32 | Tzcnt_r32_rm32 | Popcnt_r32_rm32 => Some(4),
        Lzcnt_r64_rm64 | Tzcnt_r64_rm64 | Popcnt_r64_rm64 => Some(8),
        _ => None,
    }
}
pub(super) fn supported(instruction: &Instruction) -> bool {
    width(instruction.code()).is_some()
}
fn slot(
    instruction: &Instruction,
    operand: u32,
    size: usize,
) -> Result<usize, NativeDispatchError> {
    let register = instruction.op_register(operand);
    if instruction.op_kind(operand) != OpKind::Register || register.size() != size {
        return Err(NativeDispatchError::UnsupportedIntegerInstruction);
    }
    gpr_slot(register.full_register()).ok_or(NativeDispatchError::UnsupportedIntegerInstruction)
}
fn calculate(mnemonic: Mnemonic, value: u64, size: usize, flags: u64) -> (u64, u64) {
    let bits = (size * 8) as u32;
    let count = match mnemonic {
        Mnemonic::Lzcnt => value.leading_zeros() - (64 - bits),
        Mnemonic::Tzcnt => value.trailing_zeros().min(bits),
        Mnemonic::Popcnt => value.count_ones(),
        _ => unreachable!("exact count form required"),
    };
    let flags = if mnemonic == Mnemonic::Popcnt {
        (flags & !0x8d5) | (u64::from(value == 0) << 6)
    } else {
        (flags & !0x41) | u64::from(value == 0) | (u64::from(count == 0) << 6)
    };
    (u64::from(count), flags)
}

impl NativeBlockStepper<'_, '_, '_> {
    pub(super) fn emulate_bit_count(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, '_, '_>,
    ) -> Result<NativeBlockStop, NativeDispatchError> {
        let instruction = block
            .exit()
            .instruction()
            .ok_or(NativeDispatchError::UnsupportedIntegerInstruction)?;
        let size =
            width(instruction.code()).ok_or(NativeDispatchError::UnsupportedIntegerInstruction)?;
        if instruction.op_count() != 2
            || instruction.has_segment_prefix()
            || instruction.has_lock_prefix()
            || instruction.has_rep_prefix()
            || instruction.has_repne_prefix()
            || instruction.has_xacquire_prefix()
            || instruction.has_xrelease_prefix()
        {
            return Err(NativeDispatchError::UnsupportedIntegerInstruction);
        }
        let destination = slot(instruction, 0, size)?;
        charge(
            &mut self.machine.usage.work_units,
            self.machine
                .target_visits
                .checked_add(128)
                .ok_or(NativeDispatchError::Budget("integer work"))?,
            self.machine.limits.max_work_units,
            "integer work",
        )?;
        let next_pc = block.exit().next_guest_pc();
        check_target(self.source, memory, next_pc)?;
        let operations = self
            .machine
            .usage
            .integer_operations
            .checked_add(1)
            .ok_or(NativeDispatchError::Budget("integer operations"))?;
        let mut next = self.machine.state;
        let (value, transferred) = if instruction.op1_kind() == OpKind::Memory {
            if instruction.memory_size().size() != size {
                return Err(NativeDispatchError::UnsupportedIntegerInstruction);
            }
            self.machine
                .usage
                .integer_bytes
                .checked_add(size as u64)
                .ok_or(NativeDispatchError::Budget("integer bytes"))?;
            let address = instruction
                .virtual_address(1, 0, |register, _, _| {
                    if matches!(
                        register,
                        Register::None | Register::DS | Register::ES | Register::SS
                    ) {
                        Some(0)
                    } else if register.is_gpr64() {
                        Some(next.gpr[gpr_slot(register)?])
                    } else if register.is_gpr32() {
                        Some(u64::from(
                            next.gpr[gpr_slot(register.full_register())?] as u32,
                        ))
                    } else {
                        None
                    }
                })
                .ok_or(NativeDispatchError::UnsupportedIntegerInstruction)?;
            let mut bytes = [0; 8];
            memory.read(address, &mut bytes[..size])?;
            (u64::from_le_bytes(bytes), size as u64)
        } else {
            let source = slot(instruction, 1, size)?;
            let value = match size {
                2 => u64::from(next.gpr[source] as u16),
                4 => u64::from(next.gpr[source] as u32),
                _ => next.gpr[source],
            };
            (value, 0)
        };
        let (count, flags) = calculate(instruction.mnemonic(), value, size, next.rflags);
        let result = if size == 2 {
            (next.gpr[destination] & !0xffff) | count
        } else {
            count
        };
        if destination == RSP {
            check_rsp(&self.machine.stack, result)?;
        }
        next.gpr[destination] = result;
        next.rflags = flags;
        next.guest_pc = next_pc;
        self.machine.state = next;
        self.machine.usage.integer_operations = operations;
        self.machine.usage.integer_bytes += transferred;
        self.machine.phase = Phase::Ready;
        Ok(NativeBlockStop::Resume { guest_pc: next_pc })
    }
}
