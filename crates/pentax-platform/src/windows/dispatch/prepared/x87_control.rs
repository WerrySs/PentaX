// SPDX-License-Identifier: GPL-2.0-only
//! x87 control instructions over retained guest data, never the host FPU.

use super::{NativeCpuState, NativeDispatchError, NativeX87State};
use crate::native_memory::PausedNativeMemory;
use pentax_codegen::{Code, Instruction, OpKind, Register};

#[derive(Clone, Copy)]
enum Operation {
    Wait,
    Load,
    Control,
    Status,
    Ax,
    Clear,
    Init,
}

fn form(code: Code) -> Option<(Operation, bool)> {
    use Code::*;
    Some(match code {
        Wait => (Operation::Wait, true),
        Fldcw_m2byte => (Operation::Load, true),
        Fnstcw_m2byte => (Operation::Control, false),
        Fstcw_m2byte => (Operation::Control, true),
        Fnstsw_m2byte => (Operation::Status, false),
        Fstsw_m2byte => (Operation::Status, true),
        Fnstsw_AX => (Operation::Ax, false),
        Fstsw_AX => (Operation::Ax, true),
        Fnclex => (Operation::Clear, false),
        Fclex => (Operation::Clear, true),
        Fninit => (Operation::Init, false),
        Finit => (Operation::Init, true),
        _ => return None,
    })
}

pub(super) fn supported(code: Code) -> bool {
    form(code).is_some()
}

// Reserved high bits and bit7 read zero; bit6 reads one. Retain the legacy
// infinity-control bit along with precision, rounding and exception masks.
pub(super) const fn control_word(value: u16) -> u16 {
    (value & 0x1f3f) | 0x0040
}

fn pending(state: &NativeX87State) -> bool {
    state.status_word & !state.control_word & 0x003f != 0
}

pub(super) fn execute(
    instruction: &Instruction,
    next: &mut NativeCpuState,
    memory: &mut PausedNativeMemory<'_>,
    used_bytes: u64,
) -> Result<u64, NativeDispatchError> {
    let (operation, waiting) =
        form(instruction.code()).ok_or(NativeDispatchError::UnsupportedVectorInstruction)?;
    if waiting && pending(&next.x87) {
        return Err(NativeDispatchError::UnsupportedX87Exception);
    }
    match operation {
        Operation::Load | Operation::Control | Operation::Status => {
            if instruction.op_count() != 1
                || instruction.op0_kind() != OpKind::Memory
                || instruction.memory_size().size() != 2
            {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            used_bytes
                .checked_add(2)
                .ok_or(NativeDispatchError::Budget("vector bytes"))?;
            let address = super::vectors::address(instruction, 0, next)?;
            if matches!(operation, Operation::Load) {
                let mut bytes = [0; 2];
                memory.read(address, &mut bytes)?;
                next.x87.control_word = control_word(u16::from_le_bytes(bytes));
                let summary = if pending(&next.x87) { 0x8080 } else { 0 };
                next.x87.status_word = (next.x87.status_word & !0x8080) | summary;
            } else {
                let value = if matches!(operation, Operation::Control) {
                    next.x87.control_word
                } else {
                    next.x87.status_word
                };
                memory.write(address, &value.to_le_bytes())?;
            }
            Ok(2)
        }
        Operation::Ax => {
            if instruction.op_count() != 1
                || instruction.op0_kind() != OpKind::Register
                || instruction.op0_register() != Register::AX
            {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            next.gpr[super::RAX] =
                (next.gpr[super::RAX] & !0xffff) | u64::from(next.x87.status_word);
            Ok(0)
        }
        Operation::Wait | Operation::Clear | Operation::Init => {
            if instruction.op_count() != 0 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            match operation {
                Operation::Clear => next.x87.status_word &= 0x7f00,
                Operation::Init => {
                    let registers = next.x87.registers;
                    next.x87 = NativeX87State::new();
                    next.x87.registers = registers;
                }
                _ => {}
            }
            Ok(0)
        }
    }
}
