// SPDX-License-Identifier: GPL-2.0-only
//! Original VMOVMSK source plans retain operands without native AVX admission.

use super::*;

const FORMS: [(u8, Code, Register, Register); 8] = [
    (
        0x78,
        Code::VEX_Vmovmskps_r32_xmm,
        Register::R10D,
        Register::XMM11,
    ),
    (
        0xf8,
        Code::VEX_Vmovmskps_r64_xmm,
        Register::R10,
        Register::XMM11,
    ),
    (
        0x7c,
        Code::VEX_Vmovmskps_r32_ymm,
        Register::R10D,
        Register::YMM11,
    ),
    (
        0xfc,
        Code::VEX_Vmovmskps_r64_ymm,
        Register::R10,
        Register::YMM11,
    ),
    (
        0x79,
        Code::VEX_Vmovmskpd_r32_xmm,
        Register::R10D,
        Register::XMM11,
    ),
    (
        0xf9,
        Code::VEX_Vmovmskpd_r64_xmm,
        Register::R10,
        Register::XMM11,
    ),
    (
        0x7d,
        Code::VEX_Vmovmskpd_r32_ymm,
        Register::R10D,
        Register::YMM11,
    ),
    (
        0xfd,
        Code::VEX_Vmovmskpd_r64_ymm,
        Register::R10,
        Register::YMM11,
    ),
];

#[test]
fn eight_vex_vector_mask_forms_preserve_high_register_operands_and_remain_mediated() {
    for (prefix, expected, destination, vector) in FORMS {
        // R/B extension bits independently select GPR10 and vector11. W/L/pp
        // select the exact decoder form; there is no memory operand or imm8.
        let code = [0xc4, 0x41, prefix, 0x50, 0xd3, 0xc3];
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let declaration =
            declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
        for features in [
            CpuFeatures::x64_sse2(),
            CpuFeatures::x64_sse2().with(CpuidFeature::AVX),
        ] {
            let plan = plan_materialized_block(
                &declaration,
                0,
                BIAS + ENTRY as u64,
                &code,
                placement(),
                &TranslationPolicy::new(features).with_independent_memory(),
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
            assert_eq!(
                (plan.exit().guest_pc(), plan.exit().next_guest_pc()),
                (BIAS + ENTRY as u64, BIAS + ENTRY as u64 + 5)
            );
            let record = &plan.instructions()[0];
            assert_eq!(
                (
                    record.source_offset(),
                    record.source_len(),
                    record.emitted_len(),
                    record.rip_relative_target()
                ),
                (0, 5, 0, None)
            );
            let instruction = plan.exit().instruction().unwrap();
            assert_eq!(instruction, record.instruction());
            assert_eq!((instruction.code(), instruction.op_count()), (expected, 2));
            assert_eq!(
                (instruction.op0_kind(), instruction.op1_kind()),
                (OpKind::Register, OpKind::Register)
            );
            assert_eq!(
                (instruction.op0_register(), instruction.op1_register()),
                (destination, vector)
            );
            assert_eq!(
                MemoryAccessAnalyzer::default().analyze(record, &[0; 16]),
                Err(MemoryAccessError::NotEmitted)
            );
        }
    }
}

#[test]
fn reserved_vex_mask_encodings_are_invalid_and_legacy_movmsk_forms_stay_native() {
    // iced1.21 OpCodeHandler_VEX_Gv_RX requires reserved vvvv and ModRM.mod3.
    // Reject the entire source instruction rather than admitting a nearby form.
    for (prefix, _, _, _) in FORMS {
        for (third, modrm) in [(prefix ^ 8, 0xd3), (prefix, 0x13)] {
            let code = [0xc4, 0x41, third, 0x50, modrm, 0xc3];
            let mut decoder =
                Decoder::with_ip(64, &code, BIAS + ENTRY as u64, DecoderOptions::NONE);
            assert!(decoder.decode().is_invalid());
            let bytes = image(&code);
            let source = link(&[(&bytes, BIAS)]);
            let declaration =
                declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
            assert!(matches!(
                plan_materialized_block(
                    &declaration, 0, BIAS + ENTRY as u64, &code, placement(),
                    &TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX)), &BlockLimits::default(),
                ),
                Err(CodegenError::InvalidInstruction { guest_pc }) if guest_pc == BIAS + ENTRY as u64
            ));
        }
    }
    for (double, wide, expected) in [
        (false, false, Code::Movmskps_r32_xmm),
        (false, true, Code::Movmskps_r64_xmm),
        (true, false, Code::Movmskpd_r32_xmm),
        (true, true, Code::Movmskpd_r64_xmm),
    ] {
        let mut code = Vec::new();
        if double {
            code.push(0x66);
        }
        code.extend([if wide { 0x4d } else { 0x45 }, 0x0f, 0x50, 0xd3, 0xc3]);
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = make(&source);
        assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
        let output = ordinary_output(&plan);
        assert_eq!(output.len(), 1);
        assert_eq!(
            (
                output[0].code(),
                output[0].op0_register(),
                output[0].op1_register()
            ),
            (
                expected,
                if wide { Register::R10 } else { Register::R10D },
                Register::XMM11
            )
        );
        assert_eq!(plan.instructions()[0].emitted_len(), code.len() - 1);
    }
}
