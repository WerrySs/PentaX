// SPDX-License-Identifier: GPL-2.0-only
//! Original CPUID exits never emit a host identification instruction.

use super::*;

#[test]
fn cpuid_retains_exact_no_operand_metadata_without_native_emission() {
    let code = [0x0f, 0xa2, 0xc3];
    let bytes = image(&code);
    let source = link(&[(&bytes, BIAS)]);
    let declaration = declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
    for features in [
        CpuFeatures::x64_sse2(),
        CpuFeatures::x64_sse2()
            .with(CpuidFeature::TSC)
            .with(CpuidFeature::RDTSCP)
            .with(CpuidFeature::POPCNT)
            .with(CpuidFeature::AVX),
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
            (BIAS + ENTRY as u64, BIAS + ENTRY as u64 + 2)
        );
        let record = &plan.instructions()[0];
        assert_eq!(
            (
                record.source_offset(),
                record.source_len(),
                record.emitted_len(),
                record.rip_relative_target()
            ),
            (0, 2, 0, None)
        );
        let instruction = plan.exit().instruction().unwrap();
        assert_eq!(instruction, record.instruction());
        assert_eq!(
            (instruction.code(), instruction.op_count()),
            (Code::Cpuid, 0)
        );
        assert_eq!(
            MemoryAccessAnalyzer::default().analyze(record, &[0; 16]),
            Err(MemoryAccessError::NotEmitted)
        );
    }
}

#[test]
fn cpuid_lock_is_invalid_and_prefixed_forms_never_gain_execution_authority() {
    let locked = [0xf0, 0x0f, 0xa2, 0xc3];
    let mut decoder = Decoder::with_ip(64, &locked, BIAS + ENTRY as u64, DecoderOptions::NONE);
    assert!(decoder.decode().is_invalid());
    let bytes = image(&locked);
    let source = link(&[(&bytes, BIAS)]);
    let declaration = declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
    assert!(matches!(
        plan_materialized_block(&declaration, 0, BIAS + ENTRY as u64, &locked,
            placement(), &TranslationPolicy::x64_sse2(), &BlockLimits::default()),
        Err(CodegenError::InvalidInstruction { guest_pc }) if guest_pc == BIAS + ENTRY as u64
    ));
    for (prefix, reason) in [
        (0xf3, UnsupportedReason::RepeatOrLockPrefix),
        (0xf2, UnsupportedReason::RepeatOrLockPrefix),
        (0x64, UnsupportedReason::SegmentPrefix),
        (0x66, UnsupportedReason::InstructionClass),
        (0x67, UnsupportedReason::InstructionClass),
        (0x40, UnsupportedReason::InstructionClass),
    ] {
        // Ignored operand/address/REX prefixes still produce only metadata.
        // The separate native resolver admits exclusively the two-byte form.
        let code = [prefix, 0x0f, 0xa2, 0xc3];
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
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default(),
        )
        .unwrap();
        assert_eq!(plan.exit().kind(), ExitKind::Mediated { reason });
        assert_eq!(plan.bytes(), [0x0f, 0x0b]);
        assert_eq!(plan.instructions().len(), 1);
        assert_eq!(
            (plan.exit().guest_pc(), plan.exit().next_guest_pc()),
            (BIAS + ENTRY as u64, BIAS + ENTRY as u64 + 3)
        );
        let record = &plan.instructions()[0];
        assert_eq!(
            (
                record.source_len(),
                record.emitted_len(),
                record.rip_relative_target()
            ),
            (3, 0, None)
        );
        let instruction = plan.exit().instruction().unwrap();
        assert_eq!(instruction, record.instruction());
        assert_eq!(
            (instruction.code(), instruction.op_count()),
            (Code::Cpuid, 0)
        );
        assert_eq!(
            MemoryAccessAnalyzer::default().analyze(record, &[0; 16]),
            Err(MemoryAccessError::NotEmitted)
        );
    }
}
