// SPDX-License-Identifier: GPL-2.0-only
//! Original closed approximate reciprocal/reciprocal-square-root profiles.
use super::*;

const FORMS: [(u8, bool, bool, Code); 6] = [
    (0x53, false, false, Code::VEX_Vrcpps_xmm_xmmm128),
    (0x53, true, false, Code::VEX_Vrcpps_ymm_ymmm256),
    (0x53, false, true, Code::VEX_Vrcpss_xmm_xmm_xmmm32),
    (0x52, false, false, Code::VEX_Vrsqrtps_xmm_xmmm128),
    (0x52, true, false, Code::VEX_Vrsqrtps_ymm_ymmm256),
    (0x52, false, true, Code::VEX_Vrsqrtss_xmm_xmm_xmmm32),
];

fn encoding(opcode: u8, wide: bool, scalar: bool, memory: bool) -> Vec<u8> {
    vec![
        0xc4,
        if memory { 0x61 } else { 0x41 },
        if scalar {
            0x12
        } else {
            0x78 | u8::from(wide) * 4
        },
        opcode,
        if memory { 0x38 } else { 0xfe },
        0xc3,
    ]
}

fn inspect(code: &[u8], policy: TranslationPolicy, test: impl FnOnce(&BlockPlan<'_, '_>)) {
    let bytes = image(code);
    let source = link(&[(&bytes, BIAS)]);
    let plan = plan_block(
        &source,
        0,
        BIAS + ENTRY as u64,
        placement(),
        &policy,
        &BlockLimits::default(),
    )
    .unwrap();
    test(&plan);
}

#[test]
fn approximation_register_forms_require_explicit_vector_state_and_avx_then_emit_exact_isa() {
    let features = CpuFeatures::x64_sse2().with(CpuidFeature::AVX);
    for (opcode, wide, scalar, expected) in FORMS {
        let code = encoding(opcode, wide, scalar, false);
        for requested in [false, true] {
            let policy = TranslationPolicy::new(features);
            let policy = if requested {
                policy.with_native_ymm_registers()
            } else {
                policy
            };
            inspect(&code, policy, |plan| {
                assert_eq!(plan.instructions()[0].instruction().code(), expected);
                if requested {
                    assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
                    let emitted = ordinary_output(plan);
                    assert_eq!(emitted.len(), 1);
                    assert_eq!(emitted[0].code(), expected);
                    assert_eq!(emitted[0].op_count(), if scalar { 3 } else { 2 });
                    assert_eq!(
                        emitted[0].op0_register(),
                        if wide {
                            Register::YMM15
                        } else {
                            Register::XMM15
                        }
                    );
                    assert_eq!(
                        emitted[0].op_register(if scalar { 2 } else { 1 }),
                        if wide {
                            Register::YMM14
                        } else {
                            Register::XMM14
                        }
                    );
                    if scalar {
                        assert_eq!(emitted[0].op1_register(), Register::XMM13);
                    }
                    assert!(
                        MemoryAccessAnalyzer::default()
                            .analyze(&plan.instructions()[0], &[0; 16])
                            .unwrap()
                            .accesses()
                            .is_empty()
                    );
                } else {
                    assert_eq!(
                        plan.exit().kind(),
                        ExitKind::Mediated {
                            reason: UnsupportedReason::InstructionClass
                        }
                    );
                    assert_eq!(plan.instructions()[0].emitted_len(), 0);
                }
            });
        }
        inspect(
            &code,
            TranslationPolicy::x64_sse2().with_native_ymm_registers(),
            |plan| {
                assert_eq!(
                    plan.exit().kind(),
                    ExitKind::Mediated {
                        reason: UnsupportedReason::MissingFeature(CpuidFeature::AVX)
                    }
                );
                assert_eq!(plan.instructions()[0].emitted_len(), 0);
            },
        );
    }
}

#[test]
fn approximation_memory_forms_remain_checked_and_packed_reserved_vvvv_is_not_a_source() {
    let policy = TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
        .with_native_ymm_registers()
        .with_native_packed_float_memory()
        .with_native_scalar_float();
    for (opcode, wide, scalar, expected) in FORMS {
        inspect(&encoding(opcode, wide, scalar, true), policy, |plan| {
            assert_eq!(
                plan.exit().kind(),
                ExitKind::Mediated {
                    reason: UnsupportedReason::InstructionClass
                }
            );
            assert_eq!(plan.exit().instruction().unwrap().code(), expected);
            assert_eq!(plan.instructions()[0].emitted_len(), 0);
            assert_eq!(
                MemoryAccessAnalyzer::default().analyze(&plan.instructions()[0], &[0; 16]),
                Err(MemoryAccessError::NotEmitted)
            );
        });
        if !scalar {
            let mut invalid = encoding(opcode, wide, scalar, false);
            invalid[2] &= !8;
            let bytes = image(&invalid);
            let source = link(&[(&bytes, BIAS)]);
            assert!(matches!(
                plan_block(
                    &source,
                    0,
                    BIAS + ENTRY as u64,
                    placement(),
                    &policy,
                    &BlockLimits::default()
                ),
                Err(CodegenError::InvalidInstruction { .. })
            ));
        }
    }
}

#[test]
fn legacy_approximation_forms_keep_the_existing_sse_native_contract() {
    for opcode in [0x52, 0x53] {
        for scalar in [false, true] {
            let mut code = Vec::new();
            if scalar {
                code.push(0xf3);
            }
            code.extend([0x45, 0x0f, opcode, 0xfe, 0xc3]);
            inspect(&code, TranslationPolicy::x64_sse2(), |plan| {
                assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
                let output = ordinary_output(plan);
                assert_eq!(output.len(), 1);
                assert_eq!(output[0].op0_register(), Register::XMM15);
                assert_eq!(output[0].op1_register(), Register::XMM14);
                assert_eq!(
                    output[0].mnemonic(),
                    match (opcode, scalar) {
                        (0x52, false) => Mnemonic::Rsqrtps,
                        (0x52, true) => Mnemonic::Rsqrtss,
                        (0x53, false) => Mnemonic::Rcpps,
                        _ => Mnemonic::Rcpss,
                    }
                );
            });
        }
    }
}
