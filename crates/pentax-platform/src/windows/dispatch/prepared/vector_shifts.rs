// SPDX-License-Identifier: GPL-2.0-only
//! Exact VEX packed shifts. Counts never use Rust's masked shift semantics.
use pentax_codegen::Code;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Count {
    Immediate,
    Uniform,
    Elements,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Left,
    Right,
    Arithmetic,
    BytesLeft,
    BytesRight,
}
#[derive(Clone, Copy)]
pub(super) struct Form {
    pub size: usize,
    pub element: usize,
    pub kind: Kind,
    pub count: Count,
}

pub(super) fn form(code: Code) -> Option<Form> {
    macro_rules! forms {
        ($(($x:ident, $y:ident, $element:expr, $kind:ident, $count:ident)),* $(,)?) => {
            match code {
                $(Code::$x => Some(Form { size: 16, element: $element, kind: Kind::$kind, count: Count::$count }),
                  Code::$y => Some(Form { size: 32, element: $element, kind: Kind::$kind, count: Count::$count }),)*
                _ => None,
            }
        };
    }
    forms![
        (
            VEX_Vpsllw_xmm_xmm_imm8,
            VEX_Vpsllw_ymm_ymm_imm8,
            2,
            Left,
            Immediate
        ),
        (
            VEX_Vpslld_xmm_xmm_imm8,
            VEX_Vpslld_ymm_ymm_imm8,
            4,
            Left,
            Immediate
        ),
        (
            VEX_Vpsllq_xmm_xmm_imm8,
            VEX_Vpsllq_ymm_ymm_imm8,
            8,
            Left,
            Immediate
        ),
        (
            VEX_Vpsrlw_xmm_xmm_imm8,
            VEX_Vpsrlw_ymm_ymm_imm8,
            2,
            Right,
            Immediate
        ),
        (
            VEX_Vpsrld_xmm_xmm_imm8,
            VEX_Vpsrld_ymm_ymm_imm8,
            4,
            Right,
            Immediate
        ),
        (
            VEX_Vpsrlq_xmm_xmm_imm8,
            VEX_Vpsrlq_ymm_ymm_imm8,
            8,
            Right,
            Immediate
        ),
        (
            VEX_Vpsraw_xmm_xmm_imm8,
            VEX_Vpsraw_ymm_ymm_imm8,
            2,
            Arithmetic,
            Immediate
        ),
        (
            VEX_Vpsrad_xmm_xmm_imm8,
            VEX_Vpsrad_ymm_ymm_imm8,
            4,
            Arithmetic,
            Immediate
        ),
        (
            VEX_Vpslldq_xmm_xmm_imm8,
            VEX_Vpslldq_ymm_ymm_imm8,
            1,
            BytesLeft,
            Immediate
        ),
        (
            VEX_Vpsrldq_xmm_xmm_imm8,
            VEX_Vpsrldq_ymm_ymm_imm8,
            1,
            BytesRight,
            Immediate
        ),
        (
            VEX_Vpsllw_xmm_xmm_xmmm128,
            VEX_Vpsllw_ymm_ymm_xmmm128,
            2,
            Left,
            Uniform
        ),
        (
            VEX_Vpslld_xmm_xmm_xmmm128,
            VEX_Vpslld_ymm_ymm_xmmm128,
            4,
            Left,
            Uniform
        ),
        (
            VEX_Vpsllq_xmm_xmm_xmmm128,
            VEX_Vpsllq_ymm_ymm_xmmm128,
            8,
            Left,
            Uniform
        ),
        (
            VEX_Vpsrlw_xmm_xmm_xmmm128,
            VEX_Vpsrlw_ymm_ymm_xmmm128,
            2,
            Right,
            Uniform
        ),
        (
            VEX_Vpsrld_xmm_xmm_xmmm128,
            VEX_Vpsrld_ymm_ymm_xmmm128,
            4,
            Right,
            Uniform
        ),
        (
            VEX_Vpsrlq_xmm_xmm_xmmm128,
            VEX_Vpsrlq_ymm_ymm_xmmm128,
            8,
            Right,
            Uniform
        ),
        (
            VEX_Vpsraw_xmm_xmm_xmmm128,
            VEX_Vpsraw_ymm_ymm_xmmm128,
            2,
            Arithmetic,
            Uniform
        ),
        (
            VEX_Vpsrad_xmm_xmm_xmmm128,
            VEX_Vpsrad_ymm_ymm_xmmm128,
            4,
            Arithmetic,
            Uniform
        ),
        (
            VEX_Vpsllvd_xmm_xmm_xmmm128,
            VEX_Vpsllvd_ymm_ymm_ymmm256,
            4,
            Left,
            Elements
        ),
        (
            VEX_Vpsllvq_xmm_xmm_xmmm128,
            VEX_Vpsllvq_ymm_ymm_ymmm256,
            8,
            Left,
            Elements
        ),
        (
            VEX_Vpsrlvd_xmm_xmm_xmmm128,
            VEX_Vpsrlvd_ymm_ymm_ymmm256,
            4,
            Right,
            Elements
        ),
        (
            VEX_Vpsrlvq_xmm_xmm_xmmm128,
            VEX_Vpsrlvq_ymm_ymm_ymmm256,
            8,
            Right,
            Elements
        ),
        (
            VEX_Vpsravd_xmm_xmm_xmmm128,
            VEX_Vpsravd_ymm_ymm_ymmm256,
            4,
            Arithmetic,
            Elements
        ),
    ]
}

impl Form {
    pub fn count_bytes(self) -> usize {
        match self.count {
            Count::Immediate => 0,
            Count::Uniform => 16,
            Count::Elements => self.size,
        }
    }

    pub fn calculate(self, input: &[u8; 32], counts: &[u8; 32]) -> [u8; 32] {
        let mut output = [0; 32];
        if matches!(self.kind, Kind::BytesLeft | Kind::BytesRight) {
            let count = usize::from(counts[0]).min(16);
            for lane in (0..self.size).step_by(16) {
                if self.kind == Kind::BytesLeft {
                    output[lane + count..lane + 16]
                        .copy_from_slice(&input[lane..lane + 16 - count]);
                } else {
                    output[lane..lane + 16 - count]
                        .copy_from_slice(&input[lane + count..lane + 16]);
                }
            }
            return output;
        }
        let bits = self.element * 8;
        for start in (0..self.size).step_by(self.element) {
            let mut value = [0; 8];
            value[..self.element].copy_from_slice(&input[start..start + self.element]);
            let value = u64::from_le_bytes(value);
            let count = match self.count {
                Count::Immediate => u64::from(counts[0]),
                Count::Uniform => u64::from_le_bytes(counts[..8].try_into().unwrap()),
                Count::Elements => {
                    let mut raw = [0; 8];
                    raw[..self.element].copy_from_slice(&counts[start..start + self.element]);
                    u64::from_le_bytes(raw)
                }
            };
            let shifted = if self.kind == Kind::Arithmetic {
                let signed = ((value << (64 - bits)) as i64) >> (64 - bits);
                (signed >> count.min(bits as u64 - 1)) as u64
            } else if count >= bits as u64 {
                0
            } else if self.kind == Kind::Left {
                value << count
            } else {
                value >> count
            };
            output[start..start + self.element]
                .copy_from_slice(&shifted.to_le_bytes()[..self.element]);
        }
        output
    }
}
