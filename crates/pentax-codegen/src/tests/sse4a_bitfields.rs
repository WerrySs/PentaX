// SPDX-License-Identifier: GPL-2.0-only
//! Original literal encodings of all four SSE4a register-only forms.
use super::*;

#[test]
fn sse4a_four_forms_retain_exact_operands_controls_and_always_mediate() {
    for (insert, immediate, expected) in [
        (false, true, Code::Extrq_xmm_imm8_imm8),
        (false, false, Code::Extrq_xmm_xmm),
        (true, true, Code::Insertq_xmm_xmm_imm8_imm8),
        (true, false, Code::Insertq_xmm_xmm),
    ] {
        for alias in [false, true] {
            for (length, index) in [(0, 0), (0xc8, 0xc8), (0, 1), (63, 2)] {
                let mut code = vec![
                    if insert { 0xf2 } else { 0x66 },
                    if !insert && immediate { 0x41 } else { 0x45 },
                    0x0f,
                    if immediate { 0x78 } else { 0x79 },
                    if !insert && immediate {
                        0xc6
                    } else if alias {
                        0xf6
                    } else {
                        0xf7
                    },
                ];
                if immediate {
                    code.extend([length, index]);
                }
                let size = code.len();
                code.push(0xc3);
                let bytes = image(&code);
                let source = link(&[(&bytes, BIAS)]);
                let declaration =
                    declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
                for features in [
                    CpuFeatures::x64_sse2(),
                    CpuFeatures::x64_sse2()
                        .with(CpuidFeature::SSE4A)
                        .with(CpuidFeature::AVX),
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
                    assert_eq!(plan.instructions().len(), 1);
                    assert_eq!(
                        plan.exit().kind(),
                        ExitKind::Mediated {
                            reason: UnsupportedReason::InstructionClass
                        }
                    );
                    assert_eq!(
                        plan.exit().next_guest_pc(),
                        BIAS + ENTRY as u64 + size as u64
                    );
                    let instruction = plan.exit().instruction().unwrap();
                    assert_eq!(instruction.code(), expected);
                    assert_eq!(instruction.op0_register(), Register::XMM14);
                    let registers = if insert || !immediate { 2 } else { 1 };
                    assert_eq!(
                        instruction.op_count(),
                        registers + if immediate { 2 } else { 0 }
                    );
                    if registers == 2 {
                        assert_eq!(instruction.op1_kind(), OpKind::Register);
                        assert_eq!(
                            instruction.op1_register(),
                            if alias {
                                Register::XMM14
                            } else {
                                Register::XMM15
                            }
                        );
                    }
                    if immediate {
                        assert_eq!(instruction.op_kind(registers), OpKind::Immediate8);
                        assert_eq!(instruction.op_kind(registers + 1), OpKind::Immediate8_2nd);
                        assert_eq!(instruction.immediate8(), length);
                        assert_eq!(instruction.immediate8_2nd(), index);
                    }
                }
            }
        }
    }
}
