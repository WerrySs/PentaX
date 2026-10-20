// SPDX-License-Identifier: GPL-2.0-only
// New synthetic tests written for the PentaX Rust port. No guest/game fixtures.

mod boundaries;
mod memory;
mod vector;

use std::mem::size_of;

use super::*;
use crate::decode::{scalar_destination, scalar_source};
use crate::model::Record;

const fn sop1(id: u32, dst: u32, src: u32) -> u32 {
    0xbe80_0000 | (dst << 16) | (id << 8) | src
}
const fn sop2(id: u32, dst: u32, src0: u32, src1: u32) -> u32 {
    0x8000_0000 | (id << 23) | (dst << 16) | (src1 << 8) | src0
}
const fn sopk(id: u32, dst: u32, imm: u16) -> u32 {
    0xb000_0000 | (id << 23) | (dst << 16) | imm as u32
}
const fn sopc(id: u32, src0: u32, src1: u32) -> u32 {
    0xbf00_0000 | (id << 16) | (src1 << 8) | src0
}
const fn sopp(id: u32, imm: u16) -> u32 {
    0xbf80_0000 | (id << 16) | imm as u32
}

fn decoded(words: &[u32]) -> DecodedSpan {
    decode(words, ShaderStage::Compute, &Limits::default()).unwrap()
}

#[test]
fn every_observed_family_is_classified_without_guessing_a_body() {
    // Classification is unchanged. The previously deferred families now have
    // ported decoders; a lone header word gets its source-defined outcome
    // (decoded, unmapped opcode, or a truncated second word) instead of
    // UnsupportedFamily, and no length is guessed.
    let truncated = |family| {
        Err(DecodeError::Truncated {
            at_word: 0,
            family,
            required_words: 2,
            available_words: 1,
        })
    };
    let families = [
        (0xbe80_0000, Family::Sop1, None),
        (0x8000_0000, Family::Sop2, None),
        (0xb000_0000, Family::Sopk, None),
        (0xbf00_0000, Family::Sopc, None),
        (0xbf80_0000, Family::Sopp, None),
        (0x7e00_0000, Family::Vop1, Some(Ok(1))),
        (
            0,
            Family::Vop2,
            Some(Err(DecodeError::UnsupportedOpcode {
                at_word: 0,
                family: Family::Vop2,
                opcode_id: 0,
            })),
        ),
        (0xd400_0000, Family::Vop3, Some(truncated(Family::Vop3))),
        (0xcc00_0000, Family::Vop3p, Some(truncated(Family::Vop3p))),
        (0x7c00_0000, Family::Vopc, Some(Ok(1))),
        (0xc800_0000, Family::Vintrp, None),
        (0xf400_0000, Family::Smem, Some(truncated(Family::Smem))),
        (0xe000_0000, Family::Mubuf, Some(truncated(Family::Mubuf))),
        (0xe800_0000, Family::Mtbuf, Some(truncated(Family::Mtbuf))),
        (0xdc00_0000, Family::Flat, Some(truncated(Family::Flat))),
        (0xd800_0000, Family::Ds, Some(truncated(Family::Ds))),
        (0xf000_0000, Family::Mimg, Some(truncated(Family::Mimg))),
        (0xf800_0000, Family::Exp, None),
        (0xfc00_0000, Family::Unknown, None),
    ];
    for (word, family, outcome) in families {
        assert_eq!(instruction_family(word), family);
        let result = decode(&[word], ShaderStage::Pixel, &Limits::default());
        if let Err(error) = result.as_ref() {
            assert!(!matches!(error, DecodeError::UnsupportedFamily { .. }));
        }
        if let Some(expected) = outcome {
            assert_eq!(
                result.map(|span| span.instruction_count()),
                expected,
                "{family:?}"
            );
        }
    }
}

#[test]
fn scalar_maps_preserve_sparse_opcode_acceptance() {
    // Each family is tested through encoded words, not its lookup helper.
    // Fixed map counts come from the pinned producer's scalar tables.
    for (family, count, range) in [
        (Family::Sop1, 47, 256),
        (Family::Sop2, 50, 96),
        (Family::Sopk, 23, 29),
        (Family::Sopc, 18, 128),
        (Family::Sopp, 21, 128),
    ] {
        let mut accepted = 0;
        for id in 0..range {
            let word = match family {
                Family::Sop1 => sop1(id, 0, 128),
                Family::Sop2 => sop2(id, 0, 128, 128),
                Family::Sopk => sopk(id, 0, 0),
                Family::Sopc => sopc(id, 128, 128),
                Family::Sopp => sopp(id, 0),
                _ => unreachable!(),
            };
            match decode(&[word], ShaderStage::Unknown, &Limits::default()) {
                Ok(span) => {
                    accepted += 1;
                    let inst = span.instruction(0).unwrap();
                    assert_eq!(inst.family(), family);
                    assert_eq!(u32::from(inst.opcode_id()), id);
                    assert_eq!(inst.raw_words(), &[word]);
                    assert!(!inst.opcode().mnemonic().is_empty());
                }
                Err(error) => assert_eq!(
                    error,
                    DecodeError::UnsupportedOpcode {
                        at_word: 0,
                        family,
                        opcode_id: id as u16,
                    }
                ),
            }
        }
        assert_eq!(accepted, count, "{family:?}");
    }
}

#[test]
fn representative_sparse_and_boundary_opcodes_retain_their_meaning() {
    let cases = [
        (sop1(0x45, 105, 105), Opcode::SOrn1SaveexecB32),
        (sop2(0x36, 105, 128, 208), Opcode::SMulHiI32),
        (sopc(0x13, 126, 127), Opcode::SCmpLgU64),
        (sopk(0x1c, 105, 0), Opcode::SSubvectorLoopEnd),
        (sopp(0x23, 0xffff), Opcode::SWaitcntDepctr),
    ];
    for (word, expected) in cases {
        let span = decoded(&[word]);
        assert_eq!(span.instruction(0).unwrap().opcode(), expected);
    }
}

#[test]
fn literal_word_is_shared_and_never_reclassified() {
    let header = sop2(0, 3, 255, 255);
    // This literal would be an unsupported family if treated as an instruction.
    let literal = 0xffff_ffff;
    let span = decoded(&[header, literal, sopp(1, 0)]);
    assert_eq!(span.instruction_count(), 2);
    let first = span.instruction(0).unwrap();
    assert_eq!(first.raw_words(), &[header, literal]);
    assert_eq!(first.destination().register(), 3);
    assert_eq!(first.sources().len(), 2);
    for source in first.sources() {
        assert_eq!(source.kind(), OperandKind::LiteralConstant);
        assert_eq!(source.value(), literal);
        assert_eq!(source.signed_value(), -1);
    }
    assert_eq!(span.instruction(1).unwrap().word_offset(), 2);
    assert_eq!(span.instruction(1).unwrap().byte_offset(), 8);
}

#[test]
fn literal_on_either_binary_source_and_unary_source_consumes_one_word() {
    for header in [
        sop2(0, 0, 255, 128),
        sop2(0, 0, 128, 255),
        sopc(0, 255, 255),
        sop1(3, 0, 255),
    ] {
        let span = decoded(&[header, 0x8123_4567]);
        assert_eq!(span.instruction_count(), 1);
        let inst = span.instruction(0).unwrap();
        assert_eq!(inst.raw_words().len(), 2);
        assert!(inst.sources().iter().any(|src| src.kind() == OperandKind::LiteralConstant &&
            src.value() == 0x8123_4567));
    }
}

#[test]
fn every_additional_word_path_reports_truncation_at_its_instruction() {
    for header in [
        sop1(3, 0, 255),
        sop2(0, 0, 255, 255),
        sopc(0, 255, 128),
        0xf800_0000,
    ] {
        let words = [sopp(0, 0), header];
        assert_eq!(
            decode(&words, ShaderStage::Compute, &Limits::default()).unwrap_err(),
            DecodeError::Truncated {
                at_word: 1,
                family: instruction_family(header),
                required_words: 2,
                available_words: 1
            }
        );
    }
}

#[test]
fn invalid_opcode_and_operand_do_not_request_an_unproven_literal() {
    assert_eq!(
        decode(&[sop1(0, 0, 255)], ShaderStage::Compute, &Limits::default()).unwrap_err(),
        DecodeError::UnsupportedOpcode {
            at_word: 0,
            family: Family::Sop1,
            opcode_id: 0
        }
    );
    assert_eq!(
        decode(
            &[sop2(0, 0, 255, 254)],
            ShaderStage::Compute,
            &Limits::default()
        )
        .unwrap_err(),
        DecodeError::UnsupportedOperand {
            at_word: 0,
            selector: 254,
            destination: false
        }
    );
}

#[test]
fn scalar_unary_source_error_precedes_destination_error_as_in_the_fork() {
    assert_eq!(
        decode(
            &[sop1(3, 108, 254)],
            ShaderStage::Compute,
            &Limits::default()
        )
        .unwrap_err(),
        DecodeError::UnsupportedOperand {
            at_word: 0,
            selector: 254,
            destination: false,
        }
    );
}

#[test]
fn getpc_ignores_source_and_null_swappc_is_a_jump_only_in_the_observed_case() {
    let span = decoded(&[sop1(0x1f, 8, 255), sop1(0x20, 108, 2), sop1(0x21, 125, 4)]);
    let getpc = span.instruction(0).unwrap();
    assert_eq!(getpc.opcode(), Opcode::SGetpcB64);
    assert_eq!(getpc.sources().len(), 0);
    assert_eq!(getpc.destination().register(), 8);
    for index in [1, 2] {
        let setpc = span.instruction(index).unwrap();
        assert_eq!(setpc.opcode(), Opcode::SSetpcB64);
        assert_eq!(setpc.destination().kind(), OperandKind::Null);
        assert_eq!(setpc.sources().len(), 1);
    }
    assert_eq!(span.instruction(2).unwrap().opcode_id(), 0x21);
    assert_eq!(
        decode(
            &[sop1(0x21, 0, 4)],
            ShaderStage::Compute,
            &Limits::default()
        )
        .unwrap_err(),
        DecodeError::UnsupportedOpcode {
            at_word: 0,
            family: Family::Sop1,
            opcode_id: 0x21
        }
    );
}

#[test]
fn shared_operand_rules_cover_all_ranges_and_specials() {
    for (selector, signed) in [(128, 0), (192, 64), (193, -1), (208, -16)] {
        let operand = scalar_source(selector, 0).unwrap();
        assert_eq!(operand.kind(), OperandKind::IntegerInlineConstant);
        assert_eq!(operand.signed_value(), signed);
        assert_eq!(operand.value(), signed as u32);
    }
    let floats: [f32; 9] = [0.5, -0.5, 1.0, -1.0, 2.0, -2.0, 4.0, -4.0, 0.159_154_94];
    for (index, value) in floats.iter().enumerate() {
        let operand = scalar_source((240 + index) as u16, 0).unwrap();
        assert_eq!(operand.kind(), OperandKind::FloatInlineConstant);
        assert_eq!(operand.value(), value.to_bits());
        assert_eq!(operand.signed_value(), 0);
    }
    for (selector, kind) in [
        (106, OperandKind::VccLo),
        (107, OperandKind::VccHi),
        (124, OperandKind::M0),
        (125, OperandKind::Null),
        (126, OperandKind::ExecLo),
        (127, OperandKind::ExecHi),
        (239, OperandKind::PopsExitingWaveId),
        (251, OperandKind::VccZ),
        (252, OperandKind::ExecZ),
        (253, OperandKind::Scc),
        (255, OperandKind::LiteralConstant),
    ] {
        assert_eq!(scalar_source(selector, 0).unwrap().kind(), kind);
    }
    for selector in [0, 105] {
        assert_eq!(scalar_source(selector, 0).unwrap().register(), selector);
        assert_eq!(
            scalar_destination(selector, 0).unwrap().kind(),
            OperandKind::Sgpr
        );
    }
    for (selector, register) in [(256, 0), (511, 255)] {
        let operand = scalar_source(selector, 0).unwrap();
        assert_eq!(operand.kind(), OperandKind::Vgpr);
        assert_eq!(operand.register(), register);
    }
    for selector in [108, 123, 209, 238, 249, 250, 254, 512, u16::MAX] {
        assert_eq!(
            scalar_source(selector, 7).unwrap_err(),
            DecodeError::UnsupportedOperand {
                at_word: 7,
                selector,
                destination: false,
            }
        );
    }
    for selector in [108, 123, 128, 239, 255, 256] {
        assert_eq!(
            scalar_destination(selector, 9).unwrap_err(),
            DecodeError::UnsupportedOperand {
                at_word: 9,
                selector,
                destination: true,
            }
        );
    }
}

#[test]
fn scalar_destination_specials_and_invalid_holes_are_not_confused_with_sources() {
    for selector in [106, 107, 124, 125, 126, 127] {
        let span = decoded(&[sop1(3, selector, 128)]);
        assert_eq!(
            span.instruction(0).unwrap().destination(),
            scalar_source(selector as u16, 0).unwrap()
        );
    }
    assert_eq!(
        decode(
            &[sop1(3, 108, 128)],
            ShaderStage::Compute,
            &Limits::default()
        )
        .unwrap_err(),
        DecodeError::UnsupportedOperand {
            at_word: 0,
            selector: 108,
            destination: true
        }
    );
}

#[test]
fn sopk_signed_unsigned_embedded_and_aliased_wait_immediates_are_preserved() {
    let span = decoded(&[
        sopk(0, 7, 0xffff),
        sopk(3, 7, 0xffff),
        sopk(9, 7, 0xffff),
        sopk(0xf, 7, 0xffff),
        sopk(0x13, 7, 0xffff),
        sopk(0x17, 7, 0xffff),
        sopk(0x18, 0, 0xffff),
        sopk(0x19, 0, 0xffff),
        sopk(0x1a, 0, 0xffff),
    ]);
    assert_eq!(span.instruction_count(), 9);
    let mov = span.instruction(0).unwrap();
    assert_eq!(mov.sources()[0].signed_value(), -1);
    assert_eq!(mov.destination().register(), 7);
    let signed = span.instruction(1).unwrap();
    let unsigned = span.instruction(2).unwrap();
    assert_eq!(signed.destination().kind(), OperandKind::Scc);
    assert_eq!(signed.sources()[1].signed_value(), -1);
    assert_eq!(unsigned.sources()[1].signed_value(), 65535);
    assert_eq!(unsigned.sources()[0].register(), 7);
    let add = span.instruction(3).unwrap();
    assert_eq!(add.destination().register(), 7);
    assert_eq!(add.sources()[0].register(), 7);
    let setreg = span.instruction(4).unwrap();
    assert_eq!(setreg.raw_words().len(), 1);
    assert_eq!(setreg.sources()[1].kind(), OperandKind::LiteralConstant);
    assert_eq!(setreg.sources()[1].value(), 65535);
    assert_eq!(setreg.sources()[1].signed_value(), -1);
    let vscnt = span.instruction(5).unwrap();
    assert_eq!(vscnt.sources()[0].register(), 7);
    assert_eq!(vscnt.sources()[1].signed_value(), 65535);
    for (index, opcode_id) in [(6, 0x18), (7, 0x19), (8, 0x1a)] {
        let wait = span.instruction(index).unwrap();
        assert_eq!(wait.opcode(), Opcode::SWaitcnt);
        assert_eq!(wait.opcode_id(), opcode_id);
        assert_eq!(wait.sources()[0].signed_value(), 65535);
    }
}

#[test]
fn sopp_immediates_do_not_consume_literal_words_and_trap_is_eight_bits() {
    let span = decoded(&[sopp(0, 0xffff), sopp(0x12, 0xfedd), sopp(1, 0xffff)]);
    assert_eq!(span.instruction_count(), 3);
    let nop = span.instruction(0).unwrap();
    assert_eq!(nop.sources()[0].value(), 65535);
    assert_eq!(nop.sources()[0].signed_value(), -1);
    assert_eq!(nop.raw_words().len(), 1);
    assert!(nop.branch().is_none());
    let trap = span.instruction(1).unwrap();
    assert_eq!(trap.sources()[0].value(), 0xdd);
    assert_eq!(trap.sources()[0].signed_value(), 0xdd);
    assert!(span.instruction(2).unwrap().sources().is_empty());
    // The unused encoded immediate is still retained for a future IR consumer.
    assert_eq!(
        span.instruction(2).unwrap().encoded_sources()[0].signed_value(),
        -1
    );
}

#[test]
fn branch_offsets_are_signed_metadata_without_fictitious_cfg_validation() {
    let span = decoded(&[
        sopp(2, 0xfffe),
        sopp(4, 0x7fff),
        sopk(0x1b, 1, 0xffff),
        sopp(0x17, 0xfffd),
        sopp(0x19, 0),
    ]);
    let expected = [(-2, -4), (32767, 131076), (-1, 8), (-3, 4), (0, 20)];
    for (inst, (displacement, target)) in span.instructions().zip(expected) {
        let branch = inst.branch().unwrap();
        assert_eq!(branch.displacement_words(), displacement);
        assert_eq!(branch.target_byte_offset(), target);
    }
}

#[test]
fn export_words_flags_source_counts_and_encoded_registers_match_the_port() {
    // Case order checks the special single-source export overrides compression.
    for (target, enabled, compressed, done, count) in [
        (0x14, 1, true, true, 1),
        (3, 15, true, false, 2),
        (3, 15, false, false, 4),
        (0x14, 0, true, true, 0),
    ] {
        let header = 0xf800_0000
            | ((target as u32) << 4)
            | enabled as u32
            | (u32::from(compressed) << 10)
            | (u32::from(done) << 11)
            | (1 << 12);
        let span = decoded(&[header, 0xfffe_0100]);
        let inst = span.instruction(0).unwrap();
        assert_eq!(inst.opcode(), Opcode::Exp);
        assert_eq!(inst.opcode_id(), target as u16);
        assert_eq!(inst.sources().len(), count);
        let export = inst.export().unwrap();
        assert_eq!(export.target(), target);
        assert_eq!(export.enabled(), enabled);
        assert_eq!(export.compressed(), compressed);
        assert_eq!(export.done(), done);
        assert!(export.valid_mask());
        for (operand, register) in inst.encoded_sources().iter().zip([0, 1, 254, 255]) {
            assert_eq!(operand.kind(), OperandKind::Vgpr);
            assert_eq!(operand.register(), register);
        }
    }
}

#[test]
fn interpolation_opcode_attribute_channel_and_mov_selector_rules_are_preserved() {
    for (id, expected) in [
        (0, Opcode::VInterpP1F32),
        (1, Opcode::VInterpP2F32),
        (2, Opcode::VInterpMovF32),
    ] {
        let word = 0xc800_0000 | (255 << 18) | (id << 16) | (63 << 10) | (3 << 8) | 254;
        let span = decoded(&[word]);
        let inst = span.instruction(0).unwrap();
        assert_eq!(inst.opcode(), expected);
        assert_eq!(inst.destination().register(), 255);
        assert_eq!(inst.sources()[1].value(), 63);
        assert_eq!(inst.sources()[2].value(), 3);
        if id == 2 {
            assert_eq!(inst.sources()[0].kind(), OperandKind::IntegerInlineConstant);
            assert_eq!(inst.sources()[0].value(), 2);
        } else {
            assert_eq!(inst.sources()[0].kind(), OperandKind::Vgpr);
            assert_eq!(inst.sources()[0].register(), 254);
        }
    }
    assert_eq!(
        decode(&[0xc803_0000], ShaderStage::Pixel, &Limits::default()).unwrap_err(),
        DecodeError::UnsupportedOpcode {
            at_word: 0,
            family: Family::Vintrp,
            opcode_id: 3
        }
    );
}

#[test]
fn owned_source_cannot_be_changed_by_mutating_or_dropping_the_callers_words() {
    let span = {
        let mut input = vec![sop1(3, 1, 255), 0x1020_3040];
        let span = decoded(&input);
        input.fill(0xffff_ffff);
        span
    };
    assert_eq!(span.source_words(), &[sop1(3, 1, 255), 0x1020_3040]);
    assert_eq!(
        span.instruction(0).unwrap().raw_words(),
        span.source_words()
    );
    assert_eq!(
        span.instruction(0).unwrap().sources()[0].value(),
        0x1020_3040
    );
    assert!(span.instruction(1).is_none());
    assert_eq!(span.instructions().len(), 1);
}

#[test]
fn stage_is_preserved_without_invented_operand_legality_rules() {
    for stage in [
        ShaderStage::Unknown,
        ShaderStage::Vertex,
        ShaderStage::Pixel,
        ShaderStage::Fetch,
        ShaderStage::Compute,
        ShaderStage::Mesh,
        ShaderStage::Local,
        ShaderStage::TessellationControl,
        ShaderStage::TessellationEvaluation,
    ] {
        let span = decode(&[sop1(3, 105, 248)], stage, &Limits::default()).unwrap();
        assert_eq!(span.stage(), stage);
        assert_eq!(
            span.instruction(0).unwrap().sources()[0].value(),
            0x3e22_f983
        );
    }
}

#[test]
fn endpgm_is_metadata_and_does_not_hide_unsupported_trailing_input() {
    assert_eq!(
        decode(
            &[sopp(1, 0), 0xffff_ffff],
            ShaderStage::Compute,
            &Limits::default()
        )
        .unwrap_err(),
        DecodeError::UnknownFamily { at_word: 1 }
    );
    let span = decoded(&[sopp(1, 0), sopp(0, 3)]);
    assert_eq!(span.instruction_count(), 2);
}

#[test]
fn empty_span_needs_no_allocated_storage_or_work() {
    let limits = Limits {
        max_source_words: 0,
        max_instructions: 0,
        max_record_bytes: 0,
        max_work: 0,
    };
    let span = decode(&[], ShaderStage::Unknown, &limits).unwrap();
    assert_eq!(span.instruction_count(), 0);
    assert_eq!(span.work_units(), 0);
    assert_eq!(span.source_capacity_words(), 0);
    assert_eq!(span.record_capacity_bytes(), 0);
}

#[test]
fn source_instruction_record_and_work_limits_are_independent() {
    let words = [sopp(0, 0), sopp(0, 0)];
    let cases = [
        (
            Limits {
                max_source_words: 1,
                ..Limits::default()
            },
            0,
            LimitKind::SourceWords,
        ),
        (
            Limits {
                max_instructions: 1,
                ..Limits::default()
            },
            1,
            LimitKind::Instructions,
        ),
        (
            Limits {
                max_record_bytes: size_of::<Record>(),
                ..Limits::default()
            },
            1,
            LimitKind::RecordBytes,
        ),
        (
            Limits {
                max_work: 1,
                ..Limits::default()
            },
            0,
            LimitKind::Work,
        ),
        (
            Limits {
                max_work: 2,
                ..Limits::default()
            },
            0,
            LimitKind::Work,
        ),
    ];
    for (limits, at_word, kind) in cases {
        assert_eq!(
            decode(&words, ShaderStage::Compute, &limits).unwrap_err(),
            DecodeError::LimitExceeded { at_word, kind }
        );
    }
    let limits = Limits {
        max_source_words: 2,
        max_instructions: 2,
        max_record_bytes: 2 * size_of::<Record>(),
        max_work: 4,
    };
    let span = decode(&words, ShaderStage::Compute, &limits).unwrap();
    assert_eq!(span.instruction_count(), 2);
    assert!(span.record_capacity_bytes() <= limits.max_record_bytes);
    assert!(span.source_capacity_words() <= limits.max_source_words);
}

#[test]
fn work_includes_literal_data_export_data_and_possible_record_growth_moves() {
    let words = [sop1(3, 0, 255), 0xffff_ffff, 0xf800_0000, 0xffff_ffff];
    let span = decoded(&words);
    assert_eq!(span.work_units(), 8);
    let too_small = Limits {
        max_work: 7,
        ..Limits::default()
    };
    assert_eq!(
        decode(&words, ShaderStage::Compute, &too_small).unwrap_err(),
        DecodeError::LimitExceeded {
            at_word: 2,
            kind: LimitKind::Work
        }
    );
    let words = [sopp(0, 0); 9];
    let span = decoded(&words);
    assert_eq!(span.work_units(), 30); // 9 copy + 9 decode + 4 + 8 potential moves.
    let too_small = Limits {
        max_work: 29,
        ..Limits::default()
    };
    assert_eq!(
        decode(&words, ShaderStage::Compute, &too_small).unwrap_err(),
        DecodeError::LimitExceeded {
            at_word: 8,
            kind: LimitKind::Work
        }
    );
}

#[test]
fn capacity_growth_is_clamped_to_non_power_of_two_byte_budgets() {
    let words = [sopp(0, 0); 7];
    let limits = Limits {
        max_record_bytes: 7 * size_of::<Record>() + 1,
        ..Limits::default()
    };
    let span = decode(&words, ShaderStage::Compute, &limits).unwrap();
    assert_eq!(span.instruction_count(), 7);
    assert!(span.record_capacity_bytes() <= limits.max_record_bytes);
    assert!(span.record_capacity_bytes() >= 7 * size_of::<Record>());
}

#[test]
fn all_byte_source_selectors_and_original_hostile_words_fail_without_panicking() {
    for selector in 0..=255 {
        let header = sop1(3, 0, selector);
        let words = [header, 0];
        let input = if selector == 255 {
            &words[..]
        } else {
            &words[..1]
        };
        let result = decode(input, ShaderStage::Compute, &Limits::default());
        match scalar_source(selector as u16, 0) {
            Ok(_) => assert!(result.is_ok()),
            Err(expected) => assert_eq!(result.unwrap_err(), expected),
        }
    }
    let limits = Limits {
        max_source_words: 8,
        max_instructions: 8,
        max_record_bytes: 8 * size_of::<Record>(),
        max_work: 64,
    };
    let mut seed = 0x39a7_26d5_u32;
    for length in 0..=8 {
        for _ in 0..128 {
            let mut words = [0_u32; 8];
            for word in &mut words[..length] {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *word = seed;
            }
            if let Ok(span) = decode(&words[..length], ShaderStage::Unknown, &limits) {
                assert_eq!(span.source_words(), &words[..length]);
                assert_eq!(
                    span.instructions()
                        .map(|inst| inst.raw_words().len())
                        .sum::<usize>(),
                    length
                );
                assert!(span.work_units() <= limits.max_work);
                assert!(span.record_capacity_bytes() <= limits.max_record_bytes);
            }
        }
    }
}
