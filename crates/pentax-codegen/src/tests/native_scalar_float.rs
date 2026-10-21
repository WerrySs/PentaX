// SPDX-License-Identifier: GPL-2.0-only
//! Original scalar encodings retain full logical sources and exact read spans.
use super::*;

const FORMS: [(u8, u8, usize, Code); 6] = [
    (0x58, 2, 4, Code::VEX_Vaddss_xmm_xmm_xmmm32),
    (0x58, 3, 8, Code::VEX_Vaddsd_xmm_xmm_xmmm64),
    (0x5c, 2, 4, Code::VEX_Vsubss_xmm_xmm_xmmm32),
    (0x5c, 3, 8, Code::VEX_Vsubsd_xmm_xmm_xmmm64),
    (0x59, 2, 4, Code::VEX_Vmulss_xmm_xmm_xmmm32),
    (0x59, 3, 8, Code::VEX_Vmulsd_xmm_xmm_xmmm64),
];

fn policy() -> TranslationPolicy {
    TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
        .with_native_ymm_registers()
        .with_native_scalar_float()
}

// The memory flag selects exact reads; ea covers base, SIB, RIP and absolute64 SIB.
fn encoding(
    form: &(u8, u8, usize, Code),
    registers: [u8; 3],
    memory: bool,
    ea: u8,
    w: bool,
    l: bool,
) -> Vec<u8> {
    let [destination, left, right] = registers;
    let mut code = vec![
        0xc4,
        0xe1 ^ (u8::from(destination >= 8) << 7) ^ (u8::from(!memory && right >= 8) << 5),
        ((!left & 15) << 3) | form.1 | (u8::from(w) << 7) | (u8::from(l) << 2),
        form.0,
    ];
    let reg = (destination & 7) << 3;
    if !memory {
        code.push(0xc0 | reg | (right & 7));
    } else {
        match ea {
            0 => code.extend([0x40 | reg, 3]),
            1 => code.extend([0x44 | reg, 0x88, 5]),
            2 => {
                code.push(0x05 | reg);
                let displacement = (BIAS + DATA) as i64 - (BIAS + ENTRY as u64 + 9) as i64;
                code.extend((displacement as i32).to_le_bytes());
            }
            _ => {
                code.extend([0x04 | reg, 0x25]);
                code.extend(0x8003u32.to_le_bytes());
            }
        }
    }
    code.push(0xc3);
    code
}

fn inspect(code: &[u8], selected: TranslationPolicy, check: impl FnOnce(&BlockPlan<'_, '_>)) {
    let bytes = image(code);
    let source = link(&[(&bytes, BIAS)]);
    let plan = plan_block(
        &source,
        0,
        BIAS + ENTRY as u64,
        placement(),
        &selected,
        &BlockLimits::default(),
    )
    .unwrap();
    check(&plan);
}

#[test]
fn scalar_forms_keep_order_aliases_canonical_l0_and_exact_live_read_widths() {
    let mut codes = std::collections::HashSet::new();
    for form in &FORMS {
        for registers in [
            [9, 13, 14],
            [13, 13, 14],
            [14, 13, 14],
            [0, 0, 15],
            [15, 15, 0],
        ] {
            for memory in [false, true] {
                for ea in 0..if memory { 4 } else { 1 } {
                    for w in [false, true] {
                        for l in [false, true] {
                            let code = encoding(form, registers, memory, ea, w, l);
                            inspect(&code, policy(), |plan| {
                                assert!(plan.policy().requests_native_scalar_float());
                                assert_eq!(plan.emits_scalar_float_memory(), memory);
                                assert!(!plan.emits_non_temporal_store());
                                assert!(!plan.emits_vector_memory_move());
                                let emitted = ordinary_output(plan);
                                assert_eq!(emitted.len(), 1);
                                assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
                                let decoded = plan.instructions()[0].instruction();
                                codes.insert(decoded.code());
                                assert_eq!(decoded.code(), form.3);
                                assert_eq!(decoded.op_count(), 3);
                                for operand in 0..2 {
                                    assert_eq!(decoded.op_kind(operand), OpKind::Register);
                                    assert_eq!(
                                        decoded.op_register(operand) as u32,
                                        Register::XMM0 as u32
                                            + u32::from(registers[operand as usize])
                                    );
                                    assert_eq!(
                                        emitted[0].op_register(operand),
                                        decoded.op_register(operand)
                                    );
                                }
                                // The pinned encoder canonicalizes scalar LIG to L=0.
                                // InputL=1 must never reach hardware unchanged.
                                let emitted_l = match plan.bytes()[0] {
                                    0xc5 => plan.bytes()[1] & 4,
                                    0xc4 => plan.bytes()[2] & 4,
                                    other => panic!("unexpected scalar prefix: {other:#x}"),
                                };
                                assert_eq!(emitted_l, 0);
                                for base in [0x8000u64, 0x1_0000_8000] {
                                    let mut gpr = [0; 16];
                                    gpr[0] = base;
                                    gpr[1] = 7;
                                    let access = MemoryAccessAnalyzer::default()
                                        .analyze(&plan.instructions()[0], &gpr)
                                        .unwrap();
                                    if memory {
                                        assert_eq!(decoded.op2_kind(), OpKind::Memory);
                                        assert_eq!(
                                            access.accesses(),
                                            [DeclaredMemoryAccess {
                                                address: match ea {
                                                    0 => base + 3,
                                                    1 => base + 33,
                                                    2 => BIAS + DATA,
                                                    _ => 0x8003,
                                                },
                                                size: form.2 as u64,
                                                kind: MemoryAccessKind::Read,
                                            }]
                                        );
                                        assert_eq!(
                                            access.register_dependencies().address,
                                            match ea {
                                                0 => 1,
                                                1 => 3,
                                                _ => 0,
                                            }
                                        );
                                        if ea == 2 {
                                            assert_eq!(
                                                emitted[0].ip_rel_memory_address(),
                                                BIAS + DATA
                                            );
                                        }
                                    } else {
                                        assert_eq!(decoded.op2_kind(), OpKind::Register);
                                        assert_eq!(
                                            decoded.op2_register() as u32,
                                            Register::XMM0 as u32 + u32::from(registers[2])
                                        );
                                        assert_eq!(
                                            emitted[0].op2_register(),
                                            decoded.op2_register()
                                        );
                                        assert!(access.accesses().is_empty());
                                        assert_eq!(access.register_dependencies().address, 0);
                                    }
                                    assert_eq!(access.register_dependencies().written, 0);
                                }
                            });
                        }
                    }
                }
            }
        }
    }
    assert_eq!(codes.len(), 6);
    for memory in [false, true] {
        let code = encoding(&FORMS[0], [9, 13, 14], memory, 0, false, false);
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
            &policy(),
            &BlockLimits::default(),
        )
        .unwrap();
        assert_eq!(plan.emits_scalar_float_memory(), memory);
        assert!(!plan.emits_non_temporal_store());
        assert!(!plan.emits_vector_memory_move());
    }
}

#[test]
fn scalar_request_keeps_old_profiles_features_ea_and_other_arithmetic_closed() {
    for form in &FORMS {
        for memory in [false, true] {
            let code = encoding(form, [9, 13, 14], memory, 0, false, false);
            for selected in [
                TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX)),
                TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
                    .with_native_scalar_float(),
                TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
                    .with_native_ymm_registers()
                    .with_native_packed_float_memory()
                    .with_native_vector_moves()
                    .with_native_streaming_stores(),
            ] {
                inspect(&code, selected, |plan| {
                    assert_eq!(
                        plan.exit().kind(),
                        ExitKind::Mediated {
                            reason: UnsupportedReason::InstructionClass,
                        }
                    );
                    assert!(!plan.emits_scalar_float_memory());
                });
            }
            for features in [
                CpuFeatures::x64_sse2(),
                CpuFeatures::x64_sse2().with(CpuidFeature::AVX2),
            ] {
                inspect(
                    &code,
                    TranslationPolicy::new(features)
                        .with_native_ymm_registers()
                        .with_native_scalar_float(),
                    |plan| {
                        assert_eq!(
                            plan.exit().kind(),
                            ExitKind::Mediated {
                                reason: UnsupportedReason::MissingFeature(CpuidFeature::AVX),
                            }
                        );
                        assert!(!plan.emits_scalar_float_memory());
                    },
                );
            }
            if memory {
                for ea in 0..4 {
                    let mut addr32 = encoding(form, [9, 13, 14], true, ea, false, false);
                    addr32.insert(0, 0x67);
                    inspect(&addr32, policy(), |plan| {
                        assert_eq!(plan.instructions()[0].emitted_len(), 0);
                        assert!(!plan.emits_scalar_float_memory());
                    });
                }
            }
            for segment in [0x64, 0x65, 0x2e] {
                let mut segmented = code.clone();
                segmented.insert(0, segment);
                inspect(&segmented, policy(), |plan| {
                    assert_eq!(
                        plan.exit().kind(),
                        ExitKind::Mediated {
                            reason: UnsupportedReason::SegmentPrefix,
                        }
                    );
                    assert!(!plan.emits_scalar_float_memory());
                });
            }
        }
    }
    for code in [
        vec![0xc5, 0xf2, 0x5e, 0xc2, 0xc3],             // VDIVSS
        vec![0xc5, 0xf3, 0x5e, 0x00, 0xc3],             // VDIVSD memory
        vec![0xc5, 0xf2, 0x5d, 0xc2, 0xc3],             // VMINSS
        vec![0xc5, 0xf3, 0x5f, 0x00, 0xc3],             // VMAXSD memory
        vec![0xc5, 0xf2, 0x51, 0xc2, 0xc3],             // VSQRTSS
        vec![0xc4, 0xe2, 0x71, 0x99, 0xc2, 0xc3],       // scalar FMA
        vec![0xc5, 0xf2, 0x10, 0xc2, 0xc3],             // scalar register merge move
        vec![0xc5, 0xfa, 0x11, 0x08, 0xc3],             // scalar store
        vec![0xc5, 0xfa, 0x2a, 0xc0, 0xc3],             // integer conversion
        vec![0x62, 0xf1, 0x76, 0x08, 0x58, 0xc2, 0xc3], // EVEX scalar
    ] {
        inspect(&code, policy(), |plan| {
            assert_eq!(plan.instructions()[0].emitted_len(), 0);
            assert!(!plan.emits_scalar_float_memory());
        });
    }
    for prefix in [0xf0, 0xf2, 0xf3] {
        let mut code = encoding(&FORMS[0], [9, 13, 14], true, 0, false, false);
        code.insert(0, prefix);
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        match plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &policy(),
            &BlockLimits::default(),
        ) {
            Err(CodegenError::InvalidInstruction { .. }) => {}
            Ok(plan) => {
                assert_eq!(
                    plan.exit().kind(),
                    ExitKind::Mediated {
                        reason: UnsupportedReason::RepeatOrLockPrefix,
                    }
                );
                assert!(!plan.emits_scalar_float_memory());
            }
            other => panic!("unexpected scalar prefix admission: {other:?}"),
        }
    }
}

#[test]
fn scalar_reads_keep_dependency_splits_and_only_actual_emission_sets_memory_metadata() {
    for form in &FORMS {
        let read = encoding(form, [9, 13, 14], true, 1, false, false);
        let mut sequence = read[..read.len() - 1].to_vec();
        sequence.extend([0xb0, 1]);
        sequence.extend(&read);
        inspect(&sequence, policy().with_independent_memory(), |plan| {
            assert_eq!(ordinary_output(plan).len(), 2);
            assert_eq!(
                plan.exit().kind(),
                ExitKind::Fallthrough {
                    target: BIAS + ENTRY as u64 + (read.len() + 1) as u64,
                }
            );
            assert!(plan.emits_scalar_float_memory());
        });
        let register = encoding(form, [9, 13, 14], false, 0, false, false);
        let mut before_read = register[..register.len() - 1].to_vec();
        before_read.extend([0xb1, 1]);
        before_read.extend(&read);
        inspect(&before_read, policy().with_independent_memory(), |plan| {
            assert_eq!(ordinary_output(plan).len(), 2);
            assert!(!plan.emits_scalar_float_memory());
            assert_eq!(
                plan.exit().guest_pc(),
                BIAS + ENTRY as u64 + (register.len() + 1) as u64
            );
        });
        inspect(&read, policy().with_isolated_memory(), |plan| {
            assert_eq!(ordinary_output(plan).len(), 1);
            assert_eq!(
                plan.exit().guest_pc(),
                BIAS + ENTRY as u64 + (read.len() - 1) as u64
            );
            assert!(plan.emits_scalar_float_memory());
        });
        let mut code = vec![0x90];
        code.extend(&read);
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &policy(),
            &BlockLimits {
                max_output_bytes: 3,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ordinary_output(&plan).len(), 1);
        assert!(!plan.emits_scalar_float_memory());
    }
}
