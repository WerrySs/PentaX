// SPDX-License-Identifier: GPL-2.0-only
//! Original VEX broadcast encodings and exact non-authoritative read metadata.
use super::*;
use iced_x86::{InstructionInfoFactory, OpAccess};

// Opcode, output bytes, element bytes, register input, memory input, AVX-only, Code.
type Form = (u8, usize, usize, bool, bool, bool, Code);
const FORMS: [Form; 16] = [
    (
        0x78,
        16,
        1,
        true,
        true,
        false,
        Code::VEX_Vpbroadcastb_xmm_xmmm8,
    ),
    (
        0x78,
        32,
        1,
        true,
        true,
        false,
        Code::VEX_Vpbroadcastb_ymm_xmmm8,
    ),
    (
        0x79,
        16,
        2,
        true,
        true,
        false,
        Code::VEX_Vpbroadcastw_xmm_xmmm16,
    ),
    (
        0x79,
        32,
        2,
        true,
        true,
        false,
        Code::VEX_Vpbroadcastw_ymm_xmmm16,
    ),
    (
        0x58,
        16,
        4,
        true,
        true,
        false,
        Code::VEX_Vpbroadcastd_xmm_xmmm32,
    ),
    (
        0x58,
        32,
        4,
        true,
        true,
        false,
        Code::VEX_Vpbroadcastd_ymm_xmmm32,
    ),
    (
        0x59,
        16,
        8,
        true,
        true,
        false,
        Code::VEX_Vpbroadcastq_xmm_xmmm64,
    ),
    (
        0x59,
        32,
        8,
        true,
        true,
        false,
        Code::VEX_Vpbroadcastq_ymm_xmmm64,
    ),
    (
        0x18,
        16,
        4,
        false,
        true,
        true,
        Code::VEX_Vbroadcastss_xmm_m32,
    ),
    (
        0x18,
        32,
        4,
        false,
        true,
        true,
        Code::VEX_Vbroadcastss_ymm_m32,
    ),
    (
        0x18,
        16,
        4,
        true,
        false,
        false,
        Code::VEX_Vbroadcastss_xmm_xmm,
    ),
    (
        0x18,
        32,
        4,
        true,
        false,
        false,
        Code::VEX_Vbroadcastss_ymm_xmm,
    ),
    (
        0x19,
        32,
        8,
        false,
        true,
        true,
        Code::VEX_Vbroadcastsd_ymm_m64,
    ),
    (
        0x19,
        32,
        8,
        true,
        false,
        false,
        Code::VEX_Vbroadcastsd_ymm_xmm,
    ),
    (
        0x1a,
        32,
        16,
        false,
        true,
        true,
        Code::VEX_Vbroadcastf128_ymm_m128,
    ),
    (
        0x5a,
        32,
        16,
        false,
        true,
        false,
        Code::VEX_Vbroadcasti128_ymm_m128,
    ),
];
fn features(avx2: bool) -> CpuFeatures {
    let features = CpuFeatures::x64_sse2().with(CpuidFeature::AVX);
    if avx2 {
        features.with(CpuidFeature::AVX2)
    } else {
        features
    }
}
fn policy(avx2: bool) -> TranslationPolicy {
    TranslationPolicy::new(features(avx2))
        .with_native_ymm_registers()
        .with_native_vector_broadcasts()
}
fn encoding(form: Form, [destination, source]: [u8; 2], memory: bool, ea: u8) -> Vec<u8> {
    let mut code = vec![
        0xc4,
        0xe2 ^ (u8::from(destination >= 8) << 7) ^ (u8::from(!memory && source >= 8) << 5),
        0x79 | if form.1 == 32 { 4 } else { 0 },
        form.0,
    ];
    let reg = (destination & 7) << 3;
    if !memory {
        code.push(0xc0 | reg | (source & 7));
    } else {
        match ea {
            0 => code.extend([0x40 | reg, 3]),
            1 => code.extend([0x44 | reg, 0x88, 5]),
            2 => {
                code.push(5 | reg);
                let displacement = (BIAS + DATA) as i64 - (BIAS + ENTRY as u64 + 9) as i64;
                code.extend((displacement as i32).to_le_bytes());
            }
            _ => {
                code.extend([4 | reg, 0x25]);
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
        &Default::default(),
    )
    .unwrap();
    check(&plan);
}
fn refused(code: &[u8], selected: TranslationPolicy, reason: UnsupportedReason) {
    inspect(code, selected, |plan| {
        assert!(ordinary_output(plan).is_empty());
        assert!(!plan.emits_vector_broadcast_memory());
        assert_eq!(plan.exit().kind(), ExitKind::Mediated { reason });
    });
}

#[test]
fn broadcasts_all_sixteen_codes_and_twenty_four_shapes_preserve_exact_reads_and_aliases() {
    let mut factory = InstructionInfoFactory::new();
    let mut cases = 0;
    let mut shapes = 0;
    for form in FORMS {
        for memory in [false, true] {
            if memory && !form.4 || !memory && !form.3 {
                continue;
            }
            shapes += 1;
            for registers in [[1, 2], [9, 13], [13, 13], [15, 15]] {
                for ea in 0..if memory { 4 } else { 1 } {
                    let code = encoding(form, registers, memory, ea);
                    inspect(&code, policy(true), |plan| {
                        assert_eq!(plan.emits_vector_broadcast_memory(), memory);
                        assert_eq!(plan.native_admission().is_memory_free(), !memory);
                        assert!(!plan.emits_non_temporal_store());
                        assert!(!plan.emits_vector_memory_move());
                        let output = ordinary_output(plan);
                        assert_eq!(output.len(), 1);
                        let record = &plan.instructions()[0];
                        let decoded = record.instruction();
                        assert_eq!(decoded.code(), form.6);
                        assert_eq!(output[0].code(), form.6);
                        assert_eq!(decoded.op_count(), 2);
                        assert_eq!(decoded.op0_register().size(), form.1);
                        assert_eq!(decoded.op0_register(), output[0].op0_register());
                        assert_eq!(decoded.rflags_modified(), 0);
                        assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
                        let info = factory.info(decoded);
                        assert_eq!(info.op0_access(), OpAccess::Write);
                        assert_eq!(info.op1_access(), OpAccess::Read);
                        assert_eq!(
                            decoded.cpuid_features(),
                            if form.5 {
                                &[CpuidFeature::AVX][..]
                            } else {
                                &[CpuidFeature::AVX2][..]
                            }
                        );
                        for base in [0x8000_u64, 0x1_0000_8000] {
                            let mut gpr = [0; 16];
                            gpr[0] = base;
                            gpr[1] = 7;
                            let accesses = MemoryAccessAnalyzer::default()
                                .analyze(record, &gpr)
                                .unwrap();
                            assert_eq!(accesses.register_dependencies().written, 0);
                            if memory {
                                assert_eq!(info.used_memory().len(), 1);
                                assert_eq!(info.used_memory()[0].access(), OpAccess::Read);
                                assert_eq!(
                                    accesses.accesses(),
                                    [DeclaredMemoryAccess {
                                        address: match ea {
                                            0 => base + 3,
                                            1 => base + 33,
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
                                    assert_eq!(output[0].ip_rel_memory_address(), BIAS + DATA);
                                }
                            } else {
                                assert!(info.used_memory().is_empty());
                                assert!(accesses.accesses().is_empty());
                                assert_eq!(decoded.op1_register().size(), 16);
                                assert_eq!(decoded.op1_register(), output[0].op1_register());
                                assert_eq!(accesses.register_dependencies().address, 0);
                            }
                        }
                    });
                    let bytes = image(&code);
                    let source = link(&[(&bytes, BIAS)]);
                    let declaration =
                        declare_materialized_source(&source, &[], &Default::default()).unwrap();
                    let plan = plan_materialized_block(
                        &declaration,
                        0,
                        BIAS + ENTRY as u64,
                        &code,
                        placement(),
                        &policy(true),
                        &Default::default(),
                    )
                    .unwrap();
                    assert_eq!(plan.emits_vector_broadcast_memory(), memory);
                    assert_eq!(plan.native_admission().emitted_instructions(), 1);
                    assert_eq!(plan.native_admission().is_memory_free(), !memory);
                    assert_eq!(plan.instructions()[0].instruction().code(), form.6);
                    let emitted =
                        Decoder::with_ip(64, plan.bytes(), CACHE, DecoderOptions::NONE).decode();
                    assert_eq!(emitted.code(), form.6);
                    if memory && ea == 2 {
                        assert_eq!(emitted.ip_rel_memory_address(), BIAS + DATA);
                    }
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(shapes, 24);
    assert_eq!(cases, 252); // 13 memory shapes x4 EAs,11 register shapes,4 alias pairs.
}

#[test]
fn broadcasts_keep_avx_avx2_full_ymm_prefix_and_unrelated_profiles_independent() {
    assert!(!TranslationPolicy::new(features(true)).requests_native_vector_broadcasts());
    assert!(!features(true).contains(CpuidFeature::FMA));
    let mut avx_only = 0;
    for form in FORMS {
        for memory in [false, true] {
            if memory && !form.4 || !memory && !form.3 {
                continue;
            }
            let code = encoding(form, [9, 13], memory, 0);
            for selected in [
                TranslationPolicy::new(features(true)),
                TranslationPolicy::new(features(true)).with_native_vector_broadcasts(),
                TranslationPolicy::new(features(true))
                    .with_native_ymm_registers()
                    .with_native_vector_moves()
                    .with_native_float_min_max()
                    .with_native_fma_registers()
                    .with_native_packed_float_memory()
                    .with_native_scalar_float()
                    .with_native_scalar_fma()
                    .with_native_streaming_stores(),
            ] {
                refused(&code, selected, UnsupportedReason::InstructionClass);
            }
            for selected in [
                CpuFeatures::x64_sse2(),
                CpuFeatures::x64_sse2().with(CpuidFeature::AVX2),
            ] {
                refused(
                    &code,
                    TranslationPolicy::new(selected)
                        .with_native_ymm_registers()
                        .with_native_vector_broadcasts(),
                    UnsupportedReason::MissingFeature(CpuidFeature::AVX),
                );
            }
            if form.5 {
                inspect(&code, policy(false), |plan| {
                    assert_eq!(ordinary_output(plan).len(), 1)
                });
                avx_only += 1;
            } else {
                refused(
                    &code,
                    policy(false),
                    UnsupportedReason::MissingFeature(CpuidFeature::AVX2),
                );
            }
            for segment in [0x2e, 0x64, 0x65] {
                let mut code = code.clone();
                code.insert(0, segment);
                refused(&code, policy(true), UnsupportedReason::SegmentPrefix);
            }
            for prefix in [0xf0, 0xf2, 0xf3] {
                let mut code = code.clone();
                code.insert(0, prefix);
                let bytes = image(&code);
                let source = link(&[(&bytes, BIAS)]);
                match plan_block(
                    &source,
                    0,
                    BIAS + ENTRY as u64,
                    placement(),
                    &policy(true),
                    &Default::default(),
                ) {
                    Err(CodegenError::InvalidInstruction { .. }) => {}
                    Ok(plan) => {
                        assert!(ordinary_output(&plan).is_empty());
                        assert!(!plan.emits_vector_broadcast_memory());
                        assert_eq!(
                            plan.exit().kind(),
                            ExitKind::Mediated {
                                reason: UnsupportedReason::RepeatOrLockPrefix
                            }
                        );
                    }
                    other => panic!("unexpected broadcast prefix admission: {other:?}"),
                }
            }
            // Reserved vvvv and W1 encodings are decoder refusals, never a
            // broader register/source grant derived from the mnemonic.
            for mask in [8, 128] {
                let mut code = code.clone();
                code[2] ^= mask;
                let bytes = image(&code);
                let source = link(&[(&bytes, BIAS)]);
                assert!(matches!(
                    plan_block(
                        &source,
                        0,
                        BIAS + ENTRY as u64,
                        placement(),
                        &policy(true),
                        &Default::default()
                    ),
                    Err(CodegenError::InvalidInstruction { .. })
                ));
            }
        }
        if form.4 {
            for ea in 0..4 {
                let mut code = encoding(form, [9, 13], true, ea);
                code.insert(0, 0x67);
                refused(&code, policy(true), UnsupportedReason::InstructionClass);
            }
        }
    }
    assert_eq!(avx_only, 4);
    for code in [
        vec![0xc4, 0xe2, 0x79, 0x1a, 0xc1, 0xc3], // F128 requires YMM+memory.
        vec![0xc4, 0xe2, 0x79, 0x19, 0x00, 0xc3], // No SD-to-XMM Code.
        vec![0xc4, 0xe2, 0x7d, 0x5a, 0xc1, 0xc3], // I128 requires memory.
        vec![0x62, 0xf2, 0x7d, 0x08, 0x78, 0xc1, 0xc3], // EVEX broadcast.
        vec![0xc4, 0xe2, 0x79, 0x00, 0xc1, 0xc3], // Unrelated VPSHUFB.
    ] {
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        match plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &policy(true),
            &Default::default(),
        ) {
            Err(CodegenError::InvalidInstruction { .. }) => {}
            Ok(plan) => {
                assert!(ordinary_output(&plan).is_empty());
                assert!(!plan.emits_vector_broadcast_memory());
            }
            other => panic!("unexpected non-broadcast admission: {other:?}"),
        }
    }
}

#[test]
fn broadcast_dependency_splits_and_output_caps_set_metadata_only_after_emission() {
    for form in FORMS.into_iter().filter(|form| form.4) {
        let read = encoding(form, [9, 13], true, 1);
        let mut code = vec![0xb1, 1]; // CL write invalidates the later current SIB address.
        code.extend(&read);
        inspect(&code, policy(true).with_independent_memory(), |plan| {
            assert_eq!(ordinary_output(plan).len(), 1);
            assert!(!plan.emits_vector_broadcast_memory());
            assert_eq!(
                plan.exit().kind(),
                ExitKind::Fallthrough {
                    target: BIAS + ENTRY as u64 + 2
                }
            );
        });
        let mut code = read[..read.len() - 1].to_vec();
        code.extend([0xb0, 1]);
        code.extend(&read);
        inspect(&code, policy(true).with_independent_memory(), |plan| {
            assert_eq!(ordinary_output(plan).len(), 2);
            assert!(plan.emits_vector_broadcast_memory());
            assert_eq!(
                plan.exit().guest_pc(),
                BIAS + ENTRY as u64 + (read.len() + 1) as u64
            );
        });
        let mut code = vec![0x90];
        code.extend(&read);
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        for limits in [
            BlockLimits {
                max_instructions: 1,
                ..Default::default()
            },
            BlockLimits {
                max_output_bytes: 3,
                ..Default::default()
            },
        ] {
            let plan = plan_block(
                &source,
                0,
                BIAS + ENTRY as u64,
                placement(),
                &policy(true),
                &limits,
            )
            .unwrap();
            assert_eq!(ordinary_output(&plan).len(), 1);
            assert!(!plan.emits_vector_broadcast_memory());
            assert_eq!(plan.exit().guest_pc(), BIAS + ENTRY as u64 + 1);
        }
    }
    let mut code = vec![0x41, 0xb8, 17, 0, 0, 0];
    for form in FORMS {
        let memory = !form.3;
        let next = encoding(form, [9, 13], memory, 0);
        code.extend(&next[..next.len() - 1]);
    }
    code.extend([0x41, 0x83, 0xc0, 1, 0xc3]);
    inspect(&code, policy(true).with_independent_memory(), |plan| {
        assert_eq!(ordinary_output(plan).len(), 18);
        assert_eq!(plan.native_admission().emitted_instructions(), 18);
        assert!(plan.emits_vector_broadcast_memory());
        assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
    });
}
