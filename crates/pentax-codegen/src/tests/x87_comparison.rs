// SPDX-License-Identifier: GPL-2.0-only
//! Original comparison encodings retain operands without native promotion.
use super::*;
#[test]
fn x87_comparison_fifteen_documented_forms_retain_exact_operands_source_and_checked_mediation() {
    let forms = [
        (0xd8, 0x10, Code::Fcom_m32fp, 1, 4),
        (0xdc, 0x10, Code::Fcom_m64fp, 1, 8),
        (0xd8, 0xd7, Code::Fcom_st0_sti, 2, 0),
        (0xd8, 0x18, Code::Fcomp_m32fp, 1, 4),
        (0xdc, 0x18, Code::Fcomp_m64fp, 1, 8),
        (0xd8, 0xdf, Code::Fcomp_st0_sti, 2, 0),
        (0xde, 0xd9, Code::Fcompp, 0, 0),
        (0xdd, 0xe7, Code::Fucom_st0_sti, 2, 0),
        (0xdd, 0xef, Code::Fucomp_st0_sti, 2, 0),
        (0xda, 0xe9, Code::Fucompp, 0, 0),
        (0xdb, 0xf7, Code::Fcomi_st0_sti, 2, 0),
        (0xdf, 0xf7, Code::Fcomip_st0_sti, 2, 0),
        (0xdb, 0xef, Code::Fucomi_st0_sti, 2, 0),
        (0xdf, 0xef, Code::Fucomip_st0_sti, 2, 0),
        (0xd9, 0xe4, Code::Ftst, 0, 0),
    ];
    for (first, second, expected, operands, width) in forms {
        for prefixed in [false, true] {
            let mut code = vec![];
            if prefixed {
                code.push(0x66);
            }
            code.extend_from_slice(&[first, second, 0xc3]);
            let bytes = image(&code);
            let source = link(&[(&bytes, BIAS)]);
            for features in [
                CpuFeatures::x64_sse2(),
                CpuFeatures::x64_sse2()
                    .with(CpuidFeature::FPU)
                    .with(CpuidFeature::CMOV)
                    .with(CpuidFeature::AVX),
            ] {
                let plan = plan_block(
                    &source,
                    0,
                    BIAS + ENTRY as u64,
                    placement(),
                    &TranslationPolicy::new(features)
                        .with_native_ymm_registers()
                        .with_independent_memory(),
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
                assert_eq!(instruction.op_count(), operands);
                assert_eq!(record.source_len(), if prefixed { 3 } else { 2 });
                assert_eq!(record.emitted_len(), 0);
                if width != 0 {
                    assert_eq!(instruction.op0_kind(), OpKind::Memory);
                    assert_eq!(instruction.memory_size().size(), width);
                } else if operands != 0 {
                    assert_eq!(instruction.op0_register(), Register::ST0);
                    assert_eq!(instruction.op1_register(), Register::ST7);
                }
                assert_eq!(
                    MemoryAccessAnalyzer::default().analyze(record, &[0; 16]),
                    Err(MemoryAccessError::NotEmitted)
                );
            }
        }
    }
}
#[test]
fn x87_comparison_does_not_promote_undocumented_integer_or_examine_aliases_and_rejects_bad_prefixes()
 {
    for code in [
        vec![0xdc, 0xd1, 0xc3],
        vec![0xdc, 0xd9, 0xc3],
        vec![0xde, 0xd1, 0xc3],
        vec![0xda, 0x10, 0xc3],
        vec![0xd9, 0xe5, 0xc3],
    ] {
        let bytes = image(&code);
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
        let code = [prefix, 0xdb, 0xe9, 0xc3];
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
