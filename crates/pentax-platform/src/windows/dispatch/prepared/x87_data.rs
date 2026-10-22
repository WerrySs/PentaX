// SPDX-License-Identifier: GPL-2.0-only
//! Exact integer-only x87 loads and stack transfers over retained physical state.
//! Narrowing stores and disputed extended SNaN/pseudo/unnormal
//! payloads remain explicit stops. No guest state is installed on the host FPU.
use super::{NativeCpuState, NativeDispatchError, NativeX87State};
use crate::native_memory::PausedNativeMemory;
use pentax_codegen::{Code, Instruction, OpKind, Register};

const C1: u16 = 0x0200;
const INDEFINITE: [u8; 10] = [0, 0, 0, 0, 0, 0, 0, 0xc0, 0xff, 0xff];

#[derive(Clone, Copy)]
enum Operation {
    Load(usize),
    LoadRegister,
    StoreRegister(bool),
    Store80,
    Exchange,
    Free,
    Rotate(bool),
    Constant(bool),
    Sign(bool),
}
fn form(code: Code) -> Option<Operation> {
    use Code::*;
    Some(match code {
        Fld_m32fp => Operation::Load(4),
        Fld_m64fp => Operation::Load(8),
        Fld_m80fp => Operation::Load(10),
        Fld_sti => Operation::LoadRegister,
        Fst_sti => Operation::StoreRegister(false),
        Fstp_sti => Operation::StoreRegister(true),
        Fstp_m80fp => Operation::Store80,
        Fxch_st0_sti => Operation::Exchange,
        Ffree_sti => Operation::Free,
        Fincstp => Operation::Rotate(true),
        Fdecstp => Operation::Rotate(false),
        Fld1 => Operation::Constant(true),
        Fldz => Operation::Constant(false),
        Fchs => Operation::Sign(true),
        Fabs => Operation::Sign(false),
        _ => return None,
    })
}
pub(super) fn supported(code: Code) -> bool {
    form(code).is_some()
}
pub(super) fn top(state: &NativeX87State) -> usize {
    usize::from((state.status_word >> 11) & 7)
}
fn set_top(state: &mut NativeX87State, slot: usize) {
    state.status_word = (state.status_word & !0x3800) | ((slot as u16) << 11);
}
pub(super) fn tag(state: &NativeX87State, slot: usize) -> u16 {
    (state.tag_word >> (slot * 2)) & 3
}
pub(super) fn set_tag(state: &mut NativeX87State, slot: usize, value: u16) {
    state.tag_word = (state.tag_word & !(3 << (slot * 2))) | (value << (slot * 2));
}
pub(super) fn pending(state: &NativeX87State) -> bool {
    state.status_word & !state.control_word & 0x3f != 0
}
pub(super) fn raise(
    state: &mut NativeX87State,
    flags: u16,
    stack: Option<bool>,
) -> Result<(), NativeDispatchError> {
    // Exception delivery is the same explicit unsupported boundary as WAIT.
    // Neither new unmasked flags nor partial stack/output effects are published.
    if flags & !state.control_word & 0x3f != 0 {
        return Err(NativeDispatchError::UnsupportedX87Exception);
    }
    state.status_word |= flags;
    if let Some(overflow) = stack {
        state.status_word = (state.status_word & !C1) | 0x40 | if overflow { C1 } else { 0 };
    }
    Ok(())
}
fn extended(significand: u64, sign_exponent: u16) -> [u8; 10] {
    let mut result = [0; 10];
    result[..8].copy_from_slice(&significand.to_le_bytes());
    result[8..].copy_from_slice(&sign_exponent.to_le_bytes());
    result
}
fn components(value: &[u8; 10]) -> (u64, u16) {
    (
        u64::from_le_bytes(value[..8].try_into().unwrap()),
        u16::from_le_bytes(value[8..].try_into().unwrap()) & 0x7fff,
    )
}
pub(super) fn canonical(value: &[u8; 10]) -> Result<(), NativeDispatchError> {
    let (significand, exponent) = components(value);
    let integer = significand >> 63 != 0;
    let fraction = significand & 0x7fff_ffff_ffff_ffff;
    if (exponent == 0 && integer)
        || (exponent != 0 && !integer)
        || (exponent == 0x7fff && fraction != 0 && significand & (1 << 62) == 0)
    {
        // AMD/Intel extended signaling and pseudo/unsupported transfer details
        // require a separate target policy. Never normalize or silently quiet.
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    Ok(())
}
pub(super) fn classify(value: &[u8; 10]) -> u16 {
    let (significand, exponent) = components(value);
    if exponent == 0 && significand == 0 {
        1
    } else if exponent == 0 || exponent == 0x7fff {
        2
    } else {
        0
    }
}
pub(super) fn widen(
    bytes: &[u8],
    state: &mut NativeX87State,
) -> Result<[u8; 10], NativeDispatchError> {
    let (bits, fraction_bits, exponent_bits, bias) = if bytes.len() == 4 {
        (
            u64::from(u32::from_le_bytes(bytes.try_into().unwrap())),
            23_u32,
            8_u32,
            127_i32,
        )
    } else {
        (u64::from_le_bytes(bytes.try_into().unwrap()), 52, 11, 1023)
    };
    let sign = ((bits >> (fraction_bits + exponent_bits)) as u16) << 15;
    let exponent_mask = (1_u64 << exponent_bits) - 1;
    let exponent = (bits >> fraction_bits) & exponent_mask;
    let fraction = bits & ((1_u64 << fraction_bits) - 1);
    if exponent == exponent_mask {
        let mut significand = (1_u64 << 63) | (fraction << (63 - fraction_bits));
        if fraction != 0 && fraction & (1_u64 << (fraction_bits - 1)) == 0 {
            raise(state, 1, None)?;
            significand |= 1_u64 << 62;
        }
        return Ok(extended(significand, sign | 0x7fff));
    }
    if exponent == 0 {
        if fraction == 0 {
            return Ok(extended(0, sign));
        }
        raise(state, 2, None)?;
        let highest = 63 - fraction.leading_zeros();
        let extended_exponent = 16383 + 1 - bias - fraction_bits as i32 + highest as i32;
        return Ok(extended(
            fraction << (63 - highest),
            sign | extended_exponent as u16,
        ));
    }
    Ok(extended(
        ((1_u64 << fraction_bits) | fraction) << (63 - fraction_bits),
        sign | (exponent as i32 - bias + 16383) as u16,
    ))
}
pub(super) fn register(
    instruction: &Instruction,
    operand: u32,
) -> Result<usize, NativeDispatchError> {
    let register = instruction.op_register(operand);
    if instruction.op_kind(operand) != OpKind::Register
        || !(Register::ST0..=Register::ST7).contains(&register)
    {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    Ok(register.number())
}
fn push(
    state: &mut NativeX87State,
    value: [u8; 10],
    value_tag: u16,
    missing: bool,
) -> Result<(), NativeDispatchError> {
    let destination = (top(state) + 7) & 7;
    let overflow = tag(state, destination) != 3;
    if overflow || missing {
        raise(state, 1, Some(overflow))?;
        state.registers[destination] = INDEFINITE;
        set_tag(state, destination, 2);
    } else {
        state.registers[destination] = value;
        set_tag(state, destination, value_tag);
    }
    set_top(state, destination);
    Ok(())
}
fn source(state: &mut NativeX87State, slot: usize) -> Result<([u8; 10], u16), NativeDispatchError> {
    if tag(state, slot) == 3 {
        raise(state, 1, Some(false))?;
        Ok((INDEFINITE, 2))
    } else {
        let value = state.registers[slot];
        canonical(&value)?;
        Ok((value, tag(state, slot)))
    }
}
pub(super) fn pop(state: &mut NativeX87State) {
    let old = top(state);
    set_tag(state, old, 3);
    set_top(state, (old + 1) & 7);
}

pub(super) fn execute(
    instruction: &Instruction,
    next: &mut NativeCpuState,
    memory: &mut PausedNativeMemory<'_>,
    used_bytes: u64,
) -> Result<u64, NativeDispatchError> {
    let operation =
        form(instruction.code()).ok_or(NativeDispatchError::UnsupportedVectorInstruction)?;
    if pending(&next.x87) {
        return Err(NativeDispatchError::UnsupportedX87Exception);
    }
    let mut state = next.x87;
    let old_top = top(&state);
    let mut transferred = 0;
    let mut memory_address = None;
    let mut input = [0; 10];
    let register_operand = match operation {
        Operation::Load(_) | Operation::Store80 => {
            let size = if let Operation::Load(size) = operation {
                size
            } else {
                10
            };
            if instruction.op_count() != 1
                || instruction.op0_kind() != OpKind::Memory
                || instruction.memory_size().size() != size
            {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            transferred = size as u64;
            used_bytes
                .checked_add(transferred)
                .ok_or(NativeDispatchError::Budget("vector bytes"))?;
            let address = super::vectors::address(instruction, 0, next)?;
            memory_address = Some(address);
            if matches!(operation, Operation::Store80) {
                memory.preflight_write(address, 10)?;
            } else {
                memory.read(address, &mut input[..size])?;
            }
            None
        }
        Operation::LoadRegister | Operation::StoreRegister(_) | Operation::Free => {
            if instruction.op_count() != 1 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            Some(register(instruction, 0)?)
        }
        Operation::Exchange => {
            if instruction.op_count() != 2 || register(instruction, 0)? != 0 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            Some(register(instruction, 1)?)
        }
        _ => {
            if instruction.op_count() != 0 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            None
        }
    };
    // Recover FOP from the actual admitted guest bytes, including its ModRM;
    // FIP names the original instruction start including any accepted prefix.
    let mut encoded = [0; 15];
    memory.fetch_source(instruction.ip(), &mut encoded[..instruction.len()])?;
    let first = encoded[..instruction.len()]
        .iter()
        .position(|byte| (0xd8..=0xdf).contains(byte))
        .ok_or(NativeDispatchError::UnsupportedVectorInstruction)?;
    let second = *encoded[..instruction.len()]
        .get(first + 1)
        .ok_or(NativeDispatchError::UnsupportedVectorInstruction)?;
    let opcode = (u16::from(encoded[first] & 7) << 8) | u16::from(second);
    if !matches!(operation, Operation::Free) {
        state.status_word &= !C1;
    }
    let mut output = None;
    match operation {
        Operation::Load(size) => {
            if tag(&state, (old_top + 7) & 7) != 3 {
                // Stack overflow has priority over operand conversion exceptions.
                push(&mut state, INDEFINITE, 2, false)?;
            } else {
                let value = if size == 10 {
                    canonical(&input)?;
                    input
                } else {
                    widen(&input[..size], &mut state)?
                };
                push(&mut state, value, classify(&value), false)?;
            }
        }
        Operation::LoadRegister => {
            let slot = (old_top + register_operand.unwrap()) & 7;
            let missing = tag(&state, slot) == 3;
            let value = state.registers[slot];
            if !missing && tag(&state, (old_top + 7) & 7) == 3 {
                canonical(&value)?;
            }
            let value_tag = tag(&state, slot);
            push(&mut state, value, value_tag, missing)?;
        }
        Operation::StoreRegister(popping) => {
            let destination = (old_top + register_operand.unwrap()) & 7;
            let (value, value_tag) = source(&mut state, old_top)?;
            state.registers[destination] = value;
            set_tag(&mut state, destination, value_tag);
            if popping {
                pop(&mut state);
            }
        }
        Operation::Store80 => {
            output = Some(source(&mut state, old_top)?.0);
            pop(&mut state);
        }
        Operation::Exchange => {
            let slot = (old_top + register_operand.unwrap()) & 7;
            // Masked underflow substitutes an indefinite for each empty input
            // before exchanging the complete value/tag pairs.
            let (left, left_tag) = source(&mut state, old_top)?;
            let (right, right_tag) = source(&mut state, slot)?;
            state.registers[old_top] = right;
            set_tag(&mut state, old_top, right_tag);
            state.registers[slot] = left;
            set_tag(&mut state, slot, left_tag);
        }
        Operation::Free => {
            set_tag(&mut state, (old_top + register_operand.unwrap()) & 7, 3);
        }
        Operation::Rotate(increment) => {
            set_top(&mut state, (old_top + if increment { 1 } else { 7 }) & 7);
        }
        Operation::Constant(one) => {
            let value = if one {
                extended(1 << 63, 0x3fff)
            } else {
                [0; 10]
            };
            push(&mut state, value, if one { 0 } else { 1 }, false)?;
        }
        Operation::Sign(negate) => {
            let missing = tag(&state, old_top) == 3;
            let (mut value, value_tag) = source(&mut state, old_top)?;
            if !missing {
                if negate {
                    value[9] ^= 0x80;
                } else {
                    value[9] &= 0x7f;
                }
            }
            state.registers[old_top] = value;
            set_tag(&mut state, old_top, value_tag);
        }
    }
    // All fallible admission and computation precedes the sole external write.
    // Successful output is followed only by infallible guest-state publication.
    if let Some(output) = output {
        memory.write(memory_address.unwrap(), &output)?;
    }
    state.instruction_pointer = instruction.ip();
    state.instruction_opcode = opcode;
    // Register-only FDP is implementation-dependent; retain its prior guest value.
    if let Some(address) = memory_address {
        state.data_pointer = address;
    }
    next.x87 = state;
    Ok(transferred)
}
