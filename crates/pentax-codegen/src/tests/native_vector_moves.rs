// SPDX-License-Identifier: GPL-2.0-only
//! Original exact temporal-memory encodings; no detached pointer authority.
use super::*;

struct Form {
    opcode: u8,
    pp: u8,
    scalar: usize,
    narrow: Code,
    wide: Code,
}
const FORMS: [Form; 16] = [
    Form {
        opcode: 0x6f,
        pp: 1,
        scalar: 0,
        narrow: Code::VEX_Vmovdqa_xmm_xmmm128,
        wide: Code::VEX_Vmovdqa_ymm_ymmm256,
    },
    Form {
        opcode: 0x7f,
        pp: 1,
        scalar: 0,
        narrow: Code::VEX_Vmovdqa_xmmm128_xmm,
        wide: Code::VEX_Vmovdqa_ymmm256_ymm,
    },
    Form {
        opcode: 0x6f,
        pp: 2,
        scalar: 0,
        narrow: Code::VEX_Vmovdqu_xmm_xmmm128,
        wide: Code::VEX_Vmovdqu_ymm_ymmm256,
    },
    Form {
        opcode: 0x7f,
        pp: 2,
        scalar: 0,
        narrow: Code::VEX_Vmovdqu_xmmm128_xmm,
        wide: Code::VEX_Vmovdqu_ymmm256_ymm,
    },
    Form {
        opcode: 0x28,
        pp: 0,
        scalar: 0,
        narrow: Code::VEX_Vmovaps_xmm_xmmm128,
        wide: Code::VEX_Vmovaps_ymm_ymmm256,
    },
    Form {
        opcode: 0x29,
        pp: 0,
        scalar: 0,
        narrow: Code::VEX_Vmovaps_xmmm128_xmm,
        wide: Code::VEX_Vmovaps_ymmm256_ymm,
    },
    Form {
        opcode: 0x10,
        pp: 0,
        scalar: 0,
        narrow: Code::VEX_Vmovups_xmm_xmmm128,
        wide: Code::VEX_Vmovups_ymm_ymmm256,
    },
    Form {
        opcode: 0x11,
        pp: 0,
        scalar: 0,
        narrow: Code::VEX_Vmovups_xmmm128_xmm,
        wide: Code::VEX_Vmovups_ymmm256_ymm,
    },
    Form {
        opcode: 0x28,
        pp: 1,
        scalar: 0,
        narrow: Code::VEX_Vmovapd_xmm_xmmm128,
        wide: Code::VEX_Vmovapd_ymm_ymmm256,
    },
    Form {
        opcode: 0x29,
        pp: 1,
        scalar: 0,
        narrow: Code::VEX_Vmovapd_xmmm128_xmm,
        wide: Code::VEX_Vmovapd_ymmm256_ymm,
    },
    Form {
        opcode: 0x10,
        pp: 1,
        scalar: 0,
        narrow: Code::VEX_Vmovupd_xmm_xmmm128,
        wide: Code::VEX_Vmovupd_ymm_ymmm256,
    },
    Form {
        opcode: 0x11,
        pp: 1,
        scalar: 0,
        narrow: Code::VEX_Vmovupd_xmmm128_xmm,
        wide: Code::VEX_Vmovupd_ymmm256_ymm,
    },
    Form {
        opcode: 0x10,
        pp: 2,
        scalar: 4,
        narrow: Code::VEX_Vmovss_xmm_m32,
        wide: Code::VEX_Vmovss_xmm_m32,
    },
    Form {
        opcode: 0x11,
        pp: 2,
        scalar: 4,
        narrow: Code::VEX_Vmovss_m32_xmm,
        wide: Code::VEX_Vmovss_m32_xmm,
    },
    Form {
        opcode: 0x10,
        pp: 3,
        scalar: 8,
        narrow: Code::VEX_Vmovsd_xmm_m64,
        wide: Code::VEX_Vmovsd_xmm_m64,
    },
    Form {
        opcode: 0x11,
        pp: 3,
        scalar: 8,
        narrow: Code::VEX_Vmovsd_m64_xmm,
        wide: Code::VEX_Vmovsd_m64_xmm,
    },
];
impl Form {
    fn store(&self) -> bool {
        matches!(self.opcode, 0x7f | 0x29 | 0x11)
    }
    fn width(&self, wide: bool) -> usize {
        if self.scalar != 0 {
            self.scalar
        } else if wide {
            32
        } else {
            16
        }
    }
    fn aligned(&self) -> bool {
        matches!(self.opcode, 0x28 | 0x29) || self.opcode >= 0x6f && self.pp == 1
    }
}
fn policy() -> TranslationPolicy {
    TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
        .with_native_ymm_registers()
        .with_native_vector_moves()
}
fn encoding(form: &Form, wide: bool, w: bool, register: u8, ea: u8) -> Vec<u8> {
    let mut code = vec![
        0xc4,
        0xe1 ^ (u8::from(register >= 8) << 7),
        0x78 | form.pp | (u8::from(wide) << 2) | (u8::from(w) << 7),
        form.opcode,
    ];
    let reg = (register & 7) << 3;
    match ea {
        0 => code.extend([0x40 | reg, 32]),
        1 => code.extend([0x44 | reg, 0x88, 0]),
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
fn twenty_eight_temporal_forms_keep_exact_directions_widths_avx_only_and_live_addresses() {
    let mut codes = std::collections::HashSet::new();
    for form in &FORMS {
        for wide in [false, true] {
            for w in [false, true] {
                for register in [0, 9, 15] {
                    for ea in 0..4 {
                        let code = encoding(form, wide, w, register, ea);
                        inspect(&code, policy(), |plan| {
                            assert!(plan.policy().requests_native_vector_moves());
                            assert!(plan.emits_vector_memory_move());
                            assert!(!plan.emits_non_temporal_store());
                            assert_eq!(ordinary_output(plan).len(), 1);
                            assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
                            let decoded = plan.instructions()[0].instruction();
                            codes.insert(decoded.code());
                            if form.scalar != 0 {
                                // Iced's default LIG encoding is canonical L=0, including an
                                // input with L=1. Canonical scalar encoding also avoids Intel's
                                // MOVSS L=1 warning at the actual native bridge.
                                let emitted = plan.bytes();
                                let length_bit = match emitted[0] {
                                    0xc5 => emitted[1] & 4,
                                    0xc4 => emitted[2] & 4,
                                    other => panic!("unexpected scalar VEX prefix: {other:#x}"),
                                };
                                assert_eq!(length_bit, 0);
                            }
                            assert_eq!(decoded.code(), if wide { form.wide } else { form.narrow });
                            assert_eq!(decoded.op_count(), 2);
                            assert_eq!(decoded.op_kind(u32::from(!form.store())), OpKind::Memory);
                            assert_eq!(
                                decoded.op_register(u32::from(form.store())) as u32,
                                (if form.width(wide) == 32 {
                                    Register::YMM0
                                } else {
                                    Register::XMM0
                                }) as u32
                                    + u32::from(register)
                            );
                            for base in [0x8000u64, 0x1_0000_8000] {
                                let mut gpr = [0; 16];
                                gpr[0] = base;
                                gpr[1] = 8;
                                let result = MemoryAccessAnalyzer::default()
                                    .analyze(&plan.instructions()[0], &gpr)
                                    .unwrap();
                                assert_eq!(
                                    result.accesses(),
                                    [DeclaredMemoryAccess {
                                        address: match ea {
                                            0 | 1 => base + 32,
                                            2 => BIAS + DATA,
                                            _ => 0x8000,
                                        },
                                        size: form.width(wide) as u64,
                                        kind: if form.store() {
                                            MemoryAccessKind::Write
                                        } else {
                                            MemoryAccessKind::Read
                                        }
                                    }]
                                );
                                assert_eq!(
                                    result.register_dependencies().address,
                                    match ea {
                                        0 => 1,
                                        1 => 3,
                                        _ => 0,
                                    }
                                );
                                assert_eq!(result.register_dependencies().written, 0);
                            }
                            if ea == 2 {
                                assert_eq!(
                                    ordinary_output(plan)[0].ip_rel_memory_address(),
                                    BIAS + DATA
                                );
                            }
                            if !form.aligned() && ea == 0 {
                                let mut gpr = [0; 16];
                                gpr[0] = 0x8003;
                                assert_eq!(
                                    MemoryAccessAnalyzer::default()
                                        .analyze(&plan.instructions()[0], &gpr)
                                        .unwrap()
                                        .accesses()[0]
                                        .address,
                                    0x8023
                                );
                            }
                        });
                    }
                }
            }
        }
    }
    assert_eq!(codes.len(), 28);
    let code = encoding(&FORMS[12], false, false, 15, 0);
    let bytes = image(&code);
    let source = link(&[(&bytes, BIAS)]);
    let declaration = declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
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
    assert!(plan.emits_vector_memory_move());
    assert!(!plan.emits_non_temporal_store());
}

#[test]
fn temporal_profile_keeps_separate_flags_features_ea_prefix_and_other_families_closed() {
    for form in &FORMS {
        for wide in [false, true] {
            let code = encoding(form, wide, false, 9, 0);
            for selected in [
                TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX)),
                TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
                    .with_native_ymm_registers(),
                TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
                    .with_native_vector_moves(),
                TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
                    .with_native_ymm_registers()
                    .with_native_packed_float_memory()
                    .with_native_streaming_stores(),
            ] {
                inspect(&code, selected, |plan| {
                    assert_eq!(
                        plan.exit().kind(),
                        ExitKind::Mediated {
                            reason: UnsupportedReason::InstructionClass
                        }
                    );
                    assert!(!plan.emits_vector_memory_move());
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
                        .with_native_vector_moves(),
                    |plan| {
                        assert_eq!(
                            plan.exit().kind(),
                            ExitKind::Mediated {
                                reason: UnsupportedReason::MissingFeature(CpuidFeature::AVX)
                            }
                        )
                    },
                );
            }
            for ea in 0..4 {
                let mut addr32 = encoding(form, wide, false, 9, ea);
                addr32.insert(0, 0x67);
                inspect(&addr32, policy(), |plan| {
                    assert_eq!(plan.instructions()[0].emitted_len(), 0);
                    assert!(!plan.emits_vector_memory_move());
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
        }
    }
    for code in [
        vec![0xc5, 0xfd, 0xe7, 0x08, 0xc3], // NT is a separate profile
        vec![0xc5, 0xfc, 0x2b, 0x08, 0xc3], // floating NT remains unavailable
        vec![0xc5, 0xf4, 0x58, 0x08, 0xc3], // arithmetic read is separate
        vec![0xc5, 0xf4, 0x54, 0x08, 0xc3], // logical memory is not a move
        vec![0xc5, 0xf8, 0x12, 0x08, 0xc3], // low-half/merge transfer
        vec![0xc5, 0xfa, 0x12, 0x08, 0xc3], // duplicate transfer
        vec![0xc5, 0xf2, 0x10, 0xc2, 0xc3], // scalar register merge
        vec![0xc5, 0xf9, 0x6e, 0xc0, 0xc3], // GPR transfer
        vec![0x62, 0xf1, 0x7c, 0x48, 0x10, 0x08, 0xc3], // EVEX
    ] {
        inspect(&code, policy(), |plan| {
            assert_eq!(plan.instructions()[0].emitted_len(), 0);
            assert!(!plan.emits_vector_memory_move());
        });
    }
    for prefix in [0xf0, 0xf2, 0xf3] {
        let mut code = encoding(&FORMS[0], true, false, 9, 0);
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
            Ok(plan) => assert_eq!(
                plan.exit().kind(),
                ExitKind::Mediated {
                    reason: UnsupportedReason::RepeatOrLockPrefix
                }
            ),
            other => panic!("unexpected prefix admission: {other:?}"),
        }
    }
    let mut invalid = encoding(&FORMS[0], true, false, 9, 0);
    invalid[2] &= !8; // nonreserved vvvv
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
    // A fabricated record is not executable authority; the size evaluator must
    // still refuse an unrelated vector logic32 read and NT-float32 write.
    for code in [[0xc5, 0xf4, 0x54, 0x08], [0xc5, 0xfc, 0x2b, 0x08]] {
        let instruction = Decoder::with_ip(64, &code, BIAS, DecoderOptions::NONE).decode();
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
fn temporal_alignment_dependencies_and_emitted_only_metadata_include_legacy_corrections() {
    for form in &FORMS {
        for wide in [false, true] {
            let code = encoding(form, wide, false, 9, 0);
            if form.aligned() {
                inspect(&code, policy(), |plan| {
                    for offset in 0..form.width(wide) {
                        let mut gpr = [0; 16];
                        gpr[0] = 0x8000 + offset as u64;
                        let result =
                            MemoryAccessAnalyzer::default().analyze(&plan.instructions()[0], &gpr);
                        if offset == 0 {
                            assert_eq!(result.unwrap().accesses()[0].size, form.width(wide) as u64);
                        } else {
                            assert_eq!(result, Err(MemoryAccessError::UnalignedVector));
                        }
                    }
                });
            }
            let mut sequence = code[..code.len() - 1].to_vec();
            sequence.extend([0xb0, 1]);
            sequence.extend(&code);
            inspect(&sequence, policy().with_independent_memory(), |plan| {
                assert_eq!(ordinary_output(plan).len(), 2);
                assert_eq!(
                    plan.exit().kind(),
                    ExitKind::Fallthrough {
                        target: BIAS + ENTRY as u64 + (code.len() + 1) as u64
                    }
                );
                assert!(plan.emits_vector_memory_move());
            });
            inspect(&code, policy().with_isolated_memory(), |plan| {
                assert_eq!(ordinary_output(plan).len(), 1);
                assert_eq!(
                    plan.exit().guest_pc(),
                    BIAS + ENTRY as u64 + (code.len() - 1) as u64
                );
                assert!(plan.emits_vector_memory_move());
            });
            let mut prior = vec![0xb0, 1];
            prior.extend(&code);
            inspect(&prior, policy().with_independent_memory(), |plan| {
                assert_eq!(ordinary_output(plan).len(), 1);
                assert!(!plan.emits_vector_memory_move());
            });
        }
    }
    for (opcode, pp) in [
        (0x6f, 0x66),
        (0x7f, 0x66),
        (0x28, 0),
        (0x29, 0),
        (0x28, 0x66),
        (0x29, 0x66),
    ] {
        let mut code = if pp == 0 { Vec::new() } else { vec![pp] };
        code.extend([0x0f, opcode, 0x08, 0xc3]);
        inspect(&code, TranslationPolicy::x64_sse2(), |plan| {
            assert!(plan.emits_vector_memory_move());
            assert!(!plan.emits_non_temporal_store());
            for offset in 0..16 {
                let mut gpr = [0; 16];
                gpr[0] = 0x8000 + offset;
                let result = MemoryAccessAnalyzer::default().analyze(&plan.instructions()[0], &gpr);
                if offset == 0 {
                    assert_eq!(result.unwrap().accesses()[0].size, 16);
                } else {
                    assert_eq!(result, Err(MemoryAccessError::UnalignedVector));
                }
            }
        });
    }
    for code in [
        vec![0x0f, 0x28, 0xc1, 0xc3],
        vec![0xc5, 0xfc, 0x10, 0xc1, 0xc3],
        vec![0x0f, 0x10, 0x08, 0xc3],
    ] {
        inspect(&code, policy(), |plan| {
            assert_eq!(ordinary_output(plan).len(), 1);
            assert!(!plan.emits_vector_memory_move());
        });
    }
    let mut code = vec![0x90];
    code.extend(encoding(&FORMS[0], true, false, 9, 0));
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
    assert!(!plan.emits_vector_memory_move());
}
