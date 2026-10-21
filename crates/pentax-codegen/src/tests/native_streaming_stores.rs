// SPDX-License-Identifier: GPL-2.0-only
//! Original exact-store shapes, current numeric footprints and immutable fences.
use super::*;

fn policy() -> TranslationPolicy {
    TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
        .with_native_ymm_registers()
        .with_native_streaming_stores()
}
fn encoding(wide: bool, w: bool, source: u8, ea: u8) -> Vec<u8> {
    let mut code = vec![
        0xc4,
        0xe1 ^ (u8::from(source >= 8) << 7),
        0x79 | (u8::from(wide) << 2) | (u8::from(w) << 7),
        0xe7,
    ];
    let reg = (source & 7) << 3;
    match ea {
        0 => code.extend([0x40 | reg, 32]),      // [RAX+32]
        1 => code.extend([0x44 | reg, 0x88, 0]), // [RAX+RCX*4]
        2 => {
            code.push(0x05 | reg);
            code.extend(
                ((BIAS + DATA) as i64 - (BIAS + ENTRY as u64 + 9) as i64).to_le_bytes()[..4]
                    .iter()
                    .copied(),
            );
        }
        _ => {
            code.extend([0x04 | reg, 0x25]);
            code.extend(0x8000u32.to_le_bytes());
        } // absolute64 SIB
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
fn two_avx_only_streaming_stores_keep_source_sib_rip_and_exact_write_footprints() {
    for wide in [false, true] {
        for w in [false, true] {
            for source_register in [0, 9, 15] {
                for ea in 0..4 {
                    let code = encoding(wide, w, source_register, ea);
                    inspect(&code, policy(), |plan| {
                        assert!(plan.policy().requests_native_streaming_stores());
                        assert!(plan.emits_non_temporal_store());
                        assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
                        let decoded = plan.instructions()[0].instruction();
                        assert_eq!(
                            decoded.code(),
                            if wide {
                                Code::VEX_Vmovntdq_m256_ymm
                            } else {
                                Code::VEX_Vmovntdq_m128_xmm
                            }
                        );
                        assert_eq!(decoded.op_count(), 2);
                        assert_eq!(decoded.op0_kind(), OpKind::Memory);
                        assert_eq!(
                            decoded.op1_register() as u32,
                            (if wide { Register::YMM0 } else { Register::XMM0 }) as u32
                                + u32::from(source_register)
                        );
                        assert_eq!(ordinary_output(plan).len(), 1);
                        for base in [0x8000u64, 0x1_0000_8000] {
                            let mut gpr = [0; 16];
                            gpr[0] = base;
                            gpr[1] = 8;
                            let accesses = MemoryAccessAnalyzer::default()
                                .analyze(&plan.instructions()[0], &gpr)
                                .unwrap();
                            assert_eq!(
                                accesses.accesses(),
                                [DeclaredMemoryAccess {
                                    address: match ea {
                                        0 | 1 => base + 32,
                                        2 => BIAS + DATA,
                                        _ => 0x8000,
                                    },
                                    size: if wide { 32 } else { 16 },
                                    kind: MemoryAccessKind::Write
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
                            assert_eq!(accesses.register_dependencies().written, 0);
                        }
                        if ea == 2 {
                            assert_eq!(
                                ordinary_output(plan)[0].ip_rel_memory_address(),
                                BIAS + DATA
                            );
                        }
                    });
                    // The caller-declared wrapper forwards the retained body's predicate,
                    // without granting execution or reconstructing it from detached bytes.
                    let bytes = image(&code);
                    let source = link(&[(&bytes, BIAS)]);
                    let declaration =
                        declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
                    let materialized = plan_materialized_block(
                        &declaration,
                        0,
                        BIAS + ENTRY as u64,
                        &code,
                        placement(),
                        &policy(),
                        &BlockLimits::default(),
                    )
                    .unwrap();
                    assert!(materialized.emits_non_temporal_store());
                }
            }
        }
    }
}

#[test]
fn streaming_request_keeps_other_memory_features_addresses_and_reserved_forms_closed() {
    for wide in [false, true] {
        let code = encoding(wide, false, 9, 0);
        for selected in [
            TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX)),
            TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
                .with_native_ymm_registers(),
            TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
                .with_native_streaming_stores(),
            TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
                .with_native_ymm_registers()
                .with_native_packed_float_memory(),
        ] {
            inspect(&code, selected, |plan| {
                assert_eq!(
                    plan.exit().kind(),
                    ExitKind::Mediated {
                        reason: UnsupportedReason::InstructionClass
                    }
                );
                assert!(!plan.emits_non_temporal_store());
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
                    .with_native_streaming_stores(),
                |plan| {
                    assert_eq!(
                        plan.exit().kind(),
                        ExitKind::Mediated {
                            reason: UnsupportedReason::MissingFeature(CpuidFeature::AVX)
                        }
                    );
                    assert!(!plan.emits_non_temporal_store());
                },
            );
        }
        for ea in 0..4 {
            let mut addr32 = encoding(wide, false, 9, ea);
            addr32.insert(0, 0x67);
            inspect(&addr32, policy(), |plan| {
                assert_eq!(plan.instructions()[0].emitted_len(), 0);
                assert!(!plan.emits_non_temporal_store());
            });
        }
        let mut segmented = code.clone();
        segmented.insert(0, 0x64);
        inspect(&segmented, policy(), |plan| {
            assert_eq!(
                plan.exit().kind(),
                ExitKind::Mediated {
                    reason: UnsupportedReason::SegmentPrefix
                }
            )
        });
        for prefix in [0xf0, 0xf2, 0xf3] {
            let mut prefixed = code.clone();
            prefixed.insert(0, prefix);
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
                    assert_eq!(
                        plan.exit().kind(),
                        ExitKind::Mediated {
                            reason: UnsupportedReason::RepeatOrLockPrefix
                        }
                    );
                    assert!(!plan.emits_non_temporal_store());
                }
                other => panic!("unexpected prefix admission: {other:?}"),
            }
        }
    }
    // Neighbor NT floating stores, temporal writes, scalar NT writes, EVEX and
    // arithmetic reads are not authorized by this separate two-form request.
    for code in [
        vec![0xc5, 0xfc, 0x2b, 0x08, 0xc3],
        vec![0xc5, 0xfd, 0x2b, 0x08, 0xc3],
        vec![0xc5, 0xfc, 0x11, 0x08, 0xc3],
        vec![0xc5, 0xfd, 0x7f, 0x08, 0xc3],
        vec![0x0f, 0xc3, 0x08, 0xc3],
        vec![0xc5, 0xf4, 0x58, 0x08, 0xc3],
        vec![0x62, 0xf1, 0x7d, 0x48, 0xe7, 0x08, 0xc3],
    ] {
        inspect(&code, policy(), |plan| {
            assert_eq!(plan.instructions()[0].emitted_len(), 0);
            assert!(!plan.emits_non_temporal_store());
        });
    }
    for invalid in [
        [0xc5, 0xf5, 0xe7, 0x08, 0xc3], // reserved vvvv cannot name another source
        [0xc5, 0xfd, 0xe7, 0xc9, 0xc3], // memory-only opcode cannot select register destination
    ] {
        let bytes = image(&invalid);
        let source = link(&[(&bytes, BIAS)]);
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
    // Fabricated metadata cannot turn the size exception into unrelated NT
    // floating writes. Temporal moves have a separate exact footprint profile.
    for raw in [[0xc5, 0xfc, 0x2b, 0x08], [0xc5, 0xfd, 0x2b, 0x08]] {
        let instruction = Decoder::with_ip(64, &raw, BIAS, DecoderOptions::NONE).decode();
        let mut analyzer = MemoryAccessAnalyzer::default();
        let template = analyzer.prepare(&instruction);
        let record = InstructionRecord {
            guest_pc: BIAS,
            source_offset: 0,
            source_len: instruction.len(),
            cache_offset: 0,
            emitted_len: instruction.len(),
            instruction,
            rip_target: None,
            memory_template: Some(template),
        };
        assert_eq!(
            analyzer.analyze(&record, &[0; 16]),
            Err(MemoryAccessError::UnsupportedSize)
        );
    }
}

#[test]
fn streaming_stores_keep_alignment_dependencies_and_emitted_only_fence_metadata() {
    for wide in [false, true] {
        let store = encoding(wide, false, 9, 0);
        inspect(&store, policy(), |plan| {
            let mut analyzer = MemoryAccessAnalyzer::default();
            let alignment = if wide { 32 } else { 16 };
            for offset in 0..alignment {
                let mut registers = [0; 16];
                registers[0] = 0x8000 + offset;
                if offset == 0 {
                    assert_eq!(
                        analyzer
                            .analyze(&plan.instructions()[0], &registers)
                            .unwrap()
                            .accesses()[0]
                            .size,
                        alignment
                    );
                } else {
                    assert_eq!(
                        analyzer.analyze(&plan.instructions()[0], &registers),
                        Err(MemoryAccessError::UnalignedNonTemporal)
                    );
                }
            }
        });
        let mut code = store[..store.len() - 1].to_vec();
        code.extend([0xb0, 1]);
        code.extend(&store);
        let split = BIAS + ENTRY as u64 + (store.len() + 1) as u64;
        inspect(&code, policy().with_independent_memory(), |plan| {
            assert_eq!(ordinary_output(plan).len(), 2);
            assert_eq!(plan.exit().kind(), ExitKind::Fallthrough { target: split });
            assert!(plan.emits_non_temporal_store());
        });
        inspect(&store, policy().with_isolated_memory(), |plan| {
            assert_eq!(ordinary_output(plan).len(), 1);
            assert_eq!(
                plan.exit().guest_pc(),
                BIAS + ENTRY as u64 + (store.len() - 1) as u64
            );
            assert!(plan.emits_non_temporal_store());
        });
        let mut prior = vec![0xb0, 1];
        prior.extend(&store);
        inspect(&prior, policy().with_independent_memory(), |plan| {
            assert_eq!(ordinary_output(plan).len(), 1);
            assert!(!plan.emits_non_temporal_store());
        });
        // A decoded store that cannot fit the output is not an emitted body.
        let mut bounded = vec![0x90];
        bounded.extend(&store);
        let bytes = image(&bounded);
        let source = link(&[(&bytes, BIAS)]);
        let limits = BlockLimits {
            max_output_bytes: 3,
            ..Default::default()
        };
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
        assert!(!plan.emits_non_temporal_store());
    }
    for code in [
        vec![0x66, 0x0f, 0xe7, 0x08, 0xc3],
        vec![0x0f, 0x2b, 0x08, 0xc3],
        vec![0x66, 0x0f, 0x2b, 0x08, 0xc3],
    ] {
        inspect(&code, TranslationPolicy::x64_sse2(), |plan| {
            assert_eq!(ordinary_output(plan).len(), 1);
            assert!(plan.emits_non_temporal_store());
            for offset in 0..16 {
                let mut gpr = [0; 16];
                gpr[0] = 0x8000 + offset;
                let result = MemoryAccessAnalyzer::default().analyze(&plan.instructions()[0], &gpr);
                if offset == 0 {
                    assert_eq!(
                        result.unwrap().accesses()[0],
                        DeclaredMemoryAccess {
                            address: 0x8000,
                            size: 16,
                            kind: MemoryAccessKind::Write
                        }
                    );
                } else {
                    assert_eq!(result, Err(MemoryAccessError::UnalignedNonTemporal));
                }
            }
        });
    }
    inspect(&[0x90, 0xc3], policy(), |plan| {
        assert!(!plan.emits_non_temporal_store())
    });
}
