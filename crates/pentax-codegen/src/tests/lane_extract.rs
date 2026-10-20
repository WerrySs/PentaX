// SPDX-License-Identifier: GPL-2.0-only
//! Original lane-extraction plans preserve stores without native AVX admission.

use super::*;

const FORMS: [(u8, Code); 2] = [
    (0x19, Code::VEX_Vextractf128_xmmm128_ymm_imm8),
    (0x39, Code::VEX_Vextracti128_xmmm128_ymm_imm8),
];

#[test]
fn vex_lane_extracts_retain_register_and_rip_destinations_and_full_immediates() {
    for (opcode, expected) in FORMS {
        for memory in [false, true] {
            let mut code = vec![0xc4, 0xe3, 0x7d, opcode];
            if memory {
                code.extend([0x15, 0, 1, 0, 0]); // [original RIP+256] <- YMM2
            } else {
                code.push(0xd4); // XMM4 <- YMM2
            }
            code.push(0xe7); // Retain all immediate bits for the runtime helper.
            let length = code.len();
            code.push(0xc3);
            let bytes = image(&code);
            let source = link(&[(&bytes, BIAS)]);
            let declaration =
                declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
            for features in [
                CpuFeatures::x64_sse2(),
                CpuFeatures::x64_sse2()
                    .with(CpuidFeature::AVX)
                    .with(CpuidFeature::AVX2),
            ] {
                let plan = plan_materialized_block(
                    &declaration,
                    0,
                    BIAS + ENTRY as u64,
                    &code,
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
                assert_eq!(plan.instructions().len(), 1);
                assert_eq!(
                    (plan.exit().guest_pc(), plan.exit().next_guest_pc()),
                    (BIAS + ENTRY as u64, BIAS + ENTRY as u64 + length as u64)
                );
                let record = &plan.instructions()[0];
                assert_eq!(
                    (
                        record.source_offset(),
                        record.source_len(),
                        record.emitted_len()
                    ),
                    (0, length, 0)
                );
                let instruction = plan.exit().instruction().unwrap();
                assert_eq!(instruction, record.instruction());
                assert_eq!((instruction.code(), instruction.op_count()), (expected, 3));
                assert_eq!(
                    (instruction.op1_kind(), instruction.op1_register()),
                    (OpKind::Register, Register::YMM2)
                );
                assert_eq!(
                    (instruction.op2_kind(), instruction.immediate8()),
                    (OpKind::Immediate8, 0xe7)
                );
                if memory {
                    let target = BIAS + ENTRY as u64 + length as u64 + 256;
                    assert_eq!(instruction.op0_kind(), OpKind::Memory);
                    assert_eq!(instruction.memory_size().size(), 16);
                    assert_eq!(instruction.memory_base(), Register::RIP);
                    assert_eq!(instruction.ip_rel_memory_address(), target);
                    assert_eq!(record.rip_relative_target(), Some(target));
                } else {
                    assert_eq!(instruction.op0_kind(), OpKind::Register);
                    assert_eq!(instruction.op0_register(), Register::XMM4);
                    assert_eq!(record.rip_relative_target(), None);
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
fn reserved_lane_extract_encodings_are_invalid_and_neighboring_evex_stays_mediated() {
    // Pinned iced1.21 tables require W0/L1; WVIb checks reserved vvvv.
    for (opcode, _) in FORMS {
        for prefix in [0x75, 0x79, 0xfd] {
            // nonreserved vvvv, L0, or W1
            let code = [0xc4, 0xe3, prefix, opcode, 0xd4, 0xe7, 0xc3];
            let mut decoder =
                Decoder::with_ip(64, &code, BIAS + ENTRY as u64, DecoderOptions::NONE);
            assert!(decoder.decode().is_invalid());
            let bytes = image(&code);
            let source = link(&[(&bytes, BIAS)]);
            let declaration =
                declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
            assert!(matches!(
                plan_materialized_block(
                    &declaration,
                    0,
                    BIAS + ENTRY as u64,
                    &code,
                    placement(),
                    &TranslationPolicy::new(
                        CpuFeatures::x64_sse2().with(CpuidFeature::AVX).with(CpuidFeature::AVX2)
                    ),
                    &BlockLimits::default(),
                ),
                Err(CodegenError::InvalidInstruction { guest_pc }) if guest_pc == BIAS + ENTRY as u64
            ));
        }
    }
    for (opcode, expected) in [
        (0x19, Code::EVEX_Vextractf32x4_xmmm128_k1z_ymm_imm8),
        (0x39, Code::EVEX_Vextracti32x4_xmmm128_k1z_ymm_imm8),
    ] {
        let code = [0x62, 0xf3, 0x7d, 0x28, opcode, 0xd4, 0xe7, 0xc3];
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let declaration =
            declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
        let plan = plan_materialized_block(
            &declaration,
            0,
            BIAS + ENTRY as u64,
            &code,
            placement(),
            &TranslationPolicy::new(
                CpuFeatures::x64_sse2()
                    .with(CpuidFeature::AVX512F)
                    .with(CpuidFeature::AVX512VL),
            ),
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
        assert_eq!(plan.instructions().len(), 1);
        assert_eq!(plan.instructions()[0].emitted_len(), 0);
        let instruction = plan.exit().instruction().unwrap();
        assert_eq!(instruction.code(), expected);
        assert_eq!(instruction.op0_register(), Register::XMM4);
        assert_eq!(instruction.op1_register(), Register::YMM2);
        assert_eq!(instruction.immediate8(), 0xe7);
    }
}
