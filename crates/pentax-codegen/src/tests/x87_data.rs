// SPDX-License-Identifier: GPL-2.0-only
//! Original x87 transfer encodings retain exact operands without native authority.
use super::*;

#[test]
fn x87_data_forms_retain_exact_widths_stack_indices_and_original_source_under_mediation() {
    let forms = [
        (vec![0xd9, 0x00], Code::Fld_m32fp, 1, 4),
        (vec![0xdd, 0x00], Code::Fld_m64fp, 1, 8),
        (vec![0xdb, 0x28], Code::Fld_m80fp, 1, 10),
        (vec![0xd9, 0xc7], Code::Fld_sti, 1, 0),
        (vec![0xdd, 0xd7], Code::Fst_sti, 1, 0),
        (vec![0xdd, 0xdf], Code::Fstp_sti, 1, 0),
        (vec![0xdb, 0x38], Code::Fstp_m80fp, 1, 10),
        (vec![0xd9, 0xcf], Code::Fxch_st0_sti, 2, 0),
        (vec![0xdd, 0xc7], Code::Ffree_sti, 1, 0),
        (vec![0xd9, 0xf7], Code::Fincstp, 0, 0),
        (vec![0xd9, 0xf6], Code::Fdecstp, 0, 0),
        (vec![0xd9, 0xe8], Code::Fld1, 0, 0),
        (vec![0xd9, 0xee], Code::Fldz, 0, 0),
        (vec![0xd9, 0xe0], Code::Fchs, 0, 0),
        (vec![0xd9, 0xe1], Code::Fabs, 0, 0),
    ];
    for (mut code, expected, operands, memory_width) in forms {
        let length = code.len();
        code.push(0xc3);
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        for features in [
            CpuFeatures::x64_sse2(),
            CpuFeatures::x64_sse2()
                .with(CpuidFeature::FPU)
                .with(CpuidFeature::AVX)
                .with(CpuidFeature::AVX2),
        ] {
            let plan = plan_block(
                &source,
                0,
                BIAS + ENTRY as u64,
                placement(),
                &TranslationPolicy::new(features)
                    .with_native_ymm_registers()
                    .with_independent_memory(),
                &BlockLimits::default(),
            )
            .unwrap();
            assert_eq!(
                plan.exit().kind(),
                ExitKind::Mediated {
                    reason: UnsupportedReason::InstructionClass
                }
            );
            assert_eq!(plan.bytes(), [0x0f, 0x0b]);
            assert_eq!(plan.instructions().len(), 1);
            let record = &plan.instructions()[0];
            let instruction = record.instruction();
            assert_eq!(
                (instruction.code(), instruction.op_count()),
                (expected, operands)
            );
            assert_eq!((record.source_len(), record.emitted_len()), (length, 0));
            assert_eq!(
                plan.exit().next_guest_pc(),
                BIAS + ENTRY as u64 + length as u64
            );
            if memory_width != 0 {
                assert_eq!(instruction.op0_kind(), OpKind::Memory);
                assert_eq!(instruction.memory_size().size(), memory_width);
            } else if operands != 0 {
                assert_eq!(instruction.op_register(operands - 1), Register::ST7);
            }
            assert_eq!(
                MemoryAccessAnalyzer::default().analyze(record, &[0; 16]),
                Err(MemoryAccessError::NotEmitted)
            );
        }
    }
    // RIP-relative data and a harmless operand-size prefix stay tied to original bytes.
    let code = [0x66, 0xdd, 0x05, 0x20, 0, 0, 0, 0xc3];
    let bytes = image(&code);
    let source = link(&[(&bytes, BIAS)]);
    let plan = make(&source);
    assert_eq!(plan.instructions()[0].instruction().code(), Code::Fld_m64fp);
    assert_eq!(plan.instructions()[0].source_len(), 7);
    assert_eq!(
        plan.instructions()[0].rip_relative_target(),
        Some(BIAS + ENTRY as u64 + 7 + 32)
    );
}

#[test]
fn x87_data_does_not_promote_narrow_stores_arithmetic_or_accept_repeat_lock_segment_prefixes() {
    for code in [
        vec![0xd9, 0x10, 0xc3],
        vec![0xdd, 0x10, 0xc3],
        vec![0xd9, 0x18, 0xc3],
        vec![0xdd, 0x18, 0xc3],
        vec![0xd8, 0xc1, 0xc3],
        vec![0xd9, 0xeb, 0xc3],
        vec![0xdf, 0xc1, 0xc3],
    ] {
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::FPU))
                .with_native_ymm_registers(),
            &BlockLimits::default(),
        )
        .unwrap();
        assert_eq!(
            plan.exit().kind(),
            ExitKind::Mediated {
                reason: UnsupportedReason::InstructionClass
            }
        );
        assert_eq!(plan.instructions()[0].emitted_len(), 0);
    }
    for prefix in [0xf0, 0xf2, 0xf3, 0x64] {
        let code = [prefix, 0xdd, 0x00, 0xc3];
        let mut decoder = Decoder::with_ip(64, &code, BIAS + ENTRY as u64, DecoderOptions::NONE);
        let invalid = decoder.decode().is_invalid();
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let result = plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default(),
        );
        if invalid {
            assert!(matches!(
                result,
                Err(CodegenError::InvalidInstruction { .. })
            ));
        } else {
            assert_eq!(
                result.unwrap().exit().kind(),
                ExitKind::Mediated {
                    reason: if prefix == 0x64 {
                        UnsupportedReason::SegmentPrefix
                    } else {
                        UnsupportedReason::RepeatOrLockPrefix
                    }
                }
            );
        }
    }
}
