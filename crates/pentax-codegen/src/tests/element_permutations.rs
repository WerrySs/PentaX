// SPDX-License-Identifier: GPL-2.0-only
//! Original element-permutation plans retain data and control operands.

use super::*;

// Opcode, prefix, immediate control, vector width, exact decoder form.
const FORMS: [(u8, u8, bool, bool, Code); 8] = [
    (
        0x04,
        0x79,
        true,
        false,
        Code::VEX_Vpermilps_xmm_xmmm128_imm8,
    ),
    (0x04, 0x7d, true, true, Code::VEX_Vpermilps_ymm_ymmm256_imm8),
    (
        0x05,
        0x79,
        true,
        false,
        Code::VEX_Vpermilpd_xmm_xmmm128_imm8,
    ),
    (0x05, 0x7d, true, true, Code::VEX_Vpermilpd_ymm_ymmm256_imm8),
    (
        0x0c,
        0x61,
        false,
        false,
        Code::VEX_Vpermilps_xmm_xmm_xmmm128,
    ),
    (0x0c, 0x65, false, true, Code::VEX_Vpermilps_ymm_ymm_ymmm256),
    (
        0x0d,
        0x61,
        false,
        false,
        Code::VEX_Vpermilpd_xmm_xmm_xmmm128,
    ),
    (0x0d, 0x65, false, true, Code::VEX_Vpermilpd_ymm_ymm_ymmm256),
];

#[test]
fn eight_vex_element_permutations_retain_data_control_and_rip_sources_under_mediation() {
    for (opcode, prefix, immediate, wide, expected) in FORMS {
        for memory in [false, true] {
            let mut code = vec![0xc4, if immediate { 0xe3 } else { 0xe2 }, prefix, opcode];
            if memory {
                code.extend([0x15, 0, 1, 0, 0]); // data/control at original RIP+256
            } else {
                code.push(0xd4); // destination2, r/m source4
            }
            if immediate {
                code.push(0xe7);
            }
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
                assert_eq!(instruction.op0_kind(), OpKind::Register);
                assert_eq!(
                    instruction.op0_register(),
                    if wide { Register::YMM2 } else { Register::XMM2 }
                );
                let source_operand = if immediate {
                    assert_eq!(instruction.op2_kind(), OpKind::Immediate8);
                    assert_eq!(instruction.immediate8(), 0xe7);
                    1 // Immediate forms read data from the r/m operand.
                } else {
                    assert_eq!(instruction.op1_kind(), OpKind::Register);
                    assert_eq!(
                        instruction.op1_register(),
                        if wide { Register::YMM3 } else { Register::XMM3 }
                    );
                    2 // Variable forms read controls from r/m; data is vvvv.
                };
                if memory {
                    let target = BIAS + ENTRY as u64 + length as u64 + 256;
                    assert_eq!(instruction.op_kind(source_operand), OpKind::Memory);
                    assert_eq!(instruction.memory_size().size(), if wide { 32 } else { 16 });
                    assert_eq!(instruction.memory_base(), Register::RIP);
                    assert_eq!(instruction.ip_rel_memory_address(), target);
                    assert_eq!(record.rip_relative_target(), Some(target));
                } else {
                    assert_eq!(instruction.op_kind(source_operand), OpKind::Register);
                    assert_eq!(
                        instruction.op_register(source_operand),
                        if wide { Register::YMM4 } else { Register::XMM4 }
                    );
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
fn reserved_element_permutation_fields_are_invalid_and_neighboring_evex_stays_mediated() {
    // Pinned iced1.21 tables reject W1 for every VEX form, with both L values valid.
    // Only immediate VWIb has reserved vvvv; variable VHW reads an actual register.
    for (opcode, prefix, immediate, _, _) in FORMS {
        for invalid_prefix in [Some(prefix | 0x80), immediate.then_some(prefix ^ 8)]
            .into_iter()
            .flatten()
        {
            let mut code = vec![
                0xc4,
                if immediate { 0xe3 } else { 0xe2 },
                invalid_prefix,
                opcode,
                0xd4,
            ];
            if immediate {
                code.push(0xe7);
            }
            code.push(0xc3);
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
    for (opcode, prefix, expected) in [
        (0x04, 0x7d, Code::EVEX_Vpermilps_ymm_k1z_ymmm256b32_imm8),
        (0x05, 0xfd, Code::EVEX_Vpermilpd_ymm_k1z_ymmm256b64_imm8),
    ] {
        let code = [0x62, 0xf3, prefix, 0x28, opcode, 0xd4, 0xe7, 0xc3];
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
