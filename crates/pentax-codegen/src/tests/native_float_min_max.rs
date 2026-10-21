// SPDX-License-Identifier: GPL-2.0-only
//! Original MIN/MAX encodings retain ordered operands and exact read footprints.
use super::*;
use iced_x86::{InstructionInfoFactory, OpAccess};

// Opcode, mandatory prefix, memory bytes, packed256, scalar, exact Code.
type Form = (u8, u8, usize, bool, bool, Code);
const FORMS: [Form; 12] = [
    (0x5d, 0, 16, false, false, Code::VEX_Vminps_xmm_xmm_xmmm128),
    (0x5d, 0, 32, true, false, Code::VEX_Vminps_ymm_ymm_ymmm256),
    (0x5f, 0, 16, false, false, Code::VEX_Vmaxps_xmm_xmm_xmmm128),
    (0x5f, 0, 32, true, false, Code::VEX_Vmaxps_ymm_ymm_ymmm256),
    (0x5d, 1, 16, false, false, Code::VEX_Vminpd_xmm_xmm_xmmm128),
    (0x5d, 1, 32, true, false, Code::VEX_Vminpd_ymm_ymm_ymmm256),
    (0x5f, 1, 16, false, false, Code::VEX_Vmaxpd_xmm_xmm_xmmm128),
    (0x5f, 1, 32, true, false, Code::VEX_Vmaxpd_ymm_ymm_ymmm256),
    (0x5d, 2, 4, false, true, Code::VEX_Vminss_xmm_xmm_xmmm32),
    (0x5f, 2, 4, false, true, Code::VEX_Vmaxss_xmm_xmm_xmmm32),
    (0x5d, 3, 8, false, true, Code::VEX_Vminsd_xmm_xmm_xmmm64),
    (0x5f, 3, 8, false, true, Code::VEX_Vmaxsd_xmm_xmm_xmmm64),
];

fn features() -> CpuFeatures {
    CpuFeatures::x64_sse2().with(CpuidFeature::AVX)
}
fn policy() -> TranslationPolicy {
    TranslationPolicy::new(features())
        .with_native_ymm_registers()
        .with_native_float_min_max()
}
fn encoding(
    form: Form,
    [destination, left, right]: [u8; 3],
    memory: bool,
    ea: u8,
    w: bool,
    scalar_l: bool,
) -> Vec<u8> {
    let mut code = vec![
        0xc4,
        0xe1 ^ (u8::from(destination >= 8) << 7) ^ (u8::from(!memory && right >= 8) << 5),
        ((!left & 15) << 3) | form.1 | (u8::from(w) << 7) | (u8::from(form.3 || scalar_l) << 2),
        form.0,
    ];
    let reg = (destination & 7) << 3;
    if !memory {
        code.push(0xc0 | reg | (right & 7));
    } else {
        match ea {
            0 => code.extend([0x40 | reg, 3]),       // Unaligned [RAX+3].
            1 => code.extend([0x44 | reg, 0x88, 5]), // [RAX+RCX*4+5].
            2 => {
                code.push(0x05 | reg);
                let displacement = (BIAS + DATA) as i64 - (BIAS + ENTRY as u64 + 9) as i64;
                code.extend((displacement as i32).to_le_bytes());
            }
            _ => {
                code.extend([0x04 | reg, 0x25]); // Displacement-only address64 SIB.
                code.extend(0x8003_u32.to_le_bytes());
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
fn no_emission(code: &[u8], selected: TranslationPolicy, reason: UnsupportedReason) {
    inspect(code, selected, |plan| {
        assert!(ordinary_output(plan).is_empty());
        assert!(!plan.emits_float_min_max_memory());
        assert_eq!(plan.exit().kind(), ExitKind::Mediated { reason });
    });
}

#[test]
fn min_max_all_shapes_keep_aliases_canonical_scalar_l0_and_exact_source_bound_reads() {
    assert!(!TranslationPolicy::new(features()).requests_native_float_min_max());
    assert!(!features().contains(CpuidFeature::FMA));
    assert!(!features().contains(CpuidFeature::AVX2));
    let mut factory = InstructionInfoFactory::new();
    let mut cases = 0;
    for form in FORMS {
        for registers in [[9, 13, 14], [13, 13, 14], [14, 13, 14], [15, 15, 15]] {
            for memory in [false, true] {
                for ea in 0..if memory { 4 } else { 1 } {
                    for w in [false, true] {
                        for scalar_l in [false, true] {
                            if scalar_l && !form.4 {
                                continue;
                            }
                            let code = encoding(form, registers, memory, ea, w, scalar_l);
                            inspect(&code, policy(), |plan| {
                                assert!(plan.policy().requests_native_float_min_max());
                                assert_eq!(plan.emits_float_min_max_memory(), memory);
                                assert!(!plan.emits_scalar_float_memory());
                                assert!(!plan.emits_non_temporal_store());
                                assert!(!plan.emits_vector_memory_move());
                                assert_eq!(plan.native_admission().is_memory_free(), !memory);
                                let output = ordinary_output(plan);
                                assert_eq!(output.len(), 1);
                                assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
                                let record = &plan.instructions()[0];
                                let decoded = record.instruction();
                                assert_eq!(decoded.code(), form.5);
                                assert_eq!(output[0].code(), form.5);
                                assert_eq!(decoded.op_count(), 3);
                                assert_eq!(decoded.rflags_modified(), 0);
                                let base = if form.3 {
                                    Register::YMM0
                                } else {
                                    Register::XMM0
                                };
                                for operand in 0..if memory { 2 } else { 3 } {
                                    assert_eq!(decoded.op_kind(operand), OpKind::Register);
                                    assert_eq!(
                                        decoded.op_register(operand) as u32,
                                        base as u32 + u32::from(registers[operand as usize])
                                    );
                                    assert_eq!(
                                        output[0].op_register(operand),
                                        decoded.op_register(operand)
                                    );
                                }
                                let info = factory.info(decoded);
                                assert_eq!(info.op0_access(), OpAccess::Write);
                                assert_eq!(info.op1_access(), OpAccess::Read);
                                assert_eq!(info.op2_access(), OpAccess::Read);
                                let emitted_l = match plan.bytes()[0] {
                                    0xc5 => plan.bytes()[1] & 4,
                                    0xc4 => plan.bytes()[2] & 4,
                                    _ => panic!("unexpected MIN/MAX prefix"),
                                };
                                assert_eq!(emitted_l, if form.3 { 4 } else { 0 });
                                for address_base in [0x8000_u64, 0x1_0000_8000] {
                                    let mut gpr = [0; 16];
                                    gpr[0] = address_base;
                                    gpr[1] = 7;
                                    let accesses = MemoryAccessAnalyzer::default()
                                        .analyze(record, &gpr)
                                        .unwrap();
                                    assert_eq!(accesses.register_dependencies().written, 0);
                                    if memory {
                                        assert_eq!(decoded.op2_kind(), OpKind::Memory);
                                        assert_eq!(info.used_memory().len(), 1);
                                        assert_eq!(info.used_memory()[0].access(), OpAccess::Read);
                                        assert_eq!(
                                            accesses.accesses(),
                                            [DeclaredMemoryAccess {
                                                address: match ea {
                                                    0 => address_base + 3,
                                                    1 => address_base + 33,
                                                    2 => BIAS + DATA,
                                                    _ => 0x8003,
                                                },
                                                size: form.2 as u64,
                                                kind: MemoryAccessKind::Read
                                            }]
                                        );
                                        assert_eq!(
                                            accesses.register_dependencies().address,
                                            match ea {
                                                0 => 1,
                                                1 => 3,
                                                _ => 0,
                                            }
                                        );
                                        if ea == 2 {
                                            assert_eq!(
                                                output[0].ip_rel_memory_address(),
                                                BIAS + DATA
                                            );
                                        }
                                    } else {
                                        assert!(info.used_memory().is_empty());
                                        assert!(accesses.accesses().is_empty());
                                        assert_eq!(accesses.register_dependencies().address, 0);
                                    }
                                }
                            });
                            let bytes = image(&code);
                            let source = link(&[(&bytes, BIAS)]);
                            let declaration =
                                declare_materialized_source(&source, &[], &BlockLimits::default())
                                    .unwrap();
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
                            assert_eq!(plan.instructions()[0].instruction().code(), form.5);
                            assert_ne!(plan.instructions()[0].emitted_len(), 0);
                            assert_eq!(plan.emits_float_min_max_memory(), memory);
                            assert_eq!(plan.native_admission().is_memory_free(), !memory);
                            assert_eq!(plan.native_admission().emitted_instructions(), 1);
                            assert!(!plan.emits_scalar_float_memory());
                            assert!(!plan.emits_non_temporal_store());
                            assert!(!plan.emits_vector_memory_move());
                            let mut output =
                                Decoder::with_ip(64, plan.bytes(), CACHE, DecoderOptions::NONE);
                            let emitted = output.decode();
                            assert_eq!(emitted.code(), form.5);
                            if memory && ea == 2 {
                                assert_eq!(emitted.ip_rel_memory_address(), BIAS + DATA);
                            }
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 640); // All12Codes, both shapes, aliases, WIG/scalarLIG and four EAs.
}

#[test]
fn min_max_keeps_default_profiles_features_prefixes_and_unrelated_families_closed() {
    for form in FORMS {
        for memory in [false, true] {
            let code = encoding(form, [9, 13, 14], memory, 0, false, false);
            for selected in [
                TranslationPolicy::new(features()),
                TranslationPolicy::new(features()).with_native_float_min_max(),
                TranslationPolicy::new(features())
                    .with_native_ymm_registers()
                    .with_native_scalar_float()
                    .with_native_packed_float_memory()
                    .with_native_vector_moves()
                    .with_native_streaming_stores()
                    .with_native_scalar_fma()
                    .with_native_fma_registers(),
            ] {
                no_emission(&code, selected, UnsupportedReason::InstructionClass);
            }
            for missing_avx in [
                CpuFeatures::x64_sse2(),
                CpuFeatures::x64_sse2()
                    .with(CpuidFeature::AVX2)
                    .with(CpuidFeature::FMA),
            ] {
                no_emission(
                    &code,
                    TranslationPolicy::new(missing_avx)
                        .with_native_ymm_registers()
                        .with_native_float_min_max(),
                    UnsupportedReason::MissingFeature(CpuidFeature::AVX),
                );
            }
            for segment in [0x64, 0x65, 0x2e] {
                let mut prefixed = vec![segment];
                prefixed.extend(&code);
                no_emission(&prefixed, policy(), UnsupportedReason::SegmentPrefix);
            }
            for prefix in [0xf0, 0xf2, 0xf3] {
                let mut prefixed = vec![prefix];
                prefixed.extend(&code);
                let bytes = image(&prefixed);
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
                        assert!(ordinary_output(&plan).is_empty());
                        assert!(!plan.emits_float_min_max_memory());
                        assert_eq!(
                            plan.exit().kind(),
                            ExitKind::Mediated {
                                reason: UnsupportedReason::RepeatOrLockPrefix
                            }
                        );
                    }
                    other => panic!("unexpected MIN/MAX prefix admission: {other:?}"),
                }
            }
        }
        for ea in 0..4 {
            let mut addr32 = encoding(form, [9, 13, 14], true, ea, false, false);
            addr32.insert(0, 0x67);
            no_emission(&addr32, policy(), UnsupportedReason::InstructionClass);
        }
    }
    for excluded in [
        vec![0xc5, 0xf2, 0x58, 0xc2, 0xc3], // Scalar ADD request is independent.
        vec![0xc5, 0xf1, 0x5e, 0xc2, 0xc3], // VDIVPD.
        vec![0xc5, 0xf2, 0x51, 0xc2, 0xc3], // VSQRTSS.
        vec![0xc4, 0xe2, 0x71, 0xa9, 0xc2, 0xc3], // FMA request is independent.
        vec![0x62, 0xf1, 0x74, 0x08, 0x5d, 0xc2, 0xc3], // EVEX MINPS.
    ] {
        no_emission(&excluded, policy(), UnsupportedReason::InstructionClass);
    }
    // The independent request does not alter the already admitted legacy API.
    for legacy in [
        vec![0x0f, 0x5d, 0xc1, 0xc3],
        vec![0xf2, 0x0f, 0x5f, 0xc1, 0xc3],
    ] {
        for selected in [TranslationPolicy::x64_sse2(), policy()] {
            inspect(&legacy, selected, |plan| {
                assert_eq!(ordinary_output(plan).len(), 1);
                assert!(!plan.emits_float_min_max_memory());
                assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
            });
        }
    }
}

#[test]
fn min_max_preserves_dependency_splits_and_only_actual_bounded_emission_sets_memory_metadata() {
    for form in FORMS {
        let read = encoding(form, [9, 13, 14], true, 1, false, false);
        let register = encoding(form, [9, 13, 14], false, 0, false, false);
        let mut sequence = read[..read.len() - 1].to_vec();
        sequence.extend([0xb0, 1]); // AL write changes a later RAX address.
        sequence.extend(&read);
        inspect(&sequence, policy().with_independent_memory(), |plan| {
            assert_eq!(ordinary_output(plan).len(), 2);
            assert!(plan.emits_float_min_max_memory());
            assert_eq!(
                plan.exit().kind(),
                ExitKind::Fallthrough {
                    target: BIAS + ENTRY as u64 + (read.len() + 1) as u64
                }
            );
        });
        let mut before_read = register[..register.len() - 1].to_vec();
        before_read.extend([0xb1, 1]); // CL write changes the SIB index.
        before_read.extend(&read);
        inspect(&before_read, policy().with_independent_memory(), |plan| {
            assert_eq!(ordinary_output(plan).len(), 2);
            assert!(!plan.emits_float_min_max_memory());
            assert_eq!(
                plan.exit().guest_pc(),
                BIAS + ENTRY as u64 + (register.len() + 1) as u64
            );
        });
        inspect(&read, policy().with_isolated_memory(), |plan| {
            assert_eq!(ordinary_output(plan).len(), 1);
            assert!(plan.emits_float_min_max_memory());
            assert_eq!(
                plan.exit().guest_pc(),
                BIAS + ENTRY as u64 + (read.len() - 1) as u64
            );
        });
        let mut code = vec![0x90];
        code.extend(&read);
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        for limits in [
            BlockLimits {
                max_output_bytes: 3,
                ..Default::default()
            },
            BlockLimits {
                max_instructions: 1,
                ..Default::default()
            },
        ] {
            let plan = plan_block(
                &source,
                0,
                BIAS + ENTRY as u64,
                placement(),
                &policy(),
                &limits,
            )
            .unwrap();
            assert_eq!(ordinary_output(&plan).len(), 1);
            assert!(!plan.emits_float_min_max_memory());
            assert_eq!(plan.exit().guest_pc(), BIAS + ENTRY as u64 + 1);
        }
        let bytes = image(&read);
        let source = link(&[(&bytes, BIAS)]);
        assert!(matches!(
            plan_block(
                &source,
                0,
                BIAS + ENTRY as u64,
                placement(),
                &policy(),
                &BlockLimits {
                    max_output_bytes: 2,
                    ..Default::default()
                }
            ),
            Err(CodegenError::Budget {
                kind: Budget::OutputBytes,
                limit: 2
            })
        ));
    }
    let mut coalesced = vec![0x41, 0xb8, 17, 0, 0, 0];
    for form in FORMS {
        let code = encoding(form, [9, 13, 14], false, 0, false, false);
        coalesced.extend(&code[..code.len() - 1]);
    }
    coalesced.extend([0x41, 0x83, 0xc0, 1, 0xc3]);
    inspect(&coalesced, policy().with_independent_memory(), |plan| {
        assert_eq!(ordinary_output(plan).len(), 14);
        assert_eq!(plan.native_admission().emitted_instructions(), 14);
        assert!(plan.native_admission().is_memory_free());
        assert!(!plan.emits_float_min_max_memory());
        assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
    });
}
