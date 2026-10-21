// SPDX-License-Identifier: GPL-2.0-only
//! Original scalar arithmetic, full-context comparisons and live admission.
use super::*;
use pentax_codegen::{CpuFeatures, CpuidFeature};

const RBX: usize = 3;

const SINGLE: [(u64, u64); 12] = [
    (0x40c0_0000, 0x4000_0000),
    (0x3f80_0000, 0x3380_0000),
    (0x7fa0_0031, 0xffc0_0072),
    (0xffc0_0072, 0x7fa0_0031),
    (0x7fc0_0011, 0xffc0_0022),
    (1, 0x3f80_0000),
    (0x0080_0000, 0x3eaa_aaab),
    (0x7f7f_ffff, 0x4000_0000),
    (0x8000_0000, 0),
    (0x7f80_0000, 0xff80_0000),
    (0, 0x7f80_0000),
    (0x7fc0_0055, 1),
];
const DOUBLE: [(u64, u64); 12] = [
    (0x4018_0000_0000_0000, 0x4000_0000_0000_0000),
    (0x3ff0_0000_0000_0000, 0x3ca0_0000_0000_0000),
    (0x7ff0_0000_0000_0031, 0xfff8_0000_0000_0072),
    (0xfff8_0000_0000_0072, 0x7ff0_0000_0000_0031),
    (0x7ff8_0000_0000_0011, 0xfff8_0000_0000_0022),
    (1, 0x3ff0_0000_0000_0000),
    (0x0010_0000_0000_0000, 0x3fd5_5555_5555_5555),
    (0x7fef_ffff_ffff_ffff, 0x4000_0000_0000_0000),
    (0x8000_0000_0000_0000, 0),
    (0x7ff0_0000_0000_0000, 0xfff0_0000_0000_0000),
    (0, 0x7ff0_0000_0000_0000),
    (0x7ff8_0000_0000_0055, 1),
];

fn policy() -> TranslationPolicy {
    TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
        .with_native_ymm_registers()
        .with_native_scalar_float()
}
fn encoding(opcode: u8, double: bool, destination: u8, memory: bool) -> [u8; 5] {
    [
        0xc4,
        if memory { 0x61 } else { 0x41 },
        0x12 | u8::from(double),
        opcode,
        ((destination & 7) << 3) | if memory { 3 } else { 0xc6 },
    ]
}
fn host_controls() -> (u32, u16) {
    let mut mxcsr = 0_u32;
    let mut x87 = 0_u16;
    // SAFETY: Mandatory x86-64 control stores into exact live local objects.
    unsafe {
        std::arch::asm!("stmxcsr [{mx}]", "fnstcw [{cw}]",
            mx = in(reg) &mut mxcsr, cw = in(reg) &mut x87,
            options(nostack, preserves_flags));
    }
    (mxcsr, x87)
}
fn input(
    mut state: NativeCpuState,
    double: bool,
    values: (u64, u64),
    address: u64,
    control: u32,
    memory: bool,
) -> NativeCpuState {
    for (index, value) in state.gpr.iter_mut().enumerate() {
        if index != RSP {
            *value = 0x1020_3040_5060_7080 ^ index as u64;
        }
    }
    state.gpr[RAX] = 42;
    state.gpr[RBX] = address;
    state.rflags = 0x202 | 0x8d5 | 0x400;
    state.mxcsr = control;
    for (index, byte) in state.xmm.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(17);
    }
    for (index, byte) in state.ymm_upper.iter_mut().enumerate() {
        *byte = !(index as u8).wrapping_mul(13);
    }
    // Inactive lanes contain signaling NaNs and distinct payloads: they must
    // be merged verbatim from source1 without raising arithmetic exceptions.
    for (register, low) in [(13, values.0), (14, values.1)] {
        let start = register * 16;
        state.xmm[start..start + 8].copy_from_slice(&0x7fa0_1234_7fa0_5678_u64.to_le_bytes());
        state.xmm[start + 8..start + 16]
            .copy_from_slice(&(0x7ff0_0000_0000_1234_u64 + register as u64).to_le_bytes());
        let size = if double { 8 } else { 4 };
        let low = if memory && register == 14 {
            if double {
                0x3fe0_0000_0000_0000
            } else {
                0x3f00_0000
            }
        } else {
            low
        };
        state.xmm[start..start + size].copy_from_slice(&low.to_le_bytes()[..size]);
    }
    state
}
fn expected(
    mut state: NativeCpuState,
    opcode: u8,
    double: bool,
    right: u64,
    destination: usize,
) -> NativeCpuState {
    let size = if double { 8 } else { 4 };
    let mut left = [0; 8];
    left[..size].copy_from_slice(&state.xmm[208..208 + size]);
    let left = u64::from_le_bytes(left);
    let operation = match opcode {
        0x58 => float_ops::Operation::Add,
        0x5c => float_ops::Operation::Sub,
        _ => float_ops::Operation::Mul,
    };
    let (bits, control) =
        float_ops::calculate(operation, double, left, right, state.mxcsr).unwrap();
    let (exponent, fraction, quiet) = if double {
        (
            0x7ff0_0000_0000_0000,
            0x000f_ffff_ffff_ffff,
            0x0008_0000_0000_0000,
        )
    } else {
        (0x7f80_0000, 0x007f_ffff, 0x0040_0000)
    };
    let nan = |value: u64| value & exponent == exponent && value & fraction != 0;
    if nan(left) || nan(right) {
        assert_eq!(
            bits,
            if nan(left) {
                left | quiet
            } else {
                right | quiet
            }
        );
    }
    if (left, right) == if double { DOUBLE[0] } else { SINGLE[0] } {
        assert_eq!(
            bits,
            match (opcode, double) {
                (0x58, false) => 0x4100_0000,
                (0x5c, false) => 0x4080_0000,
                (0x59, false) => 0x4140_0000,
                (0x58, true) => 0x4020_0000_0000_0000,
                (0x5c, true) => 0x4010_0000_0000_0000,
                _ => 0x4028_0000_0000_0000,
            }
        );
    }
    let mut merged = [0; 16];
    merged.copy_from_slice(&state.xmm[208..224]);
    merged[..size].copy_from_slice(&bits.to_le_bytes()[..size]);
    state.xmm[destination * 16..destination * 16 + 16].copy_from_slice(&merged);
    state.ymm_upper[destination * 16..destination * 16 + 16].fill(0);
    state.mxcsr = control;
    state.gpr[RSP] += 8;
    state.guest_pc = STOP;
    state
}

#[test]
fn scalar_register_and_exact_memory_forms_match_bits_merge_controls_and_all_aliases() {
    if VectorMode::detect().features().is_none() {
        println!(
            "native_scalar_float_available=false native_scalar_float_cases=0 native_scalar_float_mode=sse2"
        );
        return;
    }
    let controls: Vec<_> = (0..16_u32)
        .map(|bits| 0x1f84 | ((bits & 3) << 13) | ((bits & 4) << 4) | ((bits & 8) << 12))
        .filter(|&control| mxcsr_control::valid(control))
        .collect();
    assert!(!controls.is_empty());
    let mut cases = 0_u64;
    for opcode in [0x58, 0x5c, 0x59] {
        for double in [false, true] {
            for memory in [false, true] {
                let mut code = Vec::new();
                for destination in [9, 13, 14] {
                    code.extend(encoding(opcode, double, destination, memory));
                    code.push(0xc3);
                }
                with_source(Some(&code), |source, arena, stack, state| {
                    let end = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE;
                    let size = if double { 8 } else { 4 };
                    let pairs = if double { &DOUBLE } else { &SINGLE };
                    let initial = input(
                        state,
                        double,
                        pairs[0],
                        end - size as u64,
                        controls[0],
                        memory,
                    );
                    let mut engine = NativeBlockStepper::new(
                        source,
                        &mut arena.pause().unwrap(),
                        initial,
                        stack,
                        &Default::default(),
                    )
                    .unwrap();
                    // Actual host/OS admission above permits the AVX bridge;
                    // disable AVX2 authority for every actual case below.
                    engine.machine.vector_mode = VectorMode::Avx {
                        avx2: false,
                        fma: false,
                    };
                    let mut entries = 0;
                    let mut memory_entries = 0;
                    for (alias, destination) in [9, 13, 14].into_iter().enumerate() {
                        let pc = state.guest_pc + (alias * 6) as u64;
                        let plan = plan_materialized_block(
                            source,
                            0,
                            pc,
                            &code[alias * 6..alias * 6 + 6],
                            engine.placement(),
                            &policy(),
                            &BlockLimits::default(),
                        )
                        .unwrap();
                        assert_eq!(plan.emits_scalar_float_memory(), memory);
                        assert!(plan.instructions()[0].emitted_len() > 0);
                        for position in 0..if memory { 2 } else { 1 } {
                            let address = end - size as u64 - position;
                            for &values in pairs {
                                if memory {
                                    arena
                                        .protect(end - PAGE, PAGE, NativePermissions::READ_WRITE)
                                        .unwrap();
                                    arena
                                        .pause()
                                        .unwrap()
                                        .write(address, &values.1.to_le_bytes()[..size])
                                        .unwrap();
                                    arena
                                        .protect(end - PAGE, PAGE, NativePermissions::READ)
                                        .unwrap();
                                }
                                for &control in &controls {
                                    let mut current =
                                        input(state, double, values, address, control, memory);
                                    current.guest_pc = pc;
                                    if entries != 0 {
                                        engine
                                            .begin_call(&mut arena.pause().unwrap(), current)
                                            .unwrap();
                                    } else {
                                        assert_eq!(*engine.state(), current);
                                    }
                                    let host = host_controls();
                                    // SAFETY: Original same-owner code and actual AVX context. Manual
                                    // memory entry must independently preflight its exact emitted flag.
                                    let returned = unsafe {
                                        if memory && position == 0 {
                                            engine.step(&mut arena.pause().unwrap(), &plan)
                                        } else {
                                            engine.step_checked(&mut arena.pause().unwrap(), &plan)
                                        }
                                    }
                                    .unwrap();
                                    assert_eq!(returned, NativeBlockStop::Returned { value: 42 });
                                    assert_eq!(
                                        *engine.state(),
                                        expected(
                                            current,
                                            opcode,
                                            double,
                                            values.1,
                                            destination as usize
                                        )
                                    );
                                    assert_eq!(host_controls(), host);
                                    if memory {
                                        let mut unchanged = [0; 8];
                                        arena
                                            .pause()
                                            .unwrap()
                                            .read(address, &mut unchanged[..size])
                                            .unwrap();
                                        assert_eq!(
                                            &unchanged[..size],
                                            &values.1.to_le_bytes()[..size]
                                        );
                                        memory_entries += 1;
                                    }
                                    entries += 1;
                                    cases += 1;
                                    assert_eq!(engine.usage().memory_access_checks, memory_entries);
                                    assert_eq!(
                                        engine.usage().memory_bytes_checked,
                                        memory_entries * size as u64
                                    );
                                }
                            }
                        }
                    }
                    assert_eq!(engine.usage().native_entries, entries);
                    assert_eq!(engine.usage().native_instructions, entries);
                    assert_eq!(engine.usage().vector_operations, 0);
                    assert_eq!(engine.usage().vector_bytes, 0);
                });
            }
        }
    }
    assert_eq!(cases, 6 * 3 * 3 * 12 * controls.len() as u64);
    println!(
        "native_scalar_float_available=true native_scalar_float_cases={cases} native_scalar_float_mode=avx"
    );
}

#[test]
fn scalar_memory_manual_and_checked_paths_refuse_before_entry_and_recheck_sealed_cache() {
    if VectorMode::detect().features().is_none() {
        return;
    }
    for double in [false, true] {
        for manual in [false, true] {
            for case in 0..11 {
                let mut code = encoding(0x59, double, 9, true).to_vec();
                if case == 7 {
                    code.extend([0x8b, 0x00]);
                }
                code.push(0xc3);
                with_source(Some(&code), |source, arena, stack, state| {
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    let size = if double { 8 } else { 4 };
                    let end = data + PAGE;
                    if case == 1 {
                        arena
                            .map_at(end, PAGE, NativePermissions::READ_WRITE)
                            .unwrap();
                    }
                    let address = if case == 0 {
                        end - size + 1
                    } else if case == 1 {
                        end - size / 2
                    } else {
                        end - size
                    };
                    let values = if double { DOUBLE[0] } else { SINGLE[0] };
                    if case != 0 {
                        arena
                            .pause()
                            .unwrap()
                            .write(address, &values.1.to_le_bytes()[..size as usize])
                            .unwrap();
                    }
                    let current = input(state, double, values, address, 0x1f84, true);
                    let mut engine = NativeBlockStepper::new(
                        source,
                        &mut arena.pause().unwrap(),
                        current,
                        stack,
                        &Default::default(),
                    )
                    .unwrap();
                    let plan = plan_materialized_block(
                        source,
                        0,
                        current.guest_pc,
                        &code,
                        engine.placement(),
                        &policy(),
                        &BlockLimits::default(),
                    )
                    .unwrap();
                    assert!(plan.emits_scalar_float_memory());
                    match case {
                        1 => arena
                            .protect(end, PAGE, NativePermissions::new(false, false, false))
                            .unwrap(),
                        2 => arena
                            .protect(data, PAGE, NativePermissions::new(false, true, false))
                            .unwrap(),
                        3 => engine.machine.vector_mode = VectorMode::Sse2,
                        4 => engine.machine.state.mxcsr &= !0x80,
                        5 => engine.machine.state.rflags |= 1 << 18,
                        6 => engine.machine.limits.max_work_units = 0,
                        8 => arena
                            .protect(
                                state.guest_pc & !(PAGE - 1),
                                PAGE,
                                NativePermissions::READ_WRITE,
                            )
                            .unwrap(),
                        9 => engine.machine.usage.memory_bytes_checked = u64::MAX,
                        _ => {}
                    }
                    let before = *engine.state();
                    let result = if case == 10 {
                        let mut other =
                            NativeAddressSpace::reserve_any(Default::default(), 65_536).unwrap();
                        // SAFETY: Actual wrong owner must fail before any native entry.
                        unsafe {
                            if manual {
                                engine.step(&mut other.pause().unwrap(), &plan)
                            } else {
                                engine.step_checked(&mut other.pause().unwrap(), &plan)
                            }
                        }
                    } else {
                        // SAFETY: Controlled original refusal, checked before native effects.
                        unsafe {
                            if manual {
                                engine.step(&mut arena.pause().unwrap(), &plan)
                            } else {
                                engine.step_checked(&mut arena.pause().unwrap(), &plan)
                            }
                        }
                    };
                    match case {
                        0..=2 | 7 => assert!(matches!(result, Err(NativeDispatchError::Memory(_)))),
                        3..=5 => assert_eq!(result, Err(NativeDispatchError::InvalidState)),
                        6 | 9 => assert!(matches!(result, Err(NativeDispatchError::Budget(_)))),
                        8 => assert_eq!(result, Err(NativeDispatchError::WritableSource)),
                        10 => assert_eq!(result, Err(NativeDispatchError::WrongOwner)),
                        _ => unreachable!(),
                    }
                    assert_eq!(*engine.state(), before);
                    assert_eq!(engine.usage().native_entries, 0);
                    assert_eq!(engine.usage().native_instructions, 0);
                    assert_eq!(engine.usage().cache_publications, 0);
                });
            }
        }
        let mut code = encoding(0x59, double, 9, true).to_vec();
        code.push(0xc3);
        with_source(Some(&code), |source, arena, stack, state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            arena
                .map_at(data + PAGE, PAGE, NativePermissions::READ_WRITE)
                .unwrap();
            let size = if double { 8 } else { 4 };
            let addresses = [data + 64, data + 128, data + PAGE - size / 2];
            let values = if double { DOUBLE[0] } else { SINGLE[0] };
            let inputs = if double {
                [
                    0x4000_0000_0000_0000_u64,
                    0x4008_0000_0000_0000,
                    0x4010_0000_0000_0000,
                ]
            } else {
                [0x4000_0000, 0x4040_0000, 0x4080_0000]
            };
            for (address, right) in [
                (addresses[0], inputs[0]),
                (addresses[1], inputs[1]),
                (addresses[2], inputs[0]),
            ] {
                arena
                    .pause()
                    .unwrap()
                    .write(address, &right.to_le_bytes()[..size as usize])
                    .unwrap();
            }
            arena
                .protect(data, 2 * PAGE, NativePermissions::READ)
                .unwrap();
            let initial = input(state, double, values, addresses[0], 0x1f84, true);
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                initial,
                stack,
                &Default::default(),
            )
            .unwrap();
            let plan = plan_materialized_block(
                source,
                0,
                state.guest_pc,
                &code,
                engine.placement(),
                &policy(),
                &BlockLimits::default(),
            )
            .unwrap();
            for (index, address) in [addresses[0], addresses[1], addresses[0], addresses[2]]
                .into_iter()
                .enumerate()
            {
                if index == 2 {
                    arena
                        .protect(data, PAGE, NativePermissions::READ_WRITE)
                        .unwrap();
                    arena
                        .pause()
                        .unwrap()
                        .write(addresses[0], &inputs[2].to_le_bytes()[..size as usize])
                        .unwrap();
                    arena.protect(data, PAGE, NativePermissions::READ).unwrap();
                }
                let current = input(state, double, values, address, 0x1f84, true);
                if index != 0 {
                    engine
                        .begin_call(&mut arena.pause().unwrap(), current)
                        .unwrap();
                }
                // SAFETY: Current full-span live reads are independently checked even on reuse.
                unsafe { engine.step(&mut arena.pause().unwrap(), &plan) }.unwrap();
                let right = if index == 1 {
                    inputs[1]
                } else if index == 2 {
                    inputs[2]
                } else {
                    inputs[0]
                };
                assert_eq!(*engine.state(), expected(current, 0x59, double, right, 9));
            }
            assert_eq!(engine.usage().native_entries, 4);
            assert_eq!(engine.usage().cache_publications, 1);
            assert_eq!(engine.usage().cache_reuses, 3);
            let current = input(state, double, values, addresses[2], 0x1f84, true);
            engine
                .begin_call(&mut arena.pause().unwrap(), current)
                .unwrap();
            arena
                .protect(
                    data + PAGE,
                    PAGE,
                    NativePermissions::new(false, false, false),
                )
                .unwrap();
            // SAFETY: Revoked second-page permission refuses the sealed cached body before entry.
            let refused = unsafe { engine.step(&mut arena.pause().unwrap(), &plan) };
            assert!(matches!(refused, Err(NativeDispatchError::Memory(_))));
            assert_eq!(*engine.state(), current);
            assert_eq!(engine.usage().native_entries, 4);
            assert_eq!(engine.usage().cache_reuses, 3);
            arena
                .protect(data + PAGE, PAGE, NativePermissions::READ)
                .unwrap();
            // SAFETY: The same pending call now has full live rights again.
            unsafe { engine.step_checked(&mut arena.pause().unwrap(), &plan) }.unwrap();
            assert_eq!(
                *engine.state(),
                expected(current, 0x59, double, inputs[0], 9)
            );
            assert_eq!(engine.usage().native_entries, 5);
            assert_eq!(engine.usage().cache_reuses, 4);
        });
    }
}

#[test]
fn both_publishers_require_separate_request_full_context_and_actual_mode() {
    let code = [0xb8, 42, 0, 0, 0, 0xc3];
    with_source(Some(&code), |source, arena, stack, state| {
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        for (requested, mode) in [
            (
                TranslationPolicy::x64_sse2().with_native_scalar_float(),
                VectorMode::Sse2,
            ),
            (
                TranslationPolicy::x64_sse2().with_native_scalar_float(),
                VectorMode::Avx {
                    avx2: false,
                    fma: false,
                },
            ),
            (policy(), VectorMode::Sse2),
        ] {
            engine.machine.vector_mode = mode;
            let prepared = plan_materialized_block(
                source,
                0,
                state.guest_pc,
                &code,
                engine.placement(),
                &requested,
                &BlockLimits::default(),
            )
            .unwrap();
            assert_eq!(
                engine.publish(&arena.pause().unwrap(), &prepared),
                Err(NativeDispatchError::InvalidState)
            );
            let diagnostic = pentax_codegen::plan_block(
                source.source_plan(),
                0,
                state.guest_pc,
                engine.placement(),
                &requested,
                &BlockLimits::default(),
            )
            .unwrap();
            assert!(matches!(
                super::super::super::publish_block_with_mode(
                    &mut engine.machine.cache,
                    arena,
                    &diagnostic,
                    state.guest_pc,
                    mode
                ),
                Err(PlatformError::UnsupportedHostFeature {
                    feature: "native YMM context"
                })
            ));
        }
        assert_eq!(engine.usage().native_entries, 0);
        assert_eq!(engine.usage().cache_publications, 0);
    });
}
