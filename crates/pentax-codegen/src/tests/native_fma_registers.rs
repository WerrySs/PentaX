// SPDX-License-Identifier: GPL-2.0-only
//! Original opcode/order/width oracles for the complete supported FMA3 family.
use super::*;
use iced_x86::{InstructionInfoFactory, OpAccess};

#[derive(Clone, Copy)]
struct Form {
    code: Code,
    opcode: u8,
    double: bool,
    wide: bool,
    scalar: bool,
}

// Explicit Code oracles; encoded bytes independently select operation/order,
// W (single/double), L (packed width) and the destructive register aliases.
const FORMS: [Form; 96] = [
    Form {
        code: Code::VEX_Vfmadd132ss_xmm_xmm_xmmm32,
        opcode: 0x99,
        double: false,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfmadd132sd_xmm_xmm_xmmm64,
        opcode: 0x99,
        double: true,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfmadd132ps_xmm_xmm_xmmm128,
        opcode: 0x98,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmadd132ps_ymm_ymm_ymmm256,
        opcode: 0x98,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmadd132pd_xmm_xmm_xmmm128,
        opcode: 0x98,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmadd132pd_ymm_ymm_ymmm256,
        opcode: 0x98,
        double: true,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmadd213ss_xmm_xmm_xmmm32,
        opcode: 0xa9,
        double: false,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfmadd213sd_xmm_xmm_xmmm64,
        opcode: 0xa9,
        double: true,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfmadd213ps_xmm_xmm_xmmm128,
        opcode: 0xa8,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmadd213ps_ymm_ymm_ymmm256,
        opcode: 0xa8,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmadd213pd_xmm_xmm_xmmm128,
        opcode: 0xa8,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmadd213pd_ymm_ymm_ymmm256,
        opcode: 0xa8,
        double: true,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmadd231ss_xmm_xmm_xmmm32,
        opcode: 0xb9,
        double: false,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfmadd231sd_xmm_xmm_xmmm64,
        opcode: 0xb9,
        double: true,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfmadd231ps_xmm_xmm_xmmm128,
        opcode: 0xb8,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmadd231ps_ymm_ymm_ymmm256,
        opcode: 0xb8,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmadd231pd_xmm_xmm_xmmm128,
        opcode: 0xb8,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmadd231pd_ymm_ymm_ymmm256,
        opcode: 0xb8,
        double: true,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsub132ss_xmm_xmm_xmmm32,
        opcode: 0x9b,
        double: false,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfmsub132sd_xmm_xmm_xmmm64,
        opcode: 0x9b,
        double: true,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfmsub132ps_xmm_xmm_xmmm128,
        opcode: 0x9a,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsub132ps_ymm_ymm_ymmm256,
        opcode: 0x9a,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsub132pd_xmm_xmm_xmmm128,
        opcode: 0x9a,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsub132pd_ymm_ymm_ymmm256,
        opcode: 0x9a,
        double: true,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsub213ss_xmm_xmm_xmmm32,
        opcode: 0xab,
        double: false,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfmsub213sd_xmm_xmm_xmmm64,
        opcode: 0xab,
        double: true,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfmsub213ps_xmm_xmm_xmmm128,
        opcode: 0xaa,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsub213ps_ymm_ymm_ymmm256,
        opcode: 0xaa,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsub213pd_xmm_xmm_xmmm128,
        opcode: 0xaa,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsub213pd_ymm_ymm_ymmm256,
        opcode: 0xaa,
        double: true,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsub231ss_xmm_xmm_xmmm32,
        opcode: 0xbb,
        double: false,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfmsub231sd_xmm_xmm_xmmm64,
        opcode: 0xbb,
        double: true,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfmsub231ps_xmm_xmm_xmmm128,
        opcode: 0xba,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsub231ps_ymm_ymm_ymmm256,
        opcode: 0xba,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsub231pd_xmm_xmm_xmmm128,
        opcode: 0xba,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsub231pd_ymm_ymm_ymmm256,
        opcode: 0xba,
        double: true,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmadd132ss_xmm_xmm_xmmm32,
        opcode: 0x9d,
        double: false,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfnmadd132sd_xmm_xmm_xmmm64,
        opcode: 0x9d,
        double: true,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfnmadd132ps_xmm_xmm_xmmm128,
        opcode: 0x9c,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmadd132ps_ymm_ymm_ymmm256,
        opcode: 0x9c,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmadd132pd_xmm_xmm_xmmm128,
        opcode: 0x9c,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmadd132pd_ymm_ymm_ymmm256,
        opcode: 0x9c,
        double: true,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmadd213ss_xmm_xmm_xmmm32,
        opcode: 0xad,
        double: false,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfnmadd213sd_xmm_xmm_xmmm64,
        opcode: 0xad,
        double: true,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfnmadd213ps_xmm_xmm_xmmm128,
        opcode: 0xac,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmadd213ps_ymm_ymm_ymmm256,
        opcode: 0xac,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmadd213pd_xmm_xmm_xmmm128,
        opcode: 0xac,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmadd213pd_ymm_ymm_ymmm256,
        opcode: 0xac,
        double: true,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmadd231ss_xmm_xmm_xmmm32,
        opcode: 0xbd,
        double: false,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfnmadd231sd_xmm_xmm_xmmm64,
        opcode: 0xbd,
        double: true,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfnmadd231ps_xmm_xmm_xmmm128,
        opcode: 0xbc,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmadd231ps_ymm_ymm_ymmm256,
        opcode: 0xbc,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmadd231pd_xmm_xmm_xmmm128,
        opcode: 0xbc,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmadd231pd_ymm_ymm_ymmm256,
        opcode: 0xbc,
        double: true,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmsub132ss_xmm_xmm_xmmm32,
        opcode: 0x9f,
        double: false,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfnmsub132sd_xmm_xmm_xmmm64,
        opcode: 0x9f,
        double: true,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfnmsub132ps_xmm_xmm_xmmm128,
        opcode: 0x9e,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmsub132ps_ymm_ymm_ymmm256,
        opcode: 0x9e,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmsub132pd_xmm_xmm_xmmm128,
        opcode: 0x9e,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmsub132pd_ymm_ymm_ymmm256,
        opcode: 0x9e,
        double: true,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmsub213ss_xmm_xmm_xmmm32,
        opcode: 0xaf,
        double: false,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfnmsub213sd_xmm_xmm_xmmm64,
        opcode: 0xaf,
        double: true,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfnmsub213ps_xmm_xmm_xmmm128,
        opcode: 0xae,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmsub213ps_ymm_ymm_ymmm256,
        opcode: 0xae,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmsub213pd_xmm_xmm_xmmm128,
        opcode: 0xae,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmsub213pd_ymm_ymm_ymmm256,
        opcode: 0xae,
        double: true,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmsub231ss_xmm_xmm_xmmm32,
        opcode: 0xbf,
        double: false,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfnmsub231sd_xmm_xmm_xmmm64,
        opcode: 0xbf,
        double: true,
        wide: false,
        scalar: true,
    },
    Form {
        code: Code::VEX_Vfnmsub231ps_xmm_xmm_xmmm128,
        opcode: 0xbe,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmsub231ps_ymm_ymm_ymmm256,
        opcode: 0xbe,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmsub231pd_xmm_xmm_xmmm128,
        opcode: 0xbe,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfnmsub231pd_ymm_ymm_ymmm256,
        opcode: 0xbe,
        double: true,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmaddsub132ps_xmm_xmm_xmmm128,
        opcode: 0x96,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmaddsub132ps_ymm_ymm_ymmm256,
        opcode: 0x96,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmaddsub132pd_xmm_xmm_xmmm128,
        opcode: 0x96,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmaddsub132pd_ymm_ymm_ymmm256,
        opcode: 0x96,
        double: true,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmaddsub213ps_xmm_xmm_xmmm128,
        opcode: 0xa6,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmaddsub213ps_ymm_ymm_ymmm256,
        opcode: 0xa6,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmaddsub213pd_xmm_xmm_xmmm128,
        opcode: 0xa6,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmaddsub213pd_ymm_ymm_ymmm256,
        opcode: 0xa6,
        double: true,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmaddsub231ps_xmm_xmm_xmmm128,
        opcode: 0xb6,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmaddsub231ps_ymm_ymm_ymmm256,
        opcode: 0xb6,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmaddsub231pd_xmm_xmm_xmmm128,
        opcode: 0xb6,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmaddsub231pd_ymm_ymm_ymmm256,
        opcode: 0xb6,
        double: true,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsubadd132ps_xmm_xmm_xmmm128,
        opcode: 0x97,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsubadd132ps_ymm_ymm_ymmm256,
        opcode: 0x97,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsubadd132pd_xmm_xmm_xmmm128,
        opcode: 0x97,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsubadd132pd_ymm_ymm_ymmm256,
        opcode: 0x97,
        double: true,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsubadd213ps_xmm_xmm_xmmm128,
        opcode: 0xa7,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsubadd213ps_ymm_ymm_ymmm256,
        opcode: 0xa7,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsubadd213pd_xmm_xmm_xmmm128,
        opcode: 0xa7,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsubadd213pd_ymm_ymm_ymmm256,
        opcode: 0xa7,
        double: true,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsubadd231ps_xmm_xmm_xmmm128,
        opcode: 0xb7,
        double: false,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsubadd231ps_ymm_ymm_ymmm256,
        opcode: 0xb7,
        double: false,
        wide: true,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsubadd231pd_xmm_xmm_xmmm128,
        opcode: 0xb7,
        double: true,
        wide: false,
        scalar: false,
    },
    Form {
        code: Code::VEX_Vfmsubadd231pd_ymm_ymm_ymmm256,
        opcode: 0xb7,
        double: true,
        wide: true,
        scalar: false,
    },
];

fn features() -> CpuFeatures {
    CpuFeatures::x64_sse2()
        .with(CpuidFeature::AVX)
        .with(CpuidFeature::FMA)
}
fn policy() -> TranslationPolicy {
    TranslationPolicy::new(features())
        .with_native_ymm_registers()
        .with_native_fma_registers()
        .with_independent_memory()
}
fn encoding(form: Form, [destination, left, right]: [u8; 3], scalar_l: bool) -> Vec<u8> {
    vec![
        0xc4,
        0xe2 ^ (u8::from(destination >= 8) << 7) ^ (u8::from(right >= 8) << 5),
        ((!left & 15) << 3)
            | 1
            | (u8::from(form.wide || scalar_l) << 2)
            | (u8::from(form.double) << 7),
        form.opcode,
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
fn mediated(code: &[u8], selected: TranslationPolicy, reason: UnsupportedReason) {
    inspect(code, selected, |plan| {
        assert!(ordinary_output(plan).is_empty());
        assert_eq!(plan.exit().kind(), ExitKind::Mediated { reason });
    });
}

#[test]
fn fma_registers_cover_all_supported_codes_aliases_dataflow_and_canonical_scalar_lig() {
    assert!(!TranslationPolicy::x64_sse2().requests_native_fma_registers());
    assert!(!policy().requests_native_scalar_fma());
    let mut factory = InstructionInfoFactory::new();
    let mut cases = 0;
    for form in FORMS {
        for registers in [
            [9, 13, 14],
            [13, 13, 14],
            [14, 13, 14],
            [15, 0, 15],
            [0, 15, 0],
            [15, 15, 15],
        ] {
            for scalar_l in [false, true] {
                if scalar_l && !form.scalar {
                    continue;
                }
                let mut code = encoding(form, registers, scalar_l);
                code.push(0xc3);
                inspect(&code, policy(), |plan| {
                    let output = ordinary_output(plan);
                    assert_eq!(output.len(), 1);
                    assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
                    let record = &plan.instructions()[0];
                    let decoded = record.instruction();
                    assert_eq!(decoded.code(), form.code);
                    assert_eq!(output[0].code(), form.code);
                    assert_eq!(decoded.op_count(), 3);
                    let base = if form.wide {
                        Register::YMM0
                    } else {
                        Register::XMM0
                    };
                    for operand in 0..3 {
                        assert_eq!(decoded.op_kind(operand), OpKind::Register);
                        assert_eq!(
                            decoded.op_register(operand) as u32,
                            base as u32 + u32::from(registers[operand as usize])
                        );
                        assert_eq!(output[0].op_register(operand), decoded.op_register(operand));
                    }
                    assert_eq!(decoded.rflags_modified(), 0);
                    assert!(decoded.cpuid_features().contains(&CpuidFeature::FMA));
                    let info = factory.info(decoded);
                    assert_eq!(info.op0_access(), OpAccess::ReadWrite);
                    assert_eq!(info.op1_access(), OpAccess::Read);
                    assert_eq!(info.op2_access(), OpAccess::Read);
                    assert!(info.used_memory().is_empty());
                    let (memory, dependencies) = record.immutable_memory_summary().unwrap();
                    assert!(!memory);
                    assert_eq!((dependencies.address, dependencies.written), (0, 0));
                    assert!(plan.native_admission().is_memory_free());
                    assert_eq!(plan.native_admission().emitted_instructions(), 1);
                    assert!(!plan.native_admission().features_supported_by(&[0; 4]));
                    assert!(!plan.emits_scalar_float_memory());
                    assert!(!plan.emits_vector_memory_move());
                    assert!(!plan.emits_non_temporal_store());
                    assert_eq!(plan.bytes()[0], 0xc4);
                    assert_eq!(plan.bytes()[2] & 4, if form.wide { 4 } else { 0 });
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
                assert_eq!(plan.instructions()[0].instruction().code(), form.code);
                assert_ne!(plan.instructions()[0].emitted_len(), 0);
                assert_eq!(plan.native_admission().emitted_instructions(), 1);
                assert!(plan.native_admission().is_memory_free());
                let mut emitted = Decoder::with_ip(64, plan.bytes(), CACHE, DecoderOptions::NONE);
                assert_eq!(emitted.decode().code(), form.code);
                assert_eq!(plan.bytes()[2] & 4, if form.wide { 4 } else { 0 });
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 720); // 72 packed + 24 scalar * two LIG encodings, six aliases.
    let mut code = vec![0x41, 0xb8, 17, 0, 0, 0];
    for form in FORMS {
        code.extend(encoding(form, [9, 13, 14], false));
    }
    code.extend([0x41, 0x83, 0xc0, 1, 0xc3]);
    inspect(&code, policy(), |plan| {
        assert_eq!(ordinary_output(plan).len(), 98);
        assert_eq!(plan.native_admission().emitted_instructions(), 98);
        assert!(plan.native_admission().is_memory_free());
        assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
    });
}

#[test]
fn fma_registers_keep_profiles_features_memory_and_prefixes_closed() {
    for form in FORMS {
        let mut code = encoding(form, [9, 13, 14], false);
        code.push(0xc3);
        for selected in [
            TranslationPolicy::new(features()),
            TranslationPolicy::new(features()).with_native_ymm_registers(),
            TranslationPolicy::new(features()).with_native_fma_registers(),
            TranslationPolicy::new(features())
                .with_native_ymm_registers()
                .with_native_scalar_float()
                .with_native_vector_moves()
                .with_native_packed_float_memory()
                .with_native_streaming_stores(),
        ] {
            mediated(&code, selected, UnsupportedReason::InstructionClass);
        }
        for (features, missing) in [
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
            mediated(
                &code,
                TranslationPolicy::new(features)
                    .with_native_ymm_registers()
                    .with_native_fma_registers(),
                UnsupportedReason::MissingFeature(missing),
            );
        }
        let old = TranslationPolicy::new(features())
            .with_native_ymm_registers()
            .with_native_scalar_fma();
        inspect(&code, old, |plan| {
            let narrow = matches!(
                form.code,
                Code::VEX_Vfmadd213ss_xmm_xmm_xmmm32 | Code::VEX_Vfnmadd213ss_xmm_xmm_xmmm32
            );
            assert_eq!(ordinary_output(plan).len(), usize::from(narrow));
            assert_eq!(
                plan.exit().kind(),
                if narrow {
                    ExitKind::Return { adjust: 0 }
                } else {
                    ExitKind::Mediated {
                        reason: UnsupportedReason::InstructionClass,
                    }
                }
            );
        });
        let mut memory = code.clone();
        memory[1] |= 0x20; // RAX base, not an extended vector-register encoding.
        memory[4] &= 0x38;
        mediated(&memory, policy(), UnsupportedReason::InstructionClass);
        for segment in [0x64, 0x65, 0x2e] {
            let mut prefixed = vec![segment];
            prefixed.extend(&code);
            mediated(&prefixed, policy(), UnsupportedReason::SegmentPrefix);
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
                other => panic!("unexpected FMA prefix admission: {other:?}"),
            }
        }
    }
    for excluded in [
        vec![0x62, 0xf2, 0x75, 0x08, 0x98, 0xc2, 0xc3], // EVEX VFMADD132PS.
        vec![0xc4, 0xe3, 0x71, 0x68, 0xc2, 0x30, 0xc3], // FMA4 VFMADDPS.
        vec![0xc5, 0xf1, 0x5e, 0xc2, 0xc3],             // VDIVPD.
    ] {
        mediated(&excluded, policy(), UnsupportedReason::InstructionClass);
    }
}

#[test]
fn fma_registers_keep_instruction_and_emitted_output_limits_without_skipping_a_form() {
    for form in FORMS {
        let mut code = encoding(form, [9, 13, 14], false);
        code.extend(encoding(form, [14, 13, 14], false));
        code.push(0xc3);
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        for limits in [
            BlockLimits {
                max_instructions: 1,
                ..BlockLimits::default()
            },
            BlockLimits {
                max_output_bytes: 7,
                ..BlockLimits::default()
            },
        ] {
            let plan = plan_block(
                &source,
                0,
                BIAS + ENTRY as u64,
                placement(),
                &policy(),
                &limits,
            )
            .unwrap();
            let output = ordinary_output(&plan);
            assert_eq!(output.len(), 1);
            assert_eq!(output[0].code(), form.code);
            assert_eq!(plan.native_admission().emitted_instructions(), 1);
            assert_eq!(
                plan.exit().kind(),
                ExitKind::Fallthrough {
                    target: BIAS + ENTRY as u64 + 5
                }
            );
        }
        let limits = BlockLimits {
            max_output_bytes: 6,
            ..BlockLimits::default()
        };
        assert!(matches!(
            plan_block(
                &source,
                0,
                BIAS + ENTRY as u64,
                placement(),
                &policy(),
                &limits
            ),
            Err(CodegenError::Budget {
                kind: Budget::OutputBytes,
                limit: 6
            })
        ));
    }
}
