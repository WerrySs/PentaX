// SPDX-License-Identifier: GPL-2.0-only
//! Original narrowing-store encodings retain checked software mediation.
use super::*;
#[test]
fn x87_float_store_four_exact_widths_remain_mediated_under_all_native_feature_declarations() {
    for (first, second, expected, width) in [
        (0xd9, 0x10, Code::Fst_m32fp, 4),
        (0xdd, 0x10, Code::Fst_m64fp, 8),
        (0xd9, 0x18, Code::Fstp_m32fp, 4),
        (0xdd, 0x18, Code::Fstp_m64fp, 8),
    ] {
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
                    .with(CpuidFeature::SSE3)
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
                assert_eq!(instruction.op_count(), 1);
                assert_eq!(instruction.op0_kind(), OpKind::Memory);
                assert_eq!(instruction.memory_size().size(), width);
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
#[test]
fn x87_float_store_never_promotes_bcd_environment_or_bad_prefix_neighbors() {
    for code in [vec![0xdf, 0x30, 0xc3], vec![0xd9, 0x30, 0xc3]] {
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = make(&source);
        assert_eq!(plan.instructions()[0].emitted_len(), 0);
        assert!(matches!(plan.exit().kind(), ExitKind::Mediated { .. }));
    }
    for prefix in [0xf0, 0xf2, 0xf3, 0x64] {
        let code = [prefix, 0xd9, 0x18, 0xc3];
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
