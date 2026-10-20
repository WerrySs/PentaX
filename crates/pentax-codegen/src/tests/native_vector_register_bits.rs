// SPDX-License-Identifier: GPL-2.0-only
//! Independent closed encoding manifest: both copy directions and ordered ANDN.
use super::*;

const FORMS: [(u8, u8, Code, Code); 16] = [
    (
        0x10,
        0,
        Code::VEX_Vmovups_xmm_xmmm128,
        Code::VEX_Vmovups_ymm_ymmm256,
    ),
    (
        0x10,
        1,
        Code::VEX_Vmovupd_xmm_xmmm128,
        Code::VEX_Vmovupd_ymm_ymmm256,
    ),
    (
        0x11,
        0,
        Code::VEX_Vmovups_xmmm128_xmm,
        Code::VEX_Vmovups_ymmm256_ymm,
    ),
    (
        0x11,
        1,
        Code::VEX_Vmovupd_xmmm128_xmm,
        Code::VEX_Vmovupd_ymmm256_ymm,
    ),
    (
        0x28,
        0,
        Code::VEX_Vmovaps_xmm_xmmm128,
        Code::VEX_Vmovaps_ymm_ymmm256,
    ),
    (
        0x28,
        1,
        Code::VEX_Vmovapd_xmm_xmmm128,
        Code::VEX_Vmovapd_ymm_ymmm256,
    ),
    (
        0x29,
        0,
        Code::VEX_Vmovaps_xmmm128_xmm,
        Code::VEX_Vmovaps_ymmm256_ymm,
    ),
    (
        0x29,
        1,
        Code::VEX_Vmovapd_xmmm128_xmm,
        Code::VEX_Vmovapd_ymmm256_ymm,
    ),
    (
        0x54,
        0,
        Code::VEX_Vandps_xmm_xmm_xmmm128,
        Code::VEX_Vandps_ymm_ymm_ymmm256,
    ),
    (
        0x54,
        1,
        Code::VEX_Vandpd_xmm_xmm_xmmm128,
        Code::VEX_Vandpd_ymm_ymm_ymmm256,
    ),
    (
        0x55,
        0,
        Code::VEX_Vandnps_xmm_xmm_xmmm128,
        Code::VEX_Vandnps_ymm_ymm_ymmm256,
    ),
    (
        0x55,
        1,
        Code::VEX_Vandnpd_xmm_xmm_xmmm128,
        Code::VEX_Vandnpd_ymm_ymm_ymmm256,
    ),
    (
        0x56,
        0,
        Code::VEX_Vorps_xmm_xmm_xmmm128,
        Code::VEX_Vorps_ymm_ymm_ymmm256,
    ),
    (
        0x56,
        1,
        Code::VEX_Vorpd_xmm_xmm_xmmm128,
        Code::VEX_Vorpd_ymm_ymm_ymmm256,
    ),
    (
        0x57,
        0,
        Code::VEX_Vxorps_xmm_xmm_xmmm128,
        Code::VEX_Vxorps_ymm_ymm_ymmm256,
    ),
    (
        0x57,
        1,
        Code::VEX_Vxorpd_xmm_xmm_xmmm128,
        Code::VEX_Vxorpd_ymm_ymm_ymmm256,
    ),
];

fn encoding(opcode: u8, pp: u8, wide: bool, w: bool, registers: [u8; 3], memory: bool) -> Vec<u8> {
    let [destination, left, right] = registers;
    let copy = opcode < 0x50;
    let store = matches!(opcode, 0x11 | 0x29);
    let (reg, rm) = if store {
        (left, destination)
    } else {
        (destination, if copy { left } else { right })
    };
    vec![
        0xc4,
        0xe1 ^ (u8::from(reg >= 8) << 7) ^ (u8::from(!memory && rm >= 8) << 5),
        (if copy { 0x78 } else { (!left & 15) << 3 })
            | (u8::from(wide) << 2)
            | (u8::from(w) << 7)
            | pp,
        opcode,
        ((reg & 7) << 3) | if memory { 0 } else { 0xc0 | (rm & 7) },
    ]
}
fn policy() -> TranslationPolicy {
    TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
        .with_native_ymm_registers()
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

#[test]
fn thirty_two_register_bit_forms_keep_directions_aliases_and_avx_only_coalescing() {
    let mut chain = Vec::new();
    for &(opcode, pp, narrow, full) in &FORMS {
        for wide in [false, true] {
            for w in [false, true] {
                for registers in [[9, 13, 14], [13, 13, 14], [14, 13, 14]] {
                    let mut code = encoding(opcode, pp, wide, w, registers, false);
                    code.push(0xc3);
                    inspect(&code, policy(), |plan| {
                        assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
                        let emitted = ordinary_output(plan);
                        assert_eq!(emitted.len(), 1);
                        let instruction = &emitted[0];
                        assert_eq!(instruction.code(), if wide { full } else { narrow });
                        let count = if opcode < 0x50 { 2 } else { 3 };
                        assert_eq!(instruction.op_count(), count);
                        let first = if wide { Register::YMM0 } else { Register::XMM0 };
                        for (operand, register) in
                            registers.into_iter().take(count as usize).enumerate()
                        {
                            assert_eq!(instruction.op_kind(operand as u32), OpKind::Register);
                            assert_eq!(
                                instruction.op_register(operand as u32) as u32,
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
            chain.extend(encoding(opcode, pp, wide, false, [9, 13, 14], false));
        }
    }
    chain.push(0xc3);
    inspect(&chain, policy(), |plan| {
        assert_eq!(ordinary_output(plan).len(), 32);
        assert_eq!(plan.instructions().len(), 33);
        assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
    });
}

#[test]
fn bit_register_request_keeps_memory_neighbors_prefixes_and_missing_avx_closed() {
    for &(opcode, pp, _, _) in &FORMS {
        for wide in [false, true] {
            let mut code = encoding(opcode, pp, wide, false, [9, 13, 14], false);
            code.push(0xc3);
            for (selected, reason) in [
                (
                    TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX)),
                    UnsupportedReason::InstructionClass,
                ),
                (
                    TranslationPolicy::x64_sse2().with_native_ymm_registers(),
                    UnsupportedReason::MissingFeature(CpuidFeature::AVX),
                ),
                (
                    TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX2))
                        .with_native_ymm_registers(),
                    UnsupportedReason::MissingFeature(CpuidFeature::AVX),
                ),
            ] {
                inspect(&code, selected, |plan| {
                    assert_eq!(plan.exit().kind(), ExitKind::Mediated { reason })
                });
            }
            let mut memory = encoding(opcode, pp, wide, false, [9, 13, 14], true);
            memory.push(0xc3);
            inspect(
                &memory,
                policy().with_native_packed_float_memory(),
                |plan| {
                    assert_eq!(
                        plan.exit().kind(),
                        ExitKind::Mediated {
                            reason: UnsupportedReason::InstructionClass
                        }
                    );
                    assert_eq!(plan.instructions()[0].emitted_len(), 0);
                    assert_eq!(
                        plan.exit().instruction().unwrap().memory_size().size(),
                        if wide { 32 } else { 16 }
                    );
                },
            );
            code.insert(0, 0x64);
            inspect(&code, policy(), |plan| {
                assert_eq!(
                    plan.exit().kind(),
                    ExitKind::Mediated {
                        reason: UnsupportedReason::SegmentPrefix
                    }
                )
            });
        }
    }
    for mut code in [
        vec![0xc5, 0xf2, 0x10, 0xc2],             // scalar merge MOVSS
        vec![0xc5, 0xfa, 0x12, 0xc2],             // duplicate MOVSLDUP
        vec![0xc5, 0xf4, 0x5e, 0xc2],             // packed divide
        vec![0xc5, 0xf8, 0x77],                   // VZEROUPPER changes other registers
        vec![0x62, 0xf1, 0x7c, 0x48, 0x10, 0xc2], // EVEX
    ] {
        code.push(0xc3);
        inspect(&code, policy(), |plan| {
            assert!(matches!(plan.exit().kind(), ExitKind::Mediated { .. }));
            assert_eq!(plan.bytes(), [0x0f, 0x0b]);
        });
    }
    let invalid = image(&[0xc5, 0xf0, 0x10, 0xc2, 0xc3]);
    let source = link(&[(&invalid, BIAS)]);
    assert!(matches!(
        plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &policy(),
            &BlockLimits::default()
        ),
        Err(CodegenError::InvalidInstruction { .. })
    ));
}
