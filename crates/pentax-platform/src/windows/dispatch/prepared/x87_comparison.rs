// SPDX-License-Identifier: GPL-2.0-only
//! Exact x87 comparisons over retained guest bits, without host floating point.
//! COMI flags follow corroborated AMD/Intel hardware observations correcting
//! ambiguous manual tables: OF/SF/AF clear; C1 remains unless stack underflow.
//! https://github.com/qemu/qemu/commit/0924d9d3db3617f49ba9282acc6d85b349404ce6
//! https://github.com/qemu/qemu/commit/6d5fb14d36a6d11e88f04878021b2d99ce6c07e9
//! Canonical80 SNaNs are classified for comparison only; other x87 families
//! retain their existing strict unsupported-encoding policies.
#![forbid(unsafe_code)]
use super::x87_data::{pending, pop, raise, register, tag, top, widen};
use super::{NativeCpuState, NativeDispatchError};
use crate::native_memory::PausedNativeMemory;
use pentax_codegen::{Code, Instruction, OpKind};
use std::cmp::Ordering;

#[derive(Clone, Copy)]
enum Source {
    Register,
    Memory(usize),
    Next,
    Zero,
}
#[derive(Clone, Copy)]
struct Form {
    source: Source,
    unordered: bool,
    eflags: bool,
    pops: usize,
}
fn form(code: Code) -> Option<Form> {
    use Code::*;
    let (source, unordered, eflags, pops) = match code {
        Fcom_m32fp => (Source::Memory(4), false, false, 0),
        Fcom_m64fp => (Source::Memory(8), false, false, 0),
        Fcom_st0_sti => (Source::Register, false, false, 0),
        Fcomp_m32fp => (Source::Memory(4), false, false, 1),
        Fcomp_m64fp => (Source::Memory(8), false, false, 1),
        Fcomp_st0_sti => (Source::Register, false, false, 1),
        Fcompp => (Source::Next, false, false, 2),
        Fucom_st0_sti => (Source::Register, true, false, 0),
        Fucomp_st0_sti => (Source::Register, true, false, 1),
        Fucompp => (Source::Next, true, false, 2),
        Fcomi_st0_sti => (Source::Register, false, true, 0),
        Fcomip_st0_sti => (Source::Register, false, true, 1),
        Fucomi_st0_sti => (Source::Register, true, true, 0),
        Fucomip_st0_sti => (Source::Register, true, true, 1),
        Ftst => (Source::Zero, false, false, 0),
        _ => return None,
    };
    Some(Form {
        source,
        unordered,
        eflags,
        pops,
    })
}
pub(super) fn supported(code: Code) -> bool {
    form(code).is_some()
}
#[derive(Clone, Copy)]
struct Value {
    significand: u64,
    exponent: u16,
    negative: bool,
    nan: bool,
    signaling: bool,
    denormal: bool,
}
fn inspect(bytes: &[u8; 10]) -> Result<Value, NativeDispatchError> {
    let significand = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    let field = u16::from_le_bytes(bytes[8..].try_into().unwrap());
    let exponent = field & 0x7fff;
    let integer = significand >> 63 != 0;
    if exponent == 0 && integer || exponent != 0 && !integer {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    let nan = exponent == 0x7fff && significand & 0x7fff_ffff_ffff_ffff != 0;
    Ok(Value {
        significand,
        exponent,
        negative: field & 0x8000 != 0,
        nan,
        signaling: nan && significand & (1 << 62) == 0,
        denormal: exponent == 0 && significand != 0,
    })
}
fn compare(left: Value, right: Value) -> Ordering {
    if left.significand == 0 && left.exponent == 0 && right.significand == 0 && right.exponent == 0
    {
        return Ordering::Equal;
    }
    if left.negative != right.negative {
        return if left.negative {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    let magnitude = (left.exponent, left.significand).cmp(&(right.exponent, right.significand));
    if left.negative {
        magnitude.reverse()
    } else {
        magnitude
    }
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
    let mut state = next.x87;
    let old_top = top(&state);
    let mut raw = [0; 8];
    let mut transferred = 0;
    let mut address = None;
    let source_slot = match form.source {
        Source::Register => {
            if instruction.op_count() != 2 || register(instruction, 0)? != 0 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            Some((old_top + register(instruction, 1)?) & 7)
        }
        Source::Memory(size) => {
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
            let data = super::vectors::address(instruction, 0, next)?;
            memory.read(data, &mut raw[..size])?;
            address = Some(data);
            None
        }
        Source::Next | Source::Zero => {
            if instruction.op_count() != 0 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            if matches!(form.source, Source::Next) {
                Some((old_top + 1) & 7)
            } else {
                None
            }
        }
    };
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
    let missing =
        tag(&state, old_top) == 3 || source_slot.is_some_and(|slot| tag(&state, slot) == 3);
    if !form.eflags {
        state.status_word &= !0x0200;
    }
    let relation = if missing {
        // Masked stack underflow reports unordered and performs the specified
        // pops without overwriting any physical payload with an indefinite.
        raise(&mut state, 1, Some(false))?;
        None
    } else {
        let left = inspect(&state.registers[old_top])?;
        let mut conversion_flags = 0;
        let right = if let Some(slot) = source_slot {
            inspect(&state.registers[slot])?
        } else {
            let bytes = if let Source::Memory(size) = form.source {
                let mut conversion = state;
                conversion.control_word |= 0x3f;
                conversion.status_word = 0;
                let value = widen(&raw[..size], &mut conversion)?;
                conversion_flags = conversion.status_word & 3;
                value
            } else {
                [0; 10]
            };
            inspect(&bytes)?
        };
        if left.nan || right.nan {
            // NaNs have priority over DE. No input is quieted or rewritten.
            if !form.unordered || left.signaling || right.signaling || conversion_flags & 1 != 0 {
                raise(&mut state, 1, None)?;
            }
            None
        } else {
            if left.denormal || right.denormal || conversion_flags & 2 != 0 {
                raise(&mut state, 2, None)?;
            }
            Some(compare(left, right))
        }
    };
    let (zero, parity, carry) = match relation {
        Some(Ordering::Greater) => (false, false, false),
        Some(Ordering::Less) => (false, false, true),
        Some(Ordering::Equal) => (true, false, false),
        None => (true, true, true),
    };
    let mut flags = next.rflags;
    if form.eflags {
        // COMI clears OF/SF/AF while setting only ZF/PF/CF. Other RFLAGS
        // remain intact. Normal x87 C0/C1/C2/C3 are unaffected; a preceding
        // masked stack-underflow raise has already cleared C1 explicitly.
        flags =
            (flags & !0x8d5) | u64::from(carry) | (u64::from(parity) << 2) | (u64::from(zero) << 6);
    } else {
        state.status_word = (state.status_word & !0x4500)
            | (u16::from(carry) << 8)
            | (u16::from(parity) << 10)
            | (u16::from(zero) << 14);
    }
    for _ in 0..form.pops {
        pop(&mut state);
    }
    state.instruction_pointer = instruction.ip();
    state.instruction_opcode = opcode;
    if let Some(data) = address {
        state.data_pointer = data;
    }
    // Every fallible read, source check and guest exception precedes the joint
    // infallible publication of CPU flags, status/TOP/tags and guest pointers.
    next.x87 = state;
    next.rflags = flags;
    Ok(transferred)
}
