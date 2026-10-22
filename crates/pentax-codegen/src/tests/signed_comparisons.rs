// SPDX-License-Identifier: GPL-2.0-only
//! Original exact VEX signed-compare plans never grant new native authority.
use super::*;

const FORMS: [(u8, Code, Code); 3] = [
    (
        0x64,
        Code::VEX_Vpcmpgtb_xmm_xmm_xmmm128,
        Code::VEX_Vpcmpgtb_ymm_ymm_ymmm256,
    ),
    (
        0x65,
        Code::VEX_Vpcmpgtw_xmm_xmm_xmmm128,
        Code::VEX_Vpcmpgtw_ymm_ymm_ymmm256,
    ),
    (
        0x66,
        Code::VEX_Vpcmpgtd_xmm_xmm_xmmm128,
        Code::VEX_Vpcmpgtd_ymm_ymm_ymmm256,
    ),
];

#[test]
fn signed_compare_exact_vex_forms_retain_full_sources_addresses_and_features_without_native_emission()
 {
    for (opcode, xmm, ymm) in FORMS {
        for wide in [false, true] {
            for ignored_w in [false, true] {
                for addressing in 0..3 {
                    // Destination9, source1=13, source2=14 or a complete memory span.
                    let mut code = vec![
                        0xc4,
                        if addressing == 0 { 0x41 } else { 0x61 },
                        0x11 | if wide { 4 } else { 0 } | if ignored_w { 0x80 } else { 0 },
                        opcode,
                    ];
                    match addressing {
                        0 => code.push(0xce),
                        1 => code.extend([0x4c, 0x8b, 0x20]),
                        _ => code.extend([0x0d, 0, 1, 0, 0]),
                    }
                    let length = code.len();
                    code.push(0xc3);
                    let bytes = image(&code);
                    let source = link(&[(&bytes, BIAS)]);
                    let declaration =
                        declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
                    for features in [
                        CpuFeatures::x64_sse2(),
                        CpuFeatures::x64_sse2().with(CpuidFeature::AVX),
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
                        assert_eq!(plan.instructions().len(), 1);
                        let record = &plan.instructions()[0];
                        let instruction = record.instruction();
                        assert_eq!((record.source_len(), record.emitted_len()), (length, 0));
                        assert_eq!(
                            plan.exit().next_guest_pc(),
                            BIAS + ENTRY as u64 + length as u64
                        );
                        assert_eq!(
                            (instruction.code(), instruction.op_count()),
                            (if wide { ymm } else { xmm }, 3)
                        );
                        assert_eq!(
                            instruction.op0_register(),
                            if wide { Register::YMM9 } else { Register::XMM9 }
                        );
                        assert_eq!(
                            instruction.op1_register(),
                            if wide {
                                Register::YMM13
                            } else {
                                Register::XMM13
                            }
                        );
                        assert_eq!(
                            instruction.cpuid_features(),
                            &[if wide {
                                CpuidFeature::AVX2
                            } else {
                                CpuidFeature::AVX
                            }]
                        );
                        if addressing == 0 {
                            assert_eq!(
                                instruction.op2_register(),
                                if wide {
                                    Register::YMM14
                                } else {
                                    Register::XMM14
                                }
                            );
                            assert_eq!(record.rip_relative_target(), None);
                        } else {
                            assert_eq!(instruction.op2_kind(), OpKind::Memory);
                            assert_eq!(
                                instruction.memory_size().size(),
                                if wide { 32 } else { 16 }
                            );
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
    }
}

#[test]
fn signed_compare_bad_mandatory_prefixes_refuse_and_evex_mmx_remain_outside_native_profile() {
    for (opcode, _, _) in FORMS {
        for pp in [0, 2, 3] {
            let code = [0xc4, 0xe1, 0x60 | pp, opcode, 0xd4, 0xc3];
            let mut decoder =
                Decoder::with_ip(64, &code, BIAS + ENTRY as u64, DecoderOptions::NONE);
            assert!(decoder.decode().is_invalid());
            let bytes = image(&code);
            let source = link(&[(&bytes, BIAS)]);
            assert!(matches!(
                plan_block(
                    &source,
                    0,
                    BIAS + ENTRY as u64,
                    placement(),
                    &TranslationPolicy::x64_sse2(),
                    &BlockLimits::default()
                ),
                Err(CodegenError::InvalidInstruction { .. })
            ));
        }
        for prefix in [0xf0, 0xf2, 0xf3] {
            let code = [prefix, 0xc4, 0x41, 0x11, opcode, 0xce, 0xc3];
            let mut decoder =
                Decoder::with_ip(64, &code, BIAS + ENTRY as u64, DecoderOptions::NONE);
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
                        reason: UnsupportedReason::RepeatOrLockPrefix
                    }
                );
            }
        }
        for code in [
            vec![0x62, 0xf1, 0x15, 0x08, opcode, 0xce, 0xc3],
            vec![0x0f, opcode, 0xca, 0xc3],
        ] {
            let bytes = image(&code);
            let source = link(&[(&bytes, BIAS)]);
            let plan = plan_block(
                &source,
                0,
                BIAS + ENTRY as u64,
                placement(),
                &TranslationPolicy::new(
                    CpuFeatures::x64_sse2()
                        .with(CpuidFeature::AVX)
                        .with(CpuidFeature::AVX2)
                        .with(CpuidFeature::AVX512F)
                        .with(CpuidFeature::AVX512BW)
                        .with(CpuidFeature::AVX512VL)
                        .with(CpuidFeature::MMX),
                )
                .with_native_ymm_registers(),
                &BlockLimits::default(),
            )
            .unwrap();
            assert_eq!(
                plan.exit().kind(),
                ExitKind::Mediated {
                    reason: if code[0] == 0x62 {
                        UnsupportedReason::InstructionClass
                    } else {
                        UnsupportedReason::OperandClass
                    }
                }
            );
            assert_eq!(plan.instructions()[0].emitted_len(), 0);
        }
    }
    // The retained qword accelerator still needs the explicit AVX/AVX2 grants.
    for (wide, features, missing) in [
        (false, CpuFeatures::x64_sse2(), CpuidFeature::AVX),
        (
            true,
            CpuFeatures::x64_sse2().with(CpuidFeature::AVX),
            CpuidFeature::AVX2,
        ),
    ] {
        let code = [0xc4, 0xe2, if wide { 0x65 } else { 0x61 }, 0x37, 0xd4, 0xc3];
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::new(features).with_native_ymm_registers(),
            &BlockLimits::default(),
        )
        .unwrap();
        assert_eq!(
            plan.exit().kind(),
            ExitKind::Mediated {
                reason: UnsupportedReason::MissingFeature(missing)
            }
        );
        assert_eq!(plan.instructions()[0].emitted_len(), 0);
    }
}
