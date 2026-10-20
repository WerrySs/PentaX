// SPDX-License-Identifier: GPL-2.0-only
//! Exact non-MMX PALIGNR byte selection, independently within each128-bit lane.

use pentax_codegen::Code;

#[derive(Clone, Copy)]
pub(super) struct Form {
    pub vector_bytes: usize,
    pub legacy: bool,
}

pub(super) fn form(code: Code) -> Option<Form> {
    let (vector_bytes, legacy) = match code {
        Code::Palignr_xmm_xmmm128_imm8 => (16, true),
        Code::VEX_Vpalignr_xmm_xmm_xmmm128_imm8 => (16, false),
        Code::VEX_Vpalignr_ymm_ymm_ymmm256_imm8 => (32, false),
        _ => return None,
    };
    Some(Form {
        vector_bytes,
        legacy,
    })
}

pub(super) fn calculate(form: Form, high: &[u8; 32], low: &[u8; 32], immediate: u8) -> [u8; 32] {
    let mut output = [0; 32];
    for lane in (0..form.vector_bytes).step_by(16) {
        for byte in 0..16 {
            let selected = usize::from(immediate) + byte;
            output[lane + byte] = match selected {
                0..=15 => low[lane + selected],
                16..=31 => high[lane + selected - 16],
                _ => 0,
            };
        }
    }
    output
}

#[cfg(test)]
#[path = "align_right/tests.rs"]
mod tests;
