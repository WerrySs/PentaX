// SPDX-License-Identifier: GPL-2.0-only
//! Original PMADDWD shapes retain operands without broadening native policy.
use super::*;

fn encoding(form: usize, wide: bool, addressing: usize) -> Vec<u8> {
    let mut code = if form == 0 {
        vec![0x66, 0x0f, 0xf5]
    } else {
        vec![
            0xc4,
            0xe1,
            0x61 | if form == 2 { 4 } else { 0 } | if wide { 0x80 } else { 0 },
            0xf5,
        ]
    };
    match addressing {
        0 => code.push(0xd4),
        1 => code.extend([0x54, 0x8b, 0x20]),
        _ => code.extend([0x15, 0, 1, 0, 0]),
    }
    code.push(0xc3);
    code
}

#[test]
fn multiply_add_words_retains_exact_widths_operands_and_original_rip_under_mediation() {
    for form in 0..3 {
        for wide in [false, true] {
            if form == 0 && wide {
                continue;
            }
            for addressing in 0..3 {
                let code = encoding(form, wide, addressing);
                let length = code.len() - 1;
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
                    &TranslationPolicy::new(CpuFeatures::empty()),
                    &BlockLimits::default(),
                )
                .unwrap();
                assert_eq!(
                    plan.exit().kind(),
                    ExitKind::Mediated {
                        reason: if form == 0 {
                            UnsupportedReason::MissingFeature(CpuidFeature::SSE2)
                        } else {
                            UnsupportedReason::InstructionClass
                        }
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
                assert_eq!(
                    instruction.code(),
                    match form {
                        0 => Code::Pmaddwd_xmm_xmmm128,
                        1 => Code::VEX_Vpmaddwd_xmm_xmm_xmmm128,
                        _ => Code::VEX_Vpmaddwd_ymm_ymm_ymmm256,
                    }
                );
                let operand = if form == 0 { 1 } else { 2 };
                let size = if form == 2 { 32 } else { 16 };
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

#[test]
fn multiply_add_words_preserves_native_sse2_but_refuses_mmx_evex_and_invalid_encodings() {
    let features = CpuFeatures::x64_sse2()
        .with(CpuidFeature::AVX)
        .with(CpuidFeature::AVX2);
    for form in 0..3 {
        let code = encoding(form, false, 0);
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::new(features),
            &BlockLimits::default(),
        )
        .unwrap();
        if form == 0 {
            assert!(plan.instructions()[0].emitted_len() > 0);
            assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
        } else {
            assert_eq!(plan.bytes(), [0x0f, 0x0b]);
            assert_eq!(
                plan.exit().kind(),
                ExitKind::Mediated {
                    reason: UnsupportedReason::InstructionClass
                }
            );
        }
    }
    for code in [
        vec![0x0f, 0xf5, 0xd4, 0xc3],
        vec![0x62, 0xf1, 0x65, 0x08, 0xf5, 0xd4, 0xc3],
    ] {
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::new(
                features
                    .with(CpuidFeature::MMX)
                    .with(CpuidFeature::AVX512F)
                    .with(CpuidFeature::AVX512BW)
                    .with(CpuidFeature::AVX512VL),
            ),
            &BlockLimits::default(),
        )
        .unwrap();
        assert_eq!(plan.instructions()[0].emitted_len(), 0);
        assert_eq!(plan.bytes(), [0x0f, 0x0b]);
    }
    for code in [
        vec![0xf0, 0x66, 0x0f, 0xf5, 0xd4],
        vec![0xc4, 0xe1, 0x60, 0xf5, 0xd4],
        vec![0xc4, 0xe1, 0x62, 0xf5, 0xd4],
        vec![0xc4, 0xe1, 0x63, 0xf5, 0xd4],
    ] {
        let mut decoder = Decoder::with_ip(64, &code, BIAS + ENTRY as u64, DecoderOptions::NONE);
        assert!(decoder.decode().is_invalid());
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        assert!(matches!(
            plan_block(
                &source,
                0,
                BIAS + ENTRY as u64,
                placement(),
                &TranslationPolicy::new(features),
                &BlockLimits::default()
            ),
            Err(CodegenError::InvalidInstruction { .. })
        ));
    }
}
