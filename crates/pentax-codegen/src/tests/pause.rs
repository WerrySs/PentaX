// SPDX-License-Identifier: GPL-2.0-only
//! Original exact spin-hint decoding and native emission fixtures.
use super::*;

#[test]
fn pause_keeps_exact_f3_90_native_bytes_and_one_accounted_instruction() {
    let bytes = image(&[0xf3, 0x90, 0xc3]);
    let source = link(&[(&bytes, BIAS)]);
    let plan = make(&source);
    assert!(matches!(plan.exit().kind(), ExitKind::Return { adjust: 0 }));
    assert_eq!(plan.bytes(), &[0xf3, 0x90, 0x0f, 0x0b]);
    let records = plan.instructions();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].instruction().code(), Code::Pause);
    assert_eq!(records[0].source_len(), 2);
    assert_eq!(records[0].emitted_len(), 2);
    assert_eq!(records[0].instruction().op_count(), 0);
    assert_eq!(records[0].instruction().rflags_modified(), 0);
    assert_eq!(ordinary_output(&plan)[0].code(), Code::Pause);
    assert_eq!(plan.native_admission().emitted_instructions(), 1);
    assert!(plan.native_admission().is_memory_free());
    assert!(plan.usage().work_units > 0);
}

#[test]
fn pause_feature_declaration_does_not_admit_other_prefixes_or_instruction_classes() {
    let bytes = image(&[0xf3, 0x90, 0xc3]);
    let source = link(&[(&bytes, BIAS)]);
    let missing = plan_block(
        &source,
        0,
        BIAS + ENTRY as u64,
        placement(),
        &TranslationPolicy::new(CpuFeatures::empty()),
        &BlockLimits::default(),
    )
    .unwrap();
    assert!(matches!(
        missing.exit().kind(),
        ExitKind::Mediated {
            reason: UnsupportedReason::MissingFeature(CpuidFeature::PAUSE)
        }
    ));
    for code in [
        &[0x66, 0xf3, 0x90, 0xc3][..],
        &[0x48, 0xf3, 0x90, 0xc3][..],
        &[0xf3, 0xf3, 0x90, 0xc3][..],
        &[0xf3, 0x0f, 0xa2, 0xc3][..],
        &[0xf2, 0x90, 0xc3][..],
        &[0xf0, 0xf3, 0x90, 0xc3][..],
        &[0x0f, 0x01, 0xc9, 0xc3][..], // MWAIT is not a spin hint.
    ] {
        let bytes = image(code);
        let source = link(&[(&bytes, BIAS)]);
        let result = plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default(),
        );
        match result {
            Ok(plan) => {
                assert!(matches!(plan.exit().kind(), ExitKind::Mediated { .. }));
                assert_eq!(plan.native_admission().emitted_instructions(), 0);
            }
            Err(CodegenError::InvalidInstruction { .. }) => {}
            Err(other) => panic!("unexpected exact-prefix refusal: {other:?}"),
        }
    }
}

#[test]
fn pause_stays_inside_decode_instruction_output_and_work_limits() {
    let bytes = image(&[0xf3]);
    let source = link(&[(&bytes, BIAS)]);
    assert!(matches!(
        plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default()
        ),
        Err(CodegenError::TruncatedInstruction { .. })
    ));
    let bytes = image(&[0xf3, 0x90, 0xf3, 0x90, 0xc3]);
    let source = link(&[(&bytes, BIAS)]);
    let plan = plan_block(
        &source,
        0,
        BIAS + ENTRY as u64,
        placement(),
        &TranslationPolicy::x64_sse2(),
        &BlockLimits {
            max_instructions: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(plan.native_admission().emitted_instructions(), 1);
    assert!(
        matches!(plan.exit().kind(), ExitKind::Fallthrough { target } if target == BIAS + ENTRY as u64 + 2)
    );
    for limits in [
        BlockLimits {
            max_output_bytes: 3,
            ..Default::default()
        },
        BlockLimits {
            max_work_units: 0,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            plan_block(
                &source,
                0,
                BIAS + ENTRY as u64,
                placement(),
                &TranslationPolicy::x64_sse2(),
                &limits
            ),
            Err(CodegenError::Budget { .. })
        ));
    }
}
