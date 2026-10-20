// SPDX-License-Identifier: GPL-2.0-only
// New synthetic VOP1/VOP2/VOPC/VOP3/VOP3P tests for the PentaX Rust port.
// Every word is composed here from field values; no guest/game fixtures.

use super::*;

pub(super) const fn vop1(op: u32, vdst: u32, src0: u32) -> u32 {
    0x7e00_0000 | (vdst << 17) | (op << 9) | src0
}
pub(super) const fn vop2(op: u32, vdst: u32, vsrc1: u32, src0: u32) -> u32 {
    (op << 25) | (vdst << 17) | (vsrc1 << 9) | src0
}
pub(super) const fn vopc(op: u32, vsrc1: u32, src0: u32) -> u32 {
    0x7c00_0000 | (op << 17) | (vsrc1 << 9) | src0
}
/// VOP3 header with an explicit 7-bit field at bits 8..15 (ABS/OP_SEL or SDST).
pub(super) const fn vop3_header(op: u32, vdst: u32, mid: u32, clamp: u32) -> u32 {
    0xd400_0000 | (op << 16) | (clamp << 15) | (mid << 8) | vdst
}
pub(super) const fn vop3(op: u32, vdst: u32, abs: u32, op_sel: u32, clamp: u32) -> u32 {
    vop3_header(op, vdst, abs | (op_sel << 3), clamp)
}
pub(super) const fn src3(src0: u32, src1: u32, src2: u32, omod: u32, neg: u32) -> u32 {
    (neg << 29) | (omod << 27) | (src2 << 18) | (src1 << 9) | src0
}
const fn vop3p(op: u32, vdst: u32, neg_hi: u32, op_sel: u32, op_sel_hi2: u32, clamp: u32) -> u32 {
    0xcc00_0000
        | (op << 16)
        | (clamp << 15)
        | (op_sel_hi2 << 14)
        | (op_sel << 11)
        | (neg_hi << 8)
        | vdst
}

/// SDWA modifier word fields for VOP1/VOP2.
#[derive(Clone, Copy, Default)]
struct Sdwa {
    src0: u32,
    dst_sel: u32,
    dst_u: u32,
    clamp: u32,
    omod: u32,
    sel0: u32,
    sext0: u32,
    neg0: u32,
    abs0: u32,
    s0: u32,
    sel1: u32,
    sext1: u32,
    neg1: u32,
    abs1: u32,
    s1: u32,
}

impl Sdwa {
    const FULL: Self = Self {
        src0: 0,
        dst_sel: 6,
        dst_u: 0,
        clamp: 0,
        omod: 0,
        sel0: 6,
        sext0: 0,
        neg0: 0,
        abs0: 0,
        s0: 0,
        sel1: 6,
        sext1: 0,
        neg1: 0,
        abs1: 0,
        s1: 0,
    };
    fn word(self) -> u32 {
        self.src0
            | (self.dst_sel << 8)
            | (self.dst_u << 11)
            | (self.clamp << 13)
            | (self.omod << 14)
            | (self.sel0 << 16)
            | (self.sext0 << 19)
            | (self.neg0 << 20)
            | (self.abs0 << 21)
            | (self.s0 << 23)
            | (self.sel1 << 24)
            | (self.sext1 << 27)
            | (self.neg1 << 28)
            | (self.abs1 << 29)
            | (self.s1 << 31)
    }
}

#[allow(clippy::too_many_arguments)]
const fn dpp16(
    src0: u32,
    ctrl: u32,
    fi: u32,
    bc: u32,
    neg0: u32,
    abs0: u32,
    neg1: u32,
    abs1: u32,
    bank: u32,
    row: u32,
) -> u32 {
    src0 | (ctrl << 8)
        | (fi << 18)
        | (bc << 19)
        | (neg0 << 20)
        | (abs0 << 21)
        | (neg1 << 22)
        | (abs1 << 23)
        | (bank << 24)
        | (row << 28)
}

/// V_MAC_F32 v0, 0, v0: a valid one-word VOP2 filler.
const MAC: u32 = vop2(0x1f, 0, 0, 128);

fn err(words: &[u32]) -> DecodeError {
    decode(words, ShaderStage::Compute, &Limits::default()).unwrap_err()
}

pub(super) fn reason(family: Family, opcode_id: u16, reason: UnsupportedReason) -> DecodeError {
    DecodeError::Unsupported {
        at_word: 0,
        family,
        opcode_id,
        reason,
    }
}

pub(super) fn bad_opcode(family: Family, opcode_id: u16) -> DecodeError {
    DecodeError::UnsupportedOpcode {
        at_word: 0,
        family,
        opcode_id,
    }
}

pub(super) fn truncated(
    family: Family,
    required_words: usize,
    available_words: usize,
) -> DecodeError {
    DecodeError::Truncated {
        at_word: 0,
        family,
        required_words,
        available_words,
    }
}

fn accepted(
    words: impl Fn(u32) -> Vec<u32>,
    range: u32,
    unmapped: impl Fn(&DecodeError) -> bool,
) -> usize {
    let mut count = 0;
    for id in 0..range {
        match decode(&words(id), ShaderStage::Unknown, &Limits::default()) {
            Ok(span) => {
                assert_eq!(u32::from(span.instruction(0).unwrap().opcode_id()), id);
                count += 1;
            }
            Err(error) => assert!(unmapped(&error), "{id:#x}: {error:?}"),
        }
    }
    count
}

#[test]
fn vector_opcode_maps_accept_exactly_the_source_table_sizes() {
    let opcode = |e: &DecodeError| matches!(e, DecodeError::UnsupportedOpcode { .. });
    // Inline-constant sources keep these one- or two-word encodings literal-free.
    assert_eq!(accepted(|id| vec![vop1(id, 0, 128)], 256, opcode), 63);
    // V_MAC_F32 filler is consumed as the literal of literal-MAD forms.
    // VOP2 ids 0x3e/0x3f classify as VOPC/VOP1, so the sweep stops at 62.
    assert_eq!(
        accepted(|id| vec![vop2(id, 0, 0, 128), MAC], 62, opcode),
        50
    );
    assert_eq!(accepted(|id| vec![vopc(id, 0, 128)], 256, opcode), 135);
    let vop3p_word = |id| vec![vop3p(id, 0, 0, 0, 0, 0), src3(128, 128, 128, 0, 0)];
    assert_eq!(accepted(vop3p_word, 128, opcode), 22);
    // VOP3 = 82 native + 135 VOPC aliases + 42 further VOP2 aliases (seven
    // excluded, 0x12f already native) + 58 VOP1 aliases.
    let vop3_word = |id| vec![vop3(id, 0, 0, 0, 0), src3(128, 128, 128, 0, 0)];
    assert_eq!(accepted(vop3_word, 0x400, opcode), 82 + 135 + 42 + 58);
}

#[test]
fn vop3_alias_ranges_holes_and_exclusions_follow_the_source_lookup() {
    let decode3 = |id: u32| {
        decode(
            &[vop3(id, 0, 0, 0, 0), src3(128, 128, 128, 0, 0)],
            ShaderStage::Compute,
            &Limits::default(),
        )
    };
    let op = |id| decode3(id).unwrap().instruction(0).unwrap().opcode();
    assert_eq!(op(0x002), Opcode::VCmpEqF32);
    assert_eq!(op(0x103), Opcode::VAddF32);
    assert_eq!(op(0x13c), Opcode::VPkFmacF16);
    assert_eq!(op(0x12f), Opcode::VCvtPkrtzF16F32);
    assert_eq!(op(0x181), Opcode::VMovB32);
    assert_eq!(op(0x180), Opcode::VNop);
    assert_eq!(op(0x141), Opcode::VMadF32);
    // Literal-MAD forms, DOT2C and MAX/MIN_F16 are excluded from the VOP2 alias range.
    for vop2_id in [0x20, 0x21, 0x2c, 0x2d, 0x02, 0x39, 0x3a] {
        assert_eq!(
            decode3(0x100 + vop2_id).unwrap_err(),
            bad_opcode(Family::Vop3, 0x100 + vop2_id as u16)
        );
        assert!(
            decode(
                &[vop2(vop2_id, 0, 0, 128), MAC],
                ShaderStage::Compute,
                &Limits::default()
            )
            .is_ok()
        );
    }
    // The VOP1 alias table lacks entries present in VOP1 (0x0b, 0x11..0x14).
    for vop1_id in [0x0b, 0x11, 0x12, 0x13, 0x14] {
        assert!(
            decode(
                &[vop1(vop1_id, 0, 128)],
                ShaderStage::Compute,
                &Limits::default()
            )
            .is_ok()
        );
        assert_eq!(
            decode3(0x180 + vop1_id).unwrap_err(),
            bad_opcode(Family::Vop3, 0x180 + vop1_id as u16)
        );
    }
    // Range 0x140..0x17f is native-only; 0x10 is a VOPC hole.
    assert_eq!(decode3(0x140).unwrap_err(), bad_opcode(Family::Vop3, 0x140));
    assert_eq!(decode3(0x010).unwrap_err(), bad_opcode(Family::Vop3, 0x010));
}

#[test]
fn vop1_destinations_literals_and_nop_follow_source_order() {
    let span = decoded(&[
        vop1(0x02, 105, 300),
        vop1(0x01, 255, 255),
        0xdead_beef,
        vop1(0, 7, 249),
    ]);
    assert_eq!(span.instruction_count(), 3);
    let readfirstlane = span.instruction(0).unwrap();
    assert_eq!(readfirstlane.destination().kind(), OperandKind::Sgpr);
    assert_eq!(readfirstlane.destination().register(), 105);
    assert_eq!(readfirstlane.sources()[0].register(), 44);
    let mov = span.instruction(1).unwrap();
    assert_eq!(mov.raw_words(), &[vop1(0x01, 255, 255), 0xdead_beef]);
    assert_eq!(mov.destination().register(), 255);
    assert_eq!(mov.sources()[0].value(), 0xdead_beef);
    // V_NOP returns before the SDWA selector test: no modifier word follows.
    let nop = span.instruction(2).unwrap();
    assert_eq!(nop.opcode(), Opcode::VNop);
    assert_eq!(nop.raw_words().len(), 1);
    assert_eq!(nop.destination().kind(), OperandKind::Null);
    assert!(nop.sources().is_empty());
    // A scalar-destination opcode rejects a VGPR-only destination selector.
    assert_eq!(
        err(&[vop1(0x02, 200, 0)]),
        DecodeError::UnsupportedOperand {
            at_word: 0,
            selector: 200,
            destination: true
        }
    );
}

#[test]
fn vop1_sdwa_fields_are_retained_and_scalar_literal_follows_the_modifier() {
    let m = Sdwa {
        src0: 7,
        dst_sel: 5,
        dst_u: 0,
        sel0: 2,
        sext0: 1,
        ..Sdwa::FULL
    };
    let span = decoded(&[vop1(0x01, 3, 249), m.word()]);
    let inst = span.instruction(0).unwrap();
    assert_eq!(inst.raw_words().len(), 2);
    let (dst, src) = (inst.destination(), inst.sources()[0]);
    assert_eq!(
        (dst.register(), dst.sdwa_select(), dst.sdwa_unused()),
        (3, SdwaSelect::Word1, SdwaUnused::Pad)
    );
    assert!(dst.explicit_sdwa_destination());
    assert_eq!((src.kind(), src.register()), (OperandKind::Vgpr, 7));
    assert_eq!(src.sdwa_select(), SdwaSelect::Byte2);
    assert!(src.sdwa_sign_extend());
    // S0 selects the scalar space; 255 is then a literal after the modifier word.
    let m = Sdwa {
        src0: 255,
        s0: 1,
        ..Sdwa::FULL
    };
    let span = decoded(&[vop1(0x01, 3, 249), m.word(), 0x1234_5678]);
    let inst = span.instruction(0).unwrap();
    assert_eq!(inst.raw_words().len(), 3);
    assert_eq!(inst.sources()[0].value(), 0x1234_5678);
    assert_eq!(
        err(&[vop1(0x01, 3, 249), m.word()]),
        truncated(Family::Vop1, 3, 2)
    );
    assert_eq!(err(&[vop1(0x01, 3, 249)]), truncated(Family::Vop1, 2, 1));
    // An S0 scalar selector the operand decoder rejects (for example 209).
    let m = Sdwa {
        src0: 209,
        s0: 1,
        ..Sdwa::FULL
    };
    assert_eq!(
        err(&[vop1(0x01, 3, 249), m.word()]),
        DecodeError::UnsupportedOperand {
            at_word: 0,
            selector: 209,
            destination: false
        }
    );
}

#[test]
fn vop1_sdwa_restrictions_return_the_source_reasons() {
    use UnsupportedReason as R;
    let case = |op: u32, m: Sdwa| err(&[vop1(op, 0, 249), m.word()]);
    let r = |op: u32, why| reason(Family::Vop1, op as u16, why);
    assert_eq!(case(0x04, Sdwa::FULL), r(0x04, R::Vop1SdwaFloat64));
    assert_eq!(
        case(
            0x01,
            Sdwa {
                sel0: 7,
                ..Sdwa::FULL
            }
        ),
        r(0x01, R::Vop1SdwaSelector)
    );
    assert_eq!(
        case(
            0x01,
            Sdwa {
                dst_sel: 7,
                ..Sdwa::FULL
            }
        ),
        r(0x01, R::Vop1SdwaSelector)
    );
    assert_eq!(
        case(
            0x01,
            Sdwa {
                clamp: 1,
                ..Sdwa::FULL
            }
        ),
        r(0x01, R::Vop1SdwaOutputModifiers)
    );
    assert_eq!(
        case(
            0x37,
            Sdwa {
                omod: 1,
                ..Sdwa::FULL
            }
        ),
        r(0x37, R::Vop1SdwaOutputModifiers)
    );
    assert_eq!(
        case(
            0x01,
            Sdwa {
                dst_u: 3,
                ..Sdwa::FULL
            }
        ),
        r(0x01, R::Vop1SdwaDestinationSelector)
    );
    // V_BFREV_B32 has no partial destination selectors.
    assert_eq!(
        case(
            0x38,
            Sdwa {
                dst_sel: 4,
                ..Sdwa::FULL
            }
        ),
        r(0x38, R::Vop1SdwaDestinationSelector)
    );
    // V_MOV_B32 has no source modifiers on a partial source; full-width float source allows them.
    assert_eq!(
        case(
            0x01,
            Sdwa {
                sel0: 0,
                neg0: 1,
                ..Sdwa::FULL
            }
        ),
        r(0x01, R::Vop1SdwaSourceSelector)
    );
    assert!(
        decoded(&[
            vop1(0x01, 0, 249),
            Sdwa {
                neg0: 1,
                ..Sdwa::FULL
            }
            .word()
        ])
        .instruction(0)
        .unwrap()
        .sources()[0]
            .negate()
    );
    // Integer full-width source with modifiers is rejected.
    assert_eq!(
        case(
            0x37,
            Sdwa {
                abs0: 1,
                ..Sdwa::FULL
            }
        ),
        r(0x37, R::Vop1SdwaSourceSelector)
    );
    // V_CVT_F16_U16 rejects a sign-extended byte source.
    assert_eq!(
        case(
            0x50,
            Sdwa {
                sel0: 1,
                sext0: 1,
                ..Sdwa::FULL
            }
        ),
        r(0x50, R::Vop1SdwaSourceSelector)
    );
    assert!(
        decode(
            &[
                vop1(0x50, 0, 249),
                Sdwa {
                    sel0: 1,
                    ..Sdwa::FULL
                }
                .word()
            ],
            ShaderStage::Compute,
            &Limits::default()
        )
        .is_ok()
    );
    // Float result opcodes accept clamp and omod.
    let span = decoded(&[
        vop1(0x2a, 0, 249),
        Sdwa {
            clamp: 1,
            omod: 2,
            ..Sdwa::FULL
        }
        .word(),
    ]);
    let dst = span.instruction(0).unwrap().destination();
    assert!(dst.clamp());
    assert_eq!(dst.output_modifier(), 2);
}

#[test]
fn vop1_dpp16_and_dpp8_fields_and_integer_modifier_rules() {
    let m = dpp16(9, 0x1ab, 1, 1, 1, 1, 0, 0, 0x5, 0xa);
    let span = decoded(&[vop1(0x01, 3, 250), m]);
    let src = span.instruction(0).unwrap().sources()[0];
    assert_eq!((src.kind(), src.register()), (OperandKind::Vgpr, 9));
    assert!(src.negate() && src.absolute());
    let dpp = src.dpp().unwrap();
    assert_eq!(dpp.kind(), DppKind::Dpp16);
    assert_eq!(
        (dpp.control(), dpp.fetch_inactive(), dpp.bound_control()),
        (0x1ab, true, true)
    );
    assert_eq!((dpp.bank_mask(), dpp.row_mask()), (0x5, 0xa));
    // DPP8: 24-bit lane selects, FI from selector 234, default masks, no modifiers.
    for (selector, fi) in [(233, false), (234, true)] {
        let m = 9 | (0xfad688 << 8);
        let span = decoded(&[vop1(0x37, 3, selector), m]);
        let src = span.instruction(0).unwrap().sources()[0];
        let dpp = src.dpp().unwrap();
        assert_eq!(
            (dpp.kind(), dpp.control(), dpp.fetch_inactive()),
            (DppKind::Dpp8, 0xfad688, fi)
        );
        assert_eq!(
            (dpp.bank_mask(), dpp.row_mask(), dpp.bound_control()),
            (0xf, 0xf, false)
        );
        // Bit 20 is a lane-select bit here, not a negate on an integer opcode.
        assert!(!src.negate());
    }
    assert_eq!(
        err(&[vop1(0x37, 3, 250), dpp16(9, 0, 0, 0, 1, 0, 0, 0, 0xf, 0xf)]),
        reason(
            Family::Vop1,
            0x37,
            UnsupportedReason::Vop1DppIntegerSourceModifiers
        )
    );
    // FP64 DPP is rejected before the source reads the modifier word.
    assert_eq!(
        err(&[vop1(0x04, 3, 250)]),
        reason(Family::Vop1, 0x04, UnsupportedReason::Vop1DppFloat64)
    );
    assert_eq!(err(&[vop1(0x01, 3, 233)]), truncated(Family::Vop1, 2, 1));
    assert!(
        decoded(&[vop1(0x01, 3, 128)])
            .instruction(0)
            .unwrap()
            .sources()[0]
            .dpp()
            .is_none()
    );
}

#[test]
fn vop2_implicit_literal_and_carry_operands() {
    // V_MADMK_F32 with a literal SRC0: one shared literal fills src0 and src1.
    let span = decoded(&[vop2(0x20, 1, 2, 255), 0x4000_0000]);
    let inst = span.instruction(0).unwrap();
    assert_eq!(inst.raw_words().len(), 2);
    let kinds: Vec<_> = inst
        .sources()
        .iter()
        .map(|s| (s.kind(), s.value()))
        .collect();
    assert_eq!(
        kinds,
        [
            (OperandKind::LiteralConstant, 0x4000_0000),
            (OperandKind::LiteralConstant, 0x4000_0000),
            (OperandKind::Vgpr, 0)
        ]
    );
    assert_eq!(inst.sources()[2].register(), 2);
    // V_MADAK_F32 appends the literal as src2 and must have it.
    assert_eq!(err(&[vop2(0x21, 1, 2, 3)]), truncated(Family::Vop2, 2, 1));
    let span = decoded(&[vop2(0x21, 1, 2, 3), 7]);
    assert_eq!(span.instruction(0).unwrap().sources()[2].value(), 7);
    // Carry forms read and write VCC_LO.
    let span = decoded(&[vop2(0x28, 1, 2, 3)]);
    let inst = span.instruction(0).unwrap();
    assert_eq!(inst.secondary_destination().kind(), OperandKind::VccLo);
    assert_eq!(inst.sources().len(), 3);
    assert_eq!(inst.sources()[2].kind(), OperandKind::VccLo);
    // V_PK_FMAC_F16 implicitly reads/writes high halves.
    let span = decoded(&[vop2(0x3c, 1, 2, 3)]);
    let inst = span.instruction(0).unwrap();
    assert!(
        inst.destination().op_sel_hi()
            && inst.sources()[0].op_sel_hi()
            && inst.sources()[1].op_sel_hi()
    );
    assert_eq!(
        decoded(&[vop2(0x03, 1, 2, 3)])
            .instruction(0)
            .unwrap()
            .secondary_destination()
            .kind(),
        OperandKind::Unknown
    );
}

#[test]
fn vop2_dpp_modifiers_and_literal_after_the_modifier_word() {
    let m = dpp16(4, 0x101, 0, 0, 1, 0, 1, 1, 0xf, 0xf);
    let span = decoded(&[vop2(0x03, 1, 2, 250), m]);
    let inst = span.instruction(0).unwrap();
    assert!(inst.sources()[0].negate());
    assert!(inst.sources()[1].negate() && inst.sources()[1].absolute());
    assert!(inst.sources()[1].dpp().is_none());
    // Packed FMAC mirrors negate into negate_hi.
    let span = decoded(&[vop2(0x3c, 1, 2, 250), m]);
    let inst = span.instruction(0).unwrap();
    assert!(inst.sources()[0].negate_hi() && inst.sources()[1].negate_hi());
    assert_eq!(
        err(&[
            vop2(0x1b, 1, 2, 250),
            dpp16(4, 0, 0, 0, 0, 0, 0, 1, 0xf, 0xf)
        ]),
        reason(
            Family::Vop2,
            0x1b,
            UnsupportedReason::Vop2DppIntegerSourceModifiers
        )
    );
    // DPP8 ignores the src1 modifier bits (lane-select bits).
    let span = decoded(&[vop2(0x1b, 1, 2, 233), 4 | (0xc000 << 8)]);
    assert!(!span.instruction(0).unwrap().sources()[1].absolute());
    // V_FMAMK_F32 DPP: the literal follows the modifier word (three words).
    let span = decoded(&[vop2(0x2c, 1, 2, 250), m & 0x0fff_ffff, 0x3f80_0000]);
    let inst = span.instruction(0).unwrap();
    assert_eq!(inst.raw_words().len(), 3);
    assert_eq!(inst.sources()[1].value(), 0x3f80_0000);
    assert_eq!(
        err(&[vop2(0x2c, 1, 2, 250), m]),
        truncated(Family::Vop2, 3, 2)
    );
}

#[test]
fn vop2_sdwa_profiles_and_shared_scalar_literals() {
    use UnsupportedReason as R;
    let case = |op: u32, m: Sdwa| err(&[vop2(op, 1, 2, 249), m.word()]);
    let r = |op: u32, why| reason(Family::Vop2, op as u16, why);
    // Full-width float SDWA is accepted even without a profile.
    assert!(
        decode(
            &[
                vop2(0x05, 1, 2, 249),
                Sdwa {
                    neg0: 1,
                    abs1: 1,
                    ..Sdwa::FULL
                }
                .word()
            ],
            ShaderStage::Compute,
            &Limits::default()
        )
        .is_ok()
    );
    // A non-full-width form needs a profile.
    assert_eq!(
        case(
            0x05,
            Sdwa {
                dst_u: 1,
                ..Sdwa::FULL
            }
        ),
        r(0x05, R::Vop2SdwaOpcode)
    );
    assert_eq!(
        case(
            0x1b,
            Sdwa {
                neg0: 1,
                ..Sdwa::FULL
            }
        ),
        r(0x1b, R::Vop2SdwaSourceModifiers)
    );
    assert!(
        decode(
            &[
                vop2(0x01, 1, 2, 249),
                Sdwa {
                    neg0: 1,
                    ..Sdwa::FULL
                }
                .word()
            ],
            ShaderStage::Compute,
            &Limits::default()
        )
        .is_ok()
    );
    assert_eq!(
        case(
            0x1b,
            Sdwa {
                clamp: 1,
                ..Sdwa::FULL
            }
        ),
        r(0x1b, R::Vop2SdwaOutputModifiers)
    );
    assert_eq!(
        case(
            0x1b,
            Sdwa {
                sel1: 7,
                ..Sdwa::FULL
            }
        ),
        r(0x1b, R::Vop2SdwaSelector)
    );
    // Bitwise: word destinations only; Float32: full sources only.
    let span = decoded(&[
        vop2(0x1b, 1, 2, 249),
        Sdwa {
            dst_sel: 5,
            sel0: 0,
            sel1: 3,
            ..Sdwa::FULL
        }
        .word(),
    ]);
    let inst = span.instruction(0).unwrap();
    assert_eq!(inst.destination().sdwa_select(), SdwaSelect::Word1);
    assert_eq!(
        (
            inst.sources()[0].sdwa_select(),
            inst.sources()[1].sdwa_select()
        ),
        (SdwaSelect::Byte0, SdwaSelect::Byte3)
    );
    assert_eq!(
        case(
            0x1b,
            Sdwa {
                dst_sel: 0,
                ..Sdwa::FULL
            }
        ),
        r(0x1b, R::Vop2SdwaDestinationSelector)
    );
    assert_eq!(
        case(
            0x1b,
            Sdwa {
                dst_u: 3,
                sel0: 0,
                ..Sdwa::FULL
            }
        ),
        r(0x1b, R::Vop2SdwaDestinationSelector)
    );
    assert_eq!(
        case(
            0x03,
            Sdwa {
                sel0: 0,
                ..Sdwa::FULL
            }
        ),
        r(0x03, R::Vop2SdwaSourceSelector)
    );
    assert_eq!(
        case(
            0x1b,
            Sdwa {
                sel0: 0,
                abs0: 1,
                ..Sdwa::FULL
            }
        ),
        r(0x1b, R::Vop2SdwaSourceModifiers)
    );
    // S0 and S1 both selecting literal 255 share one word after the modifier.
    let m = Sdwa {
        src0: 255,
        s0: 1,
        s1: 1,
        ..Sdwa::FULL
    };
    let span = decoded(&[vop2(0x03, 1, 255, 249), m.word(), 0x4120_0000]);
    let inst = span.instruction(0).unwrap();
    assert_eq!(inst.raw_words().len(), 3);
    assert!(
        inst.sources()
            .iter()
            .all(|s| s.kind() == OperandKind::LiteralConstant && s.value() == 0x4120_0000)
    );
    // V_MADMK_F32 full-width SDWA: src1's SDWA source moves to src2.
    let m = Sdwa {
        s1: 1,
        ..Sdwa::FULL
    };
    let span = decoded(&[vop2(0x20, 1, 5, 249), m.word(), 9]);
    let inst = span.instruction(0).unwrap();
    assert_eq!(
        (inst.sources()[2].kind(), inst.sources()[2].register()),
        (OperandKind::Sgpr, 5)
    );
    assert_eq!(inst.sources()[1].value(), 9);
}

#[test]
fn vopc_destinations_sdwa_and_dpp_rules() {
    use UnsupportedReason as R;
    let span = decoded(&[vopc(0x02, 1, 2), vopc(0x12, 1, 2), vopc(0xc2, 1, 255), 5]);
    let kinds: Vec<_> = span
        .instructions()
        .map(|i| i.destination().kind())
        .collect();
    assert_eq!(
        kinds,
        [OperandKind::VccLo, OperandKind::ExecLo, OperandKind::VccLo]
    );
    assert_eq!(span.instruction(2).unwrap().sources()[0].value(), 5);
    let sdwa =
        |sdst: u32, sd: u32, sel0: u32| 3 | (sdst << 8) | (sd << 15) | (sel0 << 16) | (6 << 24);
    let span = decoded(&[
        vopc(0xc2, 1, 249),
        sdwa(10, 1, 4),
        vopc(0xc2, 1, 249),
        sdwa(10, 0, 6),
        vopc(0x12, 1, 249),
        sdwa(10, 1, 6),
    ]);
    let dst: Vec<_> = span
        .instructions()
        .map(|i| (i.destination().kind(), i.destination().register()))
        .collect();
    assert_eq!(
        dst,
        [
            (OperandKind::Sgpr, 10),
            (OperandKind::VccLo, 0),
            (OperandKind::ExecLo, 0)
        ]
    );
    assert_eq!(
        span.instruction(0).unwrap().sources()[0].sdwa_select(),
        SdwaSelect::Word0
    );
    assert_eq!(
        err(&[vopc(0x22, 1, 249), sdwa(0, 0, 6)]),
        reason(Family::Vopc, 0x22, R::VopcSdwaOpcode)
    );
    // A map hole with SDWA reports the SDWA reason, after the selector check.
    assert_eq!(
        err(&[vopc(0x10, 1, 249), sdwa(0, 0, 6)]),
        reason(Family::Vopc, 0x10, R::VopcSdwaOpcode)
    );
    assert_eq!(
        err(&[vopc(0x10, 1, 249), sdwa(0, 0, 7)]),
        reason(Family::Vopc, 0x10, R::VopcSdwaSelector)
    );
    assert_eq!(err(&[vopc(0x10, 1, 249)]), truncated(Family::Vopc, 2, 1));
    // DPP: per-opcode support flag; a hole is an opcode error after the word.
    let m = dpp16(4, 0, 0, 0, 0, 0, 1, 0, 0xf, 0xf);
    let span = decoded(&[vopc(0x12, 1, 250), m]);
    let inst = span.instruction(0).unwrap();
    assert_eq!(inst.destination().kind(), OperandKind::ExecLo);
    assert!(inst.sources()[1].negate());
    assert_eq!(
        err(&[vopc(0x22, 1, 250), m]),
        reason(Family::Vopc, 0x22, R::VopcDppOpcode)
    );
    assert_eq!(
        err(&[vopc(0x10, 1, 250), m]),
        bad_opcode(Family::Vopc, 0x10)
    );
    assert_eq!(err(&[vopc(0x10, 1, 250)]), truncated(Family::Vopc, 2, 1));
    assert_eq!(err(&[vopc(0x10, 1, 2)]), bad_opcode(Family::Vopc, 0x10));
}

#[test]
fn vop3_reads_its_second_word_before_the_opcode_and_shares_literals() {
    assert_eq!(
        err(&[vop3(0x3ff, 0, 0, 0, 0)]),
        truncated(Family::Vop3, 2, 1)
    );
    assert_eq!(
        err(&[vop3(0x3ff, 0, 0, 0, 0), 0]),
        bad_opcode(Family::Vop3, 0x3ff)
    );
    // Modifier rejection precedes the opcode check, as in the source.
    assert_eq!(
        err(&[vop3(0x3ff, 0, 1, 0, 0), 0]),
        reason(Family::Vop3, 0x3ff, UnsupportedReason::Vop3Modifiers)
    );
    // V_MAD_F32 with SRC0 and SRC2 literal: one word at index 2.
    let span = decoded(&[
        vop3(0x141, 4, 0, 0, 0),
        src3(255, 1, 255, 0, 0),
        0x4040_0000,
    ]);
    let inst = span.instruction(0).unwrap();
    assert_eq!(inst.raw_words().len(), 3);
    let values: Vec<_> = inst.sources().iter().map(|s| s.value()).collect();
    assert_eq!(values, [0x4040_0000, 0, 0x4040_0000]);
    assert_eq!(
        err(&[vop3(0x141, 4, 0, 0, 0), src3(255, 1, 1, 0, 0)]),
        truncated(Family::Vop3, 3, 2)
    );
    // A two-source opcode does not decode SRC2, so 255 there needs no literal.
    let span = decoded(&[vop3(0x169, 4, 0, 0, 0), src3(1, 2, 255, 0, 0)]);
    assert_eq!(span.instruction(0).unwrap().raw_words().len(), 2);
    assert_eq!(
        span.instruction(0).unwrap().encoded_sources()[2].kind(),
        OperandKind::Unknown
    );
}

#[test]
fn vop3_destination_forms_follow_the_source() {
    // VOPC alias: EXEC for CMPX, otherwise VDST as a scalar destination.
    let span = decoded(&[
        vop3(0x012, 0, 0, 0, 0),
        src3(1, 2, 0, 0, 0),
        vop3(0x002, 106, 0, 0, 0),
        src3(1, 2, 0, 0, 0),
    ]);
    assert_eq!(
        span.instruction(0).unwrap().destination().kind(),
        OperandKind::ExecLo
    );
    assert_eq!(
        span.instruction(1).unwrap().destination().kind(),
        OperandKind::VccLo
    );
    assert_eq!(
        err(&[vop3(0x002, 200, 0, 0, 0), src3(1, 2, 0, 0, 0)]),
        DecodeError::UnsupportedOperand {
            at_word: 0,
            selector: 200,
            destination: true
        }
    );
    // Carry-in/out (VOP2 alias 0x128), carry-out V_ADD_I32 and V_MAD_U64_U32
    // decode SDST into the secondary destination.
    for (op, sources) in [(0x128, 3), (0x30f, 2), (0x176, 3)] {
        let span = decoded(&[vop3_header(op, 4, 106, 0), src3(1, 2, 3, 0, 0)]);
        let inst = span.instruction(0).unwrap();
        assert_eq!(
            inst.secondary_destination().kind(),
            OperandKind::VccLo,
            "{op:#x}"
        );
        assert_eq!(inst.sources().len(), sources);
        assert_eq!(inst.destination().register(), 4);
        // These forms reject clamp/omod/neg, not SDST bits.
        assert_eq!(
            err(&[vop3_header(op, 4, 106, 1), src3(1, 2, 3, 0, 0)]),
            reason(Family::Vop3, op as u16, UnsupportedReason::Vop3Modifiers)
        );
    }
    // V_READLANE_B32 uses a scalar destination.
    let span = decoded(&[vop3(0x360, 50, 0, 0, 0), src3(256, 1, 0, 0, 0)]);
    assert_eq!(
        span.instruction(0).unwrap().destination().kind(),
        OperandKind::Sgpr
    );
    // VOP1 alias has one source; V_NOP alias has none.
    assert_eq!(
        decoded(&[vop3(0x181, 1, 0, 0, 0), src3(1, 2, 3, 0, 0)])
            .instruction(0)
            .unwrap()
            .sources()
            .len(),
        1
    );
    let nop = decoded(&[vop3(0x180, 1, 0, 0, 0), src3(255, 0, 0, 0, 0)]);
    assert_eq!(nop.instruction(0).unwrap().raw_words().len(), 2);
    assert!(nop.instruction(0).unwrap().sources().is_empty());
}

#[test]
fn vop3_modifier_classes_are_ported_exactly() {
    use UnsupportedReason as R;
    let w1 = src3(1, 2, 3, 0, 0);
    let ok = |w0: u32, w1: u32| decoded(&[w0, w1]);
    let bad =
        |w0: u32, w1: u32, op: u16, why| assert_eq!(err(&[w0, w1]), reason(Family::Vop3, op, why));
    // Integer opcode: no modifiers at all.
    bad(vop3(0x148, 0, 1, 0, 0), w1, 0x148, R::Vop3Modifiers);
    bad(
        vop3(0x148, 0, 0, 0, 0),
        src3(1, 2, 3, 1, 0),
        0x148,
        R::Vop3Modifiers,
    );
    // Float source/result opcode: abs/neg/clamp/omod retained, op_sel rejected.
    let span = ok(vop3(0x141, 0, 5, 0, 1), src3(1, 2, 3, 3, 2));
    let inst = span.instruction(0).unwrap();
    assert!(
        inst.sources()[0].absolute()
            && !inst.sources()[1].absolute()
            && inst.sources()[2].absolute()
    );
    assert!(inst.sources()[1].negate());
    assert!(inst.destination().clamp());
    assert_eq!(inst.destination().output_modifier(), 3);
    bad(vop3(0x141, 0, 0, 1, 0), w1, 0x141, R::Vop3Modifiers);
    // Perm-lane: OP_SEL bits 0..1 become destination fields; bit 2 has its own reason.
    let span = ok(vop3(0x377, 0, 0, 3, 0), w1);
    let dst = span.instruction(0).unwrap().destination();
    assert!(dst.op_sel() && dst.op_sel_hi());
    bad(vop3(0x377, 0, 0, 4, 0), w1, 0x377, R::Vop3PermlaneOpSel);
    bad(vop3(0x377, 0, 1, 0, 0), w1, 0x377, R::Vop3Modifiers);
    // Native f16 ternary: OP_SEL and OP_SEL_HI, destination half from OP_SEL bit 3.
    let span = ok(vop3(0x34b, 0, 2, 0b1001, 1), src3(1, 2, 3, 1, 4));
    let inst = span.instruction(0).unwrap();
    assert!(inst.sources()[0].op_sel() && !inst.sources()[1].op_sel());
    assert!(inst.sources().iter().all(|s| s.op_sel_hi()));
    assert!(inst.sources()[1].absolute() && inst.sources()[2].negate());
    assert_eq!(inst.destination().sdwa_select(), SdwaSelect::Word1);
    assert_eq!(inst.destination().output_modifier(), 1);
    bad(
        vop3(0x351, 0, 0, 0, 0),
        src3(1, 2, 3, 1, 0),
        0x351,
        R::Vop3Modifiers,
    );
    // Native b16 binary: OP_SEL only.
    let span = ok(vop3(0x303, 0, 0, 0b0011, 0), w1);
    let inst = span.instruction(0).unwrap();
    assert!(inst.sources()[0].op_sel() && inst.sources()[1].op_sel());
    assert_eq!(inst.destination().sdwa_select(), SdwaSelect::Word0);
    bad(vop3(0x303, 0, 0, 0, 1), w1, 0x303, R::Vop3Modifiers);
    // Explicit per-opcode masks.
    assert!(
        ok(vop3(0x362, 0, 1, 0, 0), w1)
            .instruction(0)
            .unwrap()
            .sources()[0]
            .absolute()
    );
    bad(vop3(0x362, 0, 2, 0, 0), w1, 0x362, R::Vop3Modifiers);
    assert_eq!(
        ok(vop3(0x101, 0, 3, 0, 0), w1)
            .instruction(0)
            .unwrap()
            .sources()
            .len(),
        3
    );
    bad(vop3(0x101, 0, 4, 0, 0), w1, 0x101, R::Vop3Modifiers);
    let span = ok(vop3(0x311, 0, 2, 1, 0), src3(1, 2, 3, 0, 1));
    let inst = span.instruction(0).unwrap();
    assert!(
        inst.sources()[0].op_sel() && inst.sources()[0].negate() && inst.sources()[1].absolute()
    );
    bad(vop3(0x311, 0, 0, 4, 0), w1, 0x311, R::Vop3Modifiers);
}

#[test]
fn vop3p_sources_clamp_and_mix_modifiers() {
    use UnsupportedReason as R;
    assert_eq!(
        err(&[vop3p(0x13, 0, 0, 0, 0, 0)]),
        truncated(Family::Vop3p, 2, 1)
    );
    assert_eq!(
        err(&[vop3p(0x13, 0, 0, 0, 0, 0), 0]),
        bad_opcode(Family::Vop3p, 0x13)
    );
    // Two-source packed integer: SRC2 is not decoded, so its 255 needs no literal,
    // yet the source still writes modifier bits into the third slot.
    let span = decoded(&[
        vop3p(0x02, 1, 0b100, 0b001, 1, 0),
        src3(1, 2, 255, 0b10, 0b011),
    ]);
    let inst = span.instruction(0).unwrap();
    assert_eq!(inst.raw_words().len(), 2);
    assert_eq!(inst.sources().len(), 2);
    assert!(inst.sources()[0].op_sel() && inst.sources()[0].negate() && inst.sources()[1].negate());
    assert!(!inst.sources()[0].op_sel_hi() && inst.sources()[1].op_sel_hi());
    let third = inst.encoded_sources()[2];
    assert_eq!(third.kind(), OperandKind::Unknown);
    assert!(third.op_sel_hi() && third.negate_hi());
    assert_eq!(
        err(&[vop3p(0x02, 1, 0, 0, 0, 1), src3(1, 2, 3, 0, 0)]),
        reason(Family::Vop3p, 0x02, R::Vop3pIntegerClamp)
    );
    // Packed float FMA keeps clamp and NEG_HI.
    let span = decoded(&[vop3p(0x0e, 1, 0b010, 0, 0, 1), src3(1, 2, 3, 0, 0)]);
    let inst = span.instruction(0).unwrap();
    assert!(
        inst.destination().clamp()
            && inst.sources()[1].negate_hi()
            && !inst.sources()[1].absolute()
    );
    // MIX opcodes and V_FMA_F32 reinterpret NEG_HI as absolute.
    for (op, select) in [
        (0x21, SdwaSelect::Word0),
        (0x22, SdwaSelect::Word1),
        (0x20, SdwaSelect::Dword),
    ] {
        let span = decoded(&[vop3p(op, 1, 0b101, 0, 0, 1), src3(1, 2, 255, 0, 0), 0x3c00]);
        let inst = span.instruction(0).unwrap();
        assert_eq!(inst.raw_words().len(), 3);
        assert!(
            inst.sources()[0].absolute()
                && !inst.sources()[1].absolute()
                && inst.sources()[2].absolute()
        );
        assert!(inst.sources().iter().all(|s| !s.negate_hi()));
        assert_eq!(inst.destination().sdwa_select(), select);
        assert!(inst.destination().clamp());
    }
}
