// SPDX-License-Identifier: GPL-2.0-only
//! Original active-bank freshness, actual entry and terminal-capture contracts.
use super::*;
use pentax_codegen::{CpuFeatures, CpuidFeature};

fn mode(vector: bool) -> VectorMode {
    if vector {
        VectorMode::Avx {
            avx2: false,
            fma: false,
        }
    } else {
        VectorMode::Sse2
    }
}
fn policy(vector: bool) -> TranslationPolicy {
    let selected = if vector {
        TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
            .with_native_ymm_registers()
    } else {
        TranslationPolicy::x64_sse2()
    };
    selected.with_independent_memory().with_affine_loop_trace()
}
fn inputs(mut state: NativeCpuState, salt: u8) -> NativeCpuState {
    for (index, value) in state.gpr.iter_mut().enumerate() {
        if index != RSP {
            *value = 0x1122_3344_5500_0000 ^ (u64::from(salt) << 32) ^ (index as u64 * 0x101);
        }
    }
    state.rflags = 0x202 | if salt & 1 == 0 { 0x8d5 } else { 0x400 };
    state.mxcsr = [0x1f80, 0x3fa5, 0xdfbf, 0xffbf][salt as usize % 4];
    for index in 0..256 {
        state.xmm[index] = salt.wrapping_mul(53) ^ (index as u8).wrapping_mul(19);
        state.ymm_upper[index] = !salt.wrapping_mul(37) ^ (index as u8).wrapping_mul(11);
    }
    state
}
fn moves(vector: bool) -> Vec<u8> {
    let mut code = vec![0x4d, 0x89, 0xc8]; // MOV R8,R9.
    if vector {
        code.extend([0xc4, 0x41, 0x7e, 0x6f, 0xcd]); // VMOVDQU YMM9,YMM13.
        code.extend([0xc4, 0x41, 0x7a, 0x6f, 0xfe]); // VMOVDQU XMM15,XMM14; upper zero.
    } else {
        code.extend([0x66, 0x45, 0x0f, 0x6f, 0xcd]); // MOVDQA XMM9,XMM13.
        code.extend([0x66, 0x45, 0x0f, 0x6f, 0xfe]); // MOVDQA XMM15,XMM14.
    }
    code
}
fn expected_moves(input: NativeCpuState, vector: bool) -> NativeCpuState {
    let mut expected = input;
    expected.gpr[8] = input.gpr[9];
    expected.xmm[9 * 16..10 * 16].copy_from_slice(&input.xmm[13 * 16..14 * 16]);
    expected.xmm[15 * 16..16 * 16].copy_from_slice(&input.xmm[14 * 16..15 * 16]);
    if vector {
        expected.ymm_upper[9 * 16..10 * 16].copy_from_slice(&input.ymm_upper[13 * 16..14 * 16]);
        expected.ymm_upper[15 * 16..16 * 16].fill(0);
    }
    expected
}
fn planned<'v, 'p, 'i>(
    source: &'v MaterializedSource<'p, 'i>,
    pc: u64,
    code: &[u8],
    placement: CachePlacement,
    vector: bool,
) -> MaterializedBlockPlan<'v, 'p, 'i> {
    plan_materialized_block(
        source,
        0,
        pc,
        code,
        placement,
        &policy(vector),
        &Default::default(),
    )
    .unwrap()
}
fn assert_reset() {
    let _guard = DispatchGuard::acquire().unwrap();
    assert_eq!(
        std::mem::size_of::<super::super::super::GuestContext>(),
        944
    );
    let mut expected = [0_u8; 944];
    expected[136..144].copy_from_slice(&0x202_u64.to_ne_bytes());
    expected[168..172].copy_from_slice(&0x1f80_u32.to_ne_bytes());
    // SAFETY: The guard excludes assembly and other context users. The fixed
    // repr(C) record has explicit padding and every byte was initialized.
    let actual = unsafe { std::slice::from_raw_parts(CONTEXT.0.get().cast::<u8>(), 944) };
    assert_eq!(actual, expected);
}

#[test]
fn guarded_staging_refreshes_all_active_fields_and_drop_resets_every_context_byte() {
    for vector in [false, true] {
        for salt in [0_u8, 1, 2, 3] {
            let guard = DispatchGuard::acquire().unwrap();
            // SAFETY: The actual record contains only integers/byte arrays,
            // with every bit pattern valid. No assembly runs on this poison.
            unsafe {
                std::ptr::write_bytes(CONTEXT.0.get().cast::<u8>(), 0xa5, 944);
            }
            let state = inputs(NativeCpuState::new(0x4000, 0x8000), salt);
            native_context::stage(&guard, mode(vector), &state, 0xa000, 0x6000, 0x10000);
            {
                // SAFETY: Guard held and no assembly call is in flight.
                let actual = unsafe { &*CONTEXT.0.get() };
                assert_eq!(actual.gpr, state.gpr);
                assert_eq!(actual.rflags, state.rflags);
                assert_eq!(actual.mxcsr, state.mxcsr);
                assert_eq!(
                    (actual.guest_base, actual.guest_limit, actual.rip),
                    (0xa000, 0x6000, 0x10000)
                );
                assert_eq!(actual.host_rsp, 0xa5a5_a5a5_a5a5_a5a5);
                assert_eq!(actual._pad, 0xa5a5_a5a5);
                if vector {
                    assert_eq!(actual.xmm, [0xa5; 256]);
                    for register in 0..16 {
                        assert_eq!(
                            actual.ymm[register * 32..register * 32 + 16],
                            state.xmm[register * 16..register * 16 + 16]
                        );
                        assert_eq!(
                            actual.ymm[register * 32 + 16..register * 32 + 32],
                            state.ymm_upper[register * 16..register * 16 + 16]
                        );
                    }
                } else {
                    assert_eq!(actual.xmm, state.xmm);
                    assert_eq!(actual.ymm, [0xa5; 512]);
                }
            }
            assert!(matches!(
                DispatchGuard::acquire(),
                Err(PlatformError::TrapScopeUnavailable)
            ));
            let mut captured = inputs(NativeCpuState::new(0x5000, 0x9000), salt ^ 0xff);
            let inactive_upper = captured.ymm_upper;
            native_context::capture(&guard, mode(vector), &mut captured);
            assert_eq!(captured.gpr, state.gpr);
            assert_eq!(captured.rflags, state.rflags);
            assert_eq!(captured.mxcsr, state.mxcsr);
            assert_eq!(captured.xmm, state.xmm);
            assert_eq!(
                captured.ymm_upper,
                if vector {
                    state.ymm_upper
                } else {
                    inactive_upper
                }
            );
            assert_eq!(captured.guest_pc, 0x5000); // Ordinary capture does not read private RIP.
            assert_eq!(native_context::affine_iterations(&guard), 0x10000);
            drop(guard);
            assert_reset();
        }
    }
}

#[test]
fn actual_ordinary_and_affine_entries_keep_consecutive_full_states_and_cleanup() {
    let vector_available = VectorMode::detect().features().is_some();
    let mut cases = 0;
    for vector in [false, true] {
        if vector && !vector_available {
            continue;
        }
        let mut code = moves(vector);
        code.push(0xc3);
        with_source(Some(&code), |source, arena, stack, state| {
            let initial = inputs(state, 0);
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                initial,
                stack,
                &Default::default(),
            )
            .unwrap();
            engine.machine.vector_mode = mode(vector);
            let block = planned(source, state.guest_pc, &code, engine.placement(), vector);
            for salt in 0..4 {
                let input = inputs(state, salt);
                if salt != 0 {
                    engine
                        .begin_call(&mut arena.pause().unwrap(), input)
                        .unwrap();
                }
                let before = engine.usage();
                // SAFETY: Original owned register-only body and actual admitted mode.
                let stop =
                    unsafe { engine.step_checked(&mut arena.pause().unwrap(), &block) }.unwrap();
                assert_eq!(
                    stop,
                    NativeBlockStop::Returned {
                        value: input.gpr[RAX]
                    }
                );
                let mut expected = expected_moves(input, vector);
                expected.gpr[RSP] += 8;
                expected.guest_pc = stack.return_marker;
                assert_eq!(*engine.state(), expected);
                assert_eq!(engine.usage().native_entries - before.native_entries, 1);
                assert_eq!(
                    engine.usage().native_instructions - before.native_instructions,
                    3
                );
                assert_reset();
                cases += 1;
            }
        });
        let mut code = vec![0x8b, 0x14, 0x3b, 0x89, 0x14, 0x3b]; // Full32-bit load/store [RBX+RDI].
        code.extend(moves(vector));
        code.extend([0x48, 0x83, 0xc7, 4, 0x48, 0x39, 0xcf]); // ADD RDI,4; CMP RDI,RCX.
        let back = i8::try_from(-(code.len() as i64 + 2)).unwrap();
        code.extend([0x75, back as u8, 0xc3]);
        with_source(Some(&code), |source, arena, stack, state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + 64;
            let mut initial = inputs(state, 0);
            initial.gpr[3] = data;
            initial.gpr[RDI] = 0;
            initial.gpr[RCX] = 4;
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                initial,
                stack,
                &Default::default(),
            )
            .unwrap();
            engine.machine.vector_mode = mode(vector);
            let mut block = planned(source, state.guest_pc, &code, engine.placement(), vector);
            let limits = BlockLimits::default();
            let allowance = block.affine_loop_trace_work_bound(&limits).unwrap();
            assert!(
                block.prepare_affine_loop_trace(&limits, allowance, limits.max_allocation_bytes)
                    > 0
            );
            assert_eq!(block.native_admission().emitted_instructions(), 7);
            let ret_pc = state.guest_pc + code.len() as u64 - 1;
            let ret = planned(source, ret_pc, &[0xc3], engine.placement(), vector);
            let values = [0x1122_3344_u32, 0x5566_7788];
            for iterations in 1..=2_u64 {
                let mut input = inputs(state, (iterations - 1) as u8);
                input.gpr[3] = data;
                input.gpr[RDI] = 0;
                input.gpr[RCX] = 4 * iterations;
                if iterations != 1 {
                    engine
                        .begin_call(&mut arena.pause().unwrap(), input)
                        .unwrap();
                }
                for (index, value) in values.iter().enumerate() {
                    arena
                        .pause()
                        .unwrap()
                        .write(data + index as u64 * 4, &value.to_le_bytes())
                        .unwrap();
                }
                let before = engine.usage();
                // SAFETY: Typed finite trace, both current spans and source/owner checks.
                let stop = unsafe {
                    engine.try_step_affine_loop_checked(
                        &mut arena.pause().unwrap(),
                        &block.affine_loop_trace().unwrap(),
                        2,
                    )
                }
                .unwrap()
                .unwrap();
                assert_eq!(stop, NativeBlockStop::Resume { guest_pc: ret_pc });
                let mut expected = expected_moves(input, vector);
                expected.gpr[RDX] = u64::from(values[iterations as usize - 1]);
                expected.gpr[RDI] = iterations * 4;
                expected.rflags = (input.rflags & !0x8d5) | 0x44;
                expected.guest_pc = ret_pc;
                assert_eq!(*engine.state(), expected);
                assert_eq!(engine.usage().native_entries - before.native_entries, 1);
                assert_eq!(
                    engine.usage().affine_loop_entries - before.affine_loop_entries,
                    1
                );
                assert_eq!(
                    engine.usage().affine_loop_iterations - before.affine_loop_iterations,
                    iterations
                );
                assert_eq!(engine.usage().blocks - before.blocks, iterations);
                assert_eq!(
                    engine.usage().native_instructions - before.native_instructions,
                    7 * iterations
                );
                assert_reset();
                // SAFETY: Actual retained owned outer return; no native body.
                let stop =
                    unsafe { engine.step_checked(&mut arena.pause().unwrap(), &ret) }.unwrap();
                assert_eq!(
                    stop,
                    NativeBlockStop::Returned {
                        value: input.gpr[RAX]
                    }
                );
                expected.gpr[RSP] += 8;
                expected.guest_pc = stack.return_marker;
                assert_eq!(*engine.state(), expected);
                assert_reset();
                cases += 1;
            }
        });
    }
    assert_eq!(cases, if vector_available { 12 } else { 6 });
    println!(
        "native_context_staging_available=true native_context_staging_vector_available={vector_available} native_context_staging_cases={cases}"
    );
}

#[test]
fn actual_postentry_return_failure_retains_captured_state_and_cannot_replay() {
    for vector in [false, true] {
        if vector && VectorMode::detect().features().is_none() {
            continue;
        }
        let mut code = moves(vector);
        code.push(0xc3);
        with_source(Some(&code), |source, arena, stack, state| {
            let input = inputs(state, 3);
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                input,
                stack,
                &Default::default(),
            )
            .unwrap();
            engine.machine.vector_mode = mode(vector);
            let block = planned(source, state.guest_pc, &code, engine.placement(), vector);
            // Constructor consumed the real marker. This later owned change
            // causes RET resolution to fail only after the native body returns.
            let invalid_target = u64::MAX;
            arena
                .pause()
                .unwrap()
                .write(stack.return_slot, &invalid_target.to_le_bytes())
                .unwrap();
            // SAFETY: Current body/source/stack spans remain genuinely owned;
            // only the later modeled return target is deliberately invalid.
            let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &block) };
            assert_eq!(
                stop,
                Err(NativeDispatchError::InvalidTarget(invalid_target))
            );
            let mut expected = expected_moves(input, vector);
            expected.guest_pc = state.guest_pc + code.len() as u64 - 1;
            assert_eq!(*engine.state(), expected);
            assert_eq!(engine.usage().native_entries, 1);
            assert_eq!(engine.usage().native_instructions, 3);
            assert!(engine.machine.phase == Phase::Terminal);
            assert_reset();
            let before = engine.usage();
            arena
                .pause()
                .unwrap()
                .write(stack.return_slot, &stack.return_marker.to_le_bytes())
                .unwrap();
            // SAFETY: Repairing data cannot authorize replay of retired effects.
            let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &block) };
            assert_eq!(stop, Err(NativeDispatchError::WrongPhase));
            assert_eq!(engine.usage(), before);
            assert_eq!(*engine.state(), expected);
            assert_reset();
        });
    }
}
