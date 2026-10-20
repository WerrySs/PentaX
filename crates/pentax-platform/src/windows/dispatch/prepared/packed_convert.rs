// SPDX-License-Identifier: GPL-2.0-only

//! Twelve VEX packed conversions over copied bits with fixed-size output.
//! Arithmetic reuses the integer-only scalar conversion helpers, never host FP
//! instructions, AVX, guest pointers or a change to host MXCSR.
//!
//! Intel SDM Vol. 2A: CVTPS2DQ, CVTTPS2DQ, CVTPD2DQ, CVTTPD2DQ,
//! CVTDQ2PS and CVTDQ2PD define the element widths, rounding and exceptions.
//! https://cdrdv2-public.intel.com/812383/253666-sdm-vol-2a.pdf
//! Packed masked exceptions aggregate each lane's flags by OR (Vol. 1,
//! section 11.5.3.1); section 11.5.2.2 excludes these float-to-integer
//! conversions from denormal exceptions, and 14.12 carries the rules to AVX.
//! https://cdrdv2-public.intel.com/843827/253665-sdm-vol-1-dec-24.pdf

use super::{NativeDispatchError, scalar_convert};
use pentax_codegen::Code;

#[derive(Clone, Copy, Debug)]
enum Direction {
    FloatToSigned,
    SignedToFloat,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Form {
    pub input_element: usize,
    pub output_element: usize,
    pub lanes: usize,
    direction: Direction,
    truncate: bool,
}

pub(super) fn form(code: Code) -> Option<Form> {
    use Code::*;
    use Direction::*;
    let (input_element, output_element, lanes, direction, truncate) = match code {
        VEX_Vcvtps2dq_xmm_xmmm128 => (4, 4, 4, FloatToSigned, false),
        VEX_Vcvtps2dq_ymm_ymmm256 => (4, 4, 8, FloatToSigned, false),
        VEX_Vcvttps2dq_xmm_xmmm128 => (4, 4, 4, FloatToSigned, true),
        VEX_Vcvttps2dq_ymm_ymmm256 => (4, 4, 8, FloatToSigned, true),
        VEX_Vcvtpd2dq_xmm_xmmm128 => (8, 4, 2, FloatToSigned, false),
        VEX_Vcvtpd2dq_xmm_ymmm256 => (8, 4, 4, FloatToSigned, false),
        VEX_Vcvttpd2dq_xmm_xmmm128 => (8, 4, 2, FloatToSigned, true),
        VEX_Vcvttpd2dq_xmm_ymmm256 => (8, 4, 4, FloatToSigned, true),
        VEX_Vcvtdq2ps_xmm_xmmm128 => (4, 4, 4, SignedToFloat, false),
        VEX_Vcvtdq2ps_ymm_ymmm256 => (4, 4, 8, SignedToFloat, false),
        VEX_Vcvtdq2pd_xmm_xmmm64 => (4, 8, 2, SignedToFloat, false),
        VEX_Vcvtdq2pd_ymm_xmmm128 => (4, 8, 4, SignedToFloat, false),
        _ => return None,
    };
    Some(Form {
        input_element,
        output_element,
        lanes,
        direction,
        truncate,
    })
}

pub(super) fn calculate(
    form: Form,
    input: &[u8; 32],
    mxcsr: u32,
) -> Result<([u8; 32], u32), NativeDispatchError> {
    if !super::mxcsr_control::valid(mxcsr) {
        return Err(NativeDispatchError::InvalidState);
    }
    let shape = (form.input_element, form.output_element, form.lanes);
    let valid = match form.direction {
        Direction::FloatToSigned => {
            matches!(shape, (4, 4, 4) | (4, 4, 8) | (8, 4, 2) | (8, 4, 4))
        }
        Direction::SignedToFloat => {
            !form.truncate && matches!(shape, (4, 4, 4) | (4, 4, 8) | (4, 8, 2) | (4, 8, 4))
        }
    };
    if !valid {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    let rounding = if form.truncate { 3 } else { mxcsr >> 13 };
    // Zero from the end of the actual results, including PD->DQ's unused
    // high64 within XMM and every VEX XMM destination's complete upper half.
    let mut output = [0; 32];
    let mut exceptions = 0;
    for lane in 0..form.lanes {
        let mut source = [0; 8];
        let at = lane * form.input_element;
        source[..form.input_element].copy_from_slice(&input[at..at + form.input_element]);
        let raw = u64::from_le_bytes(source);
        let (bits, flags) = match form.direction {
            Direction::FloatToSigned => scalar_convert::to_signed_bits(
                raw,
                form.input_element == 8,
                false,
                rounding,
                mxcsr & 64 != 0,
            ),
            Direction::SignedToFloat => {
                let signed = i64::from(raw as u32 as i32);
                let (bits, inexact) =
                    scalar_convert::signed_bits(signed, form.output_element == 8, rounding);
                (bits, u32::from(inexact) << 5)
            }
        };
        let at = lane * form.output_element;
        output[at..at + form.output_element]
            .copy_from_slice(&bits.to_le_bytes()[..form.output_element]);
        // A float->integer invalid lane suppresses its own precision flag;
        // precision raised by another lane still contributes to packed MXCSR.
        exceptions |= flags;
    }
    // DAZ changes only float inputs; FTZ cannot affect an integer result or a
    // normal binary32/binary64 result obtained from a signed32 integer.
    Ok((output, mxcsr | exceptions))
}

#[cfg(test)]
#[path = "packed_convert/tests.rs"]
mod tests;
