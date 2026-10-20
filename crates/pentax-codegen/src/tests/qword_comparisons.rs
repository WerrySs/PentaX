// SPDX-License-Identifier: GPL-2.0-only
//! Exact signed-qword forms retain complete operands without native admission.
use super::*;

#[test]
fn all_qword_comparison_forms_retain_operands_rip_and_width_under_forced_mediation() {
    for form in 0..3 {
        for wide in [false, true] {
            if form == 0 && wide {
                continue;
            }
            for addressing in 0..3 {
                let mut code = if form == 0 {
                    vec![0x66, 0x0f, 0x38, 0x37]
                } else {
                    vec![
                        0xc4,
                        0xe2,
                        0x61 | if form == 2 { 4 } else { 0 } | if wide { 0x80 } else { 0 },
                        0x37,
                    ]
                };
                match addressing {
                    0 => code.push(0xd4),                 // Destination2, source2=4; VEX source1=3.
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
                        .with(CpuidFeature::SSE4_2)
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
                    assert_eq!(
                        (
                            record.source_offset(),
                            record.source_len(),
                            record.emitted_len()
                        ),
                        (0, length, 0)
                    );
                    assert_eq!(
                        (plan.exit().guest_pc(), plan.exit().next_guest_pc()),
                        (BIAS + ENTRY as u64, BIAS + ENTRY as u64 + length as u64)
                    );
                    let instruction = plan.exit().instruction().unwrap();
                    assert_eq!(instruction, record.instruction());
                    let size = if form == 2 { 32 } else { 16 };
                    assert_eq!(
                        instruction.code(),
                        match form {
                            0 => Code::Pcmpgtq_xmm_xmmm128,
                            1 => Code::VEX_Vpcmpgtq_xmm_xmm_xmmm128,
                            _ => Code::VEX_Vpcmpgtq_ymm_ymm_ymmm256,
                        }
                    );
                    let operand = if form == 0 { 1 } else { 2 };
                    assert_eq!(instruction.op_count(), operand + 1);
                    assert_eq!(
                        instruction.op0_register(),
                        if size == 16 {
                            Register::XMM2
                        } else {
                            Register::YMM2
                        }
                    );
                    if form != 0 {
                        assert_eq!(
                            instruction.op1_register(),
                            if size == 16 {
                                Register::XMM3
                            } else {
                                Register::YMM3
                            }
                        );
                    }
                    if addressing == 0 {
                        assert_eq!(instruction.op_kind(operand), OpKind::Register);
                        assert_eq!(
                            instruction.op_register(operand),
                            if size == 16 {
                                Register::XMM4
                            } else {
                                Register::YMM4
                            }
                        );
                        assert_eq!(record.rip_relative_target(), None);
                    } else {
                        assert_eq!(instruction.op_kind(operand), OpKind::Memory);
                        assert_eq!(instruction.memory_size().size(), size);
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
}

#[test]
fn qword_invalid_prefixes_refuse_and_evex_or_legacy_dword_do_not_gain_native_authority() {
    // Iced's mandatory-prefix table admits only66; W is ignored and both L
    // values are valid. VEX.vvvv is a real SRC1, not a reserved field.
    for code in [
        vec![0xf0, 0x66, 0x0f, 0x38, 0x37, 0xd4],
        vec![0xc4, 0xe2, 0x60, 0x37, 0xd4],
        vec![0xc4, 0xe2, 0x62, 0x37, 0xd4],
        vec![0xc4, 0xe2, 0x63, 0x37, 0xd4],
    ] {
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
                &TranslationPolicy::x64_sse2(),
                &BlockLimits::default()
            ),
            Err(CodegenError::InvalidInstruction { .. })
        ));
    }
    for (code, evex) in [
        (vec![0x62, 0xf2, 0xe5, 0x08, 0x37, 0xcc, 0xc3], true),
        (vec![0x66, 0x0f, 0x66, 0xd4, 0xc3], false),
    ] {
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
        if evex {
            assert_eq!(
                plan.exit().instruction().unwrap().code(),
                Code::EVEX_Vpcmpgtq_kr_k1_xmm_xmmm128b64
            );
            assert_eq!(
                plan.exit().kind(),
                ExitKind::Mediated {
                    reason: UnsupportedReason::InstructionClass
                }
            );
            assert_eq!(plan.bytes(), [0x0f, 0x0b]);
        } else {
            assert_eq!(
                plan.instructions()[0].instruction().code(),
                Code::Pcmpgtd_xmm_xmmm128
            );
            assert!(plan.instructions()[0].emitted_len() > 0);
            assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
        }
    }
}
