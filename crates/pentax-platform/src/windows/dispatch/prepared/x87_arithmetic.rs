// SPDX-License-Identifier: GPL-2.0-only
//! Checked basic x87 arithmetic over owned physical guest state. Host scratch
//! computes only validated local canonical80 numbers, with every host exception
//! masked; guest stack, exception priority and publication remain software-owned.
//! Target NaN/exception priority: AMD64 APM Vol1 6.3 Table6-7 and Table6-19:
//! https://docs.amd.com/api/khub/documents/sfvvekC9mDflu6vd3R0NXA/content
use super::x87_data::{
    canonical, classify, pending, pop, raise, register, set_tag, tag, top, widen,
};
use super::{NativeCpuState, NativeDispatchError};
use crate::native_memory::PausedNativeMemory;
use pentax_codegen::{Code, Instruction, OpKind};
#[path = "x87_arithmetic/host.rs"]
mod host;

const INDEFINITE: [u8; 10] = [0, 0, 0, 0, 0, 0, 0, 0xc0, 0xff, 0xff];
const C1: u16 = 0x0200;
#[derive(Clone, Copy)]
enum Arithmetic {
    Add,
    Subtract,
    Multiply,
    Divide,
}
#[derive(Clone, Copy)]
struct Form {
    arithmetic: Arithmetic,
    reverse: bool,
    popping: bool,
    memory: usize,
}
fn form(code: Code) -> Option<Form> {
    use Arithmetic::*;
    use Code::*;
    let (arithmetic, reverse, popping, memory) = match code {
        Fadd_st0_sti | Fadd_sti_st0 => (Add, false, false, 0),
        Faddp_sti_st0 => (Add, false, true, 0),
        Fadd_m32fp => (Add, false, false, 4),
        Fadd_m64fp => (Add, false, false, 8),
        Fsub_st0_sti | Fsub_sti_st0 => (Subtract, false, false, 0),
        Fsubp_sti_st0 => (Subtract, false, true, 0),
        Fsub_m32fp => (Subtract, false, false, 4),
        Fsub_m64fp => (Subtract, false, false, 8),
        Fsubr_st0_sti | Fsubr_sti_st0 => (Subtract, true, false, 0),
        Fsubrp_sti_st0 => (Subtract, true, true, 0),
        Fsubr_m32fp => (Subtract, true, false, 4),
        Fsubr_m64fp => (Subtract, true, false, 8),
        Fmul_st0_sti | Fmul_sti_st0 => (Multiply, false, false, 0),
        Fmulp_sti_st0 => (Multiply, false, true, 0),
        Fmul_m32fp => (Multiply, false, false, 4),
        Fmul_m64fp => (Multiply, false, false, 8),
        Fdiv_st0_sti | Fdiv_sti_st0 => (Divide, false, false, 0),
        Fdivp_sti_st0 => (Divide, false, true, 0),
        Fdiv_m32fp => (Divide, false, false, 4),
        Fdiv_m64fp => (Divide, false, false, 8),
        Fdivr_st0_sti | Fdivr_sti_st0 => (Divide, true, false, 0),
        Fdivrp_sti_st0 => (Divide, true, true, 0),
        Fdivr_m32fp => (Divide, true, false, 4),
        Fdivr_m64fp => (Divide, true, false, 8),
        _ => return None,
    };
    Some(Form {
        arithmetic,
        reverse,
        popping,
        memory,
    })
}
pub(super) fn supported(code: Code) -> bool {
    form(code).is_some()
}
fn components(value: &[u8; 10]) -> (u64, u16) {
    (
        u64::from_le_bytes(value[..8].try_into().unwrap()),
        u16::from_le_bytes(value[8..].try_into().unwrap()) & 0x7fff,
    )
}
fn nan(value: &[u8; 10]) -> bool {
    let (sig, exp) = components(value);
    exp == 0x7fff && sig & 0x7fff_ffff_ffff_ffff != 0
}
fn denormal(value: &[u8; 10]) -> bool {
    let (sig, exp) = components(value);
    exp == 0 && sig != 0 && sig >> 63 == 0
}
fn memory_signaling(bytes: &[u8]) -> bool {
    if bytes.len() == 4 {
        let bits = u32::from_le_bytes(bytes.try_into().unwrap());
        bits & 0x7f80_0000 == 0x7f80_0000 && bits & 0x007f_ffff != 0 && bits & 0x0040_0000 == 0
    } else {
        let bits = u64::from_le_bytes(bytes.try_into().unwrap());
        bits & 0x7ff0_0000_0000_0000 == 0x7ff0_0000_0000_0000
            && bits & 0x000f_ffff_ffff_ffff != 0
            && bits & 0x0008_0000_0000_0000 == 0
    }
}
fn quiet_result(
    left: &[u8; 10],
    right: &[u8; 10],
    right_signaling: bool,
) -> Result<Option<[u8; 10]>, NativeDispatchError> {
    // AMD Vol1 6.3 Table6-7: a QNaN wins over an SNaN; two QNaNs
    // select the larger significand. Equal payloads with opposite signs have
    // undefined selection, kept as an explicit boundary rather than guessed.
    match (nan(left), nan(right)) {
        (false, false) => Ok(None),
        (true, false) => Ok(Some(*left)),
        (false, true) => Ok(Some(*right)),
        (true, true) if right_signaling => Ok(Some(*left)),
        (true, true) => {
            let l = components(left).0;
            let r = components(right).0;
            if l == r && left[9] != right[9] {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            Ok(Some(if l >= r { *left } else { *right }))
        }
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
    if next.x87.control_word & 0x0300 == 0x0100 {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    if !host::available() {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    let mut state = next.x87;
    let old_top = top(&state);
    let mut raw = [0; 8];
    let (destination, source_slot, address) = if form.memory != 0 {
        if instruction.op_count() != 1
            || instruction.op0_kind() != OpKind::Memory
            || instruction.memory_size().size() != form.memory
        {
            return Err(NativeDispatchError::UnsupportedVectorInstruction);
        }
        used_bytes
            .checked_add(form.memory as u64)
            .ok_or(NativeDispatchError::Budget("vector bytes"))?;
        let address = super::vectors::address(instruction, 0, next)?;
        memory.read(address, &mut raw[..form.memory])?;
        (old_top, None, Some(address))
    } else {
        if instruction.op_count() != 2 {
            return Err(NativeDispatchError::UnsupportedVectorInstruction);
        }
        let destination = (old_top + register(instruction, 0)?) & 7;
        let source = (old_top + register(instruction, 1)?) & 7;
        (destination, Some(source), None)
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
        tag(&state, destination) == 3 || source_slot.is_some_and(|slot| tag(&state, slot) == 3);
    state.status_word &= !C1;
    let result = if missing {
        // Stack underflow precedes conversion, NaNs and all arithmetic faults.
        raise(&mut state, 1, Some(false))?;
        INDEFINITE
    } else {
        let left = state.registers[destination];
        canonical(&left)?;
        let mut conversion_flags = 0;
        let right = if let Some(slot) = source_slot {
            let value = state.registers[slot];
            canonical(&value)?;
            value
        } else {
            // Conversion is staged with masks enabled only to classify the raw
            // source. Real guest mask/priority handling follows both operands;
            // a QNaN suppresses DE, and an SNaN keeps its original identity.
            let mut conversion = state;
            conversion.control_word |= 0x3f;
            conversion.status_word = 0;
            let value = widen(&raw[..form.memory], &mut conversion)?;
            conversion_flags = conversion.status_word & 3;
            value
        };
        let signaling = form.memory != 0 && memory_signaling(&raw[..form.memory]);
        if let Some(result) = quiet_result(&left, &right, signaling)? {
            raise(&mut state, u16::from(signaling), None)?;
            result
        } else {
            let (a, b) = if form.reverse {
                (&right, &left)
            } else {
                (&left, &right)
            };
            let (result, status) = host::compute(form.arithmetic, a, b, state.control_word)?;
            canonical(&result)?;
            // Raw-source DE must survive widening, but QNaN/invalid/divide-zero
            // decisions have higher target priority. Never trust host vendor
            // operand priority or NaN payload selection as the target policy.
            let mut flags = status & 0x3d;
            if flags & (1 | 4) == 0
                && (conversion_flags & 2 != 0 || denormal(&left) || denormal(&right))
            {
                flags |= 2;
            }
            raise(&mut state, flags, None)?;
            // With fresh PE clear the canonical stored result is mathematically
            // exact. A nonzero true80 denormal is tiny at every legal guest PC;
            // unmasked UE traps even when masked hardware reports no UE.
            if state.control_word & 0x10 == 0
                && status & 0x20 == 0
                && flags & (1 | 4) == 0
                && denormal(&result)
            {
                return Err(NativeDispatchError::UnsupportedX87Exception);
            }
            state.status_word = (state.status_word & !C1) | (status & C1);
            result
        }
    };
    state.registers[destination] = result;
    set_tag(&mut state, destination, classify(&result));
    if form.popping {
        pop(&mut state);
    }
    state.instruction_pointer = instruction.ip();
    state.instruction_opcode = opcode;
    if let Some(address) = address {
        state.data_pointer = address;
    }
    next.x87 = state;
    Ok(form.memory as u64)
}

#[cfg(test)]
pub(super) fn host_available() -> bool {
    host::available()
}
