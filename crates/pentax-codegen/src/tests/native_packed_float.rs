// SPDX-License-Identifier: GPL-2.0-only
//! Exact register admission is separate from real host/OS execution authority.
use super::*;

const FORMS: [(u8, bool, Code, Code); 6] = [
    (
        0x58,
        false,
        Code::VEX_Vaddps_xmm_xmm_xmmm128,
        Code::VEX_Vaddps_ymm_ymm_ymmm256,
    ),
    (
        0x58,
        true,
        Code::VEX_Vaddpd_xmm_xmm_xmmm128,
        Code::VEX_Vaddpd_ymm_ymm_ymmm256,
    ),
    (
        0x5c,
        false,
        Code::VEX_Vsubps_xmm_xmm_xmmm128,
        Code::VEX_Vsubps_ymm_ymm_ymmm256,
    ),
    (
        0x5c,
        true,
        Code::VEX_Vsubpd_xmm_xmm_xmmm128,
        Code::VEX_Vsubpd_ymm_ymm_ymmm256,
    ),
    (
        0x59,
        false,
        Code::VEX_Vmulps_xmm_xmm_xmmm128,
        Code::VEX_Vmulps_ymm_ymm_ymmm256,
    ),
    (
        0x59,
        true,
        Code::VEX_Vmulpd_xmm_xmm_xmmm128,
        Code::VEX_Vmulpd_ymm_ymm_ymmm256,
    ),
];

fn encoding(
    opcode: u8,
    double: bool,
    wide: bool,
    w: bool,
    registers: [u8; 3],
    memory: bool,
) -> Vec<u8> {
    let [destination, left, right] = registers;
    vec![
        0xc4,
        0xe1 ^ (u8::from(destination >= 8) << 7) ^ (u8::from(!memory && right >= 8) << 5),
        ((!left & 15) << 3) | (u8::from(w) << 7) | (u8::from(wide) << 2) | u8::from(double),
        opcode,
        ((destination & 7) << 3) | if memory { 0 } else { 0xc0 | (right & 7) },
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
    check(&plan);
}

fn policy() -> TranslationPolicy {
    // AVX2 is deliberately absent: all twelve forms require only AVX.
    TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
        .with_native_ymm_registers()
}

#[test]
fn twelve_packed_float_forms_preserve_order_aliases_and_coalesce_with_avx_only() {
    for &(opcode, double, narrow, wide_code) in &FORMS {
        for wide in [false, true] {
            for w in [false, true] {
                for registers in [[9, 13, 14], [13, 13, 14], [14, 13, 14]] {
                    let mut code = encoding(opcode, double, wide, w, registers, false);
                    code.push(0xc3);
                    inspect(&code, policy(), |plan| {
                        assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
                        let emitted = ordinary_output(plan);
                        assert_eq!(emitted.len(), 1);
                        assert_eq!(emitted[0].code(), if wide { wide_code } else { narrow });
                        assert_eq!(emitted[0].op_count(), 3);
                        let first = if wide { Register::YMM0 } else { Register::XMM0 };
                        for (operand, register) in registers.into_iter().enumerate() {
                            assert_eq!(emitted[0].op_kind(operand as u32), OpKind::Register);
                            assert_eq!(
                                emitted[0].op_register(operand as u32) as u32,
                                first as u32 + u32::from(register)
                            );
                        }
                        assert!(
                            MemoryAccessAnalyzer::default()
                                .analyze(&plan.instructions()[0], &[0; 16])
                                .unwrap()
                                .accesses()
                                .is_empty()
                        );
                    });
                }
            }
        }
    }
    let mut chain = Vec::new();
    for &(opcode, double, _, _) in &FORMS {
        chain.extend(encoding(opcode, double, true, false, [9, 13, 14], false));
    }
    chain.push(0xc3);
    inspect(&chain, policy(), |plan| {
        assert_eq!(ordinary_output(plan).len(), 6);
        assert_eq!(plan.instructions().len(), 7);
        assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
    });
}

#[test]
fn packed_float_request_does_not_admit_memory_other_fp_controls_or_missing_avx() {
    for &(opcode, double, _, _) in &FORMS {
        for wide in [false, true] {
            let mut register = encoding(opcode, double, wide, false, [9, 13, 14], false);
            register.push(0xc3);
            for (features, requested, reason) in [
                (
                    CpuFeatures::x64_sse2().with(CpuidFeature::AVX),
                    false,
                    UnsupportedReason::InstructionClass,
                ),
                (
                    CpuFeatures::x64_sse2(),
                    true,
                    UnsupportedReason::MissingFeature(CpuidFeature::AVX),
                ),
                (
                    CpuFeatures::x64_sse2().with(CpuidFeature::AVX2),
                    true,
                    UnsupportedReason::MissingFeature(CpuidFeature::AVX),
                ),
            ] {
                let selected = TranslationPolicy::new(features);
                inspect(
                    &register,
                    if requested {
                        selected.with_native_ymm_registers()
                    } else {
                        selected
                    },
                    |plan| {
                        assert_eq!(plan.exit().kind(), ExitKind::Mediated { reason });
                        assert_eq!(plan.bytes(), [0x0f, 0x0b]);
                    },
                );
            }
            let mut memory = encoding(opcode, double, wide, false, [9, 13, 0], true);
            memory.push(0xc3);
            inspect(&memory, policy(), |plan| {
                assert_eq!(
                    plan.exit().kind(),
                    ExitKind::Mediated {
                        reason: UnsupportedReason::InstructionClass
                    }
                );
                let instruction = plan.exit().instruction().unwrap();
                assert_eq!(instruction.op2_kind(), OpKind::Memory);
                assert_eq!(instruction.memory_size().size(), if wide { 32 } else { 16 });
                assert_eq!(plan.instructions()[0].emitted_len(), 0);
            });
            register.insert(0, 0x64);
            inspect(&register, policy(), |plan| {
                assert_eq!(
                    plan.exit().kind(),
                    ExitKind::Mediated {
                        reason: UnsupportedReason::SegmentPrefix
                    }
                )
            });
        }
    }
    // Scalar add, packed divide/min/max/sqrt, FMA, EVEX and control changes.
    for code in [
        vec![0xc5, 0xf2, 0x58, 0xc2, 0xc3],
        vec![0xc5, 0xf4, 0x5e, 0xc2, 0xc3],
        vec![0xc5, 0xf4, 0x5d, 0xc2, 0xc3],
        vec![0xc5, 0xf4, 0x5f, 0xc2, 0xc3],
        vec![0xc5, 0xfc, 0x51, 0xc2, 0xc3],
        vec![0xc4, 0xe2, 0x75, 0x98, 0xc2, 0xc3],
        vec![0x62, 0xf1, 0x74, 0x48, 0x58, 0xc2, 0xc3],
        vec![0xc5, 0xf8, 0xae, 0x10, 0xc3],
    ] {
        inspect(&code, policy(), |plan| {
            assert_eq!(
                plan.exit().kind(),
                ExitKind::Mediated {
                    reason: UnsupportedReason::InstructionClass
                }
            );
            assert_eq!(plan.bytes(), [0x0f, 0x0b]);
        });
    }
}
