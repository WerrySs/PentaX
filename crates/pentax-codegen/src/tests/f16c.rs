// SPDX-License-Identifier: GPL-2.0-only
//! Original exact F16C shapes stay mediated even with native feature authority.
use super::*;

#[test]
fn f16c_four_forms_keep_precise_widths_operands_and_immediate_without_native_emission() {
    for (narrow, wide, expected, size) in [
        (true, false, Code::VEX_Vcvtps2ph_xmmm64_xmm_imm8, 8),
        (true, true, Code::VEX_Vcvtps2ph_xmmm128_ymm_imm8, 16),
        (false, false, Code::VEX_Vcvtph2ps_xmm_xmmm64, 8),
        (false, true, Code::VEX_Vcvtph2ps_ymm_xmmm128, 16),
    ] {
        for memory in [false, true] {
            let mut code = vec![
                0xc4,
                if narrow { 0xe3 } else { 0xe2 },
                0x79 | if wide { 4 } else { 0 },
                if narrow { 0x1d } else { 0x13 },
            ];
            if memory {
                code.extend([0x15, 0, 1, 0, 0]);
            } else {
                code.push(0xd4);
            }
            if narrow {
                code.push(0xfb);
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
                    .with(CpuidFeature::F16C),
            ] {
                let plan = plan_materialized_block(
                    &declaration,
                    0,
                    BIAS + ENTRY as u64,
                    &code,
                    placement(),
                    &TranslationPolicy::new(features)
                        .with_independent_memory()
                        .with_native_ymm_registers()
                        .with_native_packed_float_memory()
                        .with_native_vector_moves()
                        .with_native_scalar_float()
                        .with_native_float_min_max()
                        .with_native_vector_broadcasts(),
                    &BlockLimits::default(),
                )
                .unwrap();
                assert_eq!(plan.bytes(), [0x0f, 0x0b]);
                assert_eq!(
                    plan.exit().kind(),
                    ExitKind::Mediated {
                        reason: UnsupportedReason::InstructionClass
                    }
                );
                assert_eq!(plan.instructions().len(), 1);
                assert_eq!(
                    plan.exit().next_guest_pc(),
                    BIAS + ENTRY as u64 + length as u64
                );
                let instruction = plan.exit().instruction().unwrap();
                assert_eq!(instruction.code(), expected);
                assert_eq!(instruction.op_count(), if narrow { 3 } else { 2 });
                if narrow {
                    assert_eq!(instruction.immediate8(), 0xfb);
                }
                let memory_operand = u32::from(!narrow);
                if memory {
                    assert_eq!(instruction.op_kind(memory_operand), OpKind::Memory);
                    assert_eq!(instruction.memory_size().size(), size);
                    assert_eq!(
                        instruction.ip_rel_memory_address(),
                        BIAS + ENTRY as u64 + length as u64 + 256
                    );
                } else {
                    assert_eq!(
                        instruction.op0_register(),
                        if narrow {
                            Register::XMM4
                        } else if wide {
                            Register::YMM2
                        } else {
                            Register::XMM2
                        }
                    );
                    assert_eq!(
                        instruction.op1_register(),
                        if narrow && wide {
                            Register::YMM2
                        } else if narrow {
                            Register::XMM2
                        } else {
                            Register::XMM4
                        }
                    );
                }
                assert_eq!(
                    MemoryAccessAnalyzer::default().analyze(&plan.instructions()[0], &[0; 16]),
                    Err(MemoryAccessError::NotEmitted)
                );
            }
        }
    }
}

#[test]
fn f16c_reserved_vvvv_and_lock_are_rejected_without_promoting_other_encodings() {
    for (map, opcode, narrow) in [(0xe3, 0x1d, true), (0xe2, 0x13, false)] {
        for wide in [false, true] {
            let mut code = vec![0xc4, map, 0x71 | if wide { 4 } else { 0 }, opcode, 0xd4];
            if narrow {
                code.push(0);
            }
            let mut decoder = Decoder::new(64, &code, DecoderOptions::NONE);
            assert!(decoder.decode().is_invalid());
            code[2] |= 8;
            code.insert(0, 0xf0);
            let mut decoder = Decoder::new(64, &code, DecoderOptions::NONE);
            assert!(decoder.decode().is_invalid());
        }
    }
}
