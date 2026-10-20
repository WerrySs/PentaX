// SPDX-License-Identifier: GPL-2.0-only
//! Requested native VEX plans are not proof of an admitted YMM execution bridge.
use super::*;

// Map, mandatory prefix, opcode, two-operand transfer, exact128/256 Codes.
const FORMS: [(u8, u8, u8, bool, Code, Code); 13] = [
    (
        1,
        1,
        0x6f,
        true,
        Code::VEX_Vmovdqa_xmm_xmmm128,
        Code::VEX_Vmovdqa_ymm_ymmm256,
    ),
    (
        1,
        1,
        0x7f,
        true,
        Code::VEX_Vmovdqa_xmmm128_xmm,
        Code::VEX_Vmovdqa_ymmm256_ymm,
    ),
    (
        1,
        2,
        0x6f,
        true,
        Code::VEX_Vmovdqu_xmm_xmmm128,
        Code::VEX_Vmovdqu_ymm_ymmm256,
    ),
    (
        1,
        2,
        0x7f,
        true,
        Code::VEX_Vmovdqu_xmmm128_xmm,
        Code::VEX_Vmovdqu_ymmm256_ymm,
    ),
    (
        1,
        1,
        0xdb,
        false,
        Code::VEX_Vpand_xmm_xmm_xmmm128,
        Code::VEX_Vpand_ymm_ymm_ymmm256,
    ),
    (
        1,
        1,
        0xdf,
        false,
        Code::VEX_Vpandn_xmm_xmm_xmmm128,
        Code::VEX_Vpandn_ymm_ymm_ymmm256,
    ),
    (
        1,
        1,
        0xeb,
        false,
        Code::VEX_Vpor_xmm_xmm_xmmm128,
        Code::VEX_Vpor_ymm_ymm_ymmm256,
    ),
    (
        1,
        1,
        0xef,
        false,
        Code::VEX_Vpxor_xmm_xmm_xmmm128,
        Code::VEX_Vpxor_ymm_ymm_ymmm256,
    ),
    (
        2,
        1,
        0x37,
        false,
        Code::VEX_Vpcmpgtq_xmm_xmm_xmmm128,
        Code::VEX_Vpcmpgtq_ymm_ymm_ymmm256,
    ),
    (
        1,
        1,
        0x63,
        false,
        Code::VEX_Vpacksswb_xmm_xmm_xmmm128,
        Code::VEX_Vpacksswb_ymm_ymm_ymmm256,
    ),
    (
        1,
        1,
        0x6b,
        false,
        Code::VEX_Vpackssdw_xmm_xmm_xmmm128,
        Code::VEX_Vpackssdw_ymm_ymm_ymmm256,
    ),
    (
        1,
        1,
        0x67,
        false,
        Code::VEX_Vpackuswb_xmm_xmm_xmmm128,
        Code::VEX_Vpackuswb_ymm_ymm_ymmm256,
    ),
    (
        2,
        1,
        0x2b,
        false,
        Code::VEX_Vpackusdw_xmm_xmm_xmmm128,
        Code::VEX_Vpackusdw_ymm_ymm_ymmm256,
    ),
];

fn encoding(form: usize, wide: bool, memory: bool, ignored_w: bool) -> Vec<u8> {
    let (map, pp, opcode, transfer, _, _) = FORMS[form];
    vec![
        0xc4,
        (if memory { 0x60 } else { 0x40 }) | map,
        (if transfer { 0x78 } else { 0x10 })
            | pp
            | if wide { 4 } else { 0 }
            | if ignored_w { 0x80 } else { 0 },
        opcode,
        if memory { 0x08 } else { 0xce },
        0xc3,
    ]
}
fn inspect(code: &[u8], policy: TranslationPolicy, check: impl FnOnce(&BlockPlan<'_, '_>)) {
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
    assert_eq!(plan.policy(), policy);
    check(&plan);
}
fn mediated(plan: &BlockPlan<'_, '_>, reason: UnsupportedReason) {
    assert_eq!(plan.exit().kind(), ExitKind::Mediated { reason });
    assert_eq!(plan.bytes(), [0x0f, 0x0b]);
    assert_eq!(plan.instructions().len(), 1);
    assert_eq!(plan.instructions()[0].emitted_len(), 0);
}

#[test]
fn twenty_six_register_forms_require_explicit_mode_and_exact_positive_cpu_features() {
    let avx = CpuFeatures::x64_sse2().with(CpuidFeature::AVX);
    let full = avx.with(CpuidFeature::AVX2);
    for (form, &(_, _, opcode, transfer, narrow, wide_code)) in FORMS.iter().enumerate() {
        for wide in [false, true] {
            for ignored_w in [false, true] {
                let code = encoding(form, wide, false, ignored_w);
                let expected = if wide { wide_code } else { narrow };
                // Feature bits alone cannot grant vector-state authority.
                inspect(&code, TranslationPolicy::new(full), |plan| {
                    mediated(plan, UnsupportedReason::InstructionClass);
                    assert_eq!(plan.exit().instruction().unwrap().code(), expected);
                    assert!(!plan.policy().requests_native_ymm_registers());
                });
                inspect(
                    &code,
                    TranslationPolicy::new(full).with_native_ymm_registers(),
                    |plan| {
                        assert!(plan.policy().requests_native_ymm_registers());
                        assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
                        let output = ordinary_output(plan);
                        assert_eq!(output.len(), 1);
                        assert_eq!(output[0].code(), expected);
                        assert_eq!(output[0].op_count(), if transfer { 2 } else { 3 });
                        let record = &plan.instructions()[0];
                        assert_eq!(record.instruction().code(), expected);
                        assert_eq!(record.source_len(), code.len() - 1);
                        assert!(record.emitted_len() > 0);
                        for operand in 0..output[0].op_count() {
                            assert_eq!(output[0].op_kind(operand), OpKind::Register);
                            assert_eq!(
                                output[0].op_register(operand),
                                record.instruction().op_register(operand)
                            );
                            assert_eq!(
                                output[0].op_register(operand).size(),
                                if wide { 32 } else { 16 }
                            );
                        }
                        assert_eq!(
                            output[0].op0_register(),
                            if transfer && opcode == 0x7f {
                                if wide {
                                    Register::YMM14
                                } else {
                                    Register::XMM14
                                }
                            } else if wide {
                                Register::YMM9
                            } else {
                                Register::XMM9
                            }
                        );
                        assert!(
                            MemoryAccessAnalyzer::default()
                                .analyze(record, &[0; 16])
                                .unwrap()
                                .accesses()
                                .is_empty()
                        );
                    },
                );
                for features in [
                    CpuFeatures::x64_sse2(),
                    CpuFeatures::x64_sse2().with(CpuidFeature::AVX2),
                ] {
                    inspect(
                        &code,
                        TranslationPolicy::new(features).with_native_ymm_registers(),
                        |plan| {
                            mediated(plan, UnsupportedReason::MissingFeature(CpuidFeature::AVX));
                        },
                    );
                }
                inspect(
                    &code,
                    TranslationPolicy::new(avx).with_native_ymm_registers(),
                    |plan| {
                        if wide && !transfer {
                            mediated(plan, UnsupportedReason::MissingFeature(CpuidFeature::AVX2));
                        } else {
                            assert!(plan.instructions()[0].emitted_len() > 0);
                            assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
                        }
                    },
                );
            }
        }
    }
}

#[test]
fn requested_native_ymm_profile_keeps_memory_other_fp_evex_prefix_and_reserved_forms_closed() {
    let policy = TranslationPolicy::new(
        CpuFeatures::x64_sse2()
            .with(CpuidFeature::AVX)
            .with(CpuidFeature::AVX2)
            .with(CpuidFeature::AVX512F)
            .with(CpuidFeature::AVX512VL)
            .with(CpuidFeature::AVX512BW),
    )
    .with_native_ymm_registers();
    for form in 0..FORMS.len() {
        for wide in [false, true] {
            inspect(&encoding(form, wide, true, false), policy, |plan| {
                mediated(plan, UnsupportedReason::InstructionClass);
                assert!(
                    plan.exit()
                        .instruction()
                        .unwrap()
                        .op_kinds()
                        .any(|kind| kind == OpKind::Memory)
                );
                assert_eq!(
                    MemoryAccessAnalyzer::default().analyze(&plan.instructions()[0], &[0; 16]),
                    Err(MemoryAccessError::NotEmitted)
                );
            });
        }
    }
    // Valid neighboring operations retain mediation: FP divide/scalar/duplicate,
    // GPR-vector transfer, vector state control, and EVEX saturation with masking.
    for code in [
        vec![0xc4, 0x41, 0x10, 0x5e, 0xce, 0xc3],
        vec![0xc4, 0x41, 0x12, 0x58, 0xce, 0xc3], //scalar VADDSS
        vec![0xc4, 0x41, 0x7a, 0x12, 0xce, 0xc3], //VMOVSLDUP
        vec![0xc4, 0x61, 0x79, 0x6e, 0xc8, 0xc3],
        vec![0xc5, 0xf8, 0x77, 0xc3],
        vec![0x62, 0xf1, 0x15, 0x08, 0x6b, 0xce, 0xc3],
    ] {
        inspect(&code, policy, |plan| {
            mediated(plan, UnsupportedReason::InstructionClass)
        });
    }
    let mut segmented = encoding(7, true, false, false);
    segmented.insert(0, 0x64);
    inspect(&segmented, policy, |plan| {
        mediated(plan, UnsupportedReason::SegmentPrefix)
    });
    // VMOVDQA's reserved vvvv cannot be repurposed as an extra source.
    let invalid = [0xc4, 0x41, 0x71, 0x6f, 0xce, 0xc3];
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
    // Opting in does not alter the existing legacy SSE2 instruction contract.
    inspect(
        &[0x66, 0x45, 0x0f, 0xef, 0xce, 0xc3],
        TranslationPolicy::x64_sse2(),
        |plan| {
            assert_eq!(ordinary_output(plan)[0].code(), Code::Pxor_xmm_xmmm128);
            assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
        },
    );
}
