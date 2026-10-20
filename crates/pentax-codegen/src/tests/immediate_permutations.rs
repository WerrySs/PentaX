// SPDX-License-Identifier: GPL-2.0-only
//! Original permutation plans preserve operands without native AVX admission.

use super::*;

const FORMS: [(u8, u8, Code, bool); 4] = [
    (0x65, 0x06, Code::VEX_Vperm2f128_ymm_ymm_ymmm256_imm8, true),
    (0x65, 0x46, Code::VEX_Vperm2i128_ymm_ymm_ymmm256_imm8, true),
    (0xfd, 0x01, Code::VEX_Vpermpd_ymm_ymmm256_imm8, false),
    (0xfd, 0x00, Code::VEX_Vpermq_ymm_ymmm256_imm8, false),
];

#[test]
fn four_vex_immediate_permutations_retain_sources_widths_and_rip_under_mediation() {
    for (prefix, opcode, expected, two_sources) in FORMS {
        for memory in [false, true] {
            let mut code = vec![0xc4, 0xe3, prefix, opcode];
            if memory {
                code.extend([0x15, 0, 1, 0, 0]); // destination2, [original RIP+256]
            } else {
                code.push(0xd4); // destination2, register source4
            }
            code.push(0xe7);
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
                assert_eq!(instruction.code(), expected);
                assert_eq!(instruction.op0_kind(), OpKind::Register);
                assert_eq!(instruction.op0_register(), Register::YMM2);
                let source_operand = if two_sources {
                    assert_eq!(instruction.op_count(), 4);
                    assert_eq!(instruction.op1_kind(), OpKind::Register);
                    assert_eq!(instruction.op1_register(), Register::YMM3);
                    2
                } else {
                    assert_eq!(instruction.op_count(), 3);
                    1
                };
                assert_eq!(instruction.op_kind(source_operand + 1), OpKind::Immediate8);
                assert_eq!(instruction.immediate8(), 0xe7);
                if memory {
                    let target = BIAS + ENTRY as u64 + length as u64 + 256;
                    assert_eq!(instruction.op_kind(source_operand), OpKind::Memory);
                    assert_eq!(instruction.memory_size().size(), 32);
                    assert_eq!(instruction.memory_base(), Register::RIP);
                    assert_eq!(instruction.ip_rel_memory_address(), target);
                    assert_eq!(record.rip_relative_target(), Some(target));
                } else {
                    assert_eq!(instruction.op_kind(source_operand), OpKind::Register);
                    assert_eq!(instruction.op_register(source_operand), Register::YMM4);
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
fn reserved_permutation_encodings_are_invalid_and_neighboring_evex_stays_mediated() {
    // Pinned iced1.21 tables require L1 and the family-specific W value.
    // Only immediate PD/Q has reserved vvvv; PERM2 reads a real YMM operand.
    for (opcode, prefix) in [
        (0x06, 0x61),
        (0x06, 0xe5), // PERM2F: L0, W1
        (0x46, 0x61),
        (0x46, 0xe5), // PERM2I: L0, W1
        (0x01, 0xf9),
        (0x01, 0x7d),
        (0x01, 0xf5), // PD: L0, W0, vvvv
        (0x00, 0xf9),
        (0x00, 0x7d),
        (0x00, 0xf5), // Q: L0, W0, vvvv
    ] {
        let code = [0xc4, 0xe3, prefix, opcode, 0xd4, 0xe7, 0xc3];
        let mut decoder = Decoder::with_ip(64, &code, BIAS + ENTRY as u64, DecoderOptions::NONE);
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
    for (opcode, expected) in [
        (0x01, Code::EVEX_Vpermpd_ymm_k1z_ymmm256b64_imm8),
        (0x00, Code::EVEX_Vpermq_ymm_k1z_ymmm256b64_imm8),
    ] {
        let code = [0x62, 0xf3, 0xfd, 0x28, opcode, 0xd4, 0xe7, 0xc3];
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
        assert_eq!(instruction.op0_register(), Register::YMM2);
        assert_eq!(instruction.op1_register(), Register::YMM4);
        assert_eq!(instruction.immediate8(), 0xe7);
    }
}
