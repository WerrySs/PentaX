// SPDX-License-Identifier: GPL-2.0-only
// Direct Rust adaptation of KytyPS5-Performance at
// 209dffe9032d72749257ce18680d506b891284de, VectorAluOps.cpp:
// VOP1/VOP2/VOPC/VOP3/VOP3P decoding, SDWA/DPP/DPP8 modifiers and their
// translator-profile restrictions. VINTRP remains in decode.rs.
// PentaX modifications: typed errors instead of SetUnsupported/abort, every
// modifier/literal word proven present and charged before it is read, and the
// source's check order preserved so the same encoding yields the same outcome.

use crate::decode::{
    Input, record, scalar_destination, scalar_source, unsupported, unsupported_opcode,
};
use crate::model::{Record, flag};
use crate::opcode::{self, Vop2SdwaProfile};
use crate::{
    DecodeError, Family, Opcode as O, Operand, OperandKind, SdwaSelect, SdwaUnused,
    UnsupportedReason as R,
};

fn bit(value: u32, shift: u32) -> bool {
    (value >> shift) & 1 != 0
}

fn uses_scalar_destination(op: O) -> bool {
    matches!(op, O::VReadfirstlaneB32 | O::VReadlaneB32)
}

fn is_vop3b_carry_out(op: O) -> bool {
    matches!(op, O::VAddI32 | O::VSubI32 | O::VSubrevI32)
}

fn is_permlane(op: O) -> bool {
    matches!(op, O::VPermlane16B32 | O::VPermlanex16B32)
}

fn is_native_f16_ternary(op: O) -> bool {
    matches!(op, O::VMin3F16 | O::VMax3F16 | O::VMed3F16 | O::VFmaF16)
}

fn is_native_b16_ternary(op: O) -> bool {
    matches!(op, O::VMin3U16 | O::VMed3I16 | O::VMadI16)
}

fn is_native_b16_binary(op: O) -> bool {
    matches!(
        op,
        O::VAddNcU16
            | O::VSubNcU16
            | O::VMulLoU16
            | O::VMaxU16
            | O::VMaxI16
            | O::VMinU16
            | O::VMinI16
            | O::VAddNcI16
            | O::VSubNcI16
            | O::VLshlrevB16
            | O::VLshrrevB16
            | O::VAshrrevI16
    )
}

fn is_vop1_float64(op: O) -> bool {
    matches!(
        op,
        O::VCvtF64I32
            | O::VCvtF32F64
            | O::VCvtF64F32
            | O::VCvtF64U32
            | O::VRcpF64
            | O::VFractF64
            | O::VTruncF64
            | O::VCeilF64
            | O::VFloorF64
    )
}

fn is_vop1_float_source(op: O) -> bool {
    matches!(
        op,
        O::VCvtF32F64
            | O::VCvtF64F32
            | O::VRcpF64
            | O::VFractF64
            | O::VTruncF64
            | O::VCeilF64
            | O::VFloorF64
            | O::VMovB32
            | O::VCvtF32F16
            | O::VCvtU32F32
            | O::VCvtI32F32
            | O::VCvtRpiI32F32
            | O::VCvtFlrI32F32
            | O::VFrexpExpI32F32
            | O::VFrexpMantF32
            | O::VCvtF16F32
            | O::VCvtU16F16
            | O::VCvtI16F16
            | O::VRcpF32
            | O::VRcpIflagF32
            | O::VFractF32
            | O::VTruncF32
            | O::VCeilF32
            | O::VRndneF32
            | O::VFloorF32
            | O::VExpF32
            | O::VLogF32
            | O::VRsqF32
            | O::VSqrtF32
            | O::VRcpF16
            | O::VSqrtF16
            | O::VRsqF16
            | O::VLogF16
            | O::VExpF16
            | O::VFloorF16
            | O::VCeilF16
            | O::VTruncF16
            | O::VRndneF16
            | O::VFractF16
            | O::VSinF16
            | O::VCosF16
            | O::VSinF32
            | O::VCosF32
    )
}

fn is_vop2_float(op: O) -> bool {
    matches!(
        op,
        O::VAddF32
            | O::VSubF32
            | O::VSubrevF32
            | O::VMulF32
            | O::VMinF32
            | O::VMaxF32
            | O::VMacF32
            | O::VMadmkF32
            | O::VMadakF32
            | O::VFmacF32
            | O::VFmamkF32
            | O::VFmaakF32
            | O::VCvtPkrtzF16F32
            | O::VAddF16
            | O::VSubF16
            | O::VSubrevF16
            | O::VMulF16
            | O::VFmacF16
            | O::VFmamkF16
            | O::VFmaakF16
            | O::VMaxF16
            | O::VMinF16
    )
}

fn is_vop1_float_result(op: O) -> bool {
    matches!(
        op,
        O::VCvtF32F64
            | O::VCvtF32I32
            | O::VCvtF32U32
            | O::VCvtF32F16
            | O::VCvtF16F32
            | O::VCvtF16U16
            | O::VCvtF16I16
            | O::VCvtOffF32I4
            | O::VCvtF32Ubyte0
            | O::VCvtF32Ubyte1
            | O::VCvtF32Ubyte2
            | O::VCvtF32Ubyte3
            | O::VRcpF32
            | O::VRcpIflagF32
            | O::VFractF32
            | O::VTruncF32
            | O::VCeilF32
            | O::VRndneF32
            | O::VFloorF32
            | O::VFrexpMantF32
            | O::VExpF32
            | O::VLogF32
            | O::VRsqF32
            | O::VSqrtF32
            | O::VRcpF16
            | O::VSqrtF16
            | O::VRsqF16
            | O::VLogF16
            | O::VExpF16
            | O::VFloorF16
            | O::VCeilF16
            | O::VTruncF16
            | O::VRndneF16
            | O::VFractF16
            | O::VSinF16
            | O::VCosF16
            | O::VSinF32
            | O::VCosF32
    )
}

fn is_vopc_float_compare(op: O) -> bool {
    matches!(
        op,
        O::VCmpFF32
            | O::VCmpLtF32
            | O::VCmpEqF32
            | O::VCmpEqF64
            | O::VCmpLeF64
            | O::VCmpxLeF64
            | O::VCmpxGeF64
            | O::VCmpLeF32
            | O::VCmpGtF32
            | O::VCmpLgF32
            | O::VCmpGeF32
            | O::VCmpOF32
            | O::VCmpUF32
            | O::VCmpNgeF32
            | O::VCmpNlgF32
            | O::VCmpNgtF32
            | O::VCmpNleF32
            | O::VCmpNeqF32
            | O::VCmpNltF32
            | O::VCmpTruF32
            | O::VCmpxLtF32
            | O::VCmpxEqF32
            | O::VCmpxLeF32
            | O::VCmpxGtF32
            | O::VCmpxLgF32
            | O::VCmpxGeF32
            | O::VCmpxOF32
            | O::VCmpxUF32
            | O::VCmpxNgeF32
            | O::VCmpxNlgF32
            | O::VCmpxNgtF32
            | O::VCmpxNleF32
            | O::VCmpxNeqF32
            | O::VCmpxNltF32
            | O::VCmpLtF16
            | O::VCmpEqF16
            | O::VCmpLeF16
            | O::VCmpGtF16
            | O::VCmpLgF16
            | O::VCmpGeF16
            | O::VCmpNgeF16
            | O::VCmpNgtF16
            | O::VCmpNeqF16
            | O::VCmpNltF16
            | O::VCmpxLtF16
            | O::VCmpxEqF16
            | O::VCmpxLeF16
            | O::VCmpxGtF16
            | O::VCmpxGeF16
            | O::VCmpxNgtF16
            | O::VCmpxNleF16
            | O::VCmpxNeqF16
            | O::VCmpxNltF16
            | O::VCmpClassF32
            | O::VCmpClassF16
            | O::VCmpxClassF16
            | O::VCmpxClassF32
    )
}

/// Compares whose destination is EXEC (the source `IsVopcCompareExec`).
fn is_vopc_compare_exec(op: O) -> bool {
    matches!(
        op,
        O::VCmpxLeF64
            | O::VCmpxGeF64
            | O::VCmpxLtF32
            | O::VCmpxEqF32
            | O::VCmpxLeF32
            | O::VCmpxGtF32
            | O::VCmpxLgF32
            | O::VCmpxGeF32
            | O::VCmpxOF32
            | O::VCmpxUF32
            | O::VCmpxNgeF32
            | O::VCmpxNlgF32
            | O::VCmpxNgtF32
            | O::VCmpxNleF32
            | O::VCmpxNeqF32
            | O::VCmpxNltF32
            | O::VCmpxLtI32
            | O::VCmpxEqI32
            | O::VCmpxLeI32
            | O::VCmpxGtI32
            | O::VCmpxNeI32
            | O::VCmpxGeI32
            | O::VCmpxClassF32
            | O::VCmpxClassF16
            | O::VCmpxLtU32
            | O::VCmpxEqU32
            | O::VCmpxLeU32
            | O::VCmpxGtU32
            | O::VCmpxNeU32
            | O::VCmpxGeU32
            | O::VCmpxFI64
            | O::VCmpxLtI64
            | O::VCmpxEqI64
            | O::VCmpxLeI64
            | O::VCmpxNeI64
            | O::VCmpxTI64
            | O::VCmpxFU64
            | O::VCmpxLtU64
            | O::VCmpxEqU64
            | O::VCmpxLeU64
            | O::VCmpxGtU64
            | O::VCmpxNeU64
            | O::VCmpxGeU64
            | O::VCmpxTU64
            | O::VCmpxLtI16
            | O::VCmpxEqI16
            | O::VCmpxLeI16
            | O::VCmpxGtI16
            | O::VCmpxNeI16
            | O::VCmpxGeI16
            | O::VCmpxLtU16
            | O::VCmpxEqU16
            | O::VCmpxGtU16
            | O::VCmpxNeU16
            | O::VCmpxLtF16
            | O::VCmpxEqF16
            | O::VCmpxLeF16
            | O::VCmpxGtF16
            | O::VCmpxGeF16
            | O::VCmpxNgtF16
            | O::VCmpxNleF16
            | O::VCmpxNeqF16
            | O::VCmpxNltF16
    )
}

fn uses_inexact_clamp_control(op: O) -> bool {
    matches!(
        op,
        O::VCvtU32F32 | O::VCvtI32F32 | O::VCvtU16F16 | O::VCvtI16F16
    )
}

fn supports_vop1_clamp(op: O) -> bool {
    is_vop1_float_result(op) || uses_inexact_clamp_control(op)
}

// SDWA selector masks: bit N admits selector N (bytes 0-3, words 4-5, dword 6).
const BYTES: u8 = 0x0f;
const WORDS: u8 = 0x30;
const FULL: u8 = 0x40;
const ALL: u8 = BYTES | WORDS | FULL;

fn has_selector(mask: u8, selector: u32) -> bool {
    selector <= 6 && mask & (1 << selector) != 0
}

/// Source `VOP1_SDWA_RULES`: (source, partial destination, partial-destination
/// source selectors, source modifiers).
fn vop1_sdwa_rule(op: O) -> Option<(u8, u8, u8, bool)> {
    Some(match op {
        O::VMovB32 | O::VNotB32 => (ALL, BYTES | WORDS, ALL, false),
        O::VCvtF32U32
        | O::VCvtF32I32
        | O::VCvtF32Ubyte0
        | O::VBfrevB32
        | O::VFfblB32
        | O::VFfbhU32 => (ALL, 0, 0, false),
        O::VCvtF32F16 => (WORDS | FULL, 0, 0, true),
        O::VCvtF16F32 => (ALL, WORDS, ALL, true),
        O::VCvtF16U16 => (ALL, WORDS, ALL, false),
        O::VCvtF16I16 => (WORDS | FULL, WORDS, WORDS | FULL, false),
        O::VCvtU16F16
        | O::VCvtI16F16
        | O::VRcpF16
        | O::VSqrtF16
        | O::VRsqF16
        | O::VLogF16
        | O::VExpF16
        | O::VFloorF16
        | O::VCeilF16
        | O::VTruncF16
        | O::VRndneF16
        | O::VFractF16
        | O::VSinF16
        | O::VCosF16 => (WORDS | FULL, WORDS, WORDS | FULL, true),
        O::VCvtU32F32 | O::VCvtI32F32 | O::VCvtRpiI32F32 | O::VCvtFlrI32F32 => {
            (FULL, BYTES | WORDS, FULL, false)
        }
        _ => return None,
    })
}

fn vop1_sdwa_source_supported(op: O, sel: u32, neg: bool, abs: bool) -> bool {
    if sel == 6 {
        return is_vop1_float_source(op) || (!neg && !abs);
    }
    match vop1_sdwa_rule(op) {
        Some((source, _, _, modifiers)) if has_selector(source, sel) => modifiers || (!neg && !abs),
        _ => false,
    }
}

fn vop1_sdwa_destination_supported(op: O, dst_sel: u32, dst_u: u32, src_sel: u32) -> bool {
    if dst_sel == 6 {
        return dst_u != 3;
    }
    match vop1_sdwa_rule(op) {
        Some((_, partial, partial_source, _)) if dst_u != 3 => {
            has_selector(partial, dst_sel) && has_selector(partial_source, src_sel)
        }
        _ => false,
    }
}

fn validate_vop1_sdwa(op: O, m: u32) -> Result<(), R> {
    if is_vop1_float64(op) {
        return Err(R::Vop1SdwaFloat64);
    }
    let dst_sel = (m >> 8) & 7;
    let dst_u = (m >> 11) & 3;
    let clamp = bit(m, 13);
    let omod = (m >> 14) & 3;
    let src0_sel = (m >> 16) & 7;
    let src0_sext = bit(m, 19);
    let (neg, abs) = (bit(m, 20), bit(m, 21));
    if src0_sel > 6 || dst_sel > 6 {
        return Err(R::Vop1SdwaSelector);
    }
    if (clamp && !supports_vop1_clamp(op)) || (omod != 0 && !is_vop1_float_result(op)) {
        return Err(R::Vop1SdwaOutputModifiers);
    }
    if !vop1_sdwa_destination_supported(op, dst_sel, dst_u, src0_sel) {
        return Err(R::Vop1SdwaDestinationSelector);
    }
    if !vop1_sdwa_source_supported(op, src0_sel, neg, abs)
        || (op == O::VCvtF16U16 && src0_sel <= 3 && src0_sext)
    {
        return Err(R::Vop1SdwaSourceSelector);
    }
    Ok(())
}

fn select(
    bits: u32,
    family: Family,
    id: u16,
    at: usize,
    reason: R,
) -> Result<SdwaSelect, DecodeError> {
    SdwaSelect::from_bits(bits).ok_or(unsupported(at, family, id, reason))
}

/// Apply an SDWA source selector, sign-extension, negate and absolute fields.
fn sdwa_source(
    operand: &mut Operand,
    m: u32,
    shift: u32,
    family: Family,
    id: u16,
    at: usize,
    reason: R,
) -> Result<(), DecodeError> {
    operand.sdwa_select = select((m >> shift) & 7, family, id, at, reason)?;
    operand.set(flag::SDWA_SEXT, bit(m, shift + 3));
    operand.set(flag::NEGATE, bit(m, shift + 4));
    operand.set(flag::ABSOLUTE, bit(m, shift + 5));
    Ok(())
}

/// Explicit SDWA destination: selector, unused policy, clamp and omod.
fn sdwa_destination(
    dst: &mut Operand,
    m: u32,
    family: Family,
    id: u16,
    at: usize,
    reason: R,
) -> Result<(), DecodeError> {
    dst.sdwa_select = select((m >> 8) & 7, family, id, at, reason)?;
    dst.sdwa_unused =
        SdwaUnused::from_bits((m >> 11) & 3).ok_or(unsupported(at, family, id, reason))?;
    dst.set(flag::EXPLICIT_SDWA_DST, true);
    dst.set(flag::CLAMP, bit(m, 13));
    dst.omod = ((m >> 14) & 3) as u8;
    Ok(())
}

/// Source `ApplyDppModifier`; `encoding` is the header's 9-bit SRC0 selector.
fn apply_dpp(operand: &mut Operand, m: u32, encoding: u32) {
    operand.set(flag::DPP, true);
    if encoding == 233 || encoding == 234 {
        operand.set(flag::DPP8, true);
        operand.dpp_control = m >> 8;
        operand.set(flag::DPP_FETCH_INACTIVE, encoding == 234);
        return;
    }
    operand.set(flag::NEGATE, bit(m, 20));
    operand.set(flag::ABSOLUTE, bit(m, 21));
    operand.dpp_control = (m >> 8) & 0x1ff;
    operand.set(flag::DPP_FETCH_INACTIVE, bit(m, 18));
    operand.set(flag::DPP_BOUND_CTRL, bit(m, 19));
    operand.dpp_bank_mask = ((m >> 24) & 0xf) as u8;
    operand.dpp_row_mask = ((m >> 28) & 0xf) as u8;
}

fn vector_or_scalar_destination(op: O, vdst: u16, at: usize) -> Result<Operand, DecodeError> {
    if uses_scalar_destination(op) {
        scalar_destination(vdst, at)
    } else {
        Ok(Operand::vgpr(vdst))
    }
}

/// SDWA S0/S1: a clear bit selects a VGPR, a set bit the scalar selector space.
fn sdwa_selector(field: u32, scalar: bool) -> u16 {
    (field & 0xff) as u16 + if scalar { 0 } else { 256 }
}

pub(crate) fn decode_vop1(
    input: &mut Input<'_>,
    word: u32,
    at: usize,
) -> Result<Record, DecodeError> {
    let family = Family::Vop1;
    let id = ((word >> 9) & 0xff) as u16;
    let src0 = (word & 0x1ff) as u16;
    let vdst = ((word >> 17) & 0xff) as u16;
    let op = opcode::vop1(id).ok_or(unsupported_opcode(at, family, id))?;
    let mut inst = record(at, family, id, op);
    if op == O::VNop {
        // Returns before the DPP/SDWA selector test: no modifier word.
        inst.dst = Operand::special(OperandKind::Null);
        return Ok(inst);
    }
    match src0 {
        233 | 234 | 250 => {
            if is_vop1_float64(op) {
                // Checked before the source reads the DPP word.
                return Err(unsupported(at, family, id, R::Vop1DppFloat64));
            }
            input.require(&mut inst, 2)?;
            let m = input.word(&inst, 1)?;
            inst.dst = vector_or_scalar_destination(op, vdst, at)?;
            inst.src[0] = scalar_source(sdwa_selector(m, false), at)?;
            apply_dpp(&mut inst.src[0], m, word & 0x1ff);
            inst.src_count = 1;
            if !is_vop1_float_source(op) && (inst.src[0].negate() || inst.src[0].absolute()) {
                return Err(unsupported(
                    at,
                    family,
                    id,
                    R::Vop1DppIntegerSourceModifiers,
                ));
            }
        }
        249 => {
            input.require(&mut inst, 2)?;
            let m = input.word(&inst, 1)?;
            validate_vop1_sdwa(op, m).map_err(|reason| unsupported(at, family, id, reason))?;
            inst.dst = vector_or_scalar_destination(op, vdst, at)?;
            inst.src[0] = scalar_source(sdwa_selector(m, bit(m, 23)), at)?;
            sdwa_destination(&mut inst.dst, m, family, id, at, R::Vop1SdwaSelector)?;
            sdwa_source(&mut inst.src[0], m, 16, family, id, at, R::Vop1SdwaSelector)?;
            inst.src_count = 1;
        }
        _ => {
            inst.dst = vector_or_scalar_destination(op, vdst, at)?;
            inst.src[0] = scalar_source(src0, at)?;
            inst.src_count = 1;
        }
    }
    input.literal(&mut inst)?;
    Ok(inst)
}

struct Vop2SdwaRule {
    dst: u8,
    src0: u8,
    src1: u8,
    partial_dst: bool,
    source_modifiers: bool,
}

/// Source `VOP2_SDWA_RULES`, indexed by the opcode table's profile.
fn vop2_sdwa_rule(profile: Vop2SdwaProfile) -> Option<Vop2SdwaRule> {
    use Vop2SdwaProfile as P;
    let rule = |dst, src0, src1, partial_dst, source_modifiers| {
        Some(Vop2SdwaRule {
            dst,
            src0,
            src1,
            partial_dst,
            source_modifiers,
        })
    };
    match profile {
        P::None => None,
        P::Float32 => rule(FULL, FULL, FULL, false, true),
        P::Float16 => rule(WORDS | FULL, WORDS | FULL, WORDS | FULL, true, true),
        P::PackedFloat16 => rule(ALL, ALL, ALL, true, true),
        P::IntegerFullDestination => rule(FULL, ALL, ALL, false, false),
        P::IntegerPartialDestination => rule(ALL, ALL, ALL, true, false),
        P::ReverseRightShift | P::ReverseLogicalLeft => rule(FULL, ALL, ALL, false, false),
        P::Bitwise => rule(WORDS | FULL, ALL, ALL, true, false),
    }
}

fn validate_vop2_sdwa(op: O, profile: Vop2SdwaProfile, m: u32) -> Result<(), R> {
    let dst_sel = (m >> 8) & 7;
    let dst_u = (m >> 11) & 3;
    let clamp = bit(m, 13);
    let omod = (m >> 14) & 3;
    let src0_sel = (m >> 16) & 7;
    let src1_sel = (m >> 24) & 7;
    let modifiers = bit(m, 20) || bit(m, 21) || bit(m, 28) || bit(m, 29);
    if src0_sel > 6 || src1_sel > 6 || dst_sel > 6 {
        return Err(R::Vop2SdwaSelector);
    }
    if (clamp || omod != 0) && !is_vop2_float(op) {
        return Err(R::Vop2SdwaOutputModifiers);
    }
    if dst_sel == 6 && dst_u == 0 && src0_sel == 6 && src1_sel == 6 {
        if !is_vop2_float(op) && op != O::VCndmaskB32 && modifiers {
            return Err(R::Vop2SdwaSourceModifiers);
        }
        return Ok(());
    }
    let rule = vop2_sdwa_rule(profile).ok_or(R::Vop2SdwaOpcode)?;
    let dst_ok = has_selector(rule.dst, dst_sel)
        && if dst_sel == 6 {
            dst_u != 3
        } else {
            dst_u != 3 && rule.partial_dst
        };
    if !dst_ok {
        return Err(R::Vop2SdwaDestinationSelector);
    }
    if !has_selector(rule.src0, src0_sel) || !has_selector(rule.src1, src1_sel) {
        return Err(R::Vop2SdwaSourceSelector);
    }
    let cndmask_float_modifiers = op == O::VCndmaskB32 && src0_sel == 6 && src1_sel == 6;
    if !rule.source_modifiers && modifiers && !cndmask_float_modifiers {
        return Err(R::Vop2SdwaSourceModifiers);
    }
    Ok(())
}

/// Source `FinalizeVop2Instruction`: implicit literal/VCC operands, then the
/// shared literal word (after any modifier word).
fn finalize_vop2(input: &mut Input<'_>, inst: &mut Record) -> Result<(), DecodeError> {
    match inst.opcode {
        O::VMadmkF32 | O::VFmamkF32 | O::VFmamkF16 => {
            inst.src[2] = inst.src[1];
            inst.src[1] = Operand::special(OperandKind::LiteralConstant);
            inst.src_count = 3;
        }
        O::VMadakF32 | O::VFmaakF32 | O::VFmaakF16 => {
            inst.src[2] = Operand::special(OperandKind::LiteralConstant);
            inst.src_count = 3;
        }
        O::VAddcU32 | O::VSubCoCiU32 | O::VSubrevCoCiU32 => {
            inst.dst2.kind = OperandKind::VccLo;
            inst.src[2].kind = OperandKind::VccLo;
            inst.src_count = 3;
        }
        O::VPkFmacF16 => {
            // VOP2 has no OP_SEL fields, so the high operation implicitly reads high halves.
            inst.src[0].set(flag::OP_SEL_HI, true);
            inst.src[1].set(flag::OP_SEL_HI, true);
            inst.dst.set(flag::OP_SEL_HI, true);
            inst.src_count = 2;
        }
        _ => inst.src_count = 2,
    }
    input.literal(inst)
}

pub(crate) fn decode_vop2(
    input: &mut Input<'_>,
    word: u32,
    at: usize,
) -> Result<Record, DecodeError> {
    let family = Family::Vop2;
    let id = ((word >> 25) & 0x3f) as u16;
    let vdst = ((word >> 17) & 0xff) as u16;
    let src0 = (word & 0x1ff) as u16;
    let vsrc1 = (word >> 9) & 0xff;
    let (op, profile) = opcode::vop2(id).ok_or(unsupported_opcode(at, family, id))?;
    let mut inst = record(at, family, id, op);
    match src0 {
        233 | 234 | 250 => {
            input.require(&mut inst, 2)?;
            let m = input.word(&inst, 1)?;
            inst.dst = Operand::vgpr(vdst);
            inst.src[1] = Operand::vgpr(vsrc1 as u16);
            inst.src[0] = scalar_source(sdwa_selector(m, false), at)?;
            apply_dpp(&mut inst.src[0], m, word & 0x1ff);
            if !inst.src[0].has(flag::DPP8) {
                inst.src[1].set(flag::NEGATE, bit(m, 22));
                inst.src[1].set(flag::ABSOLUTE, bit(m, 23));
            }
            let packed_fmac = op == O::VPkFmacF16;
            if packed_fmac {
                let (neg0, neg1) = (inst.src[0].negate(), inst.src[1].negate());
                inst.src[0].set(flag::NEGATE_HI, neg0);
                inst.src[1].set(flag::NEGATE_HI, neg1);
            }
            let modifiers = inst.src[0].negate()
                || inst.src[0].absolute()
                || inst.src[1].negate()
                || inst.src[1].absolute();
            if !is_vop2_float(op) && !packed_fmac && modifiers {
                return Err(unsupported(
                    at,
                    family,
                    id,
                    R::Vop2DppIntegerSourceModifiers,
                ));
            }
        }
        249 => {
            input.require(&mut inst, 2)?;
            let m = input.word(&inst, 1)?;
            validate_vop2_sdwa(op, profile, m)
                .map_err(|reason| unsupported(at, family, id, reason))?;
            inst.dst = Operand::vgpr(vdst);
            inst.src[0] = scalar_source(sdwa_selector(m, bit(m, 23)), at)?;
            inst.src[1] = scalar_source(sdwa_selector(vsrc1, bit(m, 31)), at)?;
            sdwa_destination(&mut inst.dst, m, family, id, at, R::Vop2SdwaSelector)?;
            sdwa_source(&mut inst.src[0], m, 16, family, id, at, R::Vop2SdwaSelector)?;
            sdwa_source(&mut inst.src[1], m, 24, family, id, at, R::Vop2SdwaSelector)?;
        }
        _ => {
            inst.dst = Operand::vgpr(vdst);
            inst.src[1] = Operand::vgpr(vsrc1 as u16);
            inst.src[0] = scalar_source(src0, at)?;
        }
    }
    finalize_vop2(input, &mut inst)?;
    Ok(inst)
}

fn supports_vopc_sdwa(op: Option<O>) -> bool {
    op.is_some_and(|op| {
        !matches!(
            op,
            O::VCmpNeI64 | O::VCmpEqF64 | O::VCmpLeF64 | O::VCmpxLeF64 | O::VCmpxGeF64
        )
    })
}

fn compare_destination(op: O) -> Operand {
    Operand::special(if is_vopc_compare_exec(op) {
        OperandKind::ExecLo
    } else {
        OperandKind::VccLo
    })
}

pub(crate) fn decode_vopc(
    input: &mut Input<'_>,
    word: u32,
    at: usize,
) -> Result<Record, DecodeError> {
    let family = Family::Vopc;
    let id = ((word >> 17) & 0xff) as u16;
    let src0 = (word & 0x1ff) as u16;
    let vsrc1 = (word >> 9) & 0xff;
    let entry = opcode::vopc(id);
    // A placeholder opcode is never returned: every path below fails first
    // when the map has no entry.
    let mut inst = record(at, family, id, entry.map_or(O::VCmpFF32, |(op, _)| op));
    match src0 {
        233 | 234 | 250 => {
            input.require(&mut inst, 2)?;
            let m = input.word(&inst, 1)?;
            let (op, supports_dpp) = entry.ok_or(unsupported_opcode(at, family, id))?;
            if !supports_dpp {
                return Err(unsupported(at, family, id, R::VopcDppOpcode));
            }
            inst.src[1] = Operand::vgpr(vsrc1 as u16);
            inst.src[0] = scalar_source(sdwa_selector(m, false), at)?;
            inst.dst = compare_destination(op);
            apply_dpp(&mut inst.src[0], m, word & 0x1ff);
            if !inst.src[0].has(flag::DPP8) {
                inst.src[1].set(flag::NEGATE, bit(m, 22));
                inst.src[1].set(flag::ABSOLUTE, bit(m, 23));
            }
        }
        249 => {
            input.require(&mut inst, 2)?;
            let m = input.word(&inst, 1)?;
            if (m >> 16) & 7 > 6 || (m >> 24) & 7 > 6 {
                return Err(unsupported(at, family, id, R::VopcSdwaSelector));
            }
            let op = entry
                .map(|(op, _)| op)
                .filter(|&op| supports_vopc_sdwa(Some(op)))
                .ok_or(unsupported(at, family, id, R::VopcSdwaOpcode))?;
            inst.src[0] = scalar_source(sdwa_selector(m, bit(m, 23)), at)?;
            inst.src[1] = scalar_source(sdwa_selector(vsrc1, bit(m, 31)), at)?;
            inst.dst = if is_vopc_compare_exec(op) {
                Operand::special(OperandKind::ExecLo)
            } else if !bit(m, 15) {
                Operand::special(OperandKind::VccLo)
            } else {
                scalar_destination(((m >> 8) & 0x7f) as u16, at)?
            };
            sdwa_source(&mut inst.src[0], m, 16, family, id, at, R::VopcSdwaSelector)?;
            sdwa_source(&mut inst.src[1], m, 24, family, id, at, R::VopcSdwaSelector)?;
        }
        _ => {
            let (op, _) = entry.ok_or(unsupported_opcode(at, family, id))?;
            inst.dst = compare_destination(op);
            inst.src[1] = Operand::vgpr(vsrc1 as u16);
            inst.src[0] = scalar_source(src0, at)?;
        }
    }
    inst.src_count = 2;
    input.literal(&mut inst)?;
    Ok(inst)
}

/// Source `LookupVop3Opcode`: native map, then VOPC/VOP2/VOP1 alias ranges.
fn lookup_vop3(id: u16) -> Option<O> {
    if let Some(op) = opcode::vop3_direct(id) {
        return Some(op);
    }
    match id {
        0..=0xff => opcode::vopc(id).map(|(op, _)| op),
        0x100..=0x13f => {
            let vop2 = id - 0x100;
            // Literal-MAD forms, V_DOT2C_F32_F16 and V_MAX/MIN_F16 are not aliased.
            if matches!(vop2, 0x20 | 0x21 | 0x2c | 0x2d | 0x02 | 0x39 | 0x3a) {
                None
            } else {
                opcode::vop2(vop2).map(|(op, _)| op)
            }
        }
        0x180..=0x1ff => opcode::vop3_encoded_vop1(id - 0x180),
        _ => None,
    }
}

fn native_vop3_source_count(op: O) -> usize {
    match op {
        O::VAddF64
        | O::VMulF64
        | O::VMinF64
        | O::VMaxF64
        | O::VMulLoU32
        | O::VMulHiU32
        | O::VMulLoI32
        | O::VMulHiI32
        | O::VMulI32I24
        | O::VAddNcU16
        | O::VSubNcU16
        | O::VMulLoU16
        | O::VMaxU16
        | O::VMaxI16
        | O::VMinU16
        | O::VMinI16
        | O::VAddNcI16
        | O::VSubNcI16
        | O::VLshlrevB64
        | O::VLshrrevB64
        | O::VLshlrevB16
        | O::VLshrrevB16
        | O::VAshrrevI16
        | O::VAddI32
        | O::VSubI32
        | O::VSubrevI32
        | O::VPackB32F16
        | O::VReadlaneB32
        | O::VWritelaneB32
        | O::VCvtPkrtzF16F32
        | O::VLdexpF32
        | O::VBfmB32
        | O::VBcntU32B32
        | O::VCvtPknormI16F32
        | O::VCvtPknormU16F32
        | O::VCvtPkU16U32
        | O::VCvtPkI16I32 => 2,
        _ => 3,
    }
}

fn supports_native_source_modifiers(op: O) -> bool {
    is_vop1_float_source(op)
        || is_vopc_float_compare(op)
        || is_native_f16_ternary(op)
        || matches!(
            op,
            O::VCndmaskB32
                | O::VAddF32
                | O::VSubF32
                | O::VSubrevF32
                | O::VMulF32
                | O::VMinF32
                | O::VMaxF32
                | O::VMacF32
                | O::VFmacF32
                | O::VMadF32
                | O::VAddF64
                | O::VMulF64
                | O::VMinF64
                | O::VMaxF64
                | O::VFmaF64
                | O::VFmaF32
                | O::VPackB32F16
                | O::VCubeidF32
                | O::VCubescF32
                | O::VCubetcF32
                | O::VCubemaF32
                | O::VCvtPkrtzF16F32
                | O::VCvtPknormI16F32
                | O::VCvtPknormU16F32
                | O::VMin3F32
                | O::VMax3F32
                | O::VMed3F32
                | O::VLdexpF32
        )
}

fn supports_native_result_modifiers(op: O) -> bool {
    is_vop1_float_result(op)
        || matches!(
            op,
            O::VAddF32
                | O::VSubF32
                | O::VSubrevF32
                | O::VMulF32
                | O::VMinF32
                | O::VMaxF32
                | O::VMacF32
                | O::VFmacF32
                | O::VMadF32
                | O::VFmaF32
                | O::VFmaF16
                | O::VMin3F16
                | O::VMax3F16
                | O::VMed3F16
                | O::VCubeidF32
                | O::VCvtPkrtzF16F32
                | O::VLdexpF32
                | O::VMin3F32
                | O::VMax3F32
                | O::VMed3F32
        )
}

fn supports_native_clamp(op: O) -> bool {
    supports_native_result_modifiers(op) || uses_inexact_clamp_control(op) || op == O::VMadI16
}

struct Vop3Fields {
    abs: u32,
    op_sel: u32,
    clamp: u32,
    omod: u32,
    neg: u32,
}

/// Source `HasUnsupportedNativeVop3Modifiers` with `mad_mix` fixed to false,
/// as in `DecodeVop3`. `None` is the source's UNSUPPORTED opcode.
fn has_unsupported_vop3_modifiers(op: Option<O>, carry_in_out: bool, f: &Vop3Fields) -> bool {
    let is = |p: fn(O) -> bool| op.is_some_and(p);
    let (abs, op_sel, clamp, omod, neg) = (f.abs, f.op_sel, f.clamp, f.omod, f.neg);
    let source = is(supports_native_source_modifiers);
    let result = is(supports_native_result_modifiers);
    let clamp_ok = is(supports_native_clamp);
    if is(is_permlane) {
        return abs != 0 || op_sel & !3 != 0 || clamp != 0 || omod != 0 || neg != 0;
    }
    if is(is_native_f16_ternary) {
        // Clamp is applied for every f16 ternary; omod only for V_FMA_F16.
        return op != Some(O::VFmaF16) && omod != 0;
    }
    if is(is_native_b16_ternary) {
        return abs != 0 || (clamp != 0 && !clamp_ok) || omod != 0 || neg != 0;
    }
    if is(is_native_b16_binary) {
        return abs != 0 || clamp != 0 || omod != 0 || neg != 0;
    }
    if carry_in_out || is(uses_scalar_destination) {
        return clamp != 0 || omod != 0 || neg != 0;
    }
    match op {
        Some(O::VLdexpF32) => return abs & !1 != 0 || op_sel != 0 || neg & !1 != 0,
        Some(O::VCndmaskB32) => {
            return abs & !3 != 0 || op_sel != 0 || clamp != 0 || omod != 0 || neg & !3 != 0;
        }
        Some(O::VPackB32F16) => {
            return abs & !3 != 0 || op_sel & !3 != 0 || clamp != 0 || omod != 0 || neg & !3 != 0;
        }
        _ => {}
    }
    if source {
        return op_sel != 0 || (omod != 0 && !result) || (clamp != 0 && !clamp_ok);
    }
    if result {
        return abs != 0 || op_sel != 0 || neg != 0;
    }
    abs != 0 || op_sel != 0 || clamp != 0 || omod != 0 || neg != 0
}

fn apply_native_source_modifiers(inst: &mut Record, abs: u32, neg: u32) {
    for (index, src) in inst.src.iter_mut().take(inst.src_count.min(3)).enumerate() {
        src.set(flag::ABSOLUTE, (abs >> index) & 1 != 0);
        src.set(flag::NEGATE, (neg >> index) & 1 != 0);
    }
}

fn half_destination(op_sel: u32) -> SdwaSelect {
    if op_sel & 8 != 0 {
        SdwaSelect::Word1
    } else {
        SdwaSelect::Word0
    }
}

pub(crate) fn decode_vop3(
    input: &mut Input<'_>,
    header: u32,
    at: usize,
) -> Result<Record, DecodeError> {
    let family = Family::Vop3;
    let id = ((header >> 16) & 0x3ff) as u16;
    let op = lookup_vop3(id);
    let mut inst = record(at, family, id, op.unwrap_or(O::VNop));
    // The source reads the second word before the opcode is looked up.
    input.require(&mut inst, 2)?;
    let w1 = input.word(&inst, 1)?;
    let vdst = (header & 0xff) as u16;
    let sdst = ((header >> 8) & 0x7f) as u16;
    let src0 = (w1 & 0x1ff) as u16;
    let src1 = ((w1 >> 9) & 0x1ff) as u16;
    let src2 = ((w1 >> 18) & 0x1ff) as u16;
    let fields = Vop3Fields {
        abs: (header >> 8) & 7,
        op_sel: (header >> 11) & 0xf,
        clamp: (header >> 15) & 1,
        omod: (w1 >> 27) & 3,
        neg: (w1 >> 29) & 7,
    };
    let is = |p: fn(O) -> bool| op.is_some_and(p);
    let carry_in_out = matches!(
        (op, id),
        (Some(O::VAddcU32), 0x128)
            | (Some(O::VSubCoCiU32), 0x129)
            | (Some(O::VSubrevCoCiU32), 0x12a)
    );
    if has_unsupported_vop3_modifiers(
        op,
        carry_in_out || is(is_vop3b_carry_out) || op == Some(O::VMadU64U32),
        &fields,
    ) {
        let reason = if is(is_permlane) && fields.op_sel & !3 != 0 {
            R::Vop3PermlaneOpSel
        } else {
            R::Vop3Modifiers
        };
        return Err(unsupported(at, family, id, reason));
    }
    let op = op.ok_or(unsupported_opcode(at, family, id))?;
    if op == O::VNop {
        inst.dst = Operand::special(OperandKind::Null);
        return Ok(inst);
    }
    let vop3_vopc = id <= 0xff;
    let native_source = supports_native_source_modifiers(op);
    inst.dst = if vop3_vopc && is_vopc_compare_exec(op) {
        Operand::special(OperandKind::ExecLo)
    } else if vop3_vopc || uses_scalar_destination(op) {
        // VOP3A uses VDST for VOPC and the scalar-destination lane-read opcodes.
        scalar_destination(vdst, at)?
    } else {
        Operand::vgpr(vdst)
    };
    inst.src[0] = scalar_source(src0, at)?;
    inst.dst
        .set(flag::CLAMP, supports_native_clamp(op) && fields.clamp != 0);
    inst.dst.omod = if supports_native_result_modifiers(op) {
        fields.omod as u8
    } else {
        0
    };
    if is_permlane(op) {
        inst.dst.set(flag::OP_SEL, fields.op_sel & 1 != 0);
        inst.dst.set(flag::OP_SEL_HI, fields.op_sel & 2 != 0);
    }
    if vop3_vopc {
        inst.src[1] = scalar_source(src1, at)?;
        inst.src_count = 2;
        if native_source {
            apply_native_source_modifiers(&mut inst, fields.abs, fields.neg);
        }
    } else if (0x180..=0x1ff).contains(&id) {
        inst.src_count = 1;
        if native_source {
            apply_native_source_modifiers(&mut inst, fields.abs, fields.neg);
        }
    } else if carry_in_out || op == O::VMadU64U32 {
        inst.dst2 = scalar_destination(sdst, at)?;
        inst.src[1] = scalar_source(src1, at)?;
        inst.src[2] = scalar_source(src2, at)?;
        inst.src_count = 3;
    } else if is_vop3b_carry_out(op) {
        inst.dst2 = scalar_destination(sdst, at)?;
        inst.src[1] = scalar_source(src1, at)?;
        inst.src_count = 2;
    } else if (0x100..=0x13f).contains(&id) {
        inst.src[1] = scalar_source(src1, at)?;
        if op == O::VCndmaskB32 {
            inst.src[2] = scalar_source(src2, at)?;
            inst.src_count = 3;
        } else {
            inst.src_count = 2;
        }
        if native_source {
            apply_native_source_modifiers(&mut inst, fields.abs, fields.neg);
        }
    } else {
        inst.src[1] = scalar_source(src1, at)?;
        inst.src_count = native_vop3_source_count(op);
        if inst.src_count > 2 {
            inst.src[2] = scalar_source(src2, at)?;
        }
        let (op_sel, abs, neg) = (fields.op_sel, fields.abs, fields.neg);
        if is_native_f16_ternary(op) {
            for (index, src) in inst.src.iter_mut().take(3).enumerate() {
                src.set(flag::OP_SEL, (op_sel >> index) & 1 != 0);
                src.set(flag::OP_SEL_HI, true);
                src.set(flag::NEGATE, (neg >> index) & 1 != 0);
                src.set(flag::ABSOLUTE, (abs >> index) & 1 != 0);
            }
            inst.dst.sdwa_select = half_destination(op_sel);
        } else if is_native_b16_ternary(op) {
            for (index, src) in inst.src.iter_mut().take(3).enumerate() {
                src.set(flag::OP_SEL, (op_sel >> index) & 1 != 0);
            }
            inst.dst.sdwa_select = half_destination(op_sel);
        } else if is_native_b16_binary(op) {
            inst.src[0].set(flag::OP_SEL, op_sel & 1 != 0);
            inst.src[1].set(flag::OP_SEL, op_sel & 2 != 0);
            inst.dst.sdwa_select = half_destination(op_sel);
        } else if op == O::VPackB32F16 {
            for index in 0..2 {
                inst.src[index].set(flag::OP_SEL, (op_sel >> index) & 1 != 0);
                inst.src[index].set(flag::NEGATE, (neg >> index) & 1 != 0);
                inst.src[index].set(flag::ABSOLUTE, (abs >> index) & 1 != 0);
            }
        } else if native_source {
            apply_native_source_modifiers(&mut inst, abs, neg);
        }
    }
    input.literal(&mut inst)?;
    Ok(inst)
}

fn vop3p_source_count(op: O) -> usize {
    match op {
        O::VPkMulLoU16
        | O::VPkAddI16
        | O::VPkSubI16
        | O::VPkLshlrevB16
        | O::VPkLshrrevB16
        | O::VPkAshrrevI16
        | O::VPkMaxI16
        | O::VPkMinI16
        | O::VPkAddU16
        | O::VPkSubU16
        | O::VPkMaxU16
        | O::VPkMinU16
        | O::VPkAddF16
        | O::VPkMulF16
        | O::VPkMinF16
        | O::VPkMaxF16 => 2,
        _ => 3,
    }
}

fn is_packed_vop3p(op: O) -> bool {
    matches!(
        op,
        O::VPkAddF16 | O::VPkMulF16 | O::VPkMinF16 | O::VPkMaxF16 | O::VPkFmaF16
    )
}

pub(crate) fn decode_vop3p(
    input: &mut Input<'_>,
    header: u32,
    at: usize,
) -> Result<Record, DecodeError> {
    let family = Family::Vop3p;
    let id = ((header >> 16) & 0x7f) as u16;
    let op = opcode::vop3p(id);
    let mut inst = record(at, family, id, op.unwrap_or(O::VPkMadI16));
    input.require(&mut inst, 2)?;
    let w1 = input.word(&inst, 1)?;
    let op = op.ok_or(unsupported_opcode(at, family, id))?;
    let neg_hi = (header >> 8) & 7;
    let op_sel = (header >> 11) & 7;
    let clamp = bit(header, 15);
    let op_sel_hi = ((w1 >> 27) & 3) | (((header >> 14) & 1) << 2);
    let neg = (w1 >> 29) & 7;
    inst.src_count = vop3p_source_count(op);
    inst.dst = Operand::vgpr((header & 0xff) as u16);
    inst.src[0] = scalar_source((w1 & 0x1ff) as u16, at)?;
    inst.src[1] = scalar_source(((w1 >> 9) & 0x1ff) as u16, at)?;
    if inst.src_count > 2 {
        inst.src[2] = scalar_source(((w1 >> 18) & 0x1ff) as u16, at)?;
    }
    let mad_mix = matches!(op, O::VMadMixloF16 | O::VMadMixhiF16);
    if op == O::VFmaF32 || mad_mix || is_packed_vop3p(op) {
        inst.dst.set(flag::CLAMP, clamp);
    } else if clamp {
        return Err(unsupported(at, family, id, R::Vop3pIntegerClamp));
    }
    // Modifiers apply to all three encoded slots, as in the source.
    for (index, src) in inst.src.iter_mut().take(3).enumerate() {
        src.set(flag::OP_SEL, (op_sel >> index) & 1 != 0);
        src.set(flag::OP_SEL_HI, (op_sel_hi >> index) & 1 != 0);
        src.set(flag::NEGATE, (neg >> index) & 1 != 0);
        src.set(flag::NEGATE_HI, (neg_hi >> index) & 1 != 0);
    }
    if mad_mix || op == O::VFmaF32 {
        // RDNA2 MIX opcodes reuse the VOP3P NEG_HI bits as source absolute modifiers.
        for src in inst.src.iter_mut().take(3) {
            let abs = src.negate_hi();
            src.set(flag::ABSOLUTE, abs);
            src.set(flag::NEGATE_HI, false);
        }
    }
    match op {
        O::VMadMixloF16 => inst.dst.sdwa_select = SdwaSelect::Word0,
        O::VMadMixhiF16 => inst.dst.sdwa_select = SdwaSelect::Word1,
        _ => {}
    }
    input.literal(&mut inst)?;
    Ok(inst)
}
