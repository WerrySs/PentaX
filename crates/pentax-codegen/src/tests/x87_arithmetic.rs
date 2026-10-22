// SPDX-License-Identifier: GPL-2.0-only
//! Original arithmetic encodings remain source-bound checked mediation.
use super::*;
#[test]
fn x87_arithmetic_thirty_exact_forms_keep_source_operands_and_never_gain_native_authority() {
    let forms = [
        (0xd8, 0xc7, Code::Fadd_st0_sti),
        (0xdc, 0xc7, Code::Fadd_sti_st0),
        (0xde, 0xc7, Code::Faddp_sti_st0),
        (0xd8, 0x00, Code::Fadd_m32fp),
        (0xdc, 0x00, Code::Fadd_m64fp),
        (0xd8, 0xe7, Code::Fsub_st0_sti),
        (0xdc, 0xef, Code::Fsub_sti_st0),
        (0xde, 0xef, Code::Fsubp_sti_st0),
        (0xd8, 0x20, Code::Fsub_m32fp),
        (0xdc, 0x20, Code::Fsub_m64fp),
        (0xd8, 0xef, Code::Fsubr_st0_sti),
        (0xdc, 0xe7, Code::Fsubr_sti_st0),
        (0xde, 0xe7, Code::Fsubrp_sti_st0),
        (0xd8, 0x28, Code::Fsubr_m32fp),
        (0xdc, 0x28, Code::Fsubr_m64fp),
        (0xd8, 0xcf, Code::Fmul_st0_sti),
        (0xdc, 0xcf, Code::Fmul_sti_st0),
        (0xde, 0xcf, Code::Fmulp_sti_st0),
        (0xd8, 0x08, Code::Fmul_m32fp),
        (0xdc, 0x08, Code::Fmul_m64fp),
        (0xd8, 0xf7, Code::Fdiv_st0_sti),
        (0xdc, 0xff, Code::Fdiv_sti_st0),
        (0xde, 0xff, Code::Fdivp_sti_st0),
        (0xd8, 0x30, Code::Fdiv_m32fp),
        (0xdc, 0x30, Code::Fdiv_m64fp),
        (0xd8, 0xff, Code::Fdivr_st0_sti),
        (0xdc, 0xf7, Code::Fdivr_sti_st0),
        (0xde, 0xf7, Code::Fdivrp_sti_st0),
        (0xd8, 0x38, Code::Fdivr_m32fp),
        (0xdc, 0x38, Code::Fdivr_m64fp),
    ];
    for (first, second, expected) in forms {
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
                    .with(CpuidFeature::AVX)
                    .with(CpuidFeature::AVX2),
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
                assert_eq!(record.source_len(), if prefixed { 3 } else { 2 });
                assert_eq!(record.emitted_len(), 0);
                if second < 0xc0 {
                    assert_eq!(instruction.op_count(), 1);
                    assert_eq!(instruction.op0_kind(), OpKind::Memory);
                    assert_eq!(
                        instruction.memory_size().size(),
                        if first == 0xd8 { 4 } else { 8 }
                    );
                } else {
                    assert_eq!(instruction.op_count(), 2);
                    assert!(
                        instruction.op_register(0) == Register::ST7
                            || instruction.op_register(1) == Register::ST7
                    );
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
fn x87_arithmetic_keeps_integer_sqrt_narrow_stores_and_invalid_prefixes_outside_native_emission() {
    for code in [
        vec![0xda, 0x00, 0xc3],
        vec![0xd9, 0xfa, 0xc3],
        vec![0xd9, 0x10, 0xc3],
        vec![0xdd, 0x18, 0xc3],
        vec![0xd8, 0xd1, 0xc3],
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
        let code = [prefix, 0xde, 0xe1, 0xc3];
        let mut decoder = Decoder::with_ip(64, &code, BIAS + ENTRY as u64, DecoderOptions::NONE);
        let invalid = decoder.decode().is_invalid();
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let outcome = plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default(),
        );
        if invalid {
            assert!(matches!(
                outcome,
                Err(CodegenError::InvalidInstruction { .. })
            ));
        } else {
            assert_eq!(
                outcome.unwrap().exit().kind(),
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
