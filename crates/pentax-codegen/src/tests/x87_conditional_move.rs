// SPDX-License-Identifier: GPL-2.0-only
//! Original FCMOV encodings remain software exits regardless of native feature bits.
use super::*;
#[test]
fn x87_conditional_move_eight_exact_register_forms_keep_operands_source_and_never_emit_native_code()
{
    for (first, second, expected) in [
        (0xda, 0xc0, Code::Fcmovb_st0_sti),
        (0xda, 0xc8, Code::Fcmove_st0_sti),
        (0xda, 0xd0, Code::Fcmovbe_st0_sti),
        (0xda, 0xd8, Code::Fcmovu_st0_sti),
        (0xdb, 0xc0, Code::Fcmovnb_st0_sti),
        (0xdb, 0xc8, Code::Fcmovne_st0_sti),
        (0xdb, 0xd0, Code::Fcmovnbe_st0_sti),
        (0xdb, 0xd8, Code::Fcmovnu_st0_sti),
    ] {
        for i in [0, 3, 7] {
            for prefixed in [false, true] {
                let mut code = vec![];
                if prefixed {
                    code.push(0x66);
                }
                code.extend_from_slice(&[first, second + i, 0xc3]);
                let bytes = image(&code);
                let source = link(&[(&bytes, BIAS)]);
                for features in [
                    CpuFeatures::empty(),
                    CpuFeatures::x64_sse2()
                        .with(CpuidFeature::FPU)
                        .with(CpuidFeature::CMOV),
                ] {
                    let plan = plan_block(
                        &source,
                        0,
                        BIAS + ENTRY as u64,
                        placement(),
                        &TranslationPolicy::new(features),
                        &BlockLimits::default(),
                    )
                    .unwrap();
                    assert_eq!(
                        plan.exit().kind(),
                        ExitKind::Mediated {
                            reason: UnsupportedReason::InstructionClass
                        }
                    );
                    assert_eq!(plan.bytes(), [0x0f, 0x0b]);
                    let record = &plan.instructions()[0];
                    let instruction = record.instruction();
                    assert_eq!(instruction.code(), expected);
                    assert_eq!(instruction.op_count(), 2);
                    assert_eq!(instruction.op0_register(), Register::ST0);
                    assert_eq!(instruction.op1_register().number(), i as usize);
                    assert_eq!(record.source_len(), if prefixed { 3 } else { 2 });
                    assert_eq!(record.emitted_len(), 0);
                    assert_eq!(
                        MemoryAccessAnalyzer::default().analyze(record, &[0; 16]),
                        Err(MemoryAccessError::NotEmitted)
                    );
                }
            }
        }
    }
}
#[test]
fn x87_conditional_move_does_not_promote_adjacent_integer_forms_or_explicit_bad_prefixes() {
    for first in [0xda, 0xdb] {
        let bytes = image(&[first, 0x00, 0xc3]);
        let source = link(&[(&bytes, BIAS)]);
        let plan = make(&source);
        assert_eq!(
            plan.exit().kind(),
            ExitKind::Mediated {
                reason: UnsupportedReason::InstructionClass
            }
        );
        assert_eq!(plan.instructions()[0].emitted_len(), 0);
    }
    for prefix in [0xf0, 0xf2, 0xf3, 0x64] {
        let code = [prefix, 0xdb, 0xc9, 0xc3];
        let mut decoder = Decoder::with_ip(64, &code, BIAS + ENTRY as u64, DecoderOptions::NONE);
        let invalid = decoder.decode().is_invalid();
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let result = plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default(),
        );
        if invalid {
            assert!(matches!(
                result,
                Err(CodegenError::InvalidInstruction { .. })
            ));
        } else {
            assert_eq!(
                result.unwrap().exit().kind(),
                ExitKind::Mediated {
                    reason: if prefix == 0x64 {
                        UnsupportedReason::SegmentPrefix
                    } else {
                        UnsupportedReason::RepeatOrLockPrefix
                    }
                }
            );
        }
    }
}
