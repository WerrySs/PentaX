// SPDX-License-Identifier: GPL-2.0-only
//! Exact ROUND/VROUND forms over copied IEEE bits; no host FP instruction.
//! Intel SDM Vol.2 ROUNDPS/PD/SS/SD and Vol.1 section12.8.4 define the controls.

use super::NativeDispatchError;
use pentax_codegen::Code;

pub(super) struct Form {
    pub vector_bytes: usize,
    pub element_bytes: usize,
    pub scalar: bool,
    pub legacy: bool,
}
impl Form {
    pub fn source_operand(&self) -> u32 {
        if self.scalar && !self.legacy { 2 } else { 1 }
    }
}
pub(super) fn form(code: Code) -> Option<Form> {
    use Code::*;
    let (vector_bytes, element_bytes, scalar, legacy) = match code {
        Roundps_xmm_xmmm128_imm8 => (16, 4, false, true),
        Roundpd_xmm_xmmm128_imm8 => (16, 8, false, true),
        Roundss_xmm_xmmm32_imm8 => (16, 4, true, true),
        Roundsd_xmm_xmmm64_imm8 => (16, 8, true, true),
        VEX_Vroundps_xmm_xmmm128_imm8 => (16, 4, false, false),
        VEX_Vroundps_ymm_ymmm256_imm8 => (32, 4, false, false),
        VEX_Vroundpd_xmm_xmmm128_imm8 => (16, 8, false, false),
        VEX_Vroundpd_ymm_ymmm256_imm8 => (32, 8, false, false),
        VEX_Vroundss_xmm_xmm_xmmm32_imm8 => (16, 4, true, false),
        VEX_Vroundsd_xmm_xmm_xmmm64_imm8 => (16, 8, true, false),
        _ => return None,
    };
    Some(Form {
        vector_bytes,
        element_bytes,
        scalar,
        legacy,
    })
}

/// All guest exceptions stay masked under the existing execution profile.
/// Work is fixed and integer-only: no host rounding mode or MXCSR is installed.
pub(super) fn calculate(
    double: bool,
    input: u64,
    immediate: u8,
    control: u32,
) -> Result<(u64, u32), NativeDispatchError> {
    if !super::mxcsr_control::valid(control) {
        return Err(NativeDispatchError::InvalidState);
    }
    let (fraction_bits, exponent_bits, bias) = if double { (52, 11, 1023) } else { (23, 8, 127) };
    let input = if double { input } else { input & 0xffff_ffff };
    let sign_mask = 1_u64 << (fraction_bits + exponent_bits);
    let fraction_mask = (1_u64 << fraction_bits) - 1;
    let exponent_mask = (1_u64 << exponent_bits) - 1;
    let sign = input & sign_mask;
    let negative = sign != 0;
    let exponent = (input >> fraction_bits) & exponent_mask;
    let fraction = input & fraction_mask;
    if exponent == exponent_mask {
        let quiet = 1_u64 << (fraction_bits - 1);
        return if fraction != 0 && fraction & quiet == 0 {
            Ok((input | quiet, control | 1)) // sNaN: preserve payload/sign and raise IE.
        } else {
            Ok((input, control))
        }; // Infinity or already quiet NaN.
    }
    if exponent == 0 && (fraction == 0 || control & 0x40 != 0) {
        return Ok((sign, control)); // DAZ treats a subnormal as signed zero, without DE/PE.
    }
    let power = if exponent == 0 {
        1 - bias
    } else {
        exponent as i32 - bias
    };
    if power >= fraction_bits {
        return Ok((input, control)); // Every represented finite value here is integral.
    }
    let mode = if immediate & 4 != 0 {
        (control >> 13) & 3
    } else {
        u32::from(immediate & 3)
    };
    let result = if power < 0 {
        let increment = match mode {
            0 => power == -1 && fraction != 0, // Exactly .5 ties to even signed zero.
            1 => negative,
            2 => !negative,
            _ => false,
        };
        sign | if increment {
            (bias as u64) << fraction_bits
        } else {
            0
        }
    } else {
        let shift = (fraction_bits - power) as u32;
        let low_mask = (1_u64 << shift) - 1;
        let discarded = input & low_mask;
        if discarded == 0 {
            return Ok((input, control));
        }
        let truncated = input & !low_mask;
        let halfway = 1_u64 << (shift - 1);
        let increment = match mode {
            0 => discarded > halfway || discarded == halfway && (truncated >> shift) & 1 != 0,
            1 => negative,
            2 => !negative,
            _ => false,
        };
        truncated + if increment { 1_u64 << shift } else { 0 }
    };
    // The only newly signaled status here is precision. Immediate[3] never
    // clears an existing sticky PE and does not suppress the earlier sNaN IE.
    Ok((result, control | if immediate & 8 == 0 { 0x20 } else { 0 }))
}

#[cfg(test)]
#[path = "rounding/tests.rs"]
mod tests;
