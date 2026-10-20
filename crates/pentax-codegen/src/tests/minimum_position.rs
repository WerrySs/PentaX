// SPDX-License-Identifier: GPL-2.0-only
//! Original minimum-position source plans do not grant native vector execution.

use super::*;

#[test]
fn legacy_and_vex_minimum_position_retain_register_sib_and_rip_sources_under_mediation() {
    for (header, expected) in [
        ([0x66, 0x0f, 0x38, 0x41], Code::Phminposuw_xmm_xmmm128),
        ([0xc4, 0xe2, 0x79, 0x41], Code::VEX_Vphminposuw_xmm_xmmm128),
        // VEX.W is ignored: W1 is valid and normalizes to the same exact Code.
        ([0xc4, 0xe2, 0xf9, 0x41], Code::VEX_Vphminposuw_xmm_xmmm128),
    ] {
        for addressing in 0..3 {
            let mut code = header.to_vec();
            match addressing {
                0 => code.push(0xd4),                 // XMM2 <- XMM4
                1 => code.extend([0x54, 0x8b, 0x20]), // [RBX+RCX*4+32]
                _ => code.extend([0x15, 0, 1, 0, 0]), // [original RIP+256]
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
                assert_eq!((instruction.code(), instruction.op_count()), (expected, 2));
                assert_eq!(instruction.op0_kind(), OpKind::Register);
                assert_eq!(instruction.op0_register(), Register::XMM2);
                if addressing == 0 {
                    assert_eq!(instruction.op1_kind(), OpKind::Register);
                    assert_eq!(instruction.op1_register(), Register::XMM4);
                    assert_eq!(record.rip_relative_target(), None);
                } else {
                    assert_eq!(instruction.op1_kind(), OpKind::Memory);
                    assert_eq!(instruction.memory_size().size(), 16);
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
                        assert_eq!(instruction.memory_base(), Register::RIP);
                        assert_eq!(instruction.ip_rel_memory_address(), target);
                        assert_eq!(record.rip_relative_target(), Some(target));
                    }
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
fn minimum_position_reserved_fields_are_invalid_and_neighboring_evex_stays_mediated() {
    // Pinned iced1.21 rejects L1 and nonreserved vvvv for both ignored W values.
    for prefix in [0x7d, 0xfd, 0x71, 0xf1] {
        let code = [0xc4, 0xe2, prefix, 0x41, 0xd4, 0xc3];
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
    // Adjacent EVEX opcode42 is VGETEXPPS, not a minimum-position alias.
    let code = [0x62, 0xf2, 0x7d, 0x08, 0x42, 0xd4, 0xc3];
    let bytes = image(&code);
    let source = link(&[(&bytes, BIAS)]);
    let declaration = declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
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
    assert_eq!(instruction.code(), Code::EVEX_Vgetexpps_xmm_k1z_xmmm128b32);
    assert_eq!(instruction.op0_register(), Register::XMM2);
    assert_eq!(instruction.op1_register(), Register::XMM4);
}
