// SPDX-License-Identifier: GPL-2.0-only
//! Eight checked x87 conditional register transfers over retained physical bits.
//! AMD APM Volume 5 Rev. 3.13 pp. 233-234: both inputs must be nonempty,
//! including when the condition is false. Normal C1 and all RFLAGS remain intact.
//! Transfers do not validate, quiet or otherwise change extended payloads.
#![forbid(unsafe_code)]
use super::x87_data::{pending, raise, register, set_tag, tag, top};
use super::{NativeCpuState, NativeDispatchError};
use crate::native_memory::PausedNativeMemory;
use pentax_codegen::{Code, Instruction};
fn condition(code: Code, flags: u64) -> Option<bool> {
    let carry = flags & 1 != 0;
    let zero = flags & 0x40 != 0;
    let parity = flags & 4 != 0;
    Some(match code {
        Code::Fcmovb_st0_sti => carry,
        Code::Fcmove_st0_sti => zero,
        Code::Fcmovbe_st0_sti => carry || zero,
        Code::Fcmovu_st0_sti => parity,
        Code::Fcmovnb_st0_sti => !carry,
        Code::Fcmovne_st0_sti => !zero,
        Code::Fcmovnbe_st0_sti => !carry && !zero,
        Code::Fcmovnu_st0_sti => !parity,
        _ => return None,
    })
}
pub(super) fn supported(code: Code) -> bool {
    condition(code, 0).is_some()
}
fn destination_tag(value: &[u8; 10]) -> u16 {
    let sig = u64::from_le_bytes(value[..8].try_into().unwrap());
    let exp = u16::from_le_bytes(value[8..].try_into().unwrap()) & 0x7fff;
    // Hardware can retain only empty/full internally and reconstruct full tags
    // on save. Classify a taken destination without validating or changing bits.
    if exp == 0 && sig == 0 {
        1
    } else if exp == 0 || exp == 0x7fff || sig >> 63 == 0 {
        2
    } else {
        0
    }
}
pub(super) fn execute(
    instruction: &Instruction,
    next: &mut NativeCpuState,
    memory: &mut PausedNativeMemory<'_>,
) -> Result<u64, NativeDispatchError> {
    let selected = condition(instruction.code(), next.rflags)
        .ok_or(NativeDispatchError::UnsupportedVectorInstruction)?;
    if pending(&next.x87) {
        return Err(NativeDispatchError::UnsupportedX87Exception);
    }
    if instruction.op_count() != 2 || register(instruction, 0)? != 0 {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    let old_top = top(&next.x87);
    let source = (old_top + register(instruction, 1)?) & 7;
    let mut encoded = [0; 15];
    memory.fetch_source(instruction.ip(), &mut encoded[..instruction.len()])?;
    let first = encoded[..instruction.len()]
        .iter()
        .position(|b| (0xd8..=0xdf).contains(b))
        .ok_or(NativeDispatchError::UnsupportedVectorInstruction)?;
    let second = *encoded[..instruction.len()]
        .get(first + 1)
        .ok_or(NativeDispatchError::UnsupportedVectorInstruction)?;
    let opcode = (u16::from(encoded[first] & 7) << 8) | u16::from(second);
    let mut state = next.x87;
    // Stack availability is checked independently of the CPU condition. In the
    // masked case only ST0 receives an indefinite; the source and TOP stay put.
    if tag(&state, old_top) == 3 || tag(&state, source) == 3 {
        raise(&mut state, 1, Some(false))?;
        state.registers[old_top] = [0, 0, 0, 0, 0, 0, 0, 0xc0, 0xff, 0xff];
        set_tag(&mut state, old_top, 2);
    } else if selected {
        let value = state.registers[source];
        state.registers[old_top] = value;
        set_tag(&mut state, old_top, destination_tag(&value));
    }
    // All fallible source/operand/exception checks precede publication. Register
    // FDP is implementation-dependent and retains its previous guest value.
    state.instruction_pointer = instruction.ip();
    state.instruction_opcode = opcode;
    next.x87 = state;
    Ok(0)
}
