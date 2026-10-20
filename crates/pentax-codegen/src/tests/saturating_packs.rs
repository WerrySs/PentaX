// SPDX-License-Identifier: GPL-2.0-only
//! Original pack encodings preserve existing native policy and mediated shapes.

use super::*;

fn encoding(kind: usize, form: usize, wide_bit: bool, addressing: usize) -> Vec<u8> {
    let opcode = [0x63, 0x6b, 0x67, 0x2b][kind];
    let mut code = if form == 0 {
        let mut prefix = vec![0x66, 0x0f];
        if kind == 3 {
            prefix.push(0x38);
        }
        prefix.push(opcode);
        prefix
    } else {
        vec![
            0xc4,
            if kind == 3 { 0xe2 } else { 0xe1 },
            0x61 | if form == 2 { 4 } else { 0 } | if wide_bit { 0x80 } else { 0 },
            opcode,
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

fn expected(kind: usize, form: usize) -> Code {
    match (kind, form) {
        (0, 0) => Code::Packsswb_xmm_xmmm128,
        (0, 1) => Code::VEX_Vpacksswb_xmm_xmm_xmmm128,
        (0, 2) => Code::VEX_Vpacksswb_ymm_ymm_ymmm256,
        (1, 0) => Code::Packssdw_xmm_xmmm128,
        (1, 1) => Code::VEX_Vpackssdw_xmm_xmm_xmmm128,
        (1, 2) => Code::VEX_Vpackssdw_ymm_ymm_ymmm256,
        (2, 0) => Code::Packuswb_xmm_xmmm128,
        (2, 1) => Code::VEX_Vpackuswb_xmm_xmm_xmmm128,
        (2, 2) => Code::VEX_Vpackuswb_ymm_ymm_ymmm256,
        (3, 0) => Code::Packusdw_xmm_xmmm128,
        (3, 1) => Code::VEX_Vpackusdw_xmm_xmm_xmmm128,
        (3, 2) => Code::VEX_Vpackusdw_ymm_ymm_ymmm256,
        _ => unreachable!(),
    }
}

#[test]
fn all_pack_forms_preserve_exact_operands_widths_and_rip_targets_when_mediated() {
    for kind in 0..4 {
        for form in 0..3 {
            for wide_bit in [false, true] {
                if form == 0 && wide_bit {
                    continue;
                }
                for addressing in 0..3 {
                    let code = encoding(kind, form, wide_bit, addressing);
                    let length = code.len() - 1;
                    let bytes = image(&code);
                    let source = link(&[(&bytes, BIAS)]);
                    let declaration =
                        declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
                    // Force the three already admitted SSE2 legacy families
                    // through their genuine missing-feature boundary only.
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
                    let reason = if form == 0 && kind < 3 {
                        UnsupportedReason::MissingFeature(CpuidFeature::SSE2)
                    } else {
                        UnsupportedReason::InstructionClass
                    };
                    assert_eq!(plan.exit().kind(), ExitKind::Mediated { reason });
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
                    assert_eq!(instruction.code(), expected(kind, form));
                    let operand = if form == 0 { 1 } else { 2 };
                    let width = if form == 2 { 32 } else { 16 };
                    assert_eq!(instruction.op_count(), operand + 1);
                    assert_eq!(
                        instruction.op0_register(),
                        if form == 2 {
                            Register::YMM2
                        } else {
                            Register::XMM2
                        }
                    );
                    if form != 0 {
                        assert_eq!(
                            instruction.op1_register(),
                            if form == 2 {
                                Register::YMM3
                            } else {
                                Register::XMM3
                            }
                        );
                    }
                    if addressing == 0 {
                        assert_eq!(instruction.op_kind(operand), OpKind::Register);
                        assert_eq!(
                            instruction.op_register(operand),
                            if form == 2 {
                                Register::YMM4
                            } else {
                                Register::XMM4
                            }
                        );
                        assert_eq!(record.rip_relative_target(), None);
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
fn pack_policy_preserves_native_sse2_and_never_admits_mmx_or_evex_by_family() {
    for kind in 0..4 {
        for form in 0..3 {
            let code = encoding(kind, form, false, 0);
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
                    &TranslationPolicy::new(features),
                    &BlockLimits::default(),
                )
                .unwrap();
                assert_eq!(
                    plan.instructions()[0].instruction().code(),
                    expected(kind, form)
                );
                if form == 0 && kind < 3 {
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
        }
        let mut locked = vec![0xf0];
        locked.extend(encoding(kind, 0, false, 0));
        let mut decoder = Decoder::with_ip(64, &locked, BIAS + ENTRY as u64, DecoderOptions::NONE);
        assert!(decoder.decode().is_invalid());
    }
    for code in [
        vec![0x0f, 0x63, 0xd4, 0xc3],
        vec![0x62, 0xf1, 0x65, 0x08, 0x63, 0xd4, 0xc3],
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
                    .with(CpuidFeature::MMX)
                    .with(CpuidFeature::AVX512F)
                    .with(CpuidFeature::AVX512BW)
                    .with(CpuidFeature::AVX512VL),
            ),
            &BlockLimits::default(),
        )
        .unwrap();
        assert!(matches!(plan.exit().kind(), ExitKind::Mediated { .. }));
        assert_eq!(plan.bytes(), [0x0f, 0x0b]);
        assert_eq!(plan.instructions()[0].emitted_len(), 0);
    }
}
