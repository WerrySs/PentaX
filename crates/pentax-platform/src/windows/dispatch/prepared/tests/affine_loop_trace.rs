// SPDX-License-Identifier: GPL-2.0-only
//! Original strided loops; finite traces preserve ordinary effects and limits.
use super::*;
use crate::native_memory::{NativeMemoryEffect, NativeMemoryTransaction};
use pentax_codegen::{CpuFeatures, CpuidFeature};

fn policy(vector: bool) -> TranslationPolicy {
    let policy = if vector {
        TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
            .with_native_ymm_registers()
            .with_native_vector_moves()
    } else {
        TranslationPolicy::x64_sse2()
    };
    policy.with_independent_memory().with_affine_loop_trace()
}
fn program(vector: bool, wide_induction: bool) -> Vec<u8> {
    let mut code = if vector {
        vec![
            0xc4, 0x61, 0x7c, 0x10, 0x0c, 0x3b, // VMOVUPS YMM9,[RBX+RDI].
            0xc4, 0x41, 0x34, 0x58, 0xcd, // VADDPS YMM9,YMM9,YMM13.
            0xc4, 0x61, 0x7c, 0x11, 0x0c, 0x3b,
        ] // VMOVUPS [RBX+RDI],YMM9.
    } else {
        vec![0x8b, 0x04, 0x3b, 0x83, 0xc0, 1, 0x89, 0x04, 0x3b]
    };
    if wide_induction {
        code.push(0x48);
    }
    code.extend([0x83, 0xc7, if vector { 32 } else { 4 }]); // ADD R/EDI,stride.
    if wide_induction {
        code.push(0x48);
    }
    code.extend([0x39, 0xcf]); // CMP R/EDI,R/ECX.
    let back = i8::try_from(-(code.len() as i64 + 2)).unwrap();
    code.extend([0x75, back as u8, 0xc3]);
    code
}
fn prepare<'view, 'plan, 'image>(
    source: &'view MaterializedSource<'plan, 'image>,
    pc: u64,
    code: &[u8],
    placement: CachePlacement,
    vector: bool,
) -> MaterializedBlockPlan<'view, 'plan, 'image> {
    let limits = BlockLimits::default();
    let mut block =
        plan_materialized_block(source, 0, pc, code, placement, &policy(vector), &limits).unwrap();
    let bound = block.affine_loop_trace_work_bound(&limits).unwrap();
    assert!(block.prepare_affine_loop_trace(&limits, bound, limits.max_allocation_bytes) <= bound);
    assert!(block.affine_loop_trace().is_some());
    block
}
fn input(mut state: NativeCpuState, base: u64, iterations: u64, vector: bool) -> NativeCpuState {
    for (index, value) in state.gpr.iter_mut().enumerate() {
        if index != RSP {
            *value = 0x1122_3344_5566_7700 | index as u64;
        }
    }
    state.gpr[RAX] = 42;
    state.gpr[3] = base;
    state.gpr[RDI] = 0;
    state.gpr[RCX] = iterations * if vector { 32 } else { 4 };
    state.rflags = 0x202 | 0x8d5 | 0x400;
    state.mxcsr = 0x5fa5;
    for (index, byte) in state.xmm.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(11);
    }
    for (index, byte) in state.ymm_upper.iter_mut().enumerate() {
        *byte = !(index as u8).wrapping_mul(17);
    }
    for lane in 0..4 {
        state.xmm[13 * 16 + lane * 4..13 * 16 + lane * 4 + 4]
            .copy_from_slice(&0x3f80_0000_u32.to_le_bytes());
        state.ymm_upper[13 * 16 + lane * 4..13 * 16 + lane * 4 + 4]
            .copy_from_slice(&0x3f80_0000_u32.to_le_bytes());
    }
    state
}
fn seed(vector: bool) -> Vec<u8> {
    let mut bytes = vec![0xa5; 8];
    for index in 0..3 {
        let value = if vector {
            [0x3f80_0000_u32, 0x4000_0000, 0x4040_0000][index]
        } else {
            (index as u32 + 1) * 10
        };
        for _ in 0..if vector { 8 } else { 1 } {
            bytes.extend(value.to_le_bytes());
        }
    }
    bytes.extend([0x5a; 8]);
    bytes
}
fn wanted(vector: bool, iterations: usize) -> Vec<u8> {
    let mut bytes = seed(vector);
    for index in 0..iterations {
        let value = if vector {
            [0x4000_0000_u32, 0x4040_0000, 0x4080_0000][index]
        } else {
            (index as u32 + 1) * 10 + 1
        };
        let stride = if vector { 32 } else { 4 };
        for lane in 0..if vector { 8 } else { 1 } {
            bytes[8 + index * stride + lane * 4..8 + index * stride + lane * 4 + 4]
                .copy_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}
fn host_controls() -> (u32, u16) {
    let mut mxcsr = 0_u32;
    let mut x87 = 0_u16;
    // SAFETY: Mandatory controls copied to exact local objects; no mutation.
    unsafe {
        std::arch::asm!("stmxcsr [{mx}]", "fnstcw [{cw}]", mx=in(reg) &mut mxcsr, cw=in(reg) &mut x87, options(nostack,preserves_flags));
    }
    (mxcsr, x87)
}

#[test]
fn actual_scalar_and_vector_traces_retire_one_or_two_iterations_with_identical_full_effects() {
    let mut cases = 0;
    let vector_available = VectorMode::detect().features().is_some();
    for vector in [false, true] {
        if vector && !vector_available {
            continue;
        }
        for wide in [false, true] {
            let code = program(vector, wide);
            for iterations in 1..=3 {
                with_source(Some(&code), |source, arena, stack, state| {
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + 64;
                    let current = input(state, data, iterations, vector);
                    let mut engine = NativeBlockStepper::new(
                        source,
                        &mut arena.pause().unwrap(),
                        current,
                        stack,
                        &Default::default(),
                    )
                    .unwrap();
                    engine.machine.vector_mode = if vector {
                        VectorMode::Avx {
                            avx2: false,
                            fma: false,
                        }
                    } else {
                        VectorMode::Sse2
                    };
                    let body = prepare(source, current.guest_pc, &code, engine.placement(), vector);
                    let n = body.native_admission().emitted_instructions();
                    assert_eq!(n, 5);
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
                    let initial = seed(vector);
                    let mut baseline = None;
                    for trace in [false, true] {
                        arena.pause().unwrap().write(data - 8, &initial).unwrap();
                        if trace {
                            engine
                                .begin_call(&mut arena.pause().unwrap(), current)
                                .unwrap();
                        }
                        let before = engine.usage();
                        let host = host_controls();
                        let mut finished = false;
                        for _ in 0..8 {
                            let stop = if engine.state().guest_pc == ret_pc {
                                // SAFETY: The original owned RET and exact stack.
                                unsafe { engine.step_checked(&mut arena.pause().unwrap(), &ret) }
                                    .unwrap()
                            } else if trace {
                                assert_eq!(engine.state().guest_pc, current.guest_pc);
                                // SAFETY: Same original source and exclusive owner;
                                // this fixture has no boundary inside a two-block span.
                                unsafe {
                                    engine.try_step_affine_loop_checked(
                                        &mut arena.pause().unwrap(),
                                        &body.affine_loop_trace().unwrap(),
                                        2,
                                    )
                                }
                                .unwrap()
                                .unwrap()
                            } else {
                                // SAFETY: Baseline original loop with full checks.
                                unsafe { engine.step_checked(&mut arena.pause().unwrap(), &body) }
                                    .unwrap()
                            };
                            if matches!(stop, NativeBlockStop::Returned { .. }) {
                                finished = true;
                                break;
                            }
                        }
                        assert!(finished);
                        let mut bytes = vec![0; initial.len()];
                        arena.pause().unwrap().read(data - 8, &mut bytes).unwrap();
                        assert_eq!(bytes, wanted(vector, iterations as usize));
                        assert_eq!(host_controls(), host);
                        if trace {
                            assert_eq!(*engine.state(), baseline.unwrap());
                        } else {
                            baseline = Some(*engine.state());
                        }
                        let after = engine.usage();
                        assert_eq!(after.blocks - before.blocks, iterations + 1);
                        assert_eq!(
                            after.native_instructions - before.native_instructions,
                            n * iterations
                        );
                        assert_eq!(
                            after.native_entries - before.native_entries,
                            if trace {
                                iterations.div_ceil(2)
                            } else {
                                iterations
                            }
                        );
                        assert_eq!(
                            after.affine_loop_entries - before.affine_loop_entries,
                            if trace { iterations.div_ceil(2) } else { 0 }
                        );
                        assert_eq!(
                            after.affine_loop_iterations - before.affine_loop_iterations,
                            if trace { iterations } else { 0 }
                        );
                        assert_eq!(after.vector_operations, before.vector_operations);
                    }
                });
                cases += 1;
            }
        }
    }
    assert_eq!(cases, if vector_available { 12 } else { 6 });
    let modes = if vector_available { "sse2-avx" } else { "sse2" };
    println!(
        "affine_loop_trace_available=true affine_loop_trace_vector_available={vector_available} affine_loop_trace_cases={cases} affine_loop_trace_modes={modes}"
    );
}

#[test]
fn span_one_wrap_and_worstcase_quota_refusals_leave_the_ordinary_path_available() {
    for wide in [false, true] {
        let code = program(false, wide);
        for case in 0..5 {
            with_source(Some(&code), |source, arena, stack, state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + 64;
                let mut current = input(state, data, 1, false);
                if case == 1 {
                    current.gpr[RDI] = if wide {
                        u64::MAX - 2
                    } else {
                        u64::from(u32::MAX) - 2
                    };
                }
                arena
                    .pause()
                    .unwrap()
                    .write(data, &10_u32.to_le_bytes())
                    .unwrap();
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    current,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                let body = prepare(source, current.guest_pc, &code, engine.placement(), false);
                match case {
                    2 => engine.machine.limits.max_blocks = 1,
                    3 => engine.machine.limits.max_work_units = engine.usage().work_units + 1,
                    4 => engine.machine.usage.affine_loop_iterations = u64::MAX,
                    _ => {}
                }
                let before = engine.usage();
                let slots = engine.machine.published_slots.clone();
                // SAFETY: Original optional candidate is deliberately gated
                // before any speculative scan, publication or native effects.
                let result = unsafe {
                    engine.try_step_affine_loop_checked(
                        &mut arena.pause().unwrap(),
                        &body.affine_loop_trace().unwrap(),
                        if case == 0 { 1 } else { 2 },
                    )
                }
                .unwrap();
                assert_eq!(result, None);
                assert_eq!(engine.usage(), before);
                assert_eq!(*engine.state(), current);
                assert!(engine.machine.published_slots == slots);
                if case == 0 || case == 2 || case == 4 {
                    // SAFETY: The refused optimization leaves the same original
                    // checked one-block route, with its original quotas.
                    let stop =
                        unsafe { engine.step_checked(&mut arena.pause().unwrap(), &body) }.unwrap();
                    assert!(matches!(stop, NativeBlockStop::Resume { .. }));
                    assert_eq!(engine.usage().native_entries, 1);
                }
            });
        }
    }
}

#[test]
fn second_iteration_holes_rights_and_late_source_changes_fall_back_before_data_effects() {
    let code = program(false, true);
    for case in 0..3 {
        with_source(Some(&code), |source, arena, stack, state| {
            let end = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE;
            if case == 1 {
                arena.map_at(end, PAGE, NativePermissions::READ).unwrap();
            }
            let current = input(state, end - 4, 1, false);
            arena
                .pause()
                .unwrap()
                .write(end - 4, &10_u32.to_le_bytes())
                .unwrap();
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                current,
                stack,
                &Default::default(),
            )
            .unwrap();
            let body = prepare(source, current.guest_pc, &code, engine.placement(), false);
            if case == 2 {
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
            let before = engine.usage();
            let slots = engine.machine.published_slots.clone();
            // SAFETY: Failed speculation must not execute even its valid first iteration.
            let result = unsafe {
                engine.try_step_affine_loop_checked(
                    &mut arena.pause().unwrap(),
                    &body.affine_loop_trace().unwrap(),
                    2,
                )
            }
            .unwrap();
            assert_eq!(result, None);
            assert_eq!(*engine.state(), current);
            assert_eq!(engine.usage().native_entries, before.native_entries);
            assert_eq!(engine.usage().blocks, before.blocks);
            assert!(engine.usage().work_units > before.work_units);
            assert!(engine.machine.published_slots == slots);
            let mut bytes = [0; 4];
            arena.pause().unwrap().read(end - 4, &mut bytes).unwrap();
            assert_eq!(u32::from_le_bytes(bytes), 10);
            // SAFETY: The original single block retains its precise admission;
            // only the optimization refused a speculative second iteration.
            let ordinary = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &body) };
            if case == 2 {
                assert_eq!(ordinary, Err(NativeDispatchError::SourceMismatch));
            } else {
                assert!(matches!(ordinary, Ok(NativeBlockStop::Resume { .. })));
                arena.pause().unwrap().read(end - 4, &mut bytes).unwrap();
                assert_eq!(u32::from_le_bytes(bytes), 11);
            }
        });
    }
}

#[test]
fn physical_write_aliases_of_consumed_source_refuse_without_changing_source_or_data() {
    let code = program(false, true);
    with_source(Some(&code), |source, arena, stack, state| {
        let source_page = state.guest_pc & !(PAGE - 1);
        let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
        let mut original = vec![0; PAGE as usize];
        arena
            .pause()
            .unwrap()
            .fetch_source(source_page, &mut original)
            .unwrap();
        let current = input(state, data + (state.guest_pc - source_page), 1, false);
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            current,
            stack,
            &Default::default(),
        )
        .unwrap();
        let body = prepare(source, current.guest_pc, &code, engine.placement(), false);
        let apply = |arena: &mut NativeAddressSpace, effect| {
            arena
                .pause()
                .unwrap()
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect,
                        writes: &[],
                    },
                    &Default::default(),
                )
                .unwrap();
        };
        apply(
            arena,
            NativeMemoryEffect::AllocatePhysical {
                physical: 0,
                size: PAGE,
            },
        );
        arena.unmap(source_page, PAGE).unwrap();
        arena.unmap(data, PAGE).unwrap();
        apply(
            arena,
            NativeMemoryEffect::MapPhysical {
                address: source_page,
                size: PAGE,
                physical: 0,
                permissions: NativePermissions::READ_WRITE,
            },
        );
        arena
            .pause()
            .unwrap()
            .write(source_page, &original)
            .unwrap();
        arena
            .protect(source_page, PAGE, NativePermissions::READ_EXECUTE)
            .unwrap();
        apply(
            arena,
            NativeMemoryEffect::MapPhysical {
                address: data,
                size: PAGE,
                physical: 0,
                permissions: NativePermissions::READ_WRITE,
            },
        );
        let before = engine.usage();
        // SAFETY: Same exact bytes and owner, but a data store aliases the
        // source backing. Trace admission must stop before any body executes.
        let result = unsafe {
            engine.try_step_affine_loop_checked(
                &mut arena.pause().unwrap(),
                &body.affine_loop_trace().unwrap(),
                2,
            )
        }
        .unwrap();
        assert_eq!(result, None);
        assert_eq!(*engine.state(), current);
        assert_eq!(engine.usage().native_entries, before.native_entries);
        assert_eq!(engine.usage().cache_publications, before.cache_publications);
        assert_eq!(
            engine.usage().memory_access_checks - before.memory_access_checks,
            2
        );
        let mut actual = vec![0; PAGE as usize];
        arena
            .pause()
            .unwrap()
            .fetch_source(source_page, &mut actual)
            .unwrap();
        assert_eq!(actual, original);
    });
}

#[test]
fn ordinary_and_trace_cache_flavors_repair_both_tails_cells_and_body_with_live_admission() {
    let code = program(false, true);
    with_source(Some(&code), |source, arena, stack, state| {
        let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + 64;
        let current = input(state, data, 1, false);
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            current,
            stack,
            &Default::default(),
        )
        .unwrap();
        let body = prepare(source, current.guest_pc, &code, engine.placement(), false);
        let trace = body.affine_loop_trace().unwrap();
        let ret_pc = current.guest_pc + code.len() as u64 - 1;
        let ret = plan_materialized_block(
            source,
            0,
            ret_pc,
            &[0xc3],
            engine.placement(),
            &policy(false),
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
            arena
                .pause()
                .unwrap()
                .write(data, &10_u32.to_le_bytes())
                .unwrap();
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
            let ordinary = round == 0 || round == 3;
            let stop = if ordinary {
                // SAFETY: The actual entry is the same original one-iteration
                // loop; full checked publication must repair any trace flavor.
                unsafe { engine.step_checked(&mut arena.pause().unwrap(), &body) }.unwrap()
            } else {
                // SAFETY: The opaque forward trace binds this exact plan and
                // checks the entire body, both tails and both mode-bound cells.
                unsafe {
                    engine.try_step_affine_loop_checked(&mut arena.pause().unwrap(), &trace, 2)
                }
                .unwrap()
                .unwrap()
            };
            assert_eq!(stop, NativeBlockStop::Resume { guest_pc: ret_pc });
            let after = engine.usage();
            assert_eq!(
                after.cache_publications - before.cache_publications,
                u64::from(round != 2)
            );
            assert_eq!(
                after.cache_reuses - before.cache_reuses,
                u64::from(round == 2)
            );
            assert_eq!(after.native_entries - before.native_entries, 1);
            assert_eq!(after.native_instructions - before.native_instructions, 5);
            let mut bytes = [0; 4];
            arena.pause().unwrap().read(data, &mut bytes).unwrap();
            assert_eq!(u32::from_le_bytes(bytes), 11);
            // SAFETY: The same owned, current original RET finishes the call.
            let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &ret) }.unwrap();
            assert_eq!(stop, NativeBlockStop::Returned { value: 11 });
        }
        // An already-resident trace still requires current source rights;
        // denied source admission cannot repair, rename or touch a resident.
        engine
            .begin_call(&mut arena.pause().unwrap(), current)
            .unwrap();
        let source_page = current.guest_pc & !(PAGE - 1);
        arena
            .protect(source_page, PAGE, NativePermissions::READ_WRITE)
            .unwrap();
        let slots = engine.machine.published_slots.clone();
        let before = engine.usage();
        // SAFETY: The exact candidate has deliberately lost immutable source
        // admission; no native effects or cache-event commit may occur.
        let stop =
            unsafe { engine.try_step_affine_loop_checked(&mut arena.pause().unwrap(), &trace, 2) }
                .unwrap();
        assert_eq!(stop, None);
        assert_eq!(*engine.state(), current);
        assert_eq!(engine.usage().native_entries, before.native_entries);
        assert_eq!(engine.usage().cache_publications, before.cache_publications);
        assert_eq!(engine.usage().cache_reuses, before.cache_reuses);
        assert!(engine.machine.published_slots == slots);
        arena
            .protect(source_page, PAGE, NativePermissions::READ_EXECUTE)
            .unwrap();
        let mut foreign = NativeAddressSpace::reserve_any(Default::default(), 65_536).unwrap();
        let before = engine.usage();
        // SAFETY: An unrelated owner is deliberately supplied to prove the
        // typed identity refusal occurs before work and cache admission.
        let stop = unsafe {
            engine.try_step_affine_loop_checked(&mut foreign.pause().unwrap(), &trace, 2)
        };
        assert_eq!(stop, Err(NativeDispatchError::WrongOwner));
        assert_eq!(engine.usage(), before);
        assert!(engine.machine.published_slots == slots);
    });
    if VectorMode::detect().features().is_none() {
        return;
    }
    let code = program(true, true);
    with_source(Some(&code), |source, arena, stack, state| {
        let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + 64;
        let current = input(state, data, 1, true);
        arena.pause().unwrap().write(data - 8, &seed(true)).unwrap();
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            current,
            stack,
            &Default::default(),
        )
        .unwrap();
        let body = prepare(source, current.guest_pc, &code, engine.placement(), true);
        engine.machine.vector_mode = VectorMode::Sse2;
        let before = engine.usage();
        let slots = engine.machine.published_slots.clone();
        // SAFETY: The declared AVX candidate has no actual bridge authority
        // in this forced fallback mode; no body or cache event may occur.
        let stop = unsafe {
            engine.try_step_affine_loop_checked(
                &mut arena.pause().unwrap(),
                &body.affine_loop_trace().unwrap(),
                2,
            )
        }
        .unwrap();
        assert_eq!(stop, None);
        assert_eq!(*engine.state(), current);
        assert_eq!(engine.usage().native_entries, before.native_entries);
        assert_eq!(engine.usage().cache_publications, before.cache_publications);
        assert_eq!(engine.usage().cache_reuses, before.cache_reuses);
        assert!(engine.machine.published_slots == slots);
        let mut bytes = vec![0; seed(true).len()];
        arena.pause().unwrap().read(data - 8, &mut bytes).unwrap();
        assert_eq!(bytes, seed(true));
    });
}
