// SPDX-License-Identifier: GPL-2.0-only
//! Original packed-conversion plans retain widths without native AVX admission.

use super::*;

// Prefix, opcode, exact form, destination, register source, memory source bytes.
const FORMS: [(u8, u8, Code, Register, Register, usize); 12] = [
    (
        0x79,
        0x5b,
        Code::VEX_Vcvtps2dq_xmm_xmmm128,
        Register::XMM2,
        Register::XMM4,
        16,
    ),
    (
        0x7d,
        0x5b,
        Code::VEX_Vcvtps2dq_ymm_ymmm256,
        Register::YMM2,
        Register::YMM4,
        32,
    ),
    (
        0x7a,
        0x5b,
        Code::VEX_Vcvttps2dq_xmm_xmmm128,
        Register::XMM2,
        Register::XMM4,
        16,
    ),
    (
        0x7e,
        0x5b,
        Code::VEX_Vcvttps2dq_ymm_ymmm256,
        Register::YMM2,
        Register::YMM4,
        32,
    ),
    (
        0x7b,
        0xe6,
        Code::VEX_Vcvtpd2dq_xmm_xmmm128,
        Register::XMM2,
        Register::XMM4,
        16,
    ),
    (
        0x7f,
        0xe6,
        Code::VEX_Vcvtpd2dq_xmm_ymmm256,
        Register::XMM2,
        Register::YMM4,
        32,
    ),
    (
        0x79,
        0xe6,
        Code::VEX_Vcvttpd2dq_xmm_xmmm128,
        Register::XMM2,
        Register::XMM4,
        16,
    ),
    (
        0x7d,
        0xe6,
        Code::VEX_Vcvttpd2dq_xmm_ymmm256,
        Register::XMM2,
        Register::YMM4,
        32,
    ),
    (
        0x78,
        0x5b,
        Code::VEX_Vcvtdq2ps_xmm_xmmm128,
        Register::XMM2,
        Register::XMM4,
        16,
    ),
    (
        0x7c,
        0x5b,
        Code::VEX_Vcvtdq2ps_ymm_ymmm256,
        Register::YMM2,
        Register::YMM4,
        32,
    ),
    (
        0x7a,
        0xe6,
        Code::VEX_Vcvtdq2pd_xmm_xmmm64,
        Register::XMM2,
        Register::XMM4,
        8,
    ),
    (
        0x7e,
        0xe6,
        Code::VEX_Vcvtdq2pd_ymm_xmmm128,
        Register::YMM2,
        Register::XMM4,
        16,
    ),
];

#[test]
fn twelve_vex_packed_conversions_retain_exact_widths_and_rip_sources_under_mediation() {
    for (prefix, opcode, expected, destination, register_source, memory_bytes) in FORMS {
        for memory in [false, true] {
            let mut code = vec![0xc4, 0xe1, prefix, opcode];
            if memory {
                code.extend([0x15, 0, 1, 0, 0]); // destination2, [original RIP+256]
            } else {
                code.push(0xd4); // destination2, source4
            }
            let length = code.len();
            code.push(0xc3);
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
                    (BIAS + ENTRY as u64, BIAS + ENTRY as u64 + length as u64)
                );
                let record = &plan.instructions()[0];
                assert_eq!(
                    (
                        record.source_offset(),
                        record.source_len(),
                        record.emitted_len()
                    ),
                    (0, length, 0)
                );
                let instruction = plan.exit().instruction().unwrap();
                assert_eq!(instruction, record.instruction());
                assert_eq!((instruction.code(), instruction.op_count()), (expected, 2));
                assert_eq!(instruction.op0_kind(), OpKind::Register);
                assert_eq!(instruction.op0_register(), destination);
                if memory {
                    let target = BIAS + ENTRY as u64 + length as u64 + 256;
                    assert_eq!(instruction.op1_kind(), OpKind::Memory);
                    assert_eq!(instruction.memory_size().size(), memory_bytes);
                    assert_eq!(instruction.memory_base(), Register::RIP);
                    assert_eq!(instruction.ip_rel_memory_address(), target);
                    assert_eq!(record.rip_relative_target(), Some(target));
                } else {
                    assert_eq!(instruction.op1_kind(), OpKind::Register);
                    assert_eq!(instruction.op1_register(), register_source);
                    assert_eq!(record.rip_relative_target(), None);
                }
                assert_eq!(
                    MemoryAccessAnalyzer::default().analyze(record, &[0; 16]),
                    Err(MemoryAccessError::NotEmitted)
                );
            }
        }
    }
}

#[test]
fn reserved_vvvv_packed_conversions_are_invalid_and_legacy_conversions_stay_native() {
    // Pinned iced1.21 VEX VW handlers require the unused vvvv field reserved.
    for (prefix, opcode, _, _, _, _) in FORMS {
        let code = [0xc4, 0xe1, prefix ^ 8, opcode, 0xd4, 0xc3];
        let mut decoder = Decoder::with_ip(64, &code, BIAS + ENTRY as u64, DecoderOptions::NONE);
        assert!(decoder.decode().is_invalid());
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let declaration =
            declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
        assert!(matches!(
            plan_materialized_block(
                &declaration,
                0,
                BIAS + ENTRY as u64,
                &code,
                placement(),
                &TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX)),
                &BlockLimits::default(),
            ),
            Err(CodegenError::InvalidInstruction { guest_pc }) if guest_pc == BIAS + ENTRY as u64
        ));
    }
    for (prefix, opcode, expected) in [
        (Some(0x66), 0x5b, Code::Cvtps2dq_xmm_xmmm128),
        (Some(0xf3), 0x5b, Code::Cvttps2dq_xmm_xmmm128),
        (Some(0xf2), 0xe6, Code::Cvtpd2dq_xmm_xmmm128),
        (Some(0x66), 0xe6, Code::Cvttpd2dq_xmm_xmmm128),
        (None, 0x5b, Code::Cvtdq2ps_xmm_xmmm128),
        (Some(0xf3), 0xe6, Code::Cvtdq2pd_xmm_xmmm64),
    ] {
        let mut code = prefix.into_iter().collect::<Vec<_>>();
        code.extend([0x0f, opcode, 0xd4, 0xc3]);
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
            (expected, Register::XMM2, Register::XMM4)
        );
        assert_eq!(plan.instructions()[0].emitted_len(), code.len() - 1);
    }
}
