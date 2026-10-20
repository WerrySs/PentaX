// SPDX-License-Identifier: GPL-2.0-only

//! Bounded VEX transfers, logic, shuffles, equality and sign masks over guest state.
//! No host AVX instruction or extended-context permission is needed.

use super::*;
use pentax_codegen::{Code, Instruction, Mnemonic, OpKind, Register};

fn move_width(code: Code) -> Option<usize> {
    use Code::*;
    match code {
        VEX_Vmovd_xmm_rm32 | VEX_Vmovd_rm32_xmm | VEX_Vmovss_xmm_m32 | VEX_Vmovss_m32_xmm => {
            Some(4)
        }
        VEX_Vmovq_xmm_rm64 | VEX_Vmovq_rm64_xmm | VEX_Vmovq_xmm_xmmm64 | VEX_Vmovq_xmmm64_xmm
        | VEX_Vmovsd_xmm_m64 | VEX_Vmovsd_m64_xmm => Some(8),
        VEX_Vmovups_xmm_xmmm128
        | VEX_Vmovups_xmmm128_xmm
        | VEX_Vmovupd_xmm_xmmm128
        | VEX_Vmovupd_xmmm128_xmm
        | VEX_Vmovdqu_xmm_xmmm128
        | VEX_Vmovdqu_xmmm128_xmm
        | VEX_Vmovaps_xmm_xmmm128
        | VEX_Vmovaps_xmmm128_xmm
        | VEX_Vmovapd_xmm_xmmm128
        | VEX_Vmovapd_xmmm128_xmm
        | VEX_Vmovdqa_xmm_xmmm128
        | VEX_Vmovdqa_xmmm128_xmm
        | VEX_Vmovntdqa_xmm_m128
        | VEX_Vmovntdq_m128_xmm => Some(16),
        VEX_Vmovups_ymm_ymmm256
        | VEX_Vmovups_ymmm256_ymm
        | VEX_Vmovupd_ymm_ymmm256
        | VEX_Vmovupd_ymmm256_ymm
        | VEX_Vmovdqu_ymm_ymmm256
        | VEX_Vmovdqu_ymmm256_ymm
        | VEX_Vmovaps_ymm_ymmm256
        | VEX_Vmovaps_ymmm256_ymm
        | VEX_Vmovapd_ymm_ymmm256
        | VEX_Vmovapd_ymmm256_ymm
        | VEX_Vmovdqa_ymm_ymmm256
        | VEX_Vmovdqa_ymmm256_ymm
        | VEX_Vmovntdqa_ymm_m256
        | VEX_Vmovntdq_m256_ymm => Some(32),
        _ => None,
    }
}

fn merge_width(code: Code) -> Option<usize> {
    match code {
        Code::VEX_Vmovss_xmm_xmm_xmm | Code::VEX_Vmovss_xmm_xmm_xmm_0F11 => Some(4),
        Code::VEX_Vmovsd_xmm_xmm_xmm | Code::VEX_Vmovsd_xmm_xmm_xmm_0F11 => Some(8),
        _ => None,
    }
}

enum HalfMove {
    Merge { to: usize, from: usize },
    Store { from: usize },
}

// Exact VEX.128 forms only. Loads replace one quadword of source1; register
// forms select a quadword of source2. Stores leave every vector bit unchanged.
fn half_move(code: Code) -> Option<HalfMove> {
    use Code::*;
    Some(match code {
        VEX_Vmovlps_xmm_xmm_m64 | VEX_Vmovlpd_xmm_xmm_m64 => HalfMove::Merge { to: 0, from: 0 },
        VEX_Vmovhps_xmm_xmm_m64 | VEX_Vmovhpd_xmm_xmm_m64 | VEX_Vmovlhps_xmm_xmm_xmm => {
            HalfMove::Merge { to: 8, from: 0 }
        }
        VEX_Vmovhlps_xmm_xmm_xmm => HalfMove::Merge { to: 0, from: 8 },
        VEX_Vmovlps_m64_xmm | VEX_Vmovlpd_m64_xmm => HalfMove::Store { from: 0 },
        VEX_Vmovhps_m64_xmm | VEX_Vmovhpd_m64_xmm => HalfMove::Store { from: 8 },
        _ => return None,
    })
}

// Destination width, element width, selected offset in each pair, memory width.
// VEX.128 VMOVDDUP reads only m64; its VEX.256 form reads all m256.
fn duplicate_layout(code: Code) -> Option<(usize, usize, usize, usize)> {
    use Code::*;
    Some(match code {
        VEX_Vmovddup_xmm_xmmm64 => (16, 8, 0, 8),
        VEX_Vmovddup_ymm_ymmm256 => (32, 8, 0, 32),
        VEX_Vmovsldup_xmm_xmmm128 => (16, 4, 0, 16),
        VEX_Vmovsldup_ymm_ymmm256 => (32, 4, 0, 32),
        VEX_Vmovshdup_xmm_xmmm128 => (16, 4, 4, 16),
        VEX_Vmovshdup_ymm_ymmm256 => (32, 4, 4, 32),
        _ => return None,
    })
}

fn lane_insert(code: Code) -> bool {
    matches!(
        code,
        Code::VEX_Vinsertf128_ymm_ymm_xmmm128_imm8 | Code::VEX_Vinserti128_ymm_ymm_xmmm128_imm8
    )
}

fn lane_extract(code: Code) -> bool {
    matches!(
        code,
        Code::VEX_Vextractf128_xmmm128_ymm_imm8 | Code::VEX_Vextracti128_xmmm128_ymm_imm8
    )
}

#[derive(Clone, Copy)]
enum ImmediatePermutation {
    Halves,
    Quadwords,
}

fn immediate_permutation(code: Code) -> Option<ImmediatePermutation> {
    match code {
        Code::VEX_Vperm2f128_ymm_ymm_ymmm256_imm8 | Code::VEX_Vperm2i128_ymm_ymm_ymmm256_imm8 => {
            Some(ImmediatePermutation::Halves)
        }
        Code::VEX_Vpermpd_ymm_ymmm256_imm8 | Code::VEX_Vpermq_ymm_ymmm256_imm8 => {
            Some(ImmediatePermutation::Quadwords)
        }
        _ => None,
    }
}

// Vector bytes, element bytes, and immediate rather than vector control.
fn element_permutation(code: Code) -> Option<(usize, usize, bool)> {
    use Code::*;
    Some(match code {
        VEX_Vpermilps_xmm_xmmm128_imm8 => (16, 4, true),
        VEX_Vpermilps_ymm_ymmm256_imm8 => (32, 4, true),
        VEX_Vpermilpd_xmm_xmmm128_imm8 => (16, 8, true),
        VEX_Vpermilpd_ymm_ymmm256_imm8 => (32, 8, true),
        VEX_Vpermilps_xmm_xmm_xmmm128 => (16, 4, false),
        VEX_Vpermilps_ymm_ymm_ymmm256 => (32, 4, false),
        VEX_Vpermilpd_xmm_xmm_xmmm128 => (16, 8, false),
        VEX_Vpermilpd_ymm_ymm_ymmm256 => (32, 8, false),
        _ => return None,
    })
}

// Exact VEX.128 scalar inserts: inserted bytes and register-source width.
fn scalar_insert(code: Code) -> Option<(usize, usize)> {
    use Code::*;
    Some(match code {
        VEX_Vpinsrb_xmm_xmm_r32m8_imm8 => (1, 4),
        VEX_Vpinsrb_xmm_xmm_r64m8_imm8 => (1, 8),
        VEX_Vpinsrw_xmm_xmm_r32m16_imm8 => (2, 4),
        VEX_Vpinsrw_xmm_xmm_r64m16_imm8 => (2, 8),
        VEX_Vpinsrd_xmm_xmm_rm32_imm8 => (4, 4),
        VEX_Vpinsrq_xmm_xmm_rm64_imm8 => (8, 8),
        _ => return None,
    })
}

fn scalar_extract(code: Code) -> Option<usize> {
    use Code::*;
    Some(match code {
        VEX_Vpextrb_r32m8_xmm_imm8 | VEX_Vpextrb_r64m8_xmm_imm8 => 1,
        VEX_Vpextrw_r32_xmm_imm8
        | VEX_Vpextrw_r64_xmm_imm8
        | VEX_Vpextrw_r32m16_xmm_imm8
        | VEX_Vpextrw_r64m16_xmm_imm8 => 2,
        VEX_Vpextrd_rm32_xmm_imm8
        | VEX_Vextractps_rm32_xmm_imm8
        | VEX_Vextractps_r64m32_xmm_imm8 => 4,
        VEX_Vpextrq_rm64_xmm_imm8 => 8,
        _ => return None,
    })
}

// Destination bytes, source element bytes, destination element bytes, signed.
fn integer_widen(code: Code) -> Option<(usize, usize, usize, bool)> {
    use Code::*;
    Some(match code {
        VEX_Vpmovsxbw_xmm_xmmm64 => (16, 1, 2, true),
        VEX_Vpmovsxbw_ymm_xmmm128 => (32, 1, 2, true),
        VEX_Vpmovsxbd_xmm_xmmm32 => (16, 1, 4, true),
        VEX_Vpmovsxbd_ymm_xmmm64 => (32, 1, 4, true),
        VEX_Vpmovsxbq_xmm_xmmm16 => (16, 1, 8, true),
        VEX_Vpmovsxbq_ymm_xmmm32 => (32, 1, 8, true),
        VEX_Vpmovsxwd_xmm_xmmm64 => (16, 2, 4, true),
        VEX_Vpmovsxwd_ymm_xmmm128 => (32, 2, 4, true),
        VEX_Vpmovsxwq_xmm_xmmm32 => (16, 2, 8, true),
        VEX_Vpmovsxwq_ymm_xmmm64 => (32, 2, 8, true),
        VEX_Vpmovsxdq_xmm_xmmm64 => (16, 4, 8, true),
        VEX_Vpmovsxdq_ymm_xmmm128 => (32, 4, 8, true),
        VEX_Vpmovzxbw_xmm_xmmm64 => (16, 1, 2, false),
        VEX_Vpmovzxbw_ymm_xmmm128 => (32, 1, 2, false),
        VEX_Vpmovzxbd_xmm_xmmm32 => (16, 1, 4, false),
        VEX_Vpmovzxbd_ymm_xmmm64 => (32, 1, 4, false),
        VEX_Vpmovzxbq_xmm_xmmm16 => (16, 1, 8, false),
        VEX_Vpmovzxbq_ymm_xmmm32 => (32, 1, 8, false),
        VEX_Vpmovzxwd_xmm_xmmm64 => (16, 2, 4, false),
        VEX_Vpmovzxwd_ymm_xmmm128 => (32, 2, 4, false),
        VEX_Vpmovzxwq_xmm_xmmm32 => (16, 2, 8, false),
        VEX_Vpmovzxwq_ymm_xmmm64 => (32, 2, 8, false),
        VEX_Vpmovzxdq_xmm_xmmm64 => (16, 4, 8, false),
        VEX_Vpmovzxdq_ymm_xmmm128 => (32, 4, 8, false),
        _ => return None,
    })
}

fn binary_width(code: Code) -> Option<usize> {
    use Code::*;
    match code {
        VEX_Vpand_xmm_xmm_xmmm128
        | VEX_Vpandn_xmm_xmm_xmmm128
        | VEX_Vpor_xmm_xmm_xmmm128
        | VEX_Vpxor_xmm_xmm_xmmm128
        | VEX_Vpblendd_xmm_xmm_xmmm128_imm8
        | VEX_Vpblendw_xmm_xmm_xmmm128_imm8
        | VEX_Vblendps_xmm_xmm_xmmm128_imm8
        | VEX_Vblendpd_xmm_xmm_xmmm128_imm8
        | VEX_Vpblendvb_xmm_xmm_xmmm128_xmm
        | VEX_Vblendvps_xmm_xmm_xmmm128_xmm
        | VEX_Vblendvpd_xmm_xmm_xmmm128_xmm
        | VEX_Vandps_xmm_xmm_xmmm128
        | VEX_Vandpd_xmm_xmm_xmmm128
        | VEX_Vandnps_xmm_xmm_xmmm128
        | VEX_Vandnpd_xmm_xmm_xmmm128
        | VEX_Vorps_xmm_xmm_xmmm128
        | VEX_Vorpd_xmm_xmm_xmmm128
        | VEX_Vxorps_xmm_xmm_xmmm128
        | VEX_Vxorpd_xmm_xmm_xmmm128
        | VEX_Vpaddb_xmm_xmm_xmmm128
        | VEX_Vpaddw_xmm_xmm_xmmm128
        | VEX_Vpaddd_xmm_xmm_xmmm128
        | VEX_Vpaddq_xmm_xmm_xmmm128
        | VEX_Vpminsb_xmm_xmm_xmmm128
        | VEX_Vpminsw_xmm_xmm_xmmm128
        | VEX_Vpminsd_xmm_xmm_xmmm128
        | VEX_Vpminub_xmm_xmm_xmmm128
        | VEX_Vpminuw_xmm_xmm_xmmm128
        | VEX_Vpminud_xmm_xmm_xmmm128
        | VEX_Vpmaxsb_xmm_xmm_xmmm128
        | VEX_Vpmaxsw_xmm_xmm_xmmm128
        | VEX_Vpmaxsd_xmm_xmm_xmmm128
        | VEX_Vpmaxub_xmm_xmm_xmmm128
        | VEX_Vpmaxuw_xmm_xmm_xmmm128
        | VEX_Vpmaxud_xmm_xmm_xmmm128
        | VEX_Vpmuludq_xmm_xmm_xmmm128
        | VEX_Vpmuldq_xmm_xmm_xmmm128
        | VEX_Vpmulld_xmm_xmm_xmmm128
        | VEX_Vpmullw_xmm_xmm_xmmm128
        | VEX_Vpmulhw_xmm_xmm_xmmm128
        | VEX_Vpmulhuw_xmm_xmm_xmmm128
        | VEX_Vpsubb_xmm_xmm_xmmm128
        | VEX_Vpsubw_xmm_xmm_xmmm128
        | VEX_Vpsubd_xmm_xmm_xmmm128
        | VEX_Vpsubq_xmm_xmm_xmmm128
        | VEX_Vpcmpeqb_xmm_xmm_xmmm128
        | VEX_Vpcmpeqw_xmm_xmm_xmmm128
        | VEX_Vpcmpeqd_xmm_xmm_xmmm128
        | VEX_Vpcmpeqq_xmm_xmm_xmmm128
        | VEX_Vunpcklps_xmm_xmm_xmmm128
        | VEX_Vunpckhps_xmm_xmm_xmmm128
        | VEX_Vunpcklpd_xmm_xmm_xmmm128
        | VEX_Vunpckhpd_xmm_xmm_xmmm128
        | VEX_Vpunpcklbw_xmm_xmm_xmmm128
        | VEX_Vpunpckhbw_xmm_xmm_xmmm128
        | VEX_Vpunpcklwd_xmm_xmm_xmmm128
        | VEX_Vpunpckhwd_xmm_xmm_xmmm128
        | VEX_Vpunpckldq_xmm_xmm_xmmm128
        | VEX_Vpunpckhdq_xmm_xmm_xmmm128
        | VEX_Vpunpcklqdq_xmm_xmm_xmmm128
        | VEX_Vpunpckhqdq_xmm_xmm_xmmm128
        | VEX_Vshufps_xmm_xmm_xmmm128_imm8
        | VEX_Vshufpd_xmm_xmm_xmmm128_imm8
        | VEX_Vpshufb_xmm_xmm_xmmm128 => Some(16),
        VEX_Vpand_ymm_ymm_ymmm256
        | VEX_Vpandn_ymm_ymm_ymmm256
        | VEX_Vpor_ymm_ymm_ymmm256
        | VEX_Vpxor_ymm_ymm_ymmm256
        | VEX_Vpblendd_ymm_ymm_ymmm256_imm8
        | VEX_Vpblendw_ymm_ymm_ymmm256_imm8
        | VEX_Vblendps_ymm_ymm_ymmm256_imm8
        | VEX_Vblendpd_ymm_ymm_ymmm256_imm8
        | VEX_Vpblendvb_ymm_ymm_ymmm256_ymm
        | VEX_Vblendvps_ymm_ymm_ymmm256_ymm
        | VEX_Vblendvpd_ymm_ymm_ymmm256_ymm
        | VEX_Vandps_ymm_ymm_ymmm256
        | VEX_Vandpd_ymm_ymm_ymmm256
        | VEX_Vandnps_ymm_ymm_ymmm256
        | VEX_Vandnpd_ymm_ymm_ymmm256
        | VEX_Vorps_ymm_ymm_ymmm256
        | VEX_Vorpd_ymm_ymm_ymmm256
        | VEX_Vxorps_ymm_ymm_ymmm256
        | VEX_Vxorpd_ymm_ymm_ymmm256
        | VEX_Vpaddb_ymm_ymm_ymmm256
        | VEX_Vpaddw_ymm_ymm_ymmm256
        | VEX_Vpaddd_ymm_ymm_ymmm256
        | VEX_Vpaddq_ymm_ymm_ymmm256
        | VEX_Vpminsb_ymm_ymm_ymmm256
        | VEX_Vpminsw_ymm_ymm_ymmm256
        | VEX_Vpminsd_ymm_ymm_ymmm256
        | VEX_Vpminub_ymm_ymm_ymmm256
        | VEX_Vpminuw_ymm_ymm_ymmm256
        | VEX_Vpminud_ymm_ymm_ymmm256
        | VEX_Vpmaxsb_ymm_ymm_ymmm256
        | VEX_Vpmaxsw_ymm_ymm_ymmm256
        | VEX_Vpmaxsd_ymm_ymm_ymmm256
        | VEX_Vpmaxub_ymm_ymm_ymmm256
        | VEX_Vpmaxuw_ymm_ymm_ymmm256
        | VEX_Vpmaxud_ymm_ymm_ymmm256
        | VEX_Vpmuludq_ymm_ymm_ymmm256
        | VEX_Vpmuldq_ymm_ymm_ymmm256
        | VEX_Vpmulld_ymm_ymm_ymmm256
        | VEX_Vpmullw_ymm_ymm_ymmm256
        | VEX_Vpmulhw_ymm_ymm_ymmm256
        | VEX_Vpmulhuw_ymm_ymm_ymmm256
        | VEX_Vpsubb_ymm_ymm_ymmm256
        | VEX_Vpsubw_ymm_ymm_ymmm256
        | VEX_Vpsubd_ymm_ymm_ymmm256
        | VEX_Vpsubq_ymm_ymm_ymmm256
        | VEX_Vpcmpeqb_ymm_ymm_ymmm256
        | VEX_Vpcmpeqw_ymm_ymm_ymmm256
        | VEX_Vpcmpeqd_ymm_ymm_ymmm256
        | VEX_Vpcmpeqq_ymm_ymm_ymmm256
        | VEX_Vunpcklps_ymm_ymm_ymmm256
        | VEX_Vunpckhps_ymm_ymm_ymmm256
        | VEX_Vunpcklpd_ymm_ymm_ymmm256
        | VEX_Vunpckhpd_ymm_ymm_ymmm256
        | VEX_Vpunpcklbw_ymm_ymm_ymmm256
        | VEX_Vpunpckhbw_ymm_ymm_ymmm256
        | VEX_Vpunpcklwd_ymm_ymm_ymmm256
        | VEX_Vpunpckhwd_ymm_ymm_ymmm256
        | VEX_Vpunpckldq_ymm_ymm_ymmm256
        | VEX_Vpunpckhdq_ymm_ymm_ymmm256
        | VEX_Vpunpcklqdq_ymm_ymm_ymmm256
        | VEX_Vpunpckhqdq_ymm_ymm_ymmm256
        | VEX_Vshufps_ymm_ymm_ymmm256_imm8
        | VEX_Vshufpd_ymm_ymm_ymmm256_imm8
        | VEX_Vpshufb_ymm_ymm_ymmm256
        | VEX_Vpermps_ymm_ymm_ymmm256
        | VEX_Vpermd_ymm_ymm_ymmm256 => Some(32),
        _ => None,
    }
}

pub(super) fn supported(instruction: &Instruction) -> bool {
    move_width(instruction.code()).is_some()
        || integer_conversion(instruction.code()).is_some()
        || float_integer_conversion(instruction.code()).is_some()
        || packed_convert::form(instruction.code()).is_some()
        || precision_conversion(instruction.code()).is_some()
        || float_ops::form(instruction.code()).is_some()
        || fused_float::form(instruction.code()).is_some()
        || mask_comparisons::form(instruction.code()).is_some()
        || float_ops::comparison_form(instruction.code()).is_some()
        || merge_width(instruction.code()).is_some()
        || half_move(instruction.code()).is_some()
        || duplicate_layout(instruction.code()).is_some()
        || lane_insert(instruction.code())
        || lane_extract(instruction.code())
        || immediate_permutation(instruction.code()).is_some()
        || element_permutation(instruction.code()).is_some()
        || scalar_insert(instruction.code()).is_some()
        || scalar_extract(instruction.code()).is_some()
        || integer_widen(instruction.code()).is_some()
        || shuffle_layout(instruction.code()).is_some()
        || broadcast_widths(instruction.code()).is_some()
        || binary_width(instruction.code()).is_some()
        || qword_comparison(instruction.code()).is_some()
        || saturating_packs::form(instruction.code()).is_some()
        || vector_shifts::form(instruction.code()).is_some()
        || mxcsr_control::store(instruction.code()).is_some()
        || x87_control::supported(instruction.code())
        || mask_layout(instruction.code()).is_some()
        || test_width(instruction.code()).is_some()
        || packed_vectors::supported(instruction.code())
        || matches!(
            instruction.code(),
            Code::VEX_Vzeroupper
                | Code::VEX_Vzeroall
                | Code::VEX_Vinsertps_xmm_xmm_xmmm32_imm8
                | Code::Phminposuw_xmm_xmmm128
                | Code::VEX_Vphminposuw_xmm_xmmm128
        )
}

struct PrecisionForm {
    input: usize,
    output: usize,
    lanes: usize,
    scalar: bool,
}
fn precision_conversion(code: Code) -> Option<PrecisionForm> {
    use Code::*;
    let (input, output, lanes, scalar) = match code {
        VEX_Vcvtsd2ss_xmm_xmm_xmmm64 => (8, 4, 1, true),
        VEX_Vcvtss2sd_xmm_xmm_xmmm32 => (4, 8, 1, true),
        VEX_Vcvtps2pd_xmm_xmmm64 => (4, 8, 2, false),
        VEX_Vcvtps2pd_ymm_xmmm128 => (4, 8, 4, false),
        VEX_Vcvtpd2ps_xmm_xmmm128 => (8, 4, 2, false),
        VEX_Vcvtpd2ps_xmm_ymmm256 => (8, 4, 4, false),
        _ => return None,
    };
    Some(PrecisionForm {
        input,
        output,
        lanes,
        scalar,
    })
}

// Scalar input bytes, integer result bytes and unconditional truncation.
fn float_integer_conversion(code: Code) -> Option<(usize, usize, bool)> {
    use Code::*;
    Some(match code {
        VEX_Vcvttss2si_r32_xmmm32 => (4, 4, true),
        VEX_Vcvttss2si_r64_xmmm32 => (4, 8, true),
        VEX_Vcvttsd2si_r32_xmmm64 => (8, 4, true),
        VEX_Vcvttsd2si_r64_xmmm64 => (8, 8, true),
        VEX_Vcvtss2si_r32_xmmm32 => (4, 4, false),
        VEX_Vcvtss2si_r64_xmmm32 => (4, 8, false),
        VEX_Vcvtsd2si_r32_xmmm64 => (8, 4, false),
        VEX_Vcvtsd2si_r64_xmmm64 => (8, 8, false),
        _ => return None,
    })
}

// Architectural integer input bytes and IEEE result bytes.
fn integer_conversion(code: Code) -> Option<(usize, usize)> {
    use Code::*;
    Some(match code {
        VEX_Vcvtsi2ss_xmm_xmm_rm32 => (4, 4),
        VEX_Vcvtsi2ss_xmm_xmm_rm64 => (8, 4),
        VEX_Vcvtsi2sd_xmm_xmm_rm32 => (4, 8),
        VEX_Vcvtsi2sd_xmm_xmm_rm64 => (8, 8),
        _ => return None,
    })
}

// Vector width, element width and offset within each independent 128-bit lane.
fn shuffle_layout(code: Code) -> Option<(usize, usize, usize)> {
    use Code::*;
    Some(match code {
        VEX_Vpshufd_xmm_xmmm128_imm8 => (16, 4, 0),
        VEX_Vpshufd_ymm_ymmm256_imm8 => (32, 4, 0),
        VEX_Vpshuflw_xmm_xmmm128_imm8 => (16, 2, 0),
        VEX_Vpshuflw_ymm_ymmm256_imm8 => (32, 2, 0),
        VEX_Vpshufhw_xmm_xmmm128_imm8 => (16, 2, 8),
        VEX_Vpshufhw_ymm_ymmm256_imm8 => (32, 2, 8),
        _ => return None,
    })
}

// Source tuple bytes and destination vector bytes. Decoder codes distinguish
// AVX memory-only forms from AVX2 register forms; EVEX/masks are not admitted.
fn broadcast_widths(code: Code) -> Option<(usize, usize)> {
    use Code::*;
    Some(match code {
        VEX_Vpbroadcastb_xmm_xmmm8 => (1, 16),
        VEX_Vpbroadcastb_ymm_xmmm8 => (1, 32),
        VEX_Vpbroadcastw_xmm_xmmm16 => (2, 16),
        VEX_Vpbroadcastw_ymm_xmmm16 => (2, 32),
        VEX_Vpbroadcastd_xmm_xmmm32 | VEX_Vbroadcastss_xmm_m32 | VEX_Vbroadcastss_xmm_xmm => {
            (4, 16)
        }
        VEX_Vpbroadcastd_ymm_xmmm32 | VEX_Vbroadcastss_ymm_m32 | VEX_Vbroadcastss_ymm_xmm => {
            (4, 32)
        }
        VEX_Vpbroadcastq_xmm_xmmm64 => (8, 16),
        VEX_Vpbroadcastq_ymm_xmmm64 | VEX_Vbroadcastsd_ymm_m64 | VEX_Vbroadcastsd_ymm_xmm => {
            (8, 32)
        }
        VEX_Vbroadcastf128_ymm_m128 | VEX_Vbroadcasti128_ymm_m128 => (16, 32),
        _ => return None,
    })
}

// Vector bytes and element bytes. MOVMSKPS/PD select the sign bit of each
// floating-point element without classifying it or changing MXCSR.
fn mask_layout(code: Code) -> Option<(usize, usize)> {
    match code {
        Code::VEX_Vpmovmskb_r32_xmm | Code::VEX_Vpmovmskb_r64_xmm => Some((16, 1)),
        Code::VEX_Vpmovmskb_r32_ymm | Code::VEX_Vpmovmskb_r64_ymm => Some((32, 1)),
        Code::VEX_Vmovmskps_r32_xmm | Code::VEX_Vmovmskps_r64_xmm => Some((16, 4)),
        Code::VEX_Vmovmskps_r32_ymm | Code::VEX_Vmovmskps_r64_ymm => Some((32, 4)),
        Code::VEX_Vmovmskpd_r32_xmm | Code::VEX_Vmovmskpd_r64_xmm => Some((16, 8)),
        Code::VEX_Vmovmskpd_r32_ymm | Code::VEX_Vmovmskpd_r64_ymm => Some((32, 8)),
        _ => None,
    }
}

fn equality_width(mnemonic: Mnemonic) -> Option<usize> {
    match mnemonic {
        Mnemonic::Vpcmpeqb => Some(1),
        Mnemonic::Vpcmpeqw => Some(2),
        Mnemonic::Vpcmpeqd => Some(4),
        Mnemonic::Vpcmpeqq => Some(8),
        _ => None,
    }
}

// Exact non-EVEX signed-qword comparisons: vector bytes and destructive legacy form.
fn qword_comparison(code: Code) -> Option<(usize, bool)> {
    match code {
        Code::Pcmpgtq_xmm_xmmm128 => Some((16, true)),
        Code::VEX_Vpcmpgtq_xmm_xmm_xmmm128 => Some((16, false)),
        Code::VEX_Vpcmpgtq_ymm_ymm_ymmm256 => Some((32, false)),
        _ => None,
    }
}

// Element bytes and selected half's offset within each independent 128-bit lane.
fn unpack_layout(mnemonic: Mnemonic) -> Option<(usize, usize)> {
    use Mnemonic::*;
    Some(match mnemonic {
        Vpunpcklbw => (1, 0),
        Vpunpckhbw => (1, 8),
        Vpunpcklwd => (2, 0),
        Vpunpckhwd => (2, 8),
        Vpunpckldq | Vunpcklps => (4, 0),
        Vpunpckhdq | Vunpckhps => (4, 8),
        Vpunpcklqdq | Vunpcklpd => (8, 0),
        Vpunpckhqdq | Vunpckhpd => (8, 8),
        _ => return None,
    })
}

// Element bytes, signed ordering and maximum rather than minimum selection.
fn minmax_layout(mnemonic: Mnemonic) -> Option<(usize, bool, bool)> {
    use Mnemonic::*;
    Some(match mnemonic {
        Vpminsb => (1, true, false),
        Vpminsw => (2, true, false),
        Vpminsd => (4, true, false),
        Vpminub => (1, false, false),
        Vpminuw => (2, false, false),
        Vpminud => (4, false, false),
        Vpmaxsb => (1, true, true),
        Vpmaxsw => (2, true, true),
        Vpmaxsd => (4, true, true),
        Vpmaxub => (1, false, true),
        Vpmaxuw => (2, false, true),
        Vpmaxud => (4, false, true),
        _ => return None,
    })
}

fn arithmetic_width(mnemonic: Mnemonic) -> Option<(usize, bool)> {
    use Mnemonic::*;
    Some(match mnemonic {
        Vpaddb => (1, false),
        Vpaddw => (2, false),
        Vpaddd => (4, false),
        Vpaddq => (8, false),
        Vpsubb => (1, true),
        Vpsubw => (2, true),
        Vpsubd => (4, true),
        Vpsubq => (8, true),
        _ => return None,
    })
}

// Input bytes, signed product, high-half selection, widening even-dword form.
fn multiply_layout(mnemonic: Mnemonic) -> Option<(usize, bool, bool, bool)> {
    use Mnemonic::*;
    Some(match mnemonic {
        Vpmullw => (2, false, false, false),
        Vpmulhw => (2, true, true, false),
        Vpmulhuw => (2, false, true, false),
        Vpmulld => (4, false, false, false),
        Vpmuludq => (4, false, false, true),
        Vpmuldq => (4, true, false, true),
        _ => return None,
    })
}

// Element bytes and whether the immediate mask repeats in each 128-bit lane.
fn immediate_blend(mnemonic: Mnemonic) -> Option<(usize, bool)> {
    use Mnemonic::*;
    match mnemonic {
        Vpblendd | Vblendps => Some((4, false)),
        Vblendpd => Some((8, false)),
        Vpblendw => Some((2, true)),
        _ => None,
    }
}

fn variable_blend(mnemonic: Mnemonic) -> Option<usize> {
    use Mnemonic::*;
    match mnemonic {
        Vpblendvb => Some(1),
        Vblendvps => Some(4),
        Vblendvpd => Some(8),
        _ => None,
    }
}

fn test_width(code: Code) -> Option<usize> {
    match code {
        Code::VEX_Vptest_xmm_xmmm128
        | Code::VEX_Vtestps_xmm_xmmm128
        | Code::VEX_Vtestpd_xmm_xmmm128 => Some(16),
        Code::VEX_Vptest_ymm_ymmm256
        | Code::VEX_Vtestps_ymm_ymmm256
        | Code::VEX_Vtestpd_ymm_ymmm256 => Some(32),
        _ => None,
    }
}

impl NativeBlockStepper<'_, '_, '_> {
    pub(super) fn emulate_vector(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, '_, '_>,
    ) -> Result<NativeBlockStop, NativeDispatchError> {
        let instruction = block
            .exit()
            .instruction()
            .ok_or(NativeDispatchError::UnsupportedVectorInstruction)?;
        if !supported(instruction)
            || instruction.has_segment_prefix()
            || instruction.has_rep_prefix()
            || instruction.has_repne_prefix()
            || instruction.has_lock_prefix()
            || instruction.has_xacquire_prefix()
            || instruction.has_xrelease_prefix()
        {
            return Err(NativeDispatchError::UnsupportedVectorInstruction);
        }
        charge(
            &mut self.machine.usage.work_units,
            self.machine
                .target_visits
                .checked_add(640)
                .ok_or(NativeDispatchError::Budget("vector work"))?,
            self.machine.limits.max_work_units,
            "vector work",
        )?;
        let next_pc = block.exit().next_guest_pc();
        check_target(self.source, memory, next_pc)?;
        let operations = self
            .machine
            .usage
            .vector_operations
            .checked_add(1)
            .ok_or(NativeDispatchError::Budget("vector operations"))?;
        let mut next = self.machine.state;
        let mut transferred = 0;
        if x87_control::supported(instruction.code()) {
            transferred = x87_control::execute(
                instruction,
                &mut next,
                memory,
                self.machine.usage.vector_bytes,
            )?;
        } else if let Some(store) = mxcsr_control::store(instruction.code()) {
            if instruction.op_count() != 1
                || instruction.op0_kind() != OpKind::Memory
                || instruction.memory_size().size() != 4
            {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            transferred = 4;
            self.machine
                .usage
                .vector_bytes
                .checked_add(transferred)
                .ok_or(NativeDispatchError::Budget("vector bytes"))?;
            let address = address(instruction, 0, &next)?;
            if store {
                memory.write(address, &next.mxcsr.to_le_bytes())?;
            } else {
                let mut bytes = [0; 4];
                memory.read(address, &mut bytes)?;
                let control = u32::from_le_bytes(bytes);
                if !mxcsr_control::valid(control) {
                    return Err(NativeDispatchError::InvalidState);
                }
                next.mxcsr = control;
            }
        } else if matches!(
            instruction.code(),
            Code::Phminposuw_xmm_xmmm128 | Code::VEX_Vphminposuw_xmm_xmmm128
        ) {
            if instruction.op_count() != 2 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, 16)?;
            let legacy = instruction.code() == Code::Phminposuw_xmm_xmmm128;
            let source = if instruction.op1_kind() == OpKind::Memory {
                if instruction.memory_size().size() != 16 {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                let source_address = address(instruction, 1, &next)?;
                // Legacy Type 4 memory requires alignment; VEX permits unaligned reads.
                if legacy && source_address & 15 != 0 {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = 16;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                // Read every source word even when the first one is already zero.
                memory.read(source_address, &mut bytes[..16])?;
                bytes
            } else {
                vector_register(instruction, 1, 16, &next)?
            };
            let mut minimum = u16::from_le_bytes([source[0], source[1]]);
            let mut index = 0_u16;
            for lane in 1..8 {
                let value = u16::from_le_bytes([source[2 * lane], source[2 * lane + 1]]);
                if value < minimum {
                    minimum = value;
                    index = lane as u16;
                }
            }
            let mut output = [0; 16];
            output[..2].copy_from_slice(&minimum.to_le_bytes());
            output[2..4].copy_from_slice(&index.to_le_bytes());
            next.xmm[destination..destination + 16].copy_from_slice(&output);
            if !legacy {
                next.ymm_upper[destination..destination + 16].fill(0);
            }
        } else if let Some(form) = saturating_packs::form(instruction.code()) {
            let size = form.vector_bytes;
            let right_operand = if form.legacy { 1 } else { 2 };
            if instruction.op_count() != right_operand + 1 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, size)?;
            let left = vector_register(instruction, if form.legacy { 0 } else { 1 }, size, &next)?;
            let right = if instruction.op_kind(right_operand) == OpKind::Memory {
                if instruction.memory_size().size() != size {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                let source_address = address(instruction, right_operand, &next)?;
                // All four pack families use Intel Type4 exceptions: only
                // the legacy m128 encoding requires sixteen-byte alignment.
                if form.legacy && source_address & 15 != 0 {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = size as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                memory.read(source_address, &mut bytes[..size])?;
                bytes
            } else {
                vector_register(instruction, right_operand, size, &next)?
            };
            let output = saturating_packs::calculate(form, &left, &right);
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            if !form.legacy {
                next.ymm_upper[destination..destination + 16].copy_from_slice(&output[16..]);
            }
        } else if let Some((size, legacy)) = qword_comparison(instruction.code()) {
            let right_operand = if legacy { 1 } else { 2 };
            if instruction.op_count() != right_operand + 1 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, size)?;
            // Legacy DEST is also SRC1. Both sources are fully captured before
            // any aliased destination is changed, including identical operands.
            let left = vector_register(instruction, if legacy { 0 } else { 1 }, size, &next)?;
            let right = if instruction.op_kind(right_operand) == OpKind::Memory {
                if instruction.memory_size().size() != size {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                let source_address = address(instruction, right_operand, &next)?;
                // Intel Type4: legacy requires aligned m128; VEX may be unaligned.
                if legacy && source_address & 15 != 0 {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = size as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                memory.read(source_address, &mut bytes[..size])?;
                bytes
            } else {
                vector_register(instruction, right_operand, size, &next)?
            };
            let mut output = [0; 32];
            for start in (0..size).step_by(8) {
                let a = i64::from_le_bytes(left[start..start + 8].try_into().unwrap());
                let b = i64::from_le_bytes(right[start..start + 8].try_into().unwrap());
                output[start..start + 8].fill(if a > b { 0xff } else { 0 });
            }
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            if !legacy {
                // Initialized output zeros upper YMM for VEX.128; VEX.256
                // replaces all four signed-qword lanes independently.
                next.ymm_upper[destination..destination + 16].copy_from_slice(&output[16..]);
            }
        } else if let Some(form) = mask_comparisons::form(instruction.code()) {
            if instruction.op_count() != 4 || instruction.op3_kind() != OpKind::Immediate8 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, form.vector_bytes)?;
            // Snapshot both inputs before publishing any overlapping destination.
            // Scalar forms merge SRC1's remaining low128 bits, not old DEST.
            let left = vector_register(instruction, 1, form.vector_bytes, &next)?;
            let width = if form.scalar {
                form.element_bytes
            } else {
                form.vector_bytes
            };
            let right = if instruction.op2_kind() == OpKind::Memory {
                if instruction.memory_size().size() != width {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = width as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                memory.read(address(instruction, 2, &next)?, &mut bytes[..width])?;
                bytes
            } else {
                vector_register(instruction, 2, form.vector_bytes, &next)?
            };
            let (output, control) = mask_comparisons::calculate(
                form,
                &left,
                &right,
                instruction.immediate8(),
                next.mxcsr,
            )?;
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            next.ymm_upper[destination..destination + 16].copy_from_slice(&output[16..]);
            next.mxcsr = control;
            // RFLAGS and all unrelated state remain unchanged. The local
            // snapshot commits only after complete operand/budget admission.
        } else if let Some((width, ordered)) = float_ops::comparison_form(instruction.code()) {
            if instruction.op_count() != 2 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let left = vector_register(instruction, 0, 16, &next)?;
            let right = if instruction.op1_kind() == OpKind::Memory {
                if instruction.memory_size().size() != width {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = width as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                memory.read(address(instruction, 1, &next)?, &mut bytes[..width])?;
                bytes
            } else {
                vector_register(instruction, 1, 16, &next)?
            };
            let (flags, control) = float_ops::compare(
                ordered,
                width == 8,
                u64::from_le_bytes(left[..8].try_into().unwrap()),
                u64::from_le_bytes(right[..8].try_into().unwrap()),
                next.mxcsr,
            )?;
            next.rflags = (next.rflags & !0x8d5) | flags;
            next.mxcsr = control;
        } else if let Some(form) = fused_float::form(instruction.code()) {
            if instruction.op_count() != 3 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, form.vector_bytes)?;
            let prior = vector_register(instruction, 0, form.vector_bytes, &next)?;
            let second = vector_register(instruction, 1, form.vector_bytes, &next)?;
            let width = if form.scalar {
                form.element_bytes
            } else {
                form.vector_bytes
            };
            charge(
                &mut self.machine.usage.work_units,
                (width / form.element_bytes) as u64 * 66 * 16,
                self.machine.limits.max_work_units,
                "fused arithmetic work",
            )?;
            let third = if instruction.op2_kind() == OpKind::Memory {
                if instruction.memory_size().size() != width {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = width as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                memory.read(address(instruction, 2, &next)?, &mut bytes[..width])?;
                bytes
            } else {
                vector_register(instruction, 2, form.vector_bytes, &next)?
            };
            // Scalar FMA retains DEST's other low128 bits, unlike ordinary
            // three-operand scalar arithmetic. All inputs are captured first.
            let mut output = if form.scalar { prior } else { [0; 32] };
            output[16..].fill(0);
            for (lane, offset) in (0..width).step_by(form.element_bytes).enumerate() {
                let mut inputs = [0_u64; 3];
                for (value, source) in inputs.iter_mut().zip([&prior, &second, &third]) {
                    let mut bytes = [0; 8];
                    bytes[..form.element_bytes]
                        .copy_from_slice(&source[offset..offset + form.element_bytes]);
                    *value = u64::from_le_bytes(bytes);
                }
                let [x, y, z] = form.operands(inputs[0], inputs[1], inputs[2]);
                let (neg_product, neg_addend) = form.operation.signs(lane);
                let (bits, control) = fused_float::calculate(
                    form.element_bytes == 8,
                    x,
                    y,
                    z,
                    neg_product,
                    neg_addend,
                    next.mxcsr,
                )?;
                output[offset..offset + form.element_bytes]
                    .copy_from_slice(&bits.to_le_bytes()[..form.element_bytes]);
                next.mxcsr = control;
            }
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            next.ymm_upper[destination..destination + 16].copy_from_slice(&output[16..]);
        } else if let Some(form) = float_ops::form(instruction.code()) {
            let unary = !form.scalar && form.operation == float_ops::Operation::Sqrt;
            let operand = if unary { 1 } else { 2 };
            if instruction.op_count() != operand + 1 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, form.vector_bytes)?;
            let left = if unary {
                [0; 32]
            } else {
                vector_register(instruction, 1, form.vector_bytes, &next)?
            };
            let width = if form.scalar {
                form.element_bytes
            } else {
                form.vector_bytes
            };
            let right = if instruction.op_kind(operand) == OpKind::Memory {
                if instruction.memory_size().size() != width {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = width as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                memory.read(address(instruction, operand, &next)?, &mut bytes[..width])?;
                bytes
            } else {
                vector_register(instruction, operand, form.vector_bytes, &next)?
            };
            let mut output = left;
            for offset in (0..width).step_by(form.element_bytes) {
                let mut a = [0; 8];
                let mut b = [0; 8];
                a[..form.element_bytes].copy_from_slice(&left[offset..offset + form.element_bytes]);
                b[..form.element_bytes]
                    .copy_from_slice(&right[offset..offset + form.element_bytes]);
                let (bits, control) = float_ops::calculate(
                    form.operation,
                    form.element_bytes == 8,
                    u64::from_le_bytes(a),
                    u64::from_le_bytes(b),
                    next.mxcsr,
                )?;
                output[offset..offset + form.element_bytes]
                    .copy_from_slice(&bits.to_le_bytes()[..form.element_bytes]);
                next.mxcsr = control;
            }
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            next.ymm_upper[destination..destination + 16].copy_from_slice(&output[16..]);
        } else if let Some(form) = packed_convert::form(instruction.code()) {
            if instruction.op_count() != 2 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let input_size = form.input_element * form.lanes;
            let output_size = form.output_element * form.lanes;
            let destination = register_slot(instruction, 0, output_size.max(16))?;
            // Snapshot the complete architectural source before an aliased
            // destination changes. DQ->PD.128 reads only m64, while its
            // register source is XMM; PD->DQ.256 reads YMM and writes XMM.
            let input = if instruction.op1_kind() == OpKind::Memory {
                if instruction.memory_size().size() != input_size {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = input_size as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                memory.read(address(instruction, 1, &next)?, &mut bytes[..input_size])?;
                bytes
            } else {
                vector_register(instruction, 1, input_size.max(16), &next)?
            };
            let (output, control) = packed_convert::calculate(form, &input, next.mxcsr)?;
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            next.ymm_upper[destination..destination + 16].copy_from_slice(&output[16..]);
            next.mxcsr = control;
        } else if let Some(form) = precision_conversion(instruction.code()) {
            let operand = if form.scalar { 2 } else { 1 };
            let input_size = form.input * form.lanes;
            if instruction.op_count() != operand + 1 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, (form.output * form.lanes).max(16))?;
            let mut output = if form.scalar {
                vector_register(instruction, 1, 16, &next)?
            } else {
                [0; 32]
            };
            let input = if instruction.op_kind(operand) == OpKind::Memory {
                if instruction.memory_size().size() != input_size {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = input_size as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                memory.read(
                    address(instruction, operand, &next)?,
                    &mut bytes[..input_size],
                )?;
                bytes
            } else {
                vector_register(instruction, operand, input_size.max(16), &next)?
            };
            for lane in 0..form.lanes {
                let mut raw = [0; 8];
                let at = lane * form.input;
                raw[..form.input].copy_from_slice(&input[at..at + form.input]);
                let (bits, control) = float_ops::calculate(
                    float_ops::Operation::Convert,
                    form.output == 8,
                    0,
                    u64::from_le_bytes(raw),
                    next.mxcsr,
                )?;
                let at = lane * form.output;
                output[at..at + form.output].copy_from_slice(&bits.to_le_bytes()[..form.output]);
                next.mxcsr = control;
            }
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            next.ymm_upper[destination..destination + 16].copy_from_slice(&output[16..]);
        } else if let Some((input_size, output_size)) = integer_conversion(instruction.code()) {
            if instruction.op_count() != 3 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, 16)?;
            let mut output = vector_register(instruction, 1, 16, &next)?;
            let raw = if instruction.op2_kind() == OpKind::Memory {
                if instruction.memory_size().size() != input_size {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = input_size as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 8];
                memory.read(address(instruction, 2, &next)?, &mut bytes[..input_size])?;
                u64::from_le_bytes(bytes)
            } else {
                next.gpr[integer_slot(instruction, 2, input_size)?]
            };
            let value = if input_size == 4 {
                i64::from(raw as i32)
            } else {
                raw as i64
            };
            let (bits, inexact) =
                scalar_convert::signed_bits(value, output_size == 8, next.mxcsr >> 13);
            output[..output_size].copy_from_slice(&bits.to_le_bytes()[..output_size]);
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            next.ymm_upper[destination..destination + 16].fill(0);
            next.mxcsr |= u32::from(inexact) << 5;
        } else if let Some((input_size, output_size, truncate)) =
            float_integer_conversion(instruction.code())
        {
            if instruction.op_count() != 2 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            if !mxcsr_control::valid(next.mxcsr) {
                return Err(NativeDispatchError::InvalidState);
            }
            let destination = integer_slot(instruction, 0, output_size)?;
            let raw = if instruction.op1_kind() == OpKind::Memory {
                if instruction.memory_size().size() != input_size {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = input_size as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 8];
                memory.read(address(instruction, 1, &next)?, &mut bytes[..input_size])?;
                u64::from_le_bytes(bytes)
            } else {
                let input = vector_register(instruction, 1, 16, &next)?;
                u64::from_le_bytes(input[..8].try_into().unwrap())
            };
            let (value, exceptions) = scalar_convert::to_signed_bits(
                raw,
                input_size == 8,
                output_size == 8,
                if truncate { 3 } else { next.mxcsr >> 13 },
                next.mxcsr & (1 << 6) != 0,
            );
            if destination == RSP {
                check_rsp(&self.machine.stack, value)?;
            }
            next.gpr[destination] = value;
            next.mxcsr |= exceptions;
        } else if packed_vectors::supported(instruction.code()) {
            let left = vector_register(instruction, 0, 16, &next)?;
            let right = if instruction.op1_kind() == OpKind::Memory {
                if instruction.memory_size().size() != 16 {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = 16;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                // Explicit zero lengths and early NULs do not suppress the
                // architectural full-width read or its permission checks.
                memory.read(address(instruction, 1, &next)?, &mut bytes[..16])?;
                bytes
            } else {
                vector_register(instruction, 1, 16, &next)?
            };
            packed_vectors::apply(instruction, &mut next, &left, &right)?;
        } else if instruction.code() == Code::VEX_Vinsertps_xmm_xmm_xmmm32_imm8 {
            if instruction.op_count() != 4 || instruction.op3_kind() != OpKind::Immediate8 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, 16)?;
            let mut output = vector_register(instruction, 1, 16, &next)?;
            let control = instruction.immediate8();
            let input = if instruction.op2_kind() == OpKind::Memory {
                if instruction.memory_size().size() != 4 {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = 4;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 4];
                // Memory always supplies one scalar, ignoring the source selector.
                // The read remains required even if the zero mask clears it later.
                memory.read(address(instruction, 2, &next)?, &mut bytes)?;
                bytes
            } else {
                let source = vector_register(instruction, 2, 16, &next)?;
                let from = usize::from(control >> 6) * 4;
                source[from..from + 4].try_into().unwrap()
            };
            let to = usize::from((control >> 4) & 3) * 4;
            output[to..to + 4].copy_from_slice(&input);
            for item in 0..4 {
                if control & (1 << item) != 0 {
                    output[item * 4..item * 4 + 4].fill(0);
                }
            }
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            next.ymm_upper[destination..destination + 16].fill(0);
        } else if let Some(size) = scalar_extract(instruction.code()) {
            if instruction.op_count() != 3 || instruction.op2_kind() != OpKind::Immediate8 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let source = vector_register(instruction, 1, 16, &next)?;
            let from = (usize::from(instruction.immediate8()) & (16 / size - 1)) * size;
            let mut bytes = [0; 8];
            bytes[..size].copy_from_slice(&source[from..from + size]);
            if instruction.op0_kind() == OpKind::Memory {
                if instruction.memory_size().size() != size {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = size as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                memory.write(address(instruction, 0, &next)?, &bytes[..size])?;
            } else {
                let destination = integer_slot(instruction, 0, instruction.op0_register().size())?;
                // Every integer extraction zero-extends a register destination,
                // including byte/word forms and WIG 64-bit decoder spellings.
                let value = u64::from_le_bytes(bytes);
                if destination == RSP {
                    check_rsp(&self.machine.stack, value)?;
                }
                next.gpr[destination] = value;
            }
        } else if let Some((size, input_width, output_width, signed)) =
            integer_widen(instruction.code())
        {
            if instruction.op_count() != 2 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, size)?;
            let input_size = size / output_width * input_width;
            let input = if instruction.op1_kind() == OpKind::Memory {
                if instruction.memory_size().size() != input_size {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = input_size as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut input = [0; 32];
                // Only the narrow tuple is accessed, even for a 256-bit result.
                memory.read(address(instruction, 1, &next)?, &mut input[..input_size])?;
                input
            } else {
                vector_register(instruction, 1, 16, &next)?
            };
            let mut output = [0; 32];
            for (index, element) in output[..size].chunks_exact_mut(output_width).enumerate() {
                let start = index * input_width;
                let source = &input[start..start + input_width];
                if signed && source[input_width - 1] & 0x80 != 0 {
                    element.fill(0xff);
                }
                element[..input_width].copy_from_slice(source);
            }
            // Snapshot precedes publication when source and destination alias.
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            next.ymm_upper[destination..destination + 16].copy_from_slice(&output[16..]);
        } else if let Some((size, element, offset, memory_size)) =
            duplicate_layout(instruction.code())
        {
            if instruction.op_count() != 2 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, size)?;
            let input = if instruction.op1_kind() == OpKind::Memory {
                if instruction.memory_size().size() != memory_size {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = memory_size as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                memory.read(address(instruction, 1, &next)?, &mut bytes[..memory_size])?;
                bytes
            } else {
                vector_register(instruction, 1, size, &next)?
            };
            let mut output = [0; 32];
            for pair in (0..size).step_by(2 * element) {
                let value = &input[pair + offset..pair + offset + element];
                output[pair..pair + element].copy_from_slice(value);
                output[pair + element..pair + 2 * element].copy_from_slice(value);
            }
            // Snapshot before publication handles aliases. These are bit copies:
            // no FP evaluation, MXCSR update or flags change; VEX.128 zeros YMM.HI.
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            next.ymm_upper[destination..destination + 16].copy_from_slice(&output[16..]);
        } else if let Some((size, element, offset)) = shuffle_layout(instruction.code()) {
            if instruction.op_count() != 3 || instruction.op2_kind() != OpKind::Immediate8 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, size)?;
            let input = if instruction.op1_kind() == OpKind::Memory {
                if instruction.memory_size().size() != size {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = size as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                // Selection does not suppress any part of the architectural
                // source read, even when all output elements select one word.
                memory.read(address(instruction, 1, &next)?, &mut bytes[..size])?;
                bytes
            } else {
                vector_register(instruction, 1, size, &next)?
            };
            let mut output = input;
            for lane in (0..size).step_by(16) {
                for item in 0..4 {
                    let from = lane
                        + offset
                        + usize::from((instruction.immediate8() >> (item * 2)) & 3) * element;
                    let to = lane + offset + item * element;
                    output[to..to + element].copy_from_slice(&input[from..from + element]);
                }
            }
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            // vector_register and memory buffers are zero above a128-bit source.
            next.ymm_upper[destination..destination + 16].copy_from_slice(&output[16..]);
        } else if let Some((tuple, size)) = broadcast_widths(instruction.code()) {
            if instruction.op_count() != 2 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, size)?;
            let input = if instruction.op1_kind() == OpKind::Memory {
                if instruction.memory_size().size() != tuple {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = tuple as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                // The scalar/tuple may end at the last readable byte of a page.
                // Reading a whole vector here would introduce a false fault.
                memory.read(address(instruction, 1, &next)?, &mut bytes[..tuple])?;
                bytes
            } else {
                vector_register(instruction, 1, 16, &next)?
            };
            let mut output = [0; 32];
            for lane in output[..size].chunks_exact_mut(tuple) {
                lane.copy_from_slice(&input[..tuple]);
            }
            // Source snapshot precedes publication even for an aliased register.
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            next.ymm_upper[destination..destination + 16].copy_from_slice(&output[16..]);
        } else if let Some((size, register_size)) = scalar_insert(instruction.code()) {
            if instruction.op_count() != 4 || instruction.op3_kind() != OpKind::Immediate8 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, 16)?;
            let mut output = vector_register(instruction, 1, 16, &next)?;
            let mut input = [0; 8];
            if instruction.op2_kind() == OpKind::Memory {
                if instruction.memory_size().size() != size {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = size as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                memory.read(address(instruction, 2, &next)?, &mut input[..size])?;
            } else {
                input = next.gpr[integer_slot(instruction, 2, register_size)?].to_le_bytes();
            }
            let offset = (usize::from(instruction.immediate8()) & (16 / size - 1)) * size;
            output[offset..offset + size].copy_from_slice(&input[..size]);
            // Snapshot both inputs before publishing an aliased destination.
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            next.ymm_upper[destination..destination + 16].fill(0);
        } else if lane_extract(instruction.code()) {
            if instruction.op_count() != 3 || instruction.op2_kind() != OpKind::Immediate8 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            // Capture the whole source before publishing a register alias.
            // Only bit zero selects the half; other immediate bits are ignored.
            let source = vector_register(instruction, 1, 32, &next)?;
            let offset = usize::from(instruction.immediate8() & 1) * 16;
            if instruction.op0_kind() == OpKind::Memory {
                if instruction.memory_size().size() != 16 {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = 16;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                memory.write(
                    address(instruction, 0, &next)?,
                    &source[offset..offset + 16],
                )?;
            } else {
                let destination = register_slot(instruction, 0, 16)?;
                next.xmm[destination..destination + 16]
                    .copy_from_slice(&source[offset..offset + 16]);
                next.ymm_upper[destination..destination + 16].fill(0);
            }
        } else if lane_insert(instruction.code()) {
            if instruction.op_count() != 4 || instruction.op3_kind() != OpKind::Immediate8 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, 32)?;
            let mut output = vector_register(instruction, 1, 32, &next)?;
            let input = if instruction.op2_kind() == OpKind::Memory {
                if instruction.memory_size().size() != 16 {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = 16;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                memory.read(address(instruction, 2, &next)?, &mut bytes[..16])?;
                bytes
            } else {
                vector_register(instruction, 2, 16, &next)?
            };
            // Only bit zero selects the lane; all other immediate bits are ignored.
            let offset = usize::from(instruction.immediate8() & 1) * 16;
            output[offset..offset + 16].copy_from_slice(&input[..16]);
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            next.ymm_upper[destination..destination + 16].copy_from_slice(&output[16..]);
        } else if let Some(form) = half_move(instruction.code()) {
            match form {
                HalfMove::Merge { to, from } => {
                    if instruction.op_count() != 3 {
                        return Err(NativeDispatchError::UnsupportedVectorInstruction);
                    }
                    let destination = register_slot(instruction, 0, 16)?;
                    let mut output = vector_register(instruction, 1, 16, &next)?;
                    let input = if instruction.op2_kind() == OpKind::Memory {
                        if instruction.memory_size().size() != 8 || from != 0 {
                            return Err(NativeDispatchError::UnsupportedVectorInstruction);
                        }
                        transferred = 8;
                        self.machine
                            .usage
                            .vector_bytes
                            .checked_add(transferred)
                            .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                        let mut bytes = [0; 32];
                        memory.read(address(instruction, 2, &next)?, &mut bytes[..8])?;
                        bytes
                    } else {
                        vector_register(instruction, 2, 16, &next)?
                    };
                    // Capture both inputs before modifying any aliased destination.
                    output[to..to + 8].copy_from_slice(&input[from..from + 8]);
                    next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
                    next.ymm_upper[destination..destination + 16].fill(0);
                }
                HalfMove::Store { from } => {
                    if instruction.op_count() != 2
                        || instruction.op0_kind() != OpKind::Memory
                        || instruction.memory_size().size() != 8
                    {
                        return Err(NativeDispatchError::UnsupportedVectorInstruction);
                    }
                    let input = vector_register(instruction, 1, 16, &next)?;
                    transferred = 8;
                    self.machine
                        .usage
                        .vector_bytes
                        .checked_add(transferred)
                        .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                    // Complete-span validation precedes any guest memory mutation.
                    memory.write(address(instruction, 0, &next)?, &input[from..from + 8])?;
                }
            }
        } else if let Some(size) = merge_width(instruction.code()) {
            if instruction.op_count() != 3 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, 16)?;
            // Snapshot both sources before publishing: either may alias the
            // destination. These are bit transfers, without FP evaluation.
            let mut output = vector_register(instruction, 1, 16, &next)?;
            let low = vector_register(instruction, 2, 16, &next)?;
            output[..size].copy_from_slice(&low[..size]);
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            next.ymm_upper[destination..destination + 16].fill(0);
        } else if let Some(size) = move_width(instruction.code()) {
            if instruction.op_count() != 2 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let load = instruction.op1_kind() == OpKind::Memory;
            let store = instruction.op0_kind() == OpKind::Memory;
            let address = if load || store {
                if load && store || instruction.memory_size().size() != size {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                let address = address(instruction, u32::from(load), &next)?;
                if matches!(
                    instruction.mnemonic(),
                    Mnemonic::Vmovaps
                        | Mnemonic::Vmovapd
                        | Mnemonic::Vmovdqa
                        | Mnemonic::Vmovntdq
                        | Mnemonic::Vmovntdqa
                ) && address % size as u64 != 0
                {
                    return Err(NativeDispatchError::VectorAlignment);
                }
                transferred = size as u64;
                address
            } else {
                0
            };
            // Reserve all success counters before any memory write or state change.
            self.machine
                .usage
                .vector_bytes
                .checked_add(transferred)
                .ok_or(NativeDispatchError::Budget("vector bytes"))?;
            let mut bytes = [0; 32];
            if load {
                memory.read(address, &mut bytes[..size])?;
            } else if instruction.op1_register().is_gpr() {
                let slot = integer_slot(instruction, 1, size)?;
                bytes[..size].copy_from_slice(&next.gpr[slot].to_le_bytes()[..size]);
            } else {
                let slot = register_slot(instruction, 1, size.max(16))?;
                let lower = size.min(16);
                bytes[..lower].copy_from_slice(&next.xmm[slot..slot + lower]);
                if size == 32 {
                    bytes[16..].copy_from_slice(&next.ymm_upper[slot..slot + 16]);
                }
            }
            if store {
                // Paused memory checks the complete span before copying any byte.
                // VMOVNTDQ's non-temporal cache hint does not change its bytes or
                // source registers. The serialized runtime completes this store
                // before resuming a guest thread or submitting GPU work.
                memory.write(address, &bytes[..size])?;
            } else if instruction.op0_register().is_gpr() {
                let slot = integer_slot(instruction, 0, size)?;
                // The zero-filled buffer also implements a 32-bit GPR write's
                // zero extension. VMOVD/Q leave flags and their source intact.
                let value = u64::from_le_bytes(bytes[..8].try_into().unwrap());
                if slot == RSP {
                    check_rsp(&self.machine.stack, value)?;
                }
                next.gpr[slot] = value;
            } else {
                let slot = register_slot(instruction, 0, size.max(16))?;
                next.xmm[slot..slot + 16].copy_from_slice(&bytes[..16]);
                // Scalar moves clear every destination bit above their width.
                // VEX.128 clears the upper half; VEX.256 copies it. Legacy SSE
                // continues to update only xmm through the existing native bridge.
                next.ymm_upper[slot..slot + 16].copy_from_slice(&bytes[16..]);
            }
        } else if let Some(form) = vector_shifts::form(instruction.code()) {
            if instruction.op_count() != 3 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, form.size)?;
            let input = vector_register(instruction, 1, form.size, &next)?;
            let width = form.count_bytes();
            let counts = if width == 0 {
                if instruction.op2_kind() != OpKind::Immediate8 {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                let mut bytes = [0; 32];
                bytes[0] = instruction.immediate8();
                bytes
            } else if instruction.op2_kind() == OpKind::Memory {
                if instruction.memory_size().size() != width {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = width as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                memory.read(address(instruction, 2, &next)?, &mut bytes[..width])?;
                bytes
            } else {
                vector_register(instruction, 2, width, &next)?
            };
            let output = form.calculate(&input, &counts);
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            next.ymm_upper[destination..destination + 16].copy_from_slice(&output[16..]);
        } else if let Some((size, element, immediate)) = element_permutation(instruction.code()) {
            if instruction.op_count() != 3
                || immediate && instruction.op2_kind() != OpKind::Immediate8
            {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, size)?;
            let left = if immediate {
                [0; 32]
            } else {
                vector_register(instruction, 1, size, &next)?
            };
            let source_operand = if immediate { 1 } else { 2 };
            let right = if instruction.op_kind(source_operand) == OpKind::Memory {
                if instruction.memory_size().size() != size {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = size as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                // Immediate forms read data; variable forms read controls.
                // Repeated selections never suppress the full source read.
                memory.read(
                    address(instruction, source_operand, &next)?,
                    &mut bytes[..size],
                )?;
                bytes
            } else {
                vector_register(instruction, source_operand, size, &next)?
            };
            let data = if immediate { right } else { left };
            let mut output = [0; 32];
            for start in (0..size).step_by(element) {
                let selected = if immediate {
                    let shift = if element == 4 {
                        (start % 16) / 2
                    } else {
                        start / 8
                    };
                    usize::from(
                        (instruction.immediate8() >> shift) & if element == 4 { 3 } else { 1 },
                    )
                } else {
                    // PS uses bits1:0 of each control dword; PD uses bit1 of
                    // each control qword, ignoring bit0 and every higher bit.
                    usize::from(if element == 4 {
                        right[start] & 3
                    } else {
                        (right[start] >> 1) & 1
                    })
                };
                let from = (start & !15) + selected * element;
                output[start..start + element].copy_from_slice(&data[from..from + element]);
            }
            // Sources are fully captured before a data/control alias changes.
            // Initialized output clears upper YMM for a 128-bit destination.
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            next.ymm_upper[destination..destination + 16].copy_from_slice(&output[16..]);
        } else if let Some(form) = immediate_permutation(instruction.code()) {
            let source_operand = if matches!(form, ImmediatePermutation::Halves) {
                2
            } else {
                1
            };
            if instruction.op_count() != source_operand + 2
                || instruction.op_kind(source_operand + 1) != OpKind::Immediate8
            {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, 32)?;
            let left = if matches!(form, ImmediatePermutation::Halves) {
                vector_register(instruction, 1, 32, &next)?
            } else {
                [0; 32]
            };
            let right = if instruction.op_kind(source_operand) == OpKind::Memory {
                if instruction.memory_size().size() != 32 {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = 32;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                // Zero overrides and repeated selections do not suppress the
                // complete architectural source read or its permission checks.
                memory.read(address(instruction, source_operand, &next)?, &mut bytes)?;
                bytes
            } else {
                vector_register(instruction, source_operand, 32, &next)?
            };
            // Both inputs are captured before any aliased destination changes.
            let mut output = [0; 32];
            let immediate = instruction.immediate8();
            match form {
                ImmediatePermutation::Halves => {
                    for lane in 0..2 {
                        let control = immediate >> (lane * 4);
                        if control & 8 == 0 {
                            let selected = usize::from(control & 3);
                            let source = if selected < 2 { &left } else { &right };
                            let start = (selected & 1) * 16;
                            output[lane * 16..(lane + 1) * 16]
                                .copy_from_slice(&source[start..start + 16]);
                        }
                    }
                }
                ImmediatePermutation::Quadwords => {
                    for lane in 0..4 {
                        let start = usize::from((immediate >> (lane * 2)) & 3) * 8;
                        output[lane * 8..(lane + 1) * 8].copy_from_slice(&right[start..start + 8]);
                    }
                }
            }
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            next.ymm_upper[destination..destination + 16].copy_from_slice(&output[16..]);
        } else if let Some(size) = binary_width(instruction.code()) {
            let immediate_shuffle = matches!(
                instruction.mnemonic(),
                Mnemonic::Vshufps | Mnemonic::Vshufpd
            );
            let blend = immediate_blend(instruction.mnemonic());
            let variable = variable_blend(instruction.mnemonic());
            let immediate = immediate_shuffle || blend.is_some();
            if instruction.op_count()
                != if immediate || variable.is_some() {
                    4
                } else {
                    3
                }
                || immediate && instruction.op3_kind() != OpKind::Immediate8
            {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = register_slot(instruction, 0, size)?;
            let left = vector_register(instruction, 1, size, &next)?;
            let right = if instruction.op2_kind() == OpKind::Memory {
                if instruction.memory_size().size() != size {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = size as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                memory.read(address(instruction, 2, &next)?, &mut bytes[..size])?;
                bytes
            } else {
                vector_register(instruction, 2, size, &next)?
            };
            let mut output = [0; 32];
            if matches!(instruction.mnemonic(), Mnemonic::Vpermps | Mnemonic::Vpermd) {
                // Full-width dword selection, including across 128-bit lanes.
                // Both inputs were snapshotted before any aliased destination
                // changes. Copy bits without floating-point interpretation.
                for start in (0..32).step_by(4) {
                    let selected = usize::from(left[start] & 7) * 4;
                    output[start..start + 4].copy_from_slice(&right[selected..selected + 4]);
                }
            } else if let Some((width, repeat)) = blend {
                let control = instruction.immediate8();
                for start in (0..size).step_by(width) {
                    let bit = if repeat {
                        (start % 16) / width
                    } else {
                        start / width
                    };
                    let source = if control & (1 << bit) != 0 {
                        &right
                    } else {
                        &left
                    };
                    output[start..start + width].copy_from_slice(&source[start..start + width]);
                }
            } else if let Some(width) = variable {
                let mask = vector_register(instruction, 3, size, &next)?;
                for start in (0..size).step_by(width) {
                    let source = if mask[start + width - 1] & 0x80 != 0 {
                        &right
                    } else {
                        &left
                    };
                    output[start..start + width].copy_from_slice(&source[start..start + width]);
                }
            } else if let Some((width, signed, maximum)) = minmax_layout(instruction.mnemonic()) {
                // Flipping the element's sign bit maps signed ordering onto
                // unsigned ordering without widening or touching guest flags.
                let bias = if signed { 1_u32 << (width * 8 - 1) } else { 0 };
                for start in (0..size).step_by(width) {
                    let mut a = [0; 4];
                    let mut b = [0; 4];
                    a[..width].copy_from_slice(&left[start..start + width]);
                    b[..width].copy_from_slice(&right[start..start + width]);
                    let a = u32::from_le_bytes(a) ^ bias;
                    let b = u32::from_le_bytes(b) ^ bias;
                    let source = if if maximum { a > b } else { a < b } {
                        &left
                    } else {
                        &right
                    };
                    output[start..start + width].copy_from_slice(&source[start..start + width]);
                }
            } else if let Some((width, subtract)) = arithmetic_width(instruction.mnemonic()) {
                for start in (0..size).step_by(width) {
                    let mut a = [0; 8];
                    let mut b = [0; 8];
                    a[..width].copy_from_slice(&left[start..start + width]);
                    b[..width].copy_from_slice(&right[start..start + width]);
                    let (a, b) = (u64::from_le_bytes(a), u64::from_le_bytes(b));
                    let result = if subtract {
                        a.wrapping_sub(b)
                    } else {
                        a.wrapping_add(b)
                    };
                    // Architectural wrap is per element: truncation discards
                    // carry/borrow without carrying into the neighboring lane.
                    output[start..start + width].copy_from_slice(&result.to_le_bytes()[..width]);
                }
            } else if let Some((width, signed, high, widen)) =
                multiply_layout(instruction.mnemonic())
            {
                let output_width = if widen { 8 } else { width };
                for start in (0..size).step_by(output_width) {
                    let mut a = [0; 8];
                    let mut b = [0; 8];
                    a[..width].copy_from_slice(&left[start..start + width]);
                    b[..width].copy_from_slice(&right[start..start + width]);
                    let (a, b) = (u64::from_le_bytes(a), u64::from_le_bytes(b));
                    let product = if signed {
                        let shift = 64 - width * 8;
                        let a = ((a << shift) as i64) >> shift;
                        let b = ((b << shift) as i64) >> shift;
                        (a * b) as u64
                    } else {
                        a * b
                    };
                    let selected = if high {
                        product >> (width * 8)
                    } else {
                        product
                    };
                    output[start..start + output_width]
                        .copy_from_slice(&selected.to_le_bytes()[..output_width]);
                }
            } else if immediate_shuffle {
                let control = instruction.immediate8();
                let singles = instruction.mnemonic() == Mnemonic::Vshufps;
                let width = if singles { 4 } else { 8 };
                for lane in (0..size).step_by(16) {
                    for item in 0..16 / width {
                        let source = if item < 8 / width { &left } else { &right };
                        // PS repeats four two-bit selectors in each lane. PD
                        // uses a distinct pair of one-bit selectors per lane.
                        let selected = if singles {
                            (control >> (item * 2)) & 3
                        } else {
                            (control >> (lane / 8 + item)) & 1
                        };
                        let from = lane + usize::from(selected) * width;
                        let to = lane + item * width;
                        output[to..to + width].copy_from_slice(&source[from..from + width]);
                    }
                }
            } else if let Some((width, half)) = unpack_layout(instruction.mnemonic()) {
                // The full architectural memory source was checked/read above,
                // including unselected bytes. Snapshot both inputs before writes.
                for lane in (0..size).step_by(16) {
                    for element in 0..8 / width {
                        let from = lane + half + element * width;
                        let to = lane + element * width * 2;
                        output[to..to + width].copy_from_slice(&left[from..from + width]);
                        output[to + width..to + width * 2]
                            .copy_from_slice(&right[from..from + width]);
                    }
                }
            } else if let Some(width) = equality_width(instruction.mnemonic()) {
                for ((destination, left), right) in output[..size]
                    .chunks_exact_mut(width)
                    .zip(left[..size].chunks_exact(width))
                    .zip(right[..size].chunks_exact(width))
                {
                    destination.fill(if left == right { 0xff } else { 0 });
                }
            } else {
                for i in 0..size {
                    output[i] = match instruction.mnemonic() {
                        Mnemonic::Vpand | Mnemonic::Vandps | Mnemonic::Vandpd => left[i] & right[i],
                        Mnemonic::Vpandn | Mnemonic::Vandnps | Mnemonic::Vandnpd => {
                            !left[i] & right[i]
                        }
                        Mnemonic::Vpor | Mnemonic::Vorps | Mnemonic::Vorpd => left[i] | right[i],
                        Mnemonic::Vpxor | Mnemonic::Vxorps | Mnemonic::Vxorpd => left[i] ^ right[i],
                        Mnemonic::Vpshufb => {
                            // Each 128-bit lane has its own sixteen-byte table.
                            if right[i] & 0x80 != 0 {
                                0
                            } else {
                                left[(i & !15) | usize::from(right[i] & 15)]
                            }
                        }
                        _ => return Err(NativeDispatchError::UnsupportedVectorInstruction),
                    };
                }
            }
            next.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
            next.ymm_upper[destination..destination + 16].copy_from_slice(&output[16..]);
        } else if let Some(size) = test_width(instruction.code()) {
            if instruction.op_count() != 2 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let left = vector_register(instruction, 0, size, &next)?;
            let right = if instruction.op1_kind() == OpKind::Memory {
                if instruction.memory_size().size() != size {
                    return Err(NativeDispatchError::UnsupportedVectorInstruction);
                }
                transferred = size as u64;
                self.machine
                    .usage
                    .vector_bytes
                    .checked_add(transferred)
                    .ok_or(NativeDispatchError::Budget("vector bytes"))?;
                let mut bytes = [0; 32];
                memory.read(address(instruction, 1, &next)?, &mut bytes[..size])?;
                bytes
            } else {
                vector_register(instruction, 1, size, &next)?
            };
            let (step, mask) = match instruction.mnemonic() {
                Mnemonic::Vtestps => (4, 0x80),
                Mnemonic::Vtestpd => (8, 0x80),
                _ => (1, 0xff),
            };
            let zero = (step - 1..size)
                .step_by(step)
                .all(|i| left[i] & right[i] & mask == 0);
            let carry = (step - 1..size)
                .step_by(step)
                .all(|i| !left[i] & right[i] & mask == 0);
            // VPTEST examines all bits; VTESTPS/PD examine only element sign
            // bits. All preserve vectors, clear OF/SF/AF/PF and set ZF/CF.
            next.rflags = (next.rflags & !0x8d5) | (u64::from(zero) << 6) | u64::from(carry);
        } else if let Some((size, element_bytes)) = mask_layout(instruction.code()) {
            if instruction.op_count() != 2 {
                return Err(NativeDispatchError::UnsupportedVectorInstruction);
            }
            let destination = integer_slot(instruction, 0, instruction.op0_register().size())?;
            let source = vector_register(instruction, 1, size, &next)?;
            let mask = source[element_bytes - 1..size]
                .iter()
                .step_by(element_bytes)
                .enumerate()
                .fold(0_u64, |mask, (i, byte)| mask | (u64::from(byte >> 7) << i));
            if destination == RSP {
                check_rsp(&self.machine.stack, mask)?;
            }
            // Both destination widths zero all bits above the mask. The source and
            // flags and MXCSR are unchanged, including the unused YMM upper half.
            next.gpr[destination] = mask;
        } else {
            next.ymm_upper.fill(0);
            if instruction.code() == Code::VEX_Vzeroall {
                next.xmm.fill(0);
            }
        }
        next.guest_pc = next_pc;
        self.machine.state = next;
        self.machine.usage.vector_operations = operations;
        self.machine.usage.vector_bytes += transferred;
        self.machine.phase = Phase::Ready;
        Ok(NativeBlockStop::Resume { guest_pc: next_pc })
    }
}

fn vector_register(
    instruction: &Instruction,
    operand: u32,
    size: usize,
    state: &NativeCpuState,
) -> Result<[u8; 32], NativeDispatchError> {
    let slot = register_slot(instruction, operand, size)?;
    let mut bytes = [0; 32];
    bytes[..16].copy_from_slice(&state.xmm[slot..slot + 16]);
    if size == 32 {
        bytes[16..].copy_from_slice(&state.ymm_upper[slot..slot + 16]);
    }
    Ok(bytes)
}

fn integer_slot(
    instruction: &Instruction,
    operand: u32,
    size: usize,
) -> Result<usize, NativeDispatchError> {
    let register = instruction.op_register(operand);
    if instruction.op_kind(operand) != OpKind::Register
        || !matches!(size, 4 | 8)
        || register.size() != size
    {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    gpr_slot(register.full_register()).ok_or(NativeDispatchError::UnsupportedVectorInstruction)
}

fn register_slot(
    instruction: &Instruction,
    operand: u32,
    size: usize,
) -> Result<usize, NativeDispatchError> {
    let register = instruction.op_register(operand);
    if instruction.op_kind(operand) != OpKind::Register
        || register.size() != size
        || !(register.is_xmm() && register <= Register::XMM15
            || register.is_ymm() && register <= Register::YMM15)
    {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    Ok(register.number() * 16)
}

pub(super) fn address(
    instruction: &Instruction,
    operand: u32,
    state: &NativeCpuState,
) -> Result<u64, NativeDispatchError> {
    instruction
        .virtual_address(operand, 0, |register, _, _| {
            if matches!(
                register,
                Register::None | Register::DS | Register::ES | Register::SS
            ) {
                Some(0)
            } else if register.is_gpr64() {
                Some(state.gpr[gpr_slot(register)?])
            } else if register.is_gpr32() {
                Some(state.gpr[gpr_slot(register.full_register())?] as u32 as u64)
            } else {
                None
            }
        })
        .ok_or(NativeDispatchError::UnsupportedVectorInstruction)
}
