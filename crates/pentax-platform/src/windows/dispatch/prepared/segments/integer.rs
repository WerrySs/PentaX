// SPDX-License-Identifier: GPL-2.0-only

//! Width-bounded scalar ALU over the currently paused guest segment address.
//! Intel SDM Vol.2 integer instruction semantics; logical AF is undefined and
//! deterministically retained. No host segment prefix or native body is emitted.

use super::*;

const CF: u64 = 1;
const PF: u64 = 1 << 2;
const AF: u64 = 1 << 4;
const ZF: u64 = 1 << 6;
const SF: u64 = 1 << 7;
const OF: u64 = 1 << 11;
const STATUS: u64 = CF | PF | AF | ZF | SF | OF;

pub(super) fn supported(mnemonic: Mnemonic) -> bool {
    matches!(
        mnemonic,
        Mnemonic::Add
            | Mnemonic::Adc
            | Mnemonic::Sub
            | Mnemonic::Sbb
            | Mnemonic::And
            | Mnemonic::Or
            | Mnemonic::Xor
            | Mnemonic::Cmp
            | Mnemonic::Test
            | Mnemonic::Inc
            | Mnemonic::Dec
            | Mnemonic::Neg
            | Mnemonic::Not
    )
}

pub(super) fn emulate(
    engine: &mut NativeBlockStepper<'_, '_, '_>,
    memory: &mut PausedNativeMemory<'_>,
    block: &MaterializedBlockPlan<'_, '_, '_>,
    instruction: &Instruction,
) -> Result<NativeBlockStop, NativeDispatchError> {
    let unary = matches!(
        instruction.mnemonic(),
        Mnemonic::Inc | Mnemonic::Dec | Mnemonic::Neg | Mnemonic::Not
    );
    if instruction.op_count() != if unary { 1 } else { 2 } {
        return Err(NativeDispatchError::UnsupportedSegmentInstruction);
    }
    let memory_operand = match (instruction.op0_kind(), instruction.op1_kind(), unary) {
        (OpKind::Memory, _, true) => 0,
        (OpKind::Memory, kind, false) if kind != OpKind::Memory => 0,
        (OpKind::Register, OpKind::Memory, false) => 1,
        _ => return Err(NativeDispatchError::UnsupportedSegmentInstruction),
    };
    let size = instruction.memory_size().size();
    if !matches!(size, 1 | 2 | 4 | 8) {
        return Err(NativeDispatchError::UnsupportedSegmentInstruction);
    }
    let writes = !matches!(instruction.mnemonic(), Mnemonic::Cmp | Mnemonic::Test);
    let memory_write = writes && memory_operand == 0;
    let current = engine.machine.state;
    let address = effective_address(instruction, memory_operand, &current)?;
    let next_pc = block.exit().next_guest_pc();
    check_target(engine.source, memory, next_pc)?;
    let transferred = size as u64 * (1 + u64::from(memory_write));
    charge(
        &mut engine.machine.usage.work_units,
        transferred * 2 + 32,
        engine.machine.limits.max_work_units,
        "segment bytes",
    )?;
    let operations = engine
        .machine
        .usage
        .segment_operations
        .checked_add(1)
        .ok_or(NativeDispatchError::Budget("segment operations"))?;
    let bytes = engine
        .machine
        .usage
        .segment_bytes
        .checked_add(transferred)
        .ok_or(NativeDispatchError::Budget("segment bytes"))?;
    // Admit the complete read/write spans before reading or computing. On any
    // refusal no CPU state, flags, successful counters or guest output commit.
    memory.preflight_read(address, size as u64)?;
    if memory_write {
        memory.preflight_write(address, size as u64)?;
    }
    let mut data = [0; 8];
    memory.read(address, &mut data[..size])?;
    let value = u64::from_le_bytes(data);
    let (left, right) = if unary {
        (value, 0)
    } else if memory_operand == 0 {
        (value, scalar(instruction, 1, size, &current)?)
    } else {
        (scalar(instruction, 0, size, &current)?, value)
    };
    let (result, flags) = calculate(instruction.mnemonic(), left, right, size, current.rflags)?;
    let mut next = current;
    if writes && !memory_write {
        let (slot, shift) = register_part(instruction.op0_register(), size)?;
        next.gpr[slot] = if size >= 4 {
            result
        } else {
            (next.gpr[slot] & !(width_mask(size) << shift)) | (result << shift)
        };
        check_rsp(&engine.machine.stack, next.gpr[RSP])?;
    }
    next.rflags = flags;
    next.guest_pc = next_pc;
    if memory_write {
        memory.write(address, &result.to_le_bytes()[..size])?;
    }
    // Everything after the only effectful write is infallible.
    engine.machine.state = next;
    engine.machine.usage.segment_operations = operations;
    engine.machine.usage.segment_bytes = bytes;
    engine.machine.phase = Phase::Ready;
    Ok(NativeBlockStop::Resume { guest_pc: next_pc })
}

fn scalar(
    instruction: &Instruction,
    operand: u32,
    size: usize,
    state: &NativeCpuState,
) -> Result<u64, NativeDispatchError> {
    let value = match instruction.op_kind(operand) {
        OpKind::Register => {
            let (slot, shift) = register_part(instruction.op_register(operand), size)?;
            state.gpr[slot] >> shift
        }
        OpKind::Immediate8 if size == 1 => instruction.immediate8() as u64,
        OpKind::Immediate16 if size == 2 => instruction.immediate16() as u64,
        OpKind::Immediate32 if size == 4 => instruction.immediate32() as u64,
        OpKind::Immediate8to16 if size == 2 => instruction.immediate8to16() as u64,
        OpKind::Immediate8to32 if size == 4 => instruction.immediate8to32() as u64,
        OpKind::Immediate8to64 if size == 8 => instruction.immediate8to64() as u64,
        OpKind::Immediate32to64 if size == 8 => instruction.immediate32to64() as u64,
        _ => return Err(NativeDispatchError::UnsupportedSegmentInstruction),
    };
    Ok(value & width_mask(size))
}

pub(super) fn calculate(
    mnemonic: Mnemonic,
    left: u64,
    right: u64,
    size: usize,
    old: u64,
) -> Result<(u64, u64), NativeDispatchError> {
    let mask = width_mask(size);
    let sign = 1_u64 << (size * 8 - 1);
    let left = left & mask;
    let right = right & mask;
    let carry = u64::from(old & CF != 0);
    let (result, bits, affected) = match mnemonic {
        Mnemonic::And | Mnemonic::Or | Mnemonic::Xor | Mnemonic::Test => {
            let value = match mnemonic {
                Mnemonic::Or => left | right,
                Mnemonic::Xor => left ^ right,
                _ => left & right,
            };
            (value, result_flags(value, sign), STATUS & !AF)
        }
        Mnemonic::Not => (!left & mask, 0, 0),
        Mnemonic::Add | Mnemonic::Adc | Mnemonic::Inc => {
            let rhs = if mnemonic == Mnemonic::Inc { 1 } else { right };
            let incoming = if mnemonic == Mnemonic::Adc { carry } else { 0 };
            let wide = u128::from(left) + u128::from(rhs) + u128::from(incoming);
            let value = wide as u64 & mask;
            let bits = result_flags(value, sign)
                | if wide > u128::from(mask) { CF } else { 0 }
                | if (!(left ^ rhs) & (left ^ value) & sign) != 0 {
                    OF
                } else {
                    0
                }
                | if (left ^ rhs ^ value) & 0x10 != 0 {
                    AF
                } else {
                    0
                };
            (
                value,
                bits,
                if mnemonic == Mnemonic::Inc {
                    STATUS & !CF
                } else {
                    STATUS
                },
            )
        }
        Mnemonic::Sub | Mnemonic::Sbb | Mnemonic::Cmp | Mnemonic::Dec | Mnemonic::Neg => {
            let (lhs, rhs) = if mnemonic == Mnemonic::Neg {
                (0, left)
            } else if mnemonic == Mnemonic::Dec {
                (left, 1)
            } else {
                (left, right)
            };
            let incoming = if mnemonic == Mnemonic::Sbb { carry } else { 0 };
            let value = lhs.wrapping_sub(rhs).wrapping_sub(incoming) & mask;
            let bits = result_flags(value, sign)
                | if u128::from(lhs) < u128::from(rhs) + u128::from(incoming) {
                    CF
                } else {
                    0
                }
                | if ((lhs ^ rhs) & (lhs ^ value) & sign) != 0 {
                    OF
                } else {
                    0
                }
                | if (lhs ^ rhs ^ value) & 0x10 != 0 {
                    AF
                } else {
                    0
                };
            (
                value,
                bits,
                if mnemonic == Mnemonic::Dec {
                    STATUS & !CF
                } else {
                    STATUS
                },
            )
        }
        _ => return Err(NativeDispatchError::UnsupportedSegmentInstruction),
    };
    Ok((result, (old & !affected) | (bits & affected)))
}
fn result_flags(value: u64, sign: u64) -> u64 {
    (if value == 0 { ZF } else { 0 })
        | (if value & sign != 0 { SF } else { 0 })
        | (if (value as u8).count_ones().is_multiple_of(2) {
            PF
        } else {
            0
        })
}
