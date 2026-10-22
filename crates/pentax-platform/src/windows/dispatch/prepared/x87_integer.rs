// SPDX-License-Identifier: GPL-2.0-only
//! Exact signed integer conversions over retained x87 physical state.
//! AMD APM Volume 5, Rev. 3.13, FILD pp. 253-254, FIST/P pp. 259-261,
//! FISTTP pp. 262-263. Integer stores raise IE/SF/PE, never DE; PC does
//! not pre-round. FISTTP is a checked integer-only software exit, independent
//! of native SSE3 admission. CPUID advertises a conservative bounded subset;
//! it is not a configurable disabled-guest-SSE3 contract. No host SSE3 code runs. Unsupported extended encodings remain explicit refusals.
#![forbid(unsafe_code)]
use super::x87_data::{pending, pop, push, raise, tag, top};
use super::{NativeCpuState, NativeDispatchError};
use crate::native_memory::PausedNativeMemory;
use pentax_codegen::{Code, Instruction, OpKind};

#[derive(Clone, Copy)]
struct Form {
    size: usize,
    load: bool,
    popping: bool,
    truncate: bool,
}
fn form(code: Code) -> Option<Form> {
    use Code::*;
    let (size, load, popping, truncate) = match code {
        Fild_m16int => (2, true, false, false),
        Fild_m32int => (4, true, false, false),
        Fild_m64int => (8, true, false, false),
        Fist_m16int => (2, false, false, false),
        Fist_m32int => (4, false, false, false),
        Fistp_m16int => (2, false, true, false),
        Fistp_m32int => (4, false, true, false),
        Fistp_m64int => (8, false, true, false),
        Fisttp_m16int => (2, false, true, true),
        Fisttp_m32int => (4, false, true, true),
        Fisttp_m64int => (8, false, true, true),
        _ => return None,
    };
    Some(Form {
        size,
        load,
        popping,
        truncate,
    })
}
pub(super) fn supported(code: Code) -> bool {
    form(code).is_some()
}
fn extended(integer: i64) -> [u8; 10] {
    let magnitude = integer.unsigned_abs();
    let mut bytes = [0; 10];
    if magnitude != 0 {
        let highest = 63 - magnitude.leading_zeros();
        let significand = magnitude << (63 - highest);
        let exponent = (16383 + highest as u16) | if integer < 0 { 0x8000 } else { 0 };
        bytes[..8].copy_from_slice(&significand.to_le_bytes());
        bytes[8..].copy_from_slice(&exponent.to_le_bytes());
    }
    bytes
}
// None denotes architectural invalid conversion. Unreviewed pseudo/unnormal
// encodings instead retain an explicit typed refusal, without changing state.
fn convert(
    value: &[u8; 10],
    size: usize,
    rounding: u16,
) -> Result<Option<(u64, bool, bool)>, NativeDispatchError> {
    let sig = u64::from_le_bytes(value[..8].try_into().unwrap());
    let field = u16::from_le_bytes(value[8..].try_into().unwrap());
    let exponent = field & 0x7fff;
    let negative = field & 0x8000 != 0;
    let integer_bit = sig >> 63 != 0;
    if exponent == 0 && integer_bit || exponent != 0 && !integer_bit {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    if exponent == 0x7fff {
        // Both NaN classes and infinity produce the same integer invalid result.
        return Ok(None);
    }
    let power = i32::from(exponent.max(1)) - 16383;
    if power > 63 {
        return Ok(None);
    }
    let (magnitude, remainder, half) = if power >= 0 {
        let shift = (63 - power) as u32;
        if shift == 0 {
            (u128::from(sig), 0, 0)
        } else {
            (
                u128::from(sig >> shift),
                sig & ((1_u64 << shift) - 1),
                1_u64 << (shift - 1),
            )
        }
    } else if power == -1 {
        (0, sig, 1_u64 << 63)
    } else {
        // Every nonzero value here is strictly below one half. A sentinel half
        // greater than remainder avoids any shift by 64 or exponent-sized loop.
        (0, u64::from(sig != 0), 2)
    };
    let inexact = remainder != 0;
    let increment = inexact
        && match rounding {
            0 => remainder > half || remainder == half && magnitude & 1 != 0,
            1 => negative,
            2 => !negative,
            _ => false,
        };
    let rounded = magnitude + u128::from(increment);
    let limit = 1_u128 << (size * 8 - 1);
    if rounded > limit || !negative && rounded == limit {
        return Ok(None);
    }
    let result = rounded as u64;
    Ok(Some((
        if negative {
            result.wrapping_neg()
        } else {
            result
        },
        inexact,
        increment,
    )))
}
pub(super) fn execute(
    instruction: &Instruction,
    next: &mut NativeCpuState,
    memory: &mut PausedNativeMemory<'_>,
    used_bytes: u64,
) -> Result<u64, NativeDispatchError> {
    let form = form(instruction.code()).ok_or(NativeDispatchError::UnsupportedVectorInstruction)?;
    if pending(&next.x87) {
        return Err(NativeDispatchError::UnsupportedX87Exception);
    }
    if instruction.op_count() != 1
        || instruction.op0_kind() != OpKind::Memory
        || instruction.memory_size().size() != form.size
    {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    let transferred = form.size as u64;
    used_bytes
        .checked_add(transferred)
        .ok_or(NativeDispatchError::Budget("vector bytes"))?;
    let address = super::vectors::address(instruction, 0, next)?;
    let mut input = [0; 8];
    if form.load {
        memory.read(address, &mut input[..form.size])?;
    } else {
        memory.preflight_write(address, form.size)?;
    }
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
    let mut state = next.x87;
    state.status_word &= !0x0200;
    let mut output = None;
    if form.load {
        let integer = match form.size {
            2 => i64::from(i16::from_le_bytes(input[..2].try_into().unwrap())),
            4 => i64::from(i32::from_le_bytes(input[..4].try_into().unwrap())),
            _ => i64::from_le_bytes(input),
        };
        push(
            &mut state,
            extended(integer),
            if integer == 0 { 1 } else { 0 },
            false,
        )?;
    } else {
        let slot = top(&state);
        let indefinite = 1_u64 << (form.size * 8 - 1);
        let integer = if tag(&state, slot) == 3 {
            raise(&mut state, 1, Some(false))?;
            indefinite
        } else {
            let rc = if form.truncate {
                3
            } else {
                (state.control_word >> 10) & 3
            };
            match convert(&state.registers[slot], form.size, rc)? {
                None => {
                    // Invalid dominates precision, including a fractional value
                    // whose rounded integer is out of range. Sticky PE survives.
                    raise(&mut state, 1, None)?;
                    indefinite
                }
                Some((integer, inexact, increment)) => {
                    if inexact {
                        raise(&mut state, 0x20, None)?;
                    }
                    if increment && !form.truncate {
                        state.status_word |= 0x0200;
                    }
                    integer
                }
            }
        };
        output = Some(integer.to_le_bytes());
        if form.popping {
            pop(&mut state);
        }
    }
    // Sole external output after all admissions and exception decisions; a failed
    // write publishes neither status, TOP/tags, pointers nor partial CPU state.
    if let Some(output) = output {
        memory.write(address, &output[..form.size])?;
    }
    state.instruction_pointer = instruction.ip();
    state.instruction_opcode = opcode;
    state.data_pointer = address;
    next.x87 = state;
    Ok(transferred)
}
