// SPDX-License-Identifier: GPL-2.0-only
//! Closed ROUND plans retain every source byte without native FP authority.
use super::*;

#[test]
fn ten_round_forms_retain_scalar_packed_immediates_merge_and_rip_sources_as_mediated_exits() {
    for (opcode, vex, wide, scalar, expected, width) in [
        (8, false, false, false, Code::Roundps_xmm_xmmm128_imm8, 16),
        (9, false, false, false, Code::Roundpd_xmm_xmmm128_imm8, 16),
        (10, false, false, true, Code::Roundss_xmm_xmmm32_imm8, 4),
        (11, false, false, true, Code::Roundsd_xmm_xmmm64_imm8, 8),
        (
            8,
            true,
            false,
            false,
            Code::VEX_Vroundps_xmm_xmmm128_imm8,
            16,
        ),
        (
            8,
            true,
            true,
            false,
            Code::VEX_Vroundps_ymm_ymmm256_imm8,
            32,
        ),
        (
            9,
            true,
            false,
            false,
            Code::VEX_Vroundpd_xmm_xmmm128_imm8,
            16,
        ),
        (
            9,
            true,
            true,
            false,
            Code::VEX_Vroundpd_ymm_ymmm256_imm8,
            32,
        ),
        (
            10,
            true,
            false,
            true,
            Code::VEX_Vroundss_xmm_xmm_xmmm32_imm8,
            4,
        ),
        (
            11,
            true,
            false,
            true,
            Code::VEX_Vroundsd_xmm_xmm_xmmm64_imm8,
            8,
        ),
    ] {
        for addressing in 0..3 {
            let mut code = if vex {
                vec![
                    0xc4,
                    0xe3,
                    if scalar {
                        0x61
                    } else if wide {
                        0x7d
                    } else {
                        0x79
                    },
                    opcode,
                ]
            } else {
                vec![0x66, 0x0f, 0x3a, opcode]
            };
            match addressing {
                0 => code.push(0xca),
                1 => code.extend([0x4c, 0x8b, 0x20]), // [RBX+RCX*4+32].
                _ => code.extend([0x0d, 0, 1, 0, 0]),
            }
            code.push(0x84);
            let length = code.len();
            code.push(0xc3);
            let bytes = image(&code);
            let source = link(&[(&bytes, BIAS)]);
            let declaration =
                declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
            for features in [
                CpuFeatures::x64_sse2(),
                CpuFeatures::x64_sse2()
                    .with(CpuidFeature::SSE4_1)
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
                let record = &plan.instructions()[0];
                let instruction = record.instruction();
                assert_eq!(instruction.code(), expected);
                let operand = if scalar && vex { 2 } else { 1 };
                assert_eq!(instruction.op_count(), operand + 2);
                assert_eq!(instruction.immediate8(), 0x84);
                assert_eq!(instruction.op_kind(operand + 1), OpKind::Immediate8);
                assert_eq!(
                    instruction.op0_register(),
                    if wide { Register::YMM1 } else { Register::XMM1 }
                );
                if scalar && vex {
                    assert_eq!(instruction.op1_register(), Register::XMM3);
                }
                if addressing == 0 {
                    assert_eq!(
                        instruction.op_register(operand),
                        if wide { Register::YMM2 } else { Register::XMM2 }
                    );
                } else {
                    assert_eq!(instruction.op_kind(operand), OpKind::Memory);
                    assert_eq!(instruction.memory_size().size(), width);
                    if addressing == 1 {
                        assert_eq!(
                            (
                                instruction.memory_base(),
                                instruction.memory_index(),
                                instruction.memory_index_scale(),
                                instruction.memory_displacement64()
                            ),
                            (Register::RBX, Register::RCX, 4, 32)
                        );
                    } else {
                        assert_eq!(
                            record.rip_relative_target(),
                            Some(BIAS + ENTRY as u64 + length as u64 + 256)
                        );
                    }
                }
                assert_eq!(
                    (
                        record.source_offset(),
                        record.source_len(),
                        record.emitted_len()
                    ),
                    (0, length, 0)
                );
                assert_eq!(
                    plan.exit().next_guest_pc(),
                    BIAS + ENTRY as u64 + length as u64
                );
                assert_eq!(
                    MemoryAccessAnalyzer::default().analyze(record, &[0; 16]),
                    Err(MemoryAccessError::NotEmitted)
                );
            }
        }
    }
}

#[test]
fn locked_round_and_nonreserved_packed_vvvv_are_rejected_without_new_native_policy() {
    for code in [
        vec![0xf0, 0x66, 0x0f, 0x3a, 0x08, 0xca, 0],
        vec![0xc4, 0xe3, 0x71, 0x08, 0xca, 0],
        vec![0xc4, 0xe3, 0x75, 0x09, 0xca, 0],
    ] {
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let declaration =
            declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
        assert!(
            Decoder::with_ip(64, &code, BIAS + ENTRY as u64, DecoderOptions::NONE)
                .decode()
                .is_invalid()
        );
        assert!(matches!(
            plan_materialized_block(
                &declaration,
                0,
                BIAS + ENTRY as u64,
                &code,
                placement(),
                &TranslationPolicy::x64_sse2().with_independent_memory(),
                &BlockLimits::default()
            ),
            Err(CodegenError::InvalidInstruction { .. })
        ));
    }
}
