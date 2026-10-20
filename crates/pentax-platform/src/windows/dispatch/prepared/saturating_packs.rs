// SPDX-License-Identifier: GPL-2.0-only
//! Original integer saturation for the twelve non-MMX/non-EVEX pack forms.
//! Intel SDM Vol.2B PACKSSWB/SSDW, PACKUSWB and PACKUSDW; no host SIMD.

use pentax_codegen::Code;

#[derive(Clone, Copy)]
pub(super) struct Form {
    pub vector_bytes: usize,
    pub source_element_bytes: usize,
    pub legacy: bool,
    minimum: i64,
    maximum: i64,
}

pub(super) fn form(code: Code) -> Option<Form> {
    use Code::*;
    let (vector_bytes, source_element_bytes, unsigned, legacy) = match code {
        Packsswb_xmm_xmmm128 => (16, 2, false, true),
        VEX_Vpacksswb_xmm_xmm_xmmm128 => (16, 2, false, false),
        VEX_Vpacksswb_ymm_ymm_ymmm256 => (32, 2, false, false),
        Packssdw_xmm_xmmm128 => (16, 4, false, true),
        VEX_Vpackssdw_xmm_xmm_xmmm128 => (16, 4, false, false),
        VEX_Vpackssdw_ymm_ymm_ymmm256 => (32, 4, false, false),
        Packuswb_xmm_xmmm128 => (16, 2, true, true),
        VEX_Vpackuswb_xmm_xmm_xmmm128 => (16, 2, true, false),
        VEX_Vpackuswb_ymm_ymm_ymmm256 => (32, 2, true, false),
        Packusdw_xmm_xmmm128 => (16, 4, true, true),
        VEX_Vpackusdw_xmm_xmm_xmmm128 => (16, 4, true, false),
        VEX_Vpackusdw_ymm_ymm_ymmm256 => (32, 4, true, false),
        _ => return None,
    };
    let (minimum, maximum) = match (source_element_bytes, unsigned) {
        (2, false) => (-128, 127),
        (4, false) => (-32768, 32767),
        (2, true) => (0, 255),
        (4, true) => (0, 65535),
        _ => unreachable!(),
    };
    Some(Form {
        vector_bytes,
        source_element_bytes,
        legacy,
        minimum,
        maximum,
    })
}

/// Both complete input snapshots precede this calculation. Each 128-bit lane
/// packs its SRC1 elements into its low64 and SRC2 into its high64. In
/// particular VEX.256 never concatenates all of SRC1 before all of SRC2.
pub(super) fn calculate(form: Form, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut result = [0; 32];
    let source_width = form.source_element_bytes;
    let destination_width = source_width / 2;
    for lane in (0..form.vector_bytes).step_by(16) {
        for (half, input) in [left, right].into_iter().enumerate() {
            for element in 0..16 / source_width {
                let from = lane + element * source_width;
                // Unsigned destination forms still interpret their SOURCES
                // as signed, so every negative source saturates to zero.
                let value = if source_width == 2 {
                    i64::from(i16::from_le_bytes(
                        input[from..from + 2].try_into().unwrap(),
                    ))
                } else {
                    i64::from(i32::from_le_bytes(
                        input[from..from + 4].try_into().unwrap(),
                    ))
                };
                let value = value.clamp(form.minimum, form.maximum).to_le_bytes();
                let to = lane + half * 8 + element * destination_width;
                result[to..to + destination_width].copy_from_slice(&value[..destination_width]);
            }
        }
    }
    result
}
