// SPDX-License-Identifier: GPL-2.0-only
//! Original timestamp instructions retain exit metadata without native authority.

use super::*;

const FORMS: [(&[u8], Code); 2] = [
    (&[0x0f, 0x31], Code::Rdtsc),
    (&[0x0f, 0x01, 0xf9], Code::Rdtscp),
];

#[test]
fn timestamp_counter_instructions_retain_exact_no_operand_exits_under_mediation() {
    for (instruction_bytes, expected) in FORMS {
        let mut code = instruction_bytes.to_vec();
        code.push(0xc3);
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let declaration =
            declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
        for features in [
            CpuFeatures::x64_sse2(),
            CpuFeatures::x64_sse2()
                .with(CpuidFeature::TSC)
                .with(CpuidFeature::RDTSCP),
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
                (
                    BIAS + ENTRY as u64,
                    BIAS + ENTRY as u64 + instruction_bytes.len() as u64
                )
            );
            let record = &plan.instructions()[0];
            assert_eq!(
                (
                    record.source_offset(),
                    record.source_len(),
                    record.emitted_len(),
                    record.rip_relative_target()
                ),
                (0, instruction_bytes.len(), 0, None)
            );
            let instruction = plan.exit().instruction().unwrap();
            assert_eq!(instruction, record.instruction());
            assert_eq!((instruction.code(), instruction.op_count()), (expected, 0));
            assert_eq!(
                MemoryAccessAnalyzer::default().analyze(record, &[0; 16]),
                Err(MemoryAccessError::NotEmitted)
            );
        }
    }
}

#[test]
fn timestamp_lock_is_invalid_and_repeat_or_segment_prefixes_remain_refused() {
    for (instruction_bytes, expected) in FORMS {
        let mut locked = vec![0xf0];
        locked.extend_from_slice(instruction_bytes);
        locked.push(0xc3);
        let mut decoder = Decoder::with_ip(64, &locked, BIAS + ENTRY as u64, DecoderOptions::NONE);
        assert!(decoder.decode().is_invalid());
        let bytes = image(&locked);
        let source = link(&[(&bytes, BIAS)]);
        let declaration =
            declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
        let policy = TranslationPolicy::new(
            CpuFeatures::x64_sse2()
                .with(CpuidFeature::TSC)
                .with(CpuidFeature::RDTSCP),
        );
        assert!(matches!(
            plan_materialized_block(
                &declaration, 0, BIAS + ENTRY as u64, &locked,
                placement(), &policy, &BlockLimits::default(),
            ),
            Err(CodegenError::InvalidInstruction { guest_pc }) if guest_pc == BIAS + ENTRY as u64
        ));
        // These decoded prefixes are refused by policy, not asserted as CPU #UD.
        for (prefix, reason) in [
            (0xf3, UnsupportedReason::RepeatOrLockPrefix),
            (0xf2, UnsupportedReason::RepeatOrLockPrefix),
            (0x64, UnsupportedReason::SegmentPrefix),
        ] {
            let mut code = vec![prefix];
            code.extend_from_slice(instruction_bytes);
            let length = code.len();
            code.push(0xc3);
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
                &policy,
                &BlockLimits::default(),
            )
            .unwrap();
            assert_eq!(plan.exit().kind(), ExitKind::Mediated { reason });
            assert_eq!(plan.bytes(), [0x0f, 0x0b]);
            assert_eq!(plan.instructions().len(), 1);
            assert_eq!(
                (plan.exit().guest_pc(), plan.exit().next_guest_pc()),
                (BIAS + ENTRY as u64, BIAS + ENTRY as u64 + length as u64)
            );
            let record = &plan.instructions()[0];
            assert_eq!(record.source_len(), length);
            assert_eq!(record.emitted_len(), 0);
            assert_eq!(record.rip_relative_target(), None);
            let instruction = plan.exit().instruction().unwrap();
            assert_eq!(instruction, record.instruction());
            assert_eq!((instruction.code(), instruction.op_count()), (expected, 0));
            match prefix {
                0xf3 => assert!(instruction.has_rep_prefix()),
                0xf2 => assert!(instruction.has_repne_prefix()),
                _ => assert_eq!(instruction.segment_prefix(), Register::FS),
            }
            assert_eq!(
                MemoryAccessAnalyzer::default().analyze(record, &[0; 16]),
                Err(MemoryAccessError::NotEmitted)
            );
        }
    }
}
