// SPDX-License-Identifier: GPL-2.0-only
//! Original exact scalar FMA inputs, full-state equivalence and admission.
use super::*;
use pentax_codegen::{CpuFeatures, CpuidFeature};

const ALIASES: [[u8; 3]; 6] = [
    [9, 13, 14],
    [13, 13, 14],
    [14, 13, 14],
    [9, 9, 14],
    [9, 13, 9],
    [9, 9, 9],
];
// Algebraic x=SRC2, y=old DEST, z=SRC3. All bits are original constants.
const INPUTS: [[u32; 3]; 18] = [
    [0x4040_0000, 0x4000_0000, 0x4080_0000], // 3*2+4 / -(3*2)+4.
    [0x3f7f_fffe, 0x3f80_0001, 0xbf80_0000], // Fused -2^-46.
    [0x3f7f_fffe, 0x3f80_0001, 0x3f80_0000], // Negative fused +2^-46.
    [0x7fc0_0011, 0xffa0_0022, 0x7fc0_0033],
    [0x3f80_0000, 0xffc0_0022, 0x7fa0_0033],
    [0x3f80_0000, 0x4000_0000, 0x7fa0_0033],
    [0x7fa0_0011, 0xffc0_0022, 0x7fc0_0033],
    [0x7f80_0000, 0, 0x7fc0_0055], // NaN suppresses inf*zero invalid.
    [0x7f80_0000, 0, 0],
    [0x7f80_0000, 0x3f80_0000, 0xff80_0000],
    [1, 0x3f80_0000, 0],
    [0x0080_0001, 0x3f7f_fffe, 0], // Unbounded rounding makes normal.
    [0x3f7f_ffff, 0x0080_0000, 0], // Tiny before final subnormal rounding.
    [0x7f7f_ffff, 0x4000_0000, 0],
    [0x0080_0000, 0x3eaa_aaab, 0],
    [0x8000_0000, 0x3f80_0000, 0x8000_0000],
    [0x3f80_0000, 0x3f80_0000, 0xbf80_0000],
    [0x3f80_0000, 0x3f80_0000, 0x3380_0000],
];

fn policy(native: bool) -> TranslationPolicy {
    let policy = TranslationPolicy::new(
        CpuFeatures::x64_sse2()
            .with(CpuidFeature::AVX)
            .with(CpuidFeature::FMA),
    )
    .with_native_ymm_registers();
    if native {
        policy.with_native_scalar_fma()
    } else {
        policy
    }
}
fn encoding(opcode: u8, registers: [u8; 3]) -> [u8; 5] {
    let [destination, second, third] = registers;
    [
        0xc4,
        2 | ((!destination & 8) << 4) | 0x40 | ((!third & 8) << 2),
        ((!second & 15) << 3) | 1,
        opcode,
        0xc0 | ((destination & 7) << 3) | (third & 7),
    ]
}
fn host_controls() -> (u32, u16) {
    let mut mxcsr = 0_u32;
    let mut x87 = 0_u16;
    // SAFETY: Mandatory control stores to exact local objects; no mutation.
    unsafe {
        std::arch::asm!("stmxcsr [{mx}]", "fnstcw [{cw}]",
            mx = in(reg) &mut mxcsr, cw = in(reg) &mut x87,
            options(nostack, preserves_flags));
    }
    (mxcsr, x87)
}
fn input(
    mut state: NativeCpuState,
    registers: [u8; 3],
    values: [u32; 3],
    control: u32,
) -> NativeCpuState {
    for (index, value) in state.gpr.iter_mut().enumerate() {
        if index != RSP {
            *value = 0x1020_3040_5060_7080 ^ index as u64;
        }
    }
    state.gpr[RAX] = 42;
    state.rflags = 0x202 | 0x8d5 | 0x400;
    state.mxcsr = control;
    for (index, byte) in state.xmm.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(17).wrapping_add(3);
    }
    for (index, byte) in state.ymm_upper.iter_mut().enumerate() {
        *byte = !(index as u8).wrapping_mul(13);
    }
    // Last write intentionally establishes old DEST for all alias patterns.
    for (register, low, tag) in [
        (registers[1], values[0], 0x1111_u32),
        (registers[2], values[2], 0x2222),
        (registers[0], values[1], 0x3333),
    ] {
        let at = register as usize * 16;
        state.xmm[at..at + 4].copy_from_slice(&low.to_le_bytes());
        for lane in 1..4 {
            // Inactive signaling NaNs must be preserved, never evaluated.
            state.xmm[at + lane * 4..at + lane * 4 + 4]
                .copy_from_slice(&(0x7f80_0000 | tag | lane as u32).to_le_bytes());
        }
    }
    state
}
fn apply(mut state: NativeCpuState, opcode: u8, registers: [u8; 3]) -> NativeCpuState {
    let low = |register: u8| {
        u32::from_le_bytes(
            state.xmm[register as usize * 16..register as usize * 16 + 4]
                .try_into()
                .unwrap(),
        )
    };
    let [x, y, z] = [low(registers[1]), low(registers[0]), low(registers[2])];
    let (bits, control) = fused_float::calculate(
        false,
        x as u64,
        y as u64,
        z as u64,
        opcode == 0xad,
        false,
        state.mxcsr,
    )
    .unwrap();
    let nan = |value: u32| value & 0x7f80_0000 == 0x7f80_0000 && value & 0x007f_ffff != 0;
    if let Some(value) = [x, y, z].into_iter().find(|&value| nan(value)) {
        assert_eq!(bits, u64::from(value | 0x0040_0000));
        let signaling = [x, y, z]
            .into_iter()
            .any(|value| nan(value) && value & 0x0040_0000 == 0);
        assert_eq!(control & 1, (state.mxcsr & 1) | u32::from(signaling));
    }
    if [x, y, z] == INPUTS[0] {
        assert_eq!(
            bits,
            if opcode == 0xa9 {
                0x4120_0000
            } else {
                0xc000_0000
            }
        );
        assert_eq!(control, state.mxcsr); // Inactive sNaNs do not raise IE.
    }
    if opcode == 0xa9 && [x, y, z] == INPUTS[1] {
        assert_eq!(bits, 0xa880_0000);
    }
    if opcode == 0xad && [x, y, z] == INPUTS[2] {
        assert_eq!(bits, 0x2880_0000);
    }
    let at = registers[0] as usize * 16;
    state.xmm[at..at + 4].copy_from_slice(&(bits as u32).to_le_bytes());
    state.ymm_upper[at..at + 16].fill(0);
    state.mxcsr = control;
    state
}
fn returned(mut state: NativeCpuState) -> NativeCpuState {
    state.gpr[RSP] += 8;
    state.guest_pc = STOP;
    state
}

#[test]
fn scalar_fma_matches_checked_software_full_state_rounding_nan_order_and_aliases() {
    if !VectorMode::detect().supports(CpuidFeature::FMA) {
        println!(
            "native_scalar_fma_available=false native_scalar_fma_cases=0 native_scalar_fma_mode=software"
        );
        return;
    }
    let controls: Vec<_> = (0..16_u32)
        .map(|bits| 0x1f80 | ((bits & 3) << 13) | ((bits & 4) << 4) | ((bits & 8) << 12))
        .filter(|&control| mxcsr_control::valid(control))
        .collect();
    assert!(!controls.is_empty());
    let mut cases = 0_u64;
    for opcode in [0xa9, 0xad] {
        let mut code = Vec::new();
        for registers in ALIASES {
            code.extend(encoding(opcode, registers));
            code.push(0xc3);
        }
        with_source(Some(&code), |source, arena, stack, state| {
            let initial = input(state, ALIASES[0], INPUTS[0], controls[0]);
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                initial,
                stack,
                &Default::default(),
            )
            .unwrap();
            // Actual AVX+FMA was verified above. AVX2 authority is disabled.
            engine.machine.vector_mode = VectorMode::Avx {
                avx2: false,
                fma: true,
            };
            let mut calls = 0_u64;
            for (alias, registers) in ALIASES.into_iter().enumerate() {
                let pc = state.guest_pc + (alias * 6) as u64;
                let bytes = &code[alias * 6..alias * 6 + 6];
                let native = plan_materialized_block(
                    source,
                    0,
                    pc,
                    bytes,
                    engine.placement(),
                    &policy(true),
                    &Default::default(),
                )
                .unwrap();
                let software = plan_materialized_block(
                    source,
                    0,
                    pc,
                    bytes,
                    engine.placement(),
                    &policy(false),
                    &Default::default(),
                )
                .unwrap();
                let ret = plan_materialized_block(
                    source,
                    0,
                    pc + 5,
                    &bytes[5..],
                    engine.placement(),
                    &policy(false),
                    &Default::default(),
                )
                .unwrap();
                assert!(native.instructions()[0].emitted_len() > 0);
                assert_eq!(software.instructions()[0].emitted_len(), 0);
                for values in INPUTS {
                    for &control in &controls {
                        let mut current = input(state, registers, values, control);
                        current.guest_pc = pc;
                        let expected = returned(apply(current, opcode, registers));
                        for use_native in [true, false] {
                            if calls != 0 {
                                engine
                                    .begin_call(&mut arena.pause().unwrap(), current)
                                    .unwrap();
                            }
                            let before = engine.usage();
                            let host = host_controls();
                            // SAFETY: Original same-owner source, exact live stack,
                            // masked guest controls and actual AVX/FMA capability.
                            unsafe {
                                if use_native {
                                    assert_eq!(
                                        engine
                                            .step_checked(&mut arena.pause().unwrap(), &native)
                                            .unwrap(),
                                        NativeBlockStop::Returned { value: 42 }
                                    );
                                } else {
                                    assert_eq!(
                                        engine
                                            .step_checked(&mut arena.pause().unwrap(), &software)
                                            .unwrap(),
                                        NativeBlockStop::Resume { guest_pc: pc + 5 }
                                    );
                                    assert_eq!(
                                        engine
                                            .step_checked(&mut arena.pause().unwrap(), &ret)
                                            .unwrap(),
                                        NativeBlockStop::Returned { value: 42 }
                                    );
                                }
                            }
                            assert_eq!(*engine.state(), expected);
                            assert_eq!(host_controls(), host);
                            let after = engine.usage();
                            assert_eq!(
                                after.native_entries - before.native_entries,
                                u64::from(use_native)
                            );
                            assert_eq!(
                                after.native_instructions - before.native_instructions,
                                u64::from(use_native)
                            );
                            assert_eq!(
                                after.vector_operations - before.vector_operations,
                                u64::from(!use_native)
                            );
                            assert_eq!(after.vector_bytes, before.vector_bytes);
                            assert_eq!(
                                after.blocks - before.blocks,
                                if use_native { 1 } else { 2 }
                            );
                            calls += 1;
                        }
                        cases += 1;
                    }
                }
            }
            assert_eq!(engine.usage().native_entries, calls / 2);
            assert_eq!(engine.usage().native_instructions, calls / 2);
            assert_eq!(engine.usage().vector_operations, calls / 2);
        });
    }
    assert_eq!(
        cases,
        2 * ALIASES.len() as u64 * INPUTS.len() as u64 * controls.len() as u64
    );
    println!(
        "native_scalar_fma_available=true native_scalar_fma_cases={cases} native_scalar_fma_mode=avx-fma"
    );
}

#[test]
fn scalar_fma_mode_source_controls_and_later_memory_refuse_before_native_effects() {
    for opcode in [0xa9, 0xad] {
        for case in 0..10 {
            if case >= 6 && !VectorMode::detect().supports(CpuidFeature::FMA) {
                continue;
            }
            let mut code = encoding(opcode, ALIASES[0]).to_vec();
            if case == 7 {
                code.extend([0x8b, 0x00]);
            } // Later ordinary MOV EAX,[RAX].
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, state| {
                let mut current = input(state, ALIASES[0], INPUTS[3], 0x1f80);
                current.gpr[RAX] = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE;
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
                    &policy(true).with_independent_memory(),
                    &Default::default(),
                )
                .unwrap();
                assert!(plan.instructions()[0].emitted_len() > 0);
                if case == 7 {
                    assert!(plan.instructions()[1].emitted_len() > 0);
                }
                match case {
                    0 => engine.machine.vector_mode = VectorMode::Sse2,
                    1 => {
                        engine.machine.vector_mode = VectorMode::Avx {
                            avx2: true,
                            fma: false,
                        }
                    }
                    2 => engine.machine.state.mxcsr &= !0x80,
                    3 => engine.machine.state.mxcsr |= 1 << 16,
                    4 => engine.machine.limits.max_work_units = 0,
                    5 => engine.machine.usage.native_instructions = u64::MAX,
                    6 => arena
                        .protect(
                            current.guest_pc & !(PAGE - 1),
                            PAGE,
                            NativePermissions::READ_WRITE,
                        )
                        .unwrap(),
                    9 => {
                        let page = current.guest_pc & !(PAGE - 1);
                        arena
                            .protect(page, PAGE, NativePermissions::READ_WRITE)
                            .unwrap();
                        arena
                            .pause()
                            .unwrap()
                            .write(current.guest_pc, &[0x90])
                            .unwrap();
                        arena
                            .protect(page, PAGE, NativePermissions::READ_EXECUTE)
                            .unwrap();
                    }
                    _ => {}
                }
                let before = *engine.state();
                let host = host_controls();
                let result = if case == 8 {
                    let mut other =
                        NativeAddressSpace::reserve_any(Default::default(), 65_536).unwrap();
                    // SAFETY: Deliberate wrong-owner refusal before entry.
                    unsafe { engine.step_checked(&mut other.pause().unwrap(), &plan) }
                } else {
                    // SAFETY: Controlled original invalid mode/source/control/quota/access.
                    unsafe { engine.step_checked(&mut arena.pause().unwrap(), &plan) }
                };
                match case {
                    0..=3 => assert_eq!(result, Err(NativeDispatchError::InvalidState)),
                    4 | 5 => assert!(matches!(result, Err(NativeDispatchError::Budget(_)))),
                    6 => assert_eq!(result, Err(NativeDispatchError::WritableSource)),
                    7 => assert!(matches!(result, Err(NativeDispatchError::Memory(_)))),
                    8 => assert_eq!(result, Err(NativeDispatchError::WrongOwner)),
                    9 => assert_eq!(result, Err(NativeDispatchError::SourceMismatch)),
                    _ => unreachable!(),
                }
                assert_eq!(*engine.state(), before);
                assert_eq!(host_controls(), host);
                assert_eq!(engine.usage().native_entries, 0);
                assert_eq!(engine.usage().cache_publications, 0);
                assert_eq!(engine.usage().vector_operations, 0);
            });
        }
    }
}

#[test]
fn both_fma_publishers_and_feature_masks_preserve_full_context_and_independent_fma_gates() {
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
        for (requested, mode, feature) in [
            (
                TranslationPolicy::x64_sse2().with_native_scalar_fma(),
                VectorMode::Avx {
                    avx2: false,
                    fma: true,
                },
                "native YMM context",
            ),
            (policy(true), VectorMode::Sse2, "native YMM context"),
            (
                policy(true),
                VectorMode::Avx {
                    avx2: true,
                    fma: false,
                },
                "native scalar FMA profile",
            ),
        ] {
            engine.machine.vector_mode = mode;
            let plan = plan_materialized_block(
                source,
                0,
                state.guest_pc,
                &code,
                engine.placement(),
                &requested,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                engine.publish(&arena.pause().unwrap(), &plan),
                Err(NativeDispatchError::InvalidState)
            );
            let diagnostic = pentax_codegen::plan_block(
                source.source_plan(),
                0,
                state.guest_pc,
                engine.placement(),
                &requested,
                &Default::default(),
            )
            .unwrap();
            assert!(
                matches!(super::super::super::publish_block_with_mode(&mut engine.machine.cache, arena, &diagnostic, state.guest_pc, mode), Err(PlatformError::UnsupportedHostFeature { feature: actual }) if actual == feature)
            );
        }
        assert_eq!(*engine.state(), state);
        assert_eq!(engine.usage().native_entries, 0);
        assert_eq!(engine.usage().cache_publications, 0);
    });
    let code = [0xc4, 0x42, 0x11, 0xa9, 0xce, 0xc3];
    with_source(Some(&code), |source, arena, stack, state| {
        let engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
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
            &policy(true),
            &Default::default(),
        )
        .unwrap();
        for fma in [true, false, true, false] {
            for avx2 in [false, true] {
                let mode = VectorMode::Avx { avx2, fma };
                assert_eq!(mode.supports_admission(plan.native_admission()), fma);
                assert_eq!(mode.features().unwrap().contains(CpuidFeature::FMA), fma);
            }
        }
        assert!(!VectorMode::Sse2.supports_admission(plan.native_admission()));
    });
}

#[test]
#[ignore = "finite original software/native comparison; requires actual admitted AVX and FMA"]
fn scalar_fma_original_finite_coalescing_measurement() {
    if !VectorMode::detect().supports(CpuidFeature::FMA) {
        println!("native_scalar_fma_measurement_available=false");
        return;
    }
    const CALLS: u64 = 50_000;
    let mut code = encoding(0xa9, ALIASES[0]).to_vec();
    code.extend(encoding(0xad, ALIASES[0]));
    code.push(0xc3);
    for use_native in [false, true] {
        with_source(Some(&code), |source, arena, stack, state| {
            let current = input(state, ALIASES[0], INPUTS[0], 0x1f80);
            let limits = NativeDispatchLimits {
                max_blocks: CALLS * 3,
                max_work_units: 1024 * 1024 * 1024,
                ..Default::default()
            };
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                current,
                stack,
                &limits,
            )
            .unwrap();
            engine.machine.vector_mode = VectorMode::Avx {
                avx2: false,
                fma: true,
            };
            let selected = policy(use_native);
            let first = plan_materialized_block(
                source,
                0,
                state.guest_pc,
                &code,
                engine.placement(),
                &selected,
                &Default::default(),
            )
            .unwrap();
            let second = plan_materialized_block(
                source,
                0,
                state.guest_pc + 5,
                &code[5..],
                engine.placement(),
                &selected,
                &Default::default(),
            )
            .unwrap();
            let ret = plan_materialized_block(
                source,
                0,
                state.guest_pc + 10,
                &code[10..],
                engine.placement(),
                &selected,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                first.native_admission().emitted_instructions(),
                if use_native { 2 } else { 0 }
            );
            let expected = returned(apply(apply(current, 0xa9, ALIASES[0]), 0xad, ALIASES[0]));
            assert_eq!(
                u32::from_le_bytes(expected.xmm[144..148].try_into().unwrap()),
                0xc1d0_0000
            ); // -(3*10)+4 = -26.
            let host = host_controls();
            let start = std::time::Instant::now();
            for call in 0..CALLS {
                if call != 0 {
                    engine
                        .begin_call(&mut arena.pause().unwrap(), current)
                        .unwrap();
                }
                // SAFETY: Same original bounded two-FMA program, actual mode,
                // exact live source/stack and all-masked controls on every call.
                unsafe {
                    let stop = engine
                        .step_checked(&mut arena.pause().unwrap(), &first)
                        .unwrap();
                    if use_native {
                        assert_eq!(stop, NativeBlockStop::Returned { value: 42 });
                    } else {
                        assert_eq!(
                            stop,
                            NativeBlockStop::Resume {
                                guest_pc: state.guest_pc + 5
                            }
                        );
                        assert_eq!(
                            engine
                                .step_checked(&mut arena.pause().unwrap(), &second)
                                .unwrap(),
                            NativeBlockStop::Resume {
                                guest_pc: state.guest_pc + 10
                            }
                        );
                        assert_eq!(
                            engine
                                .step_checked(&mut arena.pause().unwrap(), &ret)
                                .unwrap(),
                            NativeBlockStop::Returned { value: 42 }
                        );
                    }
                }
                assert_eq!(*engine.state(), expected);
            }
            let elapsed = start.elapsed().as_nanos();
            assert_eq!(host_controls(), host);
            let usage = engine.usage();
            assert_eq!(usage.blocks, CALLS * if use_native { 1 } else { 3 });
            assert_eq!(usage.native_entries, CALLS * u64::from(use_native));
            assert_eq!(usage.native_instructions, CALLS * 2 * u64::from(use_native));
            assert_eq!(usage.vector_operations, CALLS * 2 * u64::from(!use_native));
            assert_eq!(usage.vector_bytes, 0);
            println!(
                "native_scalar_fma_measurement_available=true native_scalar_fma_measurement_mode={} calls={CALLS} elapsed_ns={elapsed} blocks={} entries={} native_instructions={} vector_ops={} publications={} reuses={} work={}",
                if use_native { "native" } else { "software" },
                usage.blocks,
                usage.native_entries,
                usage.native_instructions,
                usage.vector_operations,
                usage.cache_publications,
                usage.cache_reuses,
                usage.work_units
            );
        });
    }
}
