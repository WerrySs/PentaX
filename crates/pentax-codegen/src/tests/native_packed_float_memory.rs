// SPDX-License-Identifier: GPL-2.0-only
//! An explicit read request retains decoded footprints, not live authority.
use super::*;

fn policy() -> TranslationPolicy {
    TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
        .with_native_ymm_registers()
        .with_native_packed_float_memory()
}
fn encoding(opcode: u8, double: bool, wide: bool, w: bool, ea: u8) -> Vec<u8> {
    // Destination9, logical source13; VEX.W is ignored by these exact forms.
    let mut code = vec![
        0xc4,
        0x61,
        0x10 | u8::from(double) | (u8::from(wide) << 2) | (u8::from(w) << 7),
        opcode,
    ];
    match ea {
        0 => code.extend([0x48, 3]),       //unaligned [RAX+3]
        1 => code.extend([0x4c, 0x88, 5]), //[RAX+RCX*4+5]
        2 => {
            code.push(0x0d);
            code.extend(
                ((BIAS + DATA) as i64 - (BIAS + ENTRY as u64 + 9) as i64).to_le_bytes()[..4]
                    .iter()
                    .copied(),
            );
        }
        _ => {
            code.extend([0x0c, 0x25]);
            code.extend(0x8003_u32.to_le_bytes());
        } //absolute64 SIB
    }
    code.push(0xc3);
    code
}
fn inspect(code: &[u8], requested: TranslationPolicy, check: impl FnOnce(&BlockPlan<'_, '_>)) {
    let bytes = image(code);
    let source = link(&[(&bytes, BIAS)]);
    let plan = plan_block(
        &source,
        0,
        BIAS + ENTRY as u64,
        placement(),
        &requested,
        &BlockLimits::default(),
    )
    .unwrap();
    check(&plan);
}

#[test]
fn twelve_memory_forms_keep_exact_read_width_sib_rip_and_live_numeric_addresses() {
    for opcode in [0x58, 0x5c, 0x59] {
        for double in [false, true] {
            for wide in [false, true] {
                for w in [false, true] {
                    for ea in 0..4 {
                        let code = encoding(opcode, double, wide, w, ea);
                        inspect(&code, policy(), |plan| {
                            assert_eq!(ordinary_output(plan).len(), 1);
                            assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
                            let decoded = plan.instructions()[0].instruction();
                            assert_eq!(decoded.op_count(), 3);
                            assert_eq!(
                                decoded.op0_register(),
                                if wide { Register::YMM9 } else { Register::XMM9 }
                            );
                            assert_eq!(
                                decoded.op1_register(),
                                if wide {
                                    Register::YMM13
                                } else {
                                    Register::XMM13
                                }
                            );
                            assert_eq!(decoded.op2_kind(), OpKind::Memory);
                            let mut analyzer = MemoryAccessAnalyzer::default();
                            for base in [0x8000_u64, 0x1_0000_8000] {
                                let mut gpr = [0; 16];
                                gpr[0] = base;
                                gpr[1] = 7;
                                let actual =
                                    analyzer.analyze(&plan.instructions()[0], &gpr).unwrap();
                                let expected = match ea {
                                    0 => base + 3,
                                    1 => base + 33,
                                    2 => BIAS + DATA,
                                    _ => 0x8003,
                                };
                                assert_eq!(
                                    actual.accesses(),
                                    [DeclaredMemoryAccess {
                                        address: expected,
                                        size: if wide { 32 } else { 16 },
                                        kind: MemoryAccessKind::Read
                                    }]
                                );
                                assert_eq!(
                                    actual.register_dependencies().address,
                                    match ea {
                                        0 => 1,
                                        1 => 3,
                                        _ => 0,
                                    }
                                );
                                assert_eq!(actual.register_dependencies().written, 0);
                            }
                            if ea == 2 {
                                assert_eq!(
                                    ordinary_output(plan)[0].ip_rel_memory_address(),
                                    BIAS + DATA
                                );
                            }
                        });
                    }
                }
            }
        }
    }
}

#[test]
fn memory_request_is_separate_and_does_not_expand_address_write_or_other_vector_authority() {
    for opcode in [0x58, 0x5c, 0x59] {
        for double in [false, true] {
            for wide in [false, true] {
                let code = encoding(opcode, double, wide, false, 0);
                for requested in [
                    TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
                        .with_native_ymm_registers(),
                    TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
                        .with_native_packed_float_memory(),
                    TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX)),
                ] {
                    inspect(&code, requested, |plan| {
                        assert_eq!(
                            plan.exit().kind(),
                            ExitKind::Mediated {
                                reason: UnsupportedReason::InstructionClass
                            }
                        )
                    });
                }
                inspect(
                    &code,
                    TranslationPolicy::x64_sse2()
                        .with_native_ymm_registers()
                        .with_native_packed_float_memory(),
                    |plan| {
                        assert_eq!(
                            plan.exit().kind(),
                            ExitKind::Mediated {
                                reason: UnsupportedReason::MissingFeature(CpuidFeature::AVX)
                            }
                        );
                    },
                );
                for ea in [0, 1, 2, 3] {
                    let mut addr32 = encoding(opcode, double, wide, false, ea);
                    addr32.insert(0, 0x67);
                    inspect(&addr32, policy(), |plan| {
                        assert_eq!(
                            plan.exit().kind(),
                            ExitKind::Mediated {
                                reason: UnsupportedReason::InstructionClass
                            }
                        )
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
    }
    for code in [
        vec![0xc5, 0xfc, 0x10, 0x00, 0xc3],             //VMOVUPS load32
        vec![0xc5, 0xfc, 0x11, 0x00, 0xc3],             //VMOVUPS write32
        vec![0xc5, 0xf4, 0x5e, 0x00, 0xc3],             //VDIVPS
        vec![0xc5, 0xf4, 0x5d, 0x00, 0xc3],             //VMINPS
        vec![0xc5, 0xf4, 0x5f, 0x00, 0xc3],             //VMAXPS
        vec![0xc5, 0xfc, 0x51, 0x00, 0xc3],             //VSQRTPS
        vec![0xc5, 0xf2, 0x58, 0x00, 0xc3],             //VADDSS
        vec![0xc4, 0xe2, 0x75, 0x98, 0x00, 0xc3],       //FMA
        vec![0x62, 0xf1, 0x74, 0x48, 0x58, 0x00, 0xc3], //EVEX
    ] {
        inspect(&code, policy(), |plan| {
            assert_eq!(plan.instructions()[0].emitted_len(), 0)
        });
    }
    for prefix in [0xf0, 0xf2, 0xf3] {
        let mut code = encoding(0x58, false, true, false, 0);
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
            other => panic!("unexpected original prefix admission: {other:?}"),
        }
    }
    // Deliberately fabricated metadata is NOT an execution plan. It proves the
    // evaluator's32-byte exception cannot become a generic load/store ceiling.
    for code in [[0xc5, 0xfc, 0x10, 0x00], [0xc5, 0xfc, 0x11, 0x00]] {
        let decoded = Decoder::with_ip(64, &code, BIAS, DecoderOptions::NONE).decode();
        let mut analyzer = MemoryAccessAnalyzer::default();
        let template = analyzer.prepare(&decoded);
        let record = InstructionRecord {
            guest_pc: BIAS,
            source_offset: 0,
            source_len: decoded.len(),
            cache_offset: 0,
            emitted_len: decoded.len(),
            instruction: decoded,
            rip_target: None,
            memory_template: Some(template),
        };
        assert_eq!(
            analyzer.analyze(&record, &[0; 16]).unwrap_err(),
            MemoryAccessError::UnsupportedSize
        );
    }
}

#[test]
fn newly_native_reads_keep_address_dependencies_and_existing_isolation_splits() {
    for wide in [false, true] {
        let read = encoding(0x58, false, wide, false, 1);
        // First read and subsequent partial RAX write can share PRE context;
        // the second read must wait for the resulting address observation.
        let mut code = read[..read.len() - 1].to_vec();
        code.extend([0xb0, 1]);
        code.extend_from_slice(&read);
        let split = BIAS + ENTRY as u64 + (read.len() - 1 + 2) as u64;
        inspect(&code, policy().with_independent_memory(), |plan| {
            assert_eq!(ordinary_output(plan).len(), 2);
            assert_eq!(plan.exit().kind(), ExitKind::Fallthrough { target: split });
        });
        let mut prior_index_write = vec![0xb1, 1];
        prior_index_write.extend_from_slice(&read);
        inspect(
            &prior_index_write,
            policy().with_independent_memory(),
            |plan| {
                assert_eq!(ordinary_output(plan).len(), 1);
                assert_eq!(plan.exit().guest_pc(), BIAS + ENTRY as u64 + 2);
            },
        );
        inspect(&read, policy().with_isolated_memory(), |plan| {
            assert_eq!(ordinary_output(plan).len(), 1);
            assert_eq!(
                plan.exit().kind(),
                ExitKind::Fallthrough {
                    target: BIAS + ENTRY as u64 + (read.len() - 1) as u64
                }
            );
        });
    }
}
