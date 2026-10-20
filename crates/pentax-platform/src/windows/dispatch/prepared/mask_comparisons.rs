// SPDX-License-Identifier: GPL-2.0-only

//! Six VEX floating mask-compare forms using copied integer bits only.
//! No host floating instruction, AVX feature, pointer or MXCSR change is needed.
//!
//! Predicate data: Intel SDM Vol. 2A, Table 3-1 and CMPPS/CMPPD/CMPSS/CMPSD.
//! https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf
//! NaN priority, DAZ and packed status aggregation: Vol. 1 sections 4.9.2,
//! 11.5.2.1-2 and 11.5.3.1; AVX exception handling follows section 14.12.
//! https://cdrdv2-public.intel.com/843827/253665-sdm-vol-1-dec-24.pdf

use super::NativeDispatchError;
use pentax_codegen::Code;
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug)]
pub(super) struct Form {
    pub vector_bytes: usize,
    pub element_bytes: usize,
    pub scalar: bool,
}

pub(super) fn form(code: Code) -> Option<Form> {
    use Code::*;
    let (vector_bytes, element_bytes, scalar) = match code {
        VEX_Vcmpps_xmm_xmm_xmmm128_imm8 => (16, 4, false),
        VEX_Vcmpps_ymm_ymm_ymmm256_imm8 => (32, 4, false),
        VEX_Vcmppd_xmm_xmm_xmmm128_imm8 => (16, 8, false),
        VEX_Vcmppd_ymm_ymm_ymmm256_imm8 => (32, 8, false),
        VEX_Vcmpss_xmm_xmm_xmmm32_imm8 => (16, 4, true),
        VEX_Vcmpsd_xmm_xmm_xmmm64_imm8 => (16, 8, true),
        _ => return None,
    };
    Some(Form {
        vector_bytes,
        element_bytes,
        scalar,
    })
}

// Each bit selects a relation in the order less, equal, greater, unordered.
// Predicates 16..31 have identical truth tables to 0..15, with different
// signaling behavior. Intel's Q/S suffix controls only QNaN invalid signaling.
const TRUTH: [u8; 16] = [2, 1, 3, 8, 13, 14, 12, 7, 10, 9, 11, 0, 5, 6, 4, 15];
const SIGNALING: u32 = 0x9999_6666;

pub(super) fn calculate(
    form: Form,
    left: &[u8; 32],
    right: &[u8; 32],
    immediate: u8,
    mxcsr: u32,
) -> Result<([u8; 32], u32), NativeDispatchError> {
    if !super::mxcsr_control::valid(mxcsr) {
        return Err(NativeDispatchError::InvalidState);
    }
    if !matches!(
        (form.vector_bytes, form.element_bytes, form.scalar),
        (16, 4, false)
            | (32, 4, false)
            | (16, 8, false)
            | (32, 8, false)
            | (16, 4, true)
            | (16, 8, true)
    ) {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    let predicate = immediate & 31;
    let signaling = SIGNALING & (1_u32 << predicate) != 0;
    let width = if form.scalar {
        form.element_bytes
    } else {
        form.vector_bytes
    };
    let mut output = if form.scalar { *left } else { [0; 32] };
    if form.vector_bytes == 16 {
        output[16..].fill(0);
    }
    let mut exceptions = 0;
    for offset in (0..width).step_by(form.element_bytes) {
        let mut a = [0; 8];
        let mut b = [0; 8];
        a[..form.element_bytes].copy_from_slice(&left[offset..offset + form.element_bytes]);
        b[..form.element_bytes].copy_from_slice(&right[offset..offset + form.element_bytes]);
        let (relation, flags) = relation(
            form.element_bytes == 8,
            u64::from_le_bytes(a),
            u64::from_le_bytes(b),
            mxcsr & 64 != 0,
            signaling,
        );
        let selected = TRUTH[usize::from(predicate & 15)] & (1 << relation) != 0;
        output[offset..offset + form.element_bytes].fill(if selected { 0xff } else { 0 });
        // Priorities apply within this lane. A NaN in one lane cannot discard
        // a denormal exception from another lane in the masked profile.
        exceptions |= flags;
    }
    Ok((output, mxcsr | exceptions))
}

fn relation(double: bool, mut left: u64, mut right: u64, daz: bool, signaling: bool) -> (u8, u32) {
    let (sign, exponent, fraction, quiet) = if double {
        (
            0x8000_0000_0000_0000,
            0x7ff0_0000_0000_0000,
            0x000f_ffff_ffff_ffff,
            0x0008_0000_0000_0000,
        )
    } else {
        (0x8000_0000, 0x7f80_0000, 0x007f_ffff, 0x0040_0000)
    };
    let left_nan = left & exponent == exponent && left & fraction != 0;
    let right_nan = right & exponent == exponent && right & fraction != 0;
    if left_nan || right_nan {
        // Both signaling and quiet NaNs take precedence over denormal inputs.
        // SNaN always signals; QNaN signals only for a signaling predicate.
        let invalid =
            signaling || (left_nan && left & quiet == 0) || (right_nan && right & quiet == 0);
        return (3, u32::from(invalid));
    }
    let left_denormal = left & exponent == 0 && left & fraction != 0;
    let right_denormal = right & exponent == 0 && right & fraction != 0;
    let flags = if !daz && (left_denormal || right_denormal) {
        2
    } else {
        0
    };
    if daz {
        if left_denormal {
            left &= sign;
        }
        if right_denormal {
            right &= sign;
        }
    }
    let magnitude = sign - 1;
    let ordering = if left == right || (left & magnitude == 0 && right & magnitude == 0) {
        Ordering::Equal
    } else if (left ^ right) & sign != 0 {
        if left & sign != 0 {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    } else if left & sign != 0 {
        right.cmp(&left)
    } else {
        left.cmp(&right)
    };
    let relation = match ordering {
        Ordering::Less => 0,
        Ordering::Equal => 1,
        Ordering::Greater => 2,
    };
    (relation, flags)
}

#[cfg(test)]
#[path = "mask_comparisons/tests.rs"]
mod tests;
