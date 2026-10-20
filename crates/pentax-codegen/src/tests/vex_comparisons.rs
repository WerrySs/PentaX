// SPDX-License-Identifier: GPL-2.0-only
//! Planning retains VEX operands without granting native AVX execution.
//! Runtime supported-form and predicate semantics require separate proofs.

use super::*;

#[test]
fn six_vex_mask_comparison_forms_retain_source_operands_and_immediate_under_mediation() {
    for (prefix, expected, wide, memory_bytes) in [
        (0xe0, Code::VEX_Vcmpps_xmm_xmm_xmmm128_imm8, false, 16),
        (0xe4, Code::VEX_Vcmpps_ymm_ymm_ymmm256_imm8, true, 32),
        (0xe1, Code::VEX_Vcmppd_xmm_xmm_xmmm128_imm8, false, 16),
        (0xe5, Code::VEX_Vcmppd_ymm_ymm_ymmm256_imm8, true, 32),
        (0xe2, Code::VEX_Vcmpss_xmm_xmm_xmmm32_imm8, false, 4),
        (0xe3, Code::VEX_Vcmpsd_xmm_xmm_xmmm64_imm8, false, 8),
    ] {
        for addressing in 0..3 {
            // Destination2, source1=3; source2=register4, [RBX+RCX*4+32]
            // or [original RIP+256]. All source bytes are original fixtures.
            let mut code = vec![0xc5, prefix, 0xc2];
            match addressing {
                0 => code.push(0xd4),
                1 => code.extend([0x54, 0x8b, 0x20]),
                _ => code.extend([0x15, 0, 1, 0, 0]),
            }
            code.push(0xe7); // Keep all eight immediate bits for runtime semantics.
            let length = code.len();
            code.push(0xc3);
            let bytes = image(&code);
            let source = link(&[(&bytes, BIAS)]);
            let declaration =
                declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
            for features in [
                CpuFeatures::x64_sse2(),
                CpuFeatures::x64_sse2().with(CpuidFeature::AVX),
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
                assert_eq!(record.instruction(), instruction);
                assert_eq!((instruction.code(), instruction.op_count()), (expected, 4));
                assert_eq!(
                    instruction.op0_register(),
                    if wide { Register::YMM2 } else { Register::XMM2 }
                );
                assert_eq!(
                    instruction.op1_register(),
                    if wide { Register::YMM3 } else { Register::XMM3 }
                );
                assert_eq!(
                    (instruction.op3_kind(), instruction.immediate8()),
                    (OpKind::Immediate8, 0xe7)
                );
                if addressing == 0 {
                    assert_eq!(
                        instruction.op2_register(),
                        if wide { Register::YMM4 } else { Register::XMM4 }
                    );
                    assert_eq!(record.rip_relative_target(), None);
                } else {
                    assert_eq!(instruction.op2_kind(), OpKind::Memory);
                    assert_eq!(instruction.memory_size().size(), memory_bytes);
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
                        assert_eq!(record.rip_relative_target(), None);
                    } else {
                        let target = BIAS + ENTRY as u64 + length as u64 + 256;
                        assert_eq!(record.rip_relative_target(), Some(target));
                        assert_eq!(instruction.ip_rel_memory_address(), target);
                    }
                }
                // A retained decoded operand is no native memory admission.
                assert_eq!(
                    MemoryAccessAnalyzer::default().analyze(record, &[0; 16]),
                    Err(MemoryAccessError::NotEmitted)
                );
            }
        }
    }
    for (prefix, reason, base, segment) in [
        (
            0x64,
            UnsupportedReason::SegmentPrefix,
            Register::RBX,
            Register::FS,
        ),
        (
            0x67,
            UnsupportedReason::InstructionClass,
            Register::EBX,
            Register::DS,
        ),
    ] {
        let code = [prefix, 0xc5, 0xe0, 0xc2, 0x54, 0x8b, 0x20, 0xe7, 0xc3];
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = make(&source);
        assert_eq!(plan.exit().kind(), ExitKind::Mediated { reason });
        assert_eq!(plan.bytes(), [0x0f, 0x0b]);
        let instruction = plan.exit().instruction().unwrap();
        assert_eq!(
            (
                instruction.memory_base(),
                instruction.memory_segment(),
                instruction.immediate8()
            ),
            (base, segment, 0xe7)
        );
    }
}

#[test]
fn legacy_mask_comparisons_stay_native_and_vex_flag_comparisons_stay_mediated() {
    for (prefix, mnemonic) in [
        (None, Mnemonic::Cmpps),
        (Some(0x66), Mnemonic::Cmppd),
        (Some(0xf3), Mnemonic::Cmpss),
        (Some(0xf2), Mnemonic::Cmpsd),
    ] {
        let mut code = prefix.into_iter().collect::<Vec<_>>();
        code.extend([0x0f, 0xc2, 0xd4, 0xe7, 0xc3]);
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = make(&source);
        assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
        let output = ordinary_output(&plan);
        assert_eq!(output.len(), 1);
        assert_eq!(
            (output[0].mnemonic(), output[0].immediate8()),
            (mnemonic, 0xe7)
        );
        assert_eq!(plan.instructions()[0].emitted_len(), code.len() - 1);
    }
    for (prefix, opcode, mnemonic) in [
        (0xf8, 0x2f, Mnemonic::Vcomiss),
        (0xf9, 0x2f, Mnemonic::Vcomisd),
        (0xf8, 0x2e, Mnemonic::Vucomiss),
        (0xf9, 0x2e, Mnemonic::Vucomisd),
    ] {
        let code = [0xc5, prefix, opcode, 0xd4, 0xc3];
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = make(&source);
        assert_eq!(
            plan.exit().kind(),
            ExitKind::Mediated {
                reason: UnsupportedReason::InstructionClass
            }
        );
        assert_eq!(plan.bytes(), [0x0f, 0x0b]);
        let instruction = plan.exit().instruction().unwrap();
        assert_eq!(
            (instruction.mnemonic(), instruction.op_count()),
            (mnemonic, 2)
        );
        assert_eq!(
            (instruction.op0_register(), instruction.op1_register()),
            (Register::XMM2, Register::XMM4)
        );
    }
}
