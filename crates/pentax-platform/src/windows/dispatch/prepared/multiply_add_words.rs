// SPDX-License-Identifier: GPL-2.0-only
//! Original signed-word products and adjacent wrapping dword sums.
//! Intel SDM Vol.2B PMADDWD, pages4-311..314; no host vector operation.

use pentax_codegen::Code;

#[derive(Clone, Copy)]
pub(super) struct Form {
    pub vector_bytes: usize,
    pub legacy: bool,
}

pub(super) fn form(code: Code) -> Option<Form> {
    let (vector_bytes, legacy) = match code {
        Code::Pmaddwd_xmm_xmmm128 => (16, true),
        Code::VEX_Vpmaddwd_xmm_xmm_xmmm128 => (16, false),
        Code::VEX_Vpmaddwd_ymm_ymm_ymmm256 => (32, false),
        _ => return None,
    };
    Some(Form {
        vector_bytes,
        legacy,
    })
}

pub(super) fn calculate(form: Form, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut result = [0; 32];
    for start in (0..form.vector_bytes).step_by(4) {
        let a = i32::from(i16::from_le_bytes(
            left[start..start + 2].try_into().unwrap(),
        ));
        let b = i32::from(i16::from_le_bytes(
            left[start + 2..start + 4].try_into().unwrap(),
        ));
        let c = i32::from(i16::from_le_bytes(
            right[start..start + 2].try_into().unwrap(),
        ));
        let d = i32::from(i16::from_le_bytes(
            right[start + 2..start + 4].try_into().unwrap(),
        ));
        // Each signed16 product fits i32. Only their sum may wrap, including
        // two(-32768 * -32768) products yielding exactly0x80000000.
        let sum = (a * c).wrapping_add(b * d);
        result[start..start + 4].copy_from_slice(&sum.to_le_bytes());
    }
    result
}
