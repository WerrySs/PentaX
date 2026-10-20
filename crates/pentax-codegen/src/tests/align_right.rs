// SPDX-License-Identifier: GPL-2.0-only
//! PALIGNR plans retain exact operands without expanding native authority.
use super::*;

#[test]
fn three_align_forms_retain_register_sib_rip_immediates_and_declared_widths_as_mediated_exits() {
    for (form, expected) in [
        (0, Code::Palignr_xmm_xmmm128_imm8),
        (1, Code::VEX_Vpalignr_xmm_xmm_xmmm128_imm8),
        (2, Code::VEX_Vpalignr_ymm_ymm_ymmm256_imm8),
    ] {
        for w in [false, true] {
            if form == 0 && w {
                continue;
            }
            for addressing in 0..3 {
                for immediate in [0, 15, 16, 17, 31, 32, 255] {
                    let mut code = if form == 0 {
                        vec![0x66, 0x0f, 0x3a, 0x0f]
                    } else {
                        vec![
                            0xc4,
                            0xe3,
                            0x61 | if form == 2 { 4 } else { 0 } | if w { 0x80 } else { 0 },
                            0x0f,
                        ]
                    };
                    match addressing {
                        0 => code.push(0xca),
                        1 => code.extend([0x4c, 0x8b, 0x20]),
                        _ => code.extend([0x0d, 0, 1, 0, 0]),
                    }
                    code.push(immediate);
                    let length = code.len();
                    code.push(0xc3);
                    let bytes = image(&code);
                    let source = link(&[(&bytes, BIAS)]);
                    let declaration =
                        declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
                    for features in [
                        CpuFeatures::x64_sse2(),
                        CpuFeatures::x64_sse2()
                            .with(CpuidFeature::SSSE3)
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
                        let low = if form == 0 { 1 } else { 2 };
                        assert_eq!(instruction.code(), expected);
                        assert_eq!(instruction.op_count(), low + 2);
                        assert_eq!(
                            instruction.op0_register(),
                            if form == 2 {
                                Register::YMM1
                            } else {
                                Register::XMM1
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
                        assert_eq!(instruction.op_kind(low + 1), OpKind::Immediate8);
                        assert_eq!(instruction.immediate8(), immediate);
                        if addressing == 0 {
                            assert_eq!(
                                instruction.op_register(low),
                                if form == 2 {
                                    Register::YMM2
                                } else {
                                    Register::XMM2
                                }
                            );
                        } else {
                            assert_eq!(instruction.op_kind(low), OpKind::Memory);
                            assert_eq!(
                                instruction.memory_size().size(),
                                if form == 2 { 32 } else { 16 }
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
    }
}

#[test]
fn lock_is_invalid_and_mmx_evex_remain_outside_the_align_profile() {
    let locked = [0xf0, 0x66, 0x0f, 0x3a, 0x0f, 0xca, 1];
    assert!(
        Decoder::with_ip(64, &locked, BIAS + ENTRY as u64, DecoderOptions::NONE)
            .decode()
            .is_invalid()
    );
    for code in [
        vec![0x0f, 0x3a, 0x0f, 0xca, 1, 0xc3],
        vec![0x62, 0xf3, 0x65, 0x08, 0x0f, 0xca, 1, 0xc3],
    ] {
        assert!(
            !Decoder::with_ip(64, &code, BIAS + ENTRY as u64, DecoderOptions::NONE)
                .decode()
                .is_invalid()
        );
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
                    .with(CpuidFeature::SSSE3)
                    .with(CpuidFeature::AVX512BW)
                    .with(CpuidFeature::AVX512VL),
            )
            .with_native_ymm_registers(),
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
    }
}
