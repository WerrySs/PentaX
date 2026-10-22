// SPDX-License-Identifier: GPL-2.0-only
//! Exact conversion decodes remain checked software exits under all native policies.
use super::*;
#[test]
fn x87_integer_eleven_documented_forms_preserve_widths_and_source_without_native_feature_authority()
{
    let forms = [
        (0xdf, 0x00, Code::Fild_m16int, 2),
        (0xdb, 0x00, Code::Fild_m32int, 4),
        (0xdf, 0x28, Code::Fild_m64int, 8),
        (0xdf, 0x10, Code::Fist_m16int, 2),
        (0xdb, 0x10, Code::Fist_m32int, 4),
        (0xdf, 0x18, Code::Fistp_m16int, 2),
        (0xdb, 0x18, Code::Fistp_m32int, 4),
        (0xdf, 0x38, Code::Fistp_m64int, 8),
        (0xdf, 0x08, Code::Fisttp_m16int, 2),
        (0xdb, 0x08, Code::Fisttp_m32int, 4),
        (0xdd, 0x08, Code::Fisttp_m64int, 8),
    ];
    for (first, second, expected, width) in forms {
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
                    .with(CpuidFeature::SSE3),
            ] {
                let plan = plan_block(
                    &source,
                    0,
                    BIAS + ENTRY as u64,
                    placement(),
                    &TranslationPolicy::new(features).with_independent_memory(),
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
fn x87_integer_feature_bits_never_promote_neighboring_bcd_forms_or_invalid_prefixes() {
    for code in [vec![0xdf, 0x20, 0xc3], vec![0xdf, 0x30, 0xc3]] {
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
        let code = [prefix, 0xdd, 0x08, 0xc3];
        let mut decoder = Decoder::with_ip(64, &code, BIAS + ENTRY as u64, DecoderOptions::NONE);
        let invalid = decoder.decode().is_invalid();
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let result = plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::SSE3)),
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
