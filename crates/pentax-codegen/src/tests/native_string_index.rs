// SPDX-License-Identifier: GPL-2.0-only
//! Original index encodings retain implicit ECX dependencies and no memory grant.
use super::*;

fn policy() -> TranslationPolicy {
    TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
        .with_native_ymm_registers()
}

fn encoding(left: u8, right: u8, control: u8, wide_w: bool) -> Vec<u8> {
    vec![
        0xc4,
        0xe3 ^ (u8::from(left >= 8) << 7) ^ (u8::from(right >= 8) << 5),
        0x79 | (u8::from(wide_w) << 7),
        0x63,
        0xc0 | ((left & 7) << 3) | (right & 7),
        control,
    ]
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
fn exact_register_index_keeps_immediates_sources_and_implicit_rcx_dependency() {
    for control in [0, 1, 0x18, 0x38, 0x40, 0x78, 0x80, 0xff] {
        for [left, right] in [[0, 1], [9, 15], [15, 9], [9, 9]] {
            for wide_w in [false, true] {
                let mut code = encoding(left, right, control, wide_w);
                code.push(0xc3);
                inspect(&code, policy(), |plan| {
                    let output = ordinary_output(plan);
                    assert_eq!(output.len(), 1);
                    assert_eq!(output[0].code(), Code::VEX_Vpcmpistri_xmm_xmmm128_imm8);
                    assert_eq!(
                        output[0].op0_register() as u32,
                        Register::XMM0 as u32 + u32::from(left)
                    );
                    assert_eq!(
                        output[0].op1_register() as u32,
                        Register::XMM0 as u32 + u32::from(right)
                    );
                    assert_eq!(output[0].immediate8(), control);
                    assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
                    let (memory, dependencies) =
                        plan.instructions()[0].immutable_memory_summary().unwrap();
                    assert!(!memory);
                    assert_eq!(dependencies.address, 0);
                    assert_eq!(dependencies.written, 1 << 1);
                    assert!(plan.native_admission().is_memory_free());
                    assert!(!plan.emits_scalar_float_memory());
                    assert!(!plan.emits_vector_memory_move());
                    assert!(!plan.emits_non_temporal_store());
                });
            }
        }
    }
    let instruction = encoding(9, 15, 0x18, false);
    for read in [&[0x8b, 0x01][..], &[0x48, 0x8b, 0x04, 0x8b][..]] {
        let mut code = instruction.clone();
        code.extend(read);
        code.push(0xc3);
        inspect(&code, policy().with_independent_memory(), |plan| {
            assert_eq!(ordinary_output(plan).len(), 1);
            assert_eq!(
                plan.exit().kind(),
                ExitKind::Fallthrough {
                    target: BIAS + ENTRY as u64 + 6
                }
            );
        });
    }
    let mut code = instruction;
    code.extend([0x83, 0xc0, 1, 0x77, 0, 0xc3]);
    inspect(&code, policy().with_independent_memory(), |plan| {
        assert_eq!(ordinary_output(plan).len(), 2);
        assert!(matches!(plan.exit().kind(), ExitKind::Conditional { .. }));
    });
}

#[test]
fn index_admission_keeps_old_profiles_memory_neighbors_and_reserved_fields_closed() {
    let mut code = encoding(9, 15, 0, false);
    code.push(0xc3);
    for (selected, reason) in [
        (
            TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX)),
            UnsupportedReason::InstructionClass,
        ),
        (
            TranslationPolicy::x64_sse2().with_native_ymm_registers(),
            UnsupportedReason::MissingFeature(CpuidFeature::AVX),
        ),
    ] {
        inspect(&code, selected, |plan| {
            assert_eq!(plan.exit().kind(), ExitKind::Mediated { reason })
        });
    }
    for code in [
        &[0xc4, 0xe3, 0x79, 0x63, 0x0f, 0, 0xc3][..], // Memory source.
        &[0xc4, 0xe3, 0x79, 0x62, 0xca, 0, 0xc3][..], // Mask result.
        &[0xc4, 0xe3, 0x79, 0x61, 0xca, 0, 0xc3][..], // Explicit lengths.
        &[0x66, 0x0f, 0x3a, 0x63, 0xca, 0, 0xc3][..], // Existing legacy policy.
    ] {
        inspect(
            code,
            policy()
                .with_native_vector_moves()
                .with_native_packed_float_memory(),
            |plan| {
                assert!(ordinary_output(plan).is_empty());
                assert!(matches!(plan.exit().kind(), ExitKind::Mediated { .. }));
            },
        );
    }
    let mut segment = vec![0x64];
    segment.extend(encoding(9, 15, 0, false));
    segment.push(0xc3);
    inspect(&segment, policy(), |plan| {
        assert_eq!(
            plan.exit().kind(),
            ExitKind::Mediated {
                reason: UnsupportedReason::SegmentPrefix
            }
        )
    });
    for code in [
        &[0xc4, 0xe3, 0x71, 0x63, 0xca, 0, 0xc3][..], // Reserved vvvv.
        &[0xc4, 0xe3, 0x7d, 0x63, 0xca, 0, 0xc3][..], // Reserved L1.
    ] {
        let bytes = image(code);
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
}
