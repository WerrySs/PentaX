// SPDX-License-Identifier: GPL-2.0-only

//! Original bounded PCMPxSTRx semantics over two complete source snapshots.
//! Intel SDM Vol. 2B section 4.1 defines validity, aggregation and polarity.

use super::{NativeCpuState, NativeDispatchError, RAX, RCX, RDX};
use pentax_codegen::{Code, Instruction, OpKind};

struct Form {
    // None = first NUL element; Some(false/true) = signed 32/64-bit length.
    explicit: Option<bool>,
    mask: bool,
    vex: bool,
}

fn form(code: Code) -> Option<Form> {
    use Code::*;
    let (explicit, mask) = match code {
        Pcmpistrm_xmm_xmmm128_imm8 | VEX_Vpcmpistrm_xmm_xmmm128_imm8 => (None, true),
        Pcmpistri_xmm_xmmm128_imm8 | VEX_Vpcmpistri_xmm_xmmm128_imm8 => (None, false),
        Pcmpestrm_xmm_xmmm128_imm8 | VEX_Vpcmpestrm_xmm_xmmm128_imm8 => (Some(false), true),
        Pcmpestrm64_xmm_xmmm128_imm8 | VEX_Vpcmpestrm64_xmm_xmmm128_imm8 => (Some(true), true),
        Pcmpestri_xmm_xmmm128_imm8 | VEX_Vpcmpestri_xmm_xmmm128_imm8 => (Some(false), false),
        Pcmpestri64_xmm_xmmm128_imm8 | VEX_Vpcmpestri64_xmm_xmmm128_imm8 => (Some(true), false),
        _ => return None,
    };
    let vex = matches!(
        code,
        VEX_Vpcmpistrm_xmm_xmmm128_imm8
            | VEX_Vpcmpistri_xmm_xmmm128_imm8
            | VEX_Vpcmpestrm_xmm_xmmm128_imm8
            | VEX_Vpcmpestrm64_xmm_xmmm128_imm8
            | VEX_Vpcmpestri_xmm_xmmm128_imm8
            | VEX_Vpcmpestri64_xmm_xmmm128_imm8
    );
    Some(Form {
        explicit,
        mask,
        vex,
    })
}

pub(super) fn supported(code: Code) -> bool {
    form(code).is_some()
}

pub(super) fn apply(
    instruction: &Instruction,
    state: &mut NativeCpuState,
    left: &[u8; 32],
    right: &[u8; 32],
) -> Result<(), NativeDispatchError> {
    let form = form(instruction.code()).ok_or(NativeDispatchError::UnsupportedVectorInstruction)?;
    if instruction.op_count() != 3 || instruction.op2_kind() != OpKind::Immediate8 {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    let lengths = form
        .explicit
        .map(|wide| (state.gpr[RAX], state.gpr[RDX], wide));
    let result = compare(
        left[..16].try_into().unwrap(),
        right[..16].try_into().unwrap(),
        lengths,
        instruction.immediate8(),
    );
    state.rflags = (state.rflags & !0x8d5) | result.flags;
    if form.mask {
        state.xmm[..16].copy_from_slice(&result.mask);
        if form.vex {
            state.ymm_upper[..16].fill(0);
        }
    } else {
        state.gpr[RCX] = result.index;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Comparison {
    pub mask: [u8; 16],
    pub index: u64,
    pub flags: u64,
}

pub(super) fn compare(
    left: &[u8; 16],
    right: &[u8; 16],
    lengths: Option<(u64, u64, bool)>,
    mode: u8,
) -> Comparison {
    let width = 1 + usize::from(mode & 1);
    let count = 16 / width;
    let decode = |bytes: &[u8; 16]| {
        let mut elements = [0_i32; 16];
        for (i, value) in elements[..count].iter_mut().enumerate() {
            *value = match mode & 3 {
                0 => i32::from(bytes[i]),
                1 => i32::from(u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]])),
                2 => i32::from(bytes[i] as i8),
                _ => i32::from(i16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]])),
            };
        }
        elements
    };
    let a = decode(left);
    let b = decode(right);
    let (a_len, b_len) = match lengths {
        Some((a, b, wide)) => (length(a, wide, count), length(b, wide, count)),
        None => (
            a[..count].iter().position(|v| *v == 0).unwrap_or(count),
            b[..count].iter().position(|v| *v == 0).unwrap_or(count),
        ),
    };
    let mut bits = 0_u16;
    for j in 0..count {
        let matches = match (mode >> 2) & 3 {
            0 => j < b_len && a[..a_len].contains(&b[j]),
            1 => {
                j < b_len
                    && a[..a_len]
                        .chunks_exact(2)
                        .any(|range| range[0] <= b[j] && b[j] <= range[1])
            }
            2 => match (j < a_len, j < b_len) {
                (true, true) => a[j] == b[j],
                (false, false) => true,
                _ => false,
            },
            // The last candidate may match a prefix ending at the vector
            // boundary. An invalid first-source element always compares true;
            // a valid first-source element against an invalid second is false.
            _ => (0..count - j).all(|i| i >= a_len || (i + j < b_len && a[i] == b[i + j])),
        };
        bits |= u16::from(matches) << j;
    }
    let valid_mask = |len: usize| ((1_u32 << len) - 1) as u16;
    if mode & 0x10 != 0 {
        bits ^= valid_mask(if mode & 0x20 != 0 { b_len } else { count });
    }
    let mut mask = [0; 16];
    if mode & 0x40 == 0 {
        mask[..2].copy_from_slice(&bits.to_le_bytes());
    } else {
        for (i, element) in mask.chunks_exact_mut(width).enumerate() {
            element.fill(if bits & (1 << i) != 0 { 0xff } else { 0 });
        }
    }
    let index = if bits == 0 {
        count as u64
    } else if mode & 0x40 == 0 {
        u64::from(bits.trailing_zeros())
    } else {
        u64::from(15 - bits.leading_zeros())
    };
    let flags = u64::from(bits != 0)
        | (u64::from(b_len < count) << 6)
        | (u64::from(a_len < count) << 7)
        | (u64::from(bits & 1) << 11);
    Comparison { mask, index, flags }
}

fn length(value: u64, wide: bool, count: usize) -> usize {
    let magnitude = if wide {
        (value as i64).unsigned_abs()
    } else {
        u64::from((value as u32 as i32).unsigned_abs())
    };
    magnitude.min(count as u64) as usize
}
