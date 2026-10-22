// SPDX-License-Identifier: GPL-2.0-only
//! Original register loops compare actual finite native traces with checked steps.
use super::*;
use pentax_codegen::{CpuFeatures, CpuidFeature};

fn policy(vector: u8) -> TranslationPolicy {
    let selected = if vector == 0 {
        TranslationPolicy::x64_sse2()
    } else {
        TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
            .with_native_ymm_registers()
    };
    selected.with_register_loop_trace()
}
fn program(vector: u8) -> Vec<u8> {
    // AH aliases AL/EAX and ADC consumes the real preceding carry. DEC preserves
    // CF while producing the actual self-backedge condition; no predicted GPRs.
    let mut code = vec![0x00, 0xc4, 0x11, 0xc3]; // ADD AH,AL; ADC EBX,EAX.
    match vector {
        0 => code.extend([0x45, 0x0f, 0x58, 0xcd]), // ADDPS XMM9,XMM13.
        1 => code.extend([0xc4, 0x41, 0x30, 0x58, 0xcd]), // VADDPS XMM9,XMM9,XMM13.
        2 => code.extend([0xc4, 0x41, 0x34, 0x58, 0xcd]), // VADDPS YMM9,YMM9,YMM13.
        _ => unreachable!(),
    }
    code.extend([0x48, 0xff, 0xc9]); // DEC RCX.
    let back = i8::try_from(-(code.len() as i64 + 2)).unwrap();
    code.extend([0x75, back as u8, 0xc3]);
    code
}
fn input(mut state: NativeCpuState, iterations: u64) -> NativeCpuState {
    for (index, value) in state.gpr.iter_mut().enumerate() {
        if index != RSP {
            *value = 0x8877_6655_4433_2200 | index as u64;
        }
    }
    state.gpr[RAX] = 0xffff_ffff_ffff_ffff;
    state.gpr[RCX] = iterations;
    state.rflags = 0x202 | 0x8d5 | 0x400;
    state.mxcsr = 0x5f85;
    for (i, value) in state.xmm.iter_mut().enumerate() {
        *value = (i as u8).wrapping_mul(17);
    }
    for (i, value) in state.ymm_upper.iter_mut().enumerate() {
        *value = !(i as u8).wrapping_mul(29);
    }
    for register in [9, 13] {
        for lane in 0..4 {
            state.xmm[register * 16 + lane * 4..register * 16 + lane * 4 + 4]
                .copy_from_slice(&0x3f80_0000_u32.to_le_bytes());
            state.ymm_upper[register * 16 + lane * 4..register * 16 + lane * 4 + 4]
                .copy_from_slice(&0x3f80_0000_u32.to_le_bytes());
        }
    }
    state
}
fn prepare<'view, 'plan, 'image>(
    source: &'view MaterializedSource<'plan, 'image>,
    current: &NativeCpuState,
    code: &[u8],
    placement: CachePlacement,
    vector: u8,
) -> MaterializedBlockPlan<'view, 'plan, 'image> {
    let limits = BlockLimits::default();
    let mut block = plan_materialized_block(
        source,
        0,
        current.guest_pc,
        code,
        placement,
        &policy(vector),
        &limits,
    )
    .unwrap();
    let bound = block.register_loop_trace_work_bound(&limits).unwrap();
    let spent = block.prepare_register_loop_trace(&limits, bound, limits.max_allocation_bytes);
    assert!(spent > 0 && spent <= bound);
    assert!(block.register_loop_trace().is_some());
    assert!(block.affine_loop_trace().is_none());
    block
}
fn host_controls() -> (u32, u16) {
    let mut mxcsr = 0;
    let mut cw = 0;
    // SAFETY: Mandatory x64 controls copied to exact stack objects, no mutation.
    unsafe {
        std::arch::asm!("stmxcsr [{mx}]", "fnstcw [{cw}]", mx=in(reg) &mut mxcsr, cw=in(reg) &mut cw, options(nostack,preserves_flags));
    }
    (mxcsr, cw)
}

#[test]
fn actual_register_traces_match_scalar_aliases_flags_mxcsr_and_complete_sse_avx_banks() {
    let mut cases = 0;
    let avx = VectorMode::detect().features().is_some();
    for vector in 0..=2 {
        if vector != 0 && !avx {
            continue;
        }
        let code = program(vector);
        for iterations in [1_u64, 2, 3, 17] {
            with_source(Some(&code), |source, arena, stack, state| {
                let current = input(state, iterations);
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    current,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                engine.machine.vector_mode = if vector == 0 {
                    VectorMode::Sse2
                } else {
                    VectorMode::Avx {
                        avx2: false,
                        fma: false,
                    }
                };
                let block = prepare(source, &current, &code, engine.placement(), vector);
                assert_eq!(block.native_admission().emitted_instructions(), 4);
                let ret_pc = current.guest_pc + code.len() as u64 - 1;
                let ret = plan_materialized_block(
                    source,
                    0,
                    ret_pc,
                    &[0xc3],
                    engine.placement(),
                    &policy(vector),
                    &Default::default(),
                )
                .unwrap();
                let mut baseline = None;
                for fused in [false, true] {
                    if fused {
                        engine
                            .begin_call(&mut arena.pause().unwrap(), current)
                            .unwrap();
                    }
                    let controls = host_controls();
                    let before = engine.usage();
                    while engine.state().guest_pc != ret_pc {
                        // SAFETY: Original register-only body, exact paused owner,
                        // full current source/CPU admission; both paths check it.
                        let stop = unsafe {
                            if fused {
                                engine
                                    .try_step_register_loop_checked(
                                        &mut arena.pause().unwrap(),
                                        &block.register_loop_trace().unwrap(),
                                        2,
                                    )
                                    .unwrap()
                                    .unwrap()
                            } else {
                                engine
                                    .step_checked(&mut arena.pause().unwrap(), &block)
                                    .unwrap()
                            }
                        };
                        assert!(matches!(stop, NativeBlockStop::Resume { .. }));
                    }
                    // SAFETY: Exact original return on the unchanged owned stack.
                    assert!(matches!(
                        // SAFETY: The original return uses the unchanged owned stack and current source.
                        unsafe { engine.step_checked(&mut arena.pause().unwrap(), &ret) }.unwrap(),
                        NativeBlockStop::Returned { .. }
                    ));
                    assert_eq!(host_controls(), controls);
                    assert_eq!(engine.state().gpr[RCX], 0);
                    assert_eq!(engine.state().gpr[RSP], current.gpr[RSP] + 8);
                    let ah = 0xff_u8.wrapping_sub(iterations as u8);
                    assert_eq!(
                        engine.state().gpr[RAX],
                        (current.gpr[RAX] & !0xff00) | (u64::from(ah) << 8)
                    );
                    assert_eq!(engine.state().rflags & 0x400, 0x400); // Guest DF survives both exits.
                    assert_eq!(engine.state().mxcsr, current.mxcsr); // Exact additions preserve prior status/control.
                    let wanted = match iterations {
                        1 => 0x4000_0000_u32,
                        2 => 0x4040_0000,
                        3 => 0x4080_0000,
                        17 => 0x4190_0000,
                        _ => unreachable!(),
                    };
                    for lane in 0..4 {
                        assert_eq!(
                            &engine.state().xmm[9 * 16 + lane * 4..9 * 16 + lane * 4 + 4],
                            &wanted.to_le_bytes()
                        );
                    }
                    let upper = &engine.state().ymm_upper[9 * 16..10 * 16];
                    match vector {
                        0 => assert_eq!(upper, &current.ymm_upper[9 * 16..10 * 16]),
                        1 => assert_eq!(upper, &[0; 16]),
                        2 => {
                            for lane in 0..4 {
                                assert_eq!(&upper[lane * 4..lane * 4 + 4], &wanted.to_le_bytes());
                            }
                        }
                        _ => unreachable!(),
                    }
                    if fused {
                        assert_eq!(*engine.state(), baseline.unwrap());
                    } else {
                        baseline = Some(*engine.state());
                    }
                    let after = engine.usage();
                    assert_eq!(after.blocks - before.blocks, iterations + 1);
                    assert_eq!(
                        after.native_instructions - before.native_instructions,
                        iterations * 4
                    );
                    assert_eq!(
                        after.native_entries - before.native_entries,
                        if fused {
                            iterations.div_ceil(2)
                        } else {
                            iterations
                        }
                    );
                    assert_eq!(
                        after.register_loop_entries - before.register_loop_entries,
                        if fused { iterations.div_ceil(2) } else { 0 }
                    );
                    assert_eq!(
                        after.register_loop_iterations - before.register_loop_iterations,
                        if fused { iterations } else { 0 }
                    );
                    assert_eq!(after.affine_loop_entries, before.affine_loop_entries);
                    assert_eq!(after.affine_loop_iterations, before.affine_loop_iterations);
                    assert_eq!(after.memory_access_checks, before.memory_access_checks); // No emitted memory operands.
                    assert_eq!(after.vector_operations, before.vector_operations);
                }
            });
            cases += 1;
        }
    }
    assert_eq!(cases, if avx { 12 } else { 4 });
    println!(
        "register_loop_trace_available=true register_loop_trace_cases={cases} register_loop_trace_avx={avx}"
    );
}

#[test]
fn register_trace_span_quotas_and_counter_overflows_refuse_before_any_effect() {
    let code = program(0);
    for case in 0..9 {
        with_source(Some(&code), |source, arena, stack, state| {
            let current = input(state, 2);
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                current,
                stack,
                &Default::default(),
            )
            .unwrap();
            let block = prepare(source, &current, &code, engine.placement(), 0);
            match case {
                1 => engine.machine.limits.max_blocks = engine.usage().blocks + 1,
                2 => engine.machine.limits.max_work_units = engine.usage().work_units + 1,
                3 => engine.machine.usage.register_loop_iterations = u64::MAX,
                4 => engine.machine.usage.register_loop_entries = u64::MAX,
                5 => engine.machine.usage.native_entries = u64::MAX,
                6 => engine.machine.usage.native_instructions = u64::MAX - 1,
                7 => engine.machine.limits.max_source_bytes = engine.usage().source_bytes,
                8 => engine.machine.limits.max_emitted_bytes = engine.usage().emitted_bytes,
                _ => {}
            }
            let before = engine.usage();
            let slots = engine.machine.published_slots.clone();
            // SAFETY: A pre-effect bound is deliberately exhausted; neither the
            // first nor second body may run or mutate resident publication data.
            let result = unsafe {
                engine.try_step_register_loop_checked(
                    &mut arena.pause().unwrap(),
                    &block.register_loop_trace().unwrap(),
                    if case == 0 { 1 } else { 2 },
                )
            }
            .unwrap();
            assert_eq!(result, None);
            assert_eq!(*engine.state(), current);
            assert_eq!(engine.usage(), before);
            assert!(engine.machine.published_slots == slots);
            if case <= 1 {
                // SAFETY: The original checked single block still fits its limit.
                assert!(matches!(
                    // SAFETY: The checked single block still fits the deliberately narrowed limit.
                    unsafe { engine.step_checked(&mut arena.pause().unwrap(), &block) }.unwrap(),
                    NativeBlockStop::Resume { .. }
                ));
                assert_eq!(engine.usage().native_entries, 1);
                assert_eq!(engine.usage().blocks, before.blocks + 1);
                assert_eq!(engine.usage().register_loop_entries, 0);
            }
        });
    }
}

#[test]
fn register_trace_resident_cache_repairs_body_both_exits_and_cells_and_retains_source_checks() {
    let code = program(0);
    with_source(Some(&code), |source, arena, stack, state| {
        let current = input(state, 1);
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            current,
            stack,
            &Default::default(),
        )
        .unwrap();
        let block = prepare(source, &current, &code, engine.placement(), 0);
        let trace = block.register_loop_trace().unwrap();
        let ret_pc = current.guest_pc + code.len() as u64 - 1;
        let ret = plan_materialized_block(
            source,
            0,
            ret_pc,
            &[0xc3],
            engine.placement(),
            &policy(0),
            &Default::default(),
        )
        .unwrap();
        let base =
            usize::try_from(trace.placement().address - engine.machine.cache.address()).unwrap();
        let capacity = usize::try_from(trace.placement().capacity).unwrap();
        let corrupt = [
            trace.second_iteration_offset(),
            trace.first_exit_offset(),
            trace.second_exit_offset(),
            capacity - 16,
            capacity - 8,
        ];
        for round in 0..10 {
            if round != 0 {
                engine
                    .begin_call(&mut arena.pause().unwrap(), current)
                    .unwrap();
            }
            if round >= 5 {
                let offset = base + corrupt[round - 5];
                let mut byte = [0];
                engine.machine.cache.read(offset, &mut byte).unwrap();
                engine.machine.cache.arm_writable().unwrap();
                engine
                    .machine
                    .cache
                    .write(offset, &[byte[0] ^ 0xff])
                    .unwrap();
                engine.machine.cache.seal_executable().unwrap();
            }
            let before = engine.usage();
            // SAFETY: All original source and current permissions are validated;
            // no cached key may replace actual body/tail/cell byte comparison.
            let stop = unsafe {
                if round == 0 || round == 3 {
                    engine
                        .step_checked(&mut arena.pause().unwrap(), &block)
                        .unwrap()
                } else {
                    engine
                        .try_step_register_loop_checked(&mut arena.pause().unwrap(), &trace, 2)
                        .unwrap()
                        .unwrap()
                }
            };
            assert_eq!(stop, NativeBlockStop::Resume { guest_pc: ret_pc });
            assert_eq!(
                engine.usage().cache_publications - before.cache_publications,
                u64::from(round != 2)
            );
            assert_eq!(
                engine.usage().cache_reuses - before.cache_reuses,
                u64::from(round == 2)
            );
            // SAFETY: The original checked return completes exactly this call.
            assert!(matches!(
                // SAFETY: The original checked return completes this exact owned call.
                unsafe { engine.step_checked(&mut arena.pause().unwrap(), &ret) }.unwrap(),
                NativeBlockStop::Returned { .. }
            ));
        }
        for source_case in 0..3 {
            engine
                .begin_call(&mut arena.pause().unwrap(), current)
                .unwrap();
            let page = current.guest_pc & !(PAGE - 1);
            arena
                .protect(page, PAGE, NativePermissions::READ_WRITE)
                .unwrap();
            if source_case == 2 {
                arena
                    .pause()
                    .unwrap()
                    .write(current.guest_pc, &[0x90])
                    .unwrap();
            }
            if source_case != 0 {
                arena
                    .protect(
                        page,
                        PAGE,
                        if source_case == 1 {
                            NativePermissions::READ
                        } else {
                            NativePermissions::READ_EXECUTE
                        },
                    )
                    .unwrap();
            }
            let before = engine.usage();
            let slots = engine.machine.published_slots.clone();
            // SAFETY: Source immutability, execute rights or exact bytes were
            // deliberately revoked; optimization must fall back before effects.
            assert_eq!(
                // SAFETY: Revoked source authority must refuse before native effects.
                unsafe {
                    engine.try_step_register_loop_checked(&mut arena.pause().unwrap(), &trace, 2)
                }
                .unwrap(),
                None
            );
            assert_eq!(*engine.state(), current);
            assert_eq!(engine.usage().native_entries, before.native_entries);
            assert_eq!(engine.usage().blocks, before.blocks);
            assert_eq!(engine.usage().cache_reuses, before.cache_reuses);
            assert_eq!(engine.usage().cache_publications, before.cache_publications);
            assert!(engine.machine.published_slots == slots);
            arena
                .protect(page, PAGE, NativePermissions::READ_WRITE)
                .unwrap();
            arena
                .pause()
                .unwrap()
                .write(current.guest_pc, &code[..1])
                .unwrap();
            arena
                .protect(page, PAGE, NativePermissions::READ_EXECUTE)
                .unwrap();
            // SAFETY: Restore the genuine source before ordinary execution.
            unsafe { engine.step_checked(&mut arena.pause().unwrap(), &block) }.unwrap();
            // SAFETY: Exact checked return under the same restored owner.
            unsafe { engine.step_checked(&mut arena.pause().unwrap(), &ret) }.unwrap();
        }
    });
}

#[test]
fn register_trace_requires_actual_owner_and_full_vector_bridge_authority() {
    let code = program(0);
    with_source(Some(&code), |source, arena, stack, state| {
        let current = input(state, 2);
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            current,
            stack,
            &Default::default(),
        )
        .unwrap();
        let block = prepare(source, &current, &code, engine.placement(), 0);
        let mut foreign = NativeAddressSpace::reserve_any(Default::default(), 65_536).unwrap();
        let before = engine.usage();
        // SAFETY: Deliberately foreign owner must fail before admission or entry.
        assert_eq!(
            // SAFETY: A deliberately foreign owner must fail before admission or entry.
            unsafe {
                engine.try_step_register_loop_checked(
                    &mut foreign.pause().unwrap(),
                    &block.register_loop_trace().unwrap(),
                    2,
                )
            },
            Err(NativeDispatchError::WrongOwner)
        );
        assert_eq!(engine.usage(), before);
        assert_eq!(*engine.state(), current);
        let slots = engine.machine.published_slots.clone();
        arena
            .protect(
                stack.stack.address,
                stack.stack.size,
                NativePermissions::READ,
            )
            .unwrap();
        // SAFETY: The original owned stack deliberately loses live write rights;
        // even a memory-free body must preserve the complete stack admission.
        assert_eq!(
            // SAFETY: The owned stack deliberately lacks write rights; entry must refuse.
            unsafe {
                engine.try_step_register_loop_checked(
                    &mut arena.pause().unwrap(),
                    &block.register_loop_trace().unwrap(),
                    2,
                )
            }
            .unwrap(),
            None
        );
        assert_eq!(*engine.state(), current);
        assert_eq!(engine.usage().native_entries, before.native_entries);
        assert_eq!(engine.usage().blocks, before.blocks);
        assert!(engine.machine.published_slots == slots);
        arena
            .protect(
                stack.stack.address,
                stack.stack.size,
                NativePermissions::READ_WRITE,
            )
            .unwrap();
        engine.machine.state.mxcsr &= !(1 << 7); // Unmask invalid-operation exceptions.
        let before = engine.usage();
        // SAFETY: Invalid guest controls must fail before any hardware entry;
        // no unmasked guest FP control is installed on the host.
        assert_eq!(
            // SAFETY: Invalid guest controls must fail before any hardware entry.
            unsafe {
                engine.try_step_register_loop_checked(
                    &mut arena.pause().unwrap(),
                    &block.register_loop_trace().unwrap(),
                    2,
                )
            },
            Err(NativeDispatchError::InvalidState)
        );
        assert_eq!(engine.usage(), before);
        assert_eq!(engine.state().mxcsr, current.mxcsr & !(1 << 7));
        assert!(engine.machine.published_slots == slots);
    });
    if VectorMode::detect().features().is_none() {
        return;
    }
    let code = program(2);
    with_source(Some(&code), |source, arena, stack, state| {
        let current = input(state, 2);
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            current,
            stack,
            &Default::default(),
        )
        .unwrap();
        let block = prepare(source, &current, &code, engine.placement(), 2);
        engine.machine.vector_mode = VectorMode::Sse2;
        let before = engine.usage();
        let slots = engine.machine.published_slots.clone();
        // SAFETY: The declared vector trace has no admitted full-YMM bridge.
        assert_eq!(
            // SAFETY: The declared YMM trace lacks an admitted full-YMM bridge.
            unsafe {
                engine.try_step_register_loop_checked(
                    &mut arena.pause().unwrap(),
                    &block.register_loop_trace().unwrap(),
                    2,
                )
            }
            .unwrap(),
            None
        );
        assert_eq!(*engine.state(), current);
        assert_eq!(engine.usage().native_entries, before.native_entries);
        assert_eq!(engine.usage().blocks, before.blocks);
        assert!(engine.machine.published_slots == slots);
    });
}
