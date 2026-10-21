// SPDX-License-Identifier: GPL-2.0-only
//! Original scalar FMA register encodings, without native execution authority.
use super::*;
use iced_x86::{InstructionInfoFactory, OpAccess};

const FORMS: [(u8, Code); 2] = [
    (0xa9, Code::VEX_Vfmadd213ss_xmm_xmm_xmmm32),
    (0xad, Code::VEX_Vfnmadd213ss_xmm_xmm_xmmm32),
];

fn features() -> CpuFeatures {
    CpuFeatures::x64_sse2()
        .with(CpuidFeature::AVX)
        .with(CpuidFeature::FMA)
}
fn policy() -> TranslationPolicy {
    TranslationPolicy::new(features())
        .with_native_ymm_registers()
        .with_native_scalar_fma()
        .with_independent_memory()
}
fn encoding(opcode: u8, [destination, left, right]: [u8; 3], l: bool) -> Vec<u8> {
    vec![
        0xc4,
        0xe2 ^ (u8::from(destination >= 8) << 7) ^ (u8::from(right >= 8) << 5),
        ((!left & 15) << 3) | 1 | (u8::from(l) << 2),
        opcode,
        0xc0 | ((destination & 7) << 3) | (right & 7),
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
fn scalar_fma_keeps_destructive_inputs_aliases_lig_and_exact_immutable_admission() {
    let mut factory = InstructionInfoFactory::new();
    for (opcode, expected) in FORMS {
        for registers in [
            [9, 13, 14],
            [13, 13, 14],
            [14, 13, 14],
            [15, 0, 15],
            [0, 15, 0],
            [15, 15, 15],
        ] {
            for l in [false, true] {
                let mut code = encoding(opcode, registers, l);
                code.push(0xc3);
                inspect(&code, policy(), |plan| {
                    assert!(plan.policy().requests_native_scalar_fma());
                    assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
                    let output = ordinary_output(plan);
                    assert_eq!(output.len(), 1);
                    let record = &plan.instructions()[0];
                    let decoded = record.instruction();
                    assert_eq!(decoded.code(), expected);
                    assert_eq!(output[0].code(), expected);
                    assert_eq!(decoded.op_count(), 3);
                    for operand in 0..3 {
                        assert_eq!(decoded.op_kind(operand), OpKind::Register);
                        assert_eq!(
                            decoded.op_register(operand) as u32,
                            Register::XMM0 as u32 + u32::from(registers[operand as usize])
                        );
                        assert_eq!(output[0].op_register(operand), decoded.op_register(operand));
                    }
                    assert_eq!(decoded.rflags_modified(), 0);
                    assert!(decoded.cpuid_features().contains(&CpuidFeature::FMA));
                    // The whole old destination is a genuine input: its low
                    // scalar participates and bits32..127 survive execution.
                    let info = factory.info(decoded);
                    assert_eq!(info.op0_access(), OpAccess::ReadWrite);
                    assert_eq!(info.op1_access(), OpAccess::Read);
                    assert_eq!(info.op2_access(), OpAccess::Read);
                    assert!(info.used_memory().is_empty());
                    let (memory, dependencies) = record.immutable_memory_summary().unwrap();
                    assert!(!memory);
                    assert_eq!(dependencies.address, 0);
                    assert_eq!(dependencies.written, 0); // No implicit GPR writes.
                    assert!(plan.native_admission().is_memory_free());
                    assert_eq!(plan.native_admission().emitted_instructions(), 1);
                    assert!(!plan.native_admission().features_supported_by(&[0; 4]));
                    assert!(!plan.emits_scalar_float_memory());
                    assert!(!plan.emits_vector_memory_move());
                    assert!(!plan.emits_non_temporal_store());
                    // Never pass a scalar L=1 encoding through to hardware.
                    assert_eq!(plan.bytes()[0], 0xc4);
                    assert_eq!(plan.bytes()[2] & 4, 0);
                });
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
                    &policy(),
                    &BlockLimits::default(),
                )
                .unwrap();
                assert_eq!(plan.instructions()[0].instruction().code(), expected);
                assert_ne!(plan.instructions()[0].emitted_len(), 0);
                assert!(plan.native_admission().is_memory_free());
                assert_eq!(plan.native_admission().emitted_instructions(), 1);
                assert!(!plan.native_admission().features_supported_by(&[0; 4]));
            }
        }
    }
    let mut coalesced = vec![0x41, 0xb8, 17, 0, 0, 0]; // MOV R8D,17.
    coalesced.extend(encoding(FORMS[0].0, [9, 13, 14], false));
    coalesced.extend(encoding(FORMS[1].0, [14, 13, 14], false));
    coalesced.extend([0x41, 0x83, 0xc0, 1, 0xc3]); // ADD R8D,1;RET.
    inspect(&coalesced, policy(), |plan| {
        assert_eq!(ordinary_output(plan).len(), 4);
        assert_eq!(plan.native_admission().emitted_instructions(), 4);
        assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
    });
}

#[test]
fn scalar_fma_keeps_features_profiles_memory_neighbors_and_prefixes_closed() {
    for (opcode, _) in FORMS {
        let mut code = encoding(opcode, [9, 13, 14], false);
        code.push(0xc3);
        for selected in [
            TranslationPolicy::new(features()),
            TranslationPolicy::new(features()).with_native_ymm_registers(),
            TranslationPolicy::new(features()).with_native_scalar_fma(),
            TranslationPolicy::new(features())
                .with_native_ymm_registers()
                .with_native_scalar_float()
                .with_native_vector_moves()
                .with_native_packed_float_memory()
                .with_native_streaming_stores(),
        ] {
            inspect(&code, selected, |plan| {
                assert!(ordinary_output(plan).is_empty());
                assert_eq!(
                    plan.exit().kind(),
                    ExitKind::Mediated {
                        reason: UnsupportedReason::InstructionClass
                    }
                );
            });
        }
        for (features, reason) in [
            (
                CpuFeatures::x64_sse2().with(CpuidFeature::FMA),
                CpuidFeature::AVX,
            ),
            (
                CpuFeatures::x64_sse2().with(CpuidFeature::AVX),
                CpuidFeature::FMA,
            ),
            (
                CpuFeatures::x64_sse2()
                    .with(CpuidFeature::AVX)
                    .with(CpuidFeature::AVX2),
                CpuidFeature::FMA,
            ),
        ] {
            inspect(
                &code,
                TranslationPolicy::new(features)
                    .with_native_ymm_registers()
                    .with_native_scalar_fma(),
                |plan| {
                    assert!(ordinary_output(plan).is_empty());
                    assert_eq!(
                        plan.exit().kind(),
                        ExitKind::Mediated {
                            reason: UnsupportedReason::MissingFeature(reason)
                        }
                    );
                },
            );
        }
        for segment in [0x64, 0x65, 0x2e] {
            let mut prefixed = vec![segment];
            prefixed.extend(&code);
            inspect(&prefixed, policy(), |plan| {
                assert_eq!(
                    plan.exit().kind(),
                    ExitKind::Mediated {
                        reason: UnsupportedReason::SegmentPrefix
                    }
                );
                assert!(ordinary_output(plan).is_empty());
            });
        }
        for prefix in [0xf0, 0xf2, 0xf3] {
            let mut prefixed = vec![prefix];
            prefixed.extend(&code);
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
                    assert!(ordinary_output(&plan).is_empty());
                    assert_eq!(
                        plan.exit().kind(),
                        ExitKind::Mediated {
                            reason: UnsupportedReason::RepeatOrLockPrefix
                        }
                    );
                }
                other => panic!("unexpected scalar FMA prefix admission: {other:?}"),
            }
        }
        let mut memory = code.clone();
        memory[1] |= 0x20; // The memory base is ordinary RAX, not an extended GPR.
        memory[4] &= 0x38;
        let mut double = code.clone();
        double[2] |= 0x80; // W1 selects scalar double, not another SS encoding.
        let mut packed = code.clone();
        packed[3] -= 1;
        let mut other_order = code.clone();
        other_order[3] -= 0x10; //132 order.
        let mut later_order = code.clone();
        later_order[3] += 0x10; //231 order.
        for excluded in [
            memory,
            double,
            packed,
            other_order,
            later_order,
            vec![0xc4, 0xe2, 0x71, 0xab, 0xc2, 0xc3], // FMSUB213SS.
            vec![0xc4, 0xe2, 0x71, 0xaf, 0xc2, 0xc3], // FNMSUB213SS.
            vec![0x62, 0xf2, 0x75, 0x08, 0xa9, 0xc2, 0xc3], // EVEX FMA.
        ] {
            inspect(&excluded, policy(), |plan| {
                assert!(ordinary_output(plan).is_empty());
                assert_eq!(
                    plan.exit().kind(),
                    ExitKind::Mediated {
                        reason: UnsupportedReason::InstructionClass
                    }
                );
            });
        }
    }
}
