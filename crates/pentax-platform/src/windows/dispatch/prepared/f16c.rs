// SPDX-License-Identifier: GPL-2.0-only
//! Four VEX half/single conversions over owned copied bits, without host FP.
//! Intel SDM VCVTPS2PH/VCVTPH2PS define exact operand widths and zeroing.
//! Immediate bits 7:3 are ignored; bit2 selects MXCSR.RC rather than bits1:0.
//! The existing all-masked guest MXCSR profile remains mandatory.
//!
//! Original implementation from Intel SDM VCVTPS2PH/VCVTPH2PS and AMD APM
//! 26568 rev3.26 pp850-853; tininess follows AMD 24592 rev3.24 pp221-223.
//! https://docs.amd.com/v/u/en-US/26568_3.26_APM_Vol4
//! https://kib.kiev.ua/x86docs/AMD/AMD64/24592_APM_v1-r3.24.pdf

use super::{NativeCpuState, NativeDispatchError, PausedNativeMemory, mxcsr_control, vectors};
use pentax_codegen::{Code, Instruction, OpKind};

#[derive(Clone, Copy)]
struct Form {
    narrow: bool,
    lanes: usize,
}

fn form(code: Code) -> Option<Form> {
    use Code::*;
    let (narrow, lanes) = match code {
        VEX_Vcvtps2ph_xmmm64_xmm_imm8 => (true, 4),
        VEX_Vcvtps2ph_xmmm128_ymm_imm8 => (true, 8),
        VEX_Vcvtph2ps_xmm_xmmm64 => (false, 4),
        VEX_Vcvtph2ps_ymm_xmmm128 => (false, 8),
        _ => return None,
    };
    Some(Form { narrow, lanes })
}

pub(super) fn supported(code: Code) -> bool {
    form(code).is_some()
}

pub(super) fn execute(
    instruction: &Instruction,
    state: &mut NativeCpuState,
    memory: &mut PausedNativeMemory<'_>,
    prior_bytes: u64,
) -> Result<u64, NativeDispatchError> {
    let form = form(instruction.code()).ok_or(NativeDispatchError::UnsupportedVectorInstruction)?;
    if instruction.op_count() != if form.narrow { 3 } else { 2 }
        || form.narrow && instruction.op2_kind() != OpKind::Immediate8
    {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    if !mxcsr_control::valid(state.mxcsr) {
        return Err(NativeDispatchError::InvalidState);
    }
    let input_size = form.lanes * if form.narrow { 4 } else { 2 };
    let output_size = form.lanes * if form.narrow { 2 } else { 4 };
    let memory_operand = if form.narrow { 0 } else { 1 };
    let memory_access = instruction.op_kind(memory_operand) == OpKind::Memory;
    let transferred = if memory_access {
        let size = if form.narrow { output_size } else { input_size };
        if instruction.memory_size().size() != size {
            return Err(NativeDispatchError::UnsupportedVectorInstruction);
        }
        prior_bytes
            .checked_add(size as u64)
            .ok_or(NativeDispatchError::Budget("vector bytes"))?;
        size as u64
    } else {
        0
    };
    // Resolve every operand before publication, including an aliased XMM source.
    let destination = if form.narrow && memory_access {
        None
    } else {
        Some(vectors::register_slot(
            instruction,
            0,
            if form.narrow { 16 } else { output_size },
        )?)
    };
    let address = if memory_access {
        Some(vectors::address(instruction, memory_operand, state)?)
    } else {
        None
    };
    let input = if !form.narrow && memory_access {
        let mut bytes = [0; 32];
        memory.read(address.unwrap(), &mut bytes[..input_size])?;
        bytes
    } else {
        vectors::vector_register(instruction, 1, input_size.max(16), state)?
    };
    let rounding = if form.narrow && instruction.immediate8() & 4 == 0 {
        u32::from(instruction.immediate8() & 3)
    } else {
        state.mxcsr >> 13 & 3
    };
    let (output, exceptions) = convert(form, &input, rounding, state.mxcsr);
    if let Some(slot) = destination {
        state.xmm[slot..slot + 16].copy_from_slice(&output[..16]);
        state.ymm_upper[slot..slot + 16].copy_from_slice(&output[16..]);
    } else {
        // A single whole-span owned write precedes all CPU/control publication.
        memory.write(address.unwrap(), &output[..output_size])?;
    }
    state.mxcsr |= exceptions;
    Ok(transferred)
}

fn convert(form: Form, input: &[u8; 32], rounding: u32, mxcsr: u32) -> ([u8; 32], u32) {
    let mut output = [0; 32];
    let mut exceptions = 0;
    for lane in 0..form.lanes {
        if form.narrow {
            let bits = u32::from_le_bytes(input[lane * 4..lane * 4 + 4].try_into().unwrap());
            let (bits, flags) = narrow(bits, rounding, mxcsr);
            output[lane * 2..lane * 2 + 2].copy_from_slice(&bits.to_le_bytes());
            exceptions |= flags;
        } else {
            let bits = u16::from_le_bytes(input[lane * 2..lane * 2 + 2].try_into().unwrap());
            let (bits, flags) = widen(bits);
            output[lane * 4..lane * 4 + 4].copy_from_slice(&bits.to_le_bytes());
            exceptions |= flags;
        }
    }
    (output, exceptions)
}

fn widen(bits: u16) -> (u32, u32) {
    let sign = u32::from(bits & 0x8000) << 16;
    let exponent = (bits >> 10) & 31;
    let fraction = u32::from(bits & 1023);
    if exponent == 31 {
        let invalid = u32::from(fraction != 0 && fraction & 512 == 0);
        let quiet = if fraction != 0 { 0x0040_0000 } else { 0 };
        return (sign | 0x7f80_0000 | fraction << 13 | quiet, invalid);
    }
    if exponent != 0 {
        return (sign | u32::from(exponent + 112) << 23 | fraction << 13, 0);
    }
    if fraction == 0 {
        return (sign, 0);
    }
    // Every half subnormal is an exactly representable normal binary32 value.
    let leading = 31 - fraction.leading_zeros();
    (
        sign | (leading + 103) << 23 | ((fraction << (23 - leading)) & 0x007f_ffff),
        0,
    )
}

fn rounded(significand: u32, shift: u32, negative: bool, rounding: u32) -> (u32, bool) {
    let (mut retained, inexact, above_half, tie) = if shift >= 32 {
        (0, significand != 0, false, false)
    } else {
        let remainder = significand & ((1_u32 << shift) - 1);
        let half = 1_u32 << (shift - 1);
        (
            significand >> shift,
            remainder != 0,
            remainder > half,
            remainder == half,
        )
    };
    let increment = match rounding & 3 {
        0 => above_half || tie && retained & 1 != 0,
        1 => negative && inexact,
        2 => !negative && inexact,
        _ => false,
    };
    retained += u32::from(increment);
    (retained, inexact)
}

fn narrow(bits: u32, rounding: u32, mxcsr: u32) -> (u16, u32) {
    let sign = (bits >> 16) as u16 & 0x8000;
    let negative = sign != 0;
    let exponent = bits >> 23 & 255;
    let fraction = bits & 0x007f_ffff;
    if exponent == 255 {
        if fraction == 0 {
            return (sign | 0x7c00, 0);
        }
        return (
            sign | 0x7e00 | (fraction >> 13) as u16,
            u32::from(fraction & 0x0040_0000 == 0),
        );
    }
    if exponent == 0 && fraction == 0 {
        return (sign, 0);
    }
    if exponent == 0 && mxcsr & 0x40 != 0 {
        return (sign, 0);
    }
    let denormal = if exponent == 0 { 2 } else { 0 };
    let exponent = if exponent == 0 {
        -126
    } else {
        exponent as i32 - 127
    };
    let significand = fraction | if bits & 0x7f80_0000 != 0 { 1 << 23 } else { 0 };
    if exponent < -14 {
        let (rounded, inexact) = rounded(significand, (-exponent - 1) as u32, negative, rounding);
        // AMD's Result_round probe rounds once to destination precision with
        // an unbounded exponent. Use it ONLY for tininess, never as the input
        // to the final subnormal quantization (which would double-round).
        let tiny = exponent < -15 || self::rounded(significand, 13, negative, rounding).0 < 2048;
        return (
            sign | rounded as u16,
            denormal
                | if inexact {
                    0x20 | u32::from(tiny) << 4
                } else {
                    0
                },
        );
    }
    let (mut significand, inexact) = rounded(significand, 13, negative, rounding);
    let mut exponent = exponent;
    if significand == 2048 {
        significand >>= 1;
        exponent += 1;
    }
    if exponent > 15 {
        let infinite = match rounding & 3 {
            0 => true,
            1 => negative,
            2 => !negative,
            _ => false,
        };
        return (sign | if infinite { 0x7c00 } else { 0x7bff }, 0x28);
    }
    (
        sign | ((exponent + 15) as u16) << 10 | (significand as u16 & 1023),
        u32::from(inexact) << 5,
    )
}

#[cfg(test)]
#[path = "f16c/tests.rs"]
mod tests;
