// SPDX-License-Identifier: GPL-2.0-only
//! Original checked calls; hints choose storage and never authorize execution.
use super::*;

const OFFSETS: [usize; 3] = [0, 0x1010, 0x2020];
fn original() -> Vec<u8> {
    let mut code = vec![0x90; OFFSETS[2] + 6];
    for (index, offset) in OFFSETS.into_iter().enumerate() {
        code[offset..offset + 6].copy_from_slice(&[0xb8, index as u8 + 1, 0, 0, 0, 0xc3]);
    }
    code
}
fn direct_index(pc: u64, slots: usize) -> usize {
    ((pc ^ (pc >> 8) ^ (pc >> 16) ^ (pc >> 32)) as usize) & (slots - 1)
}
fn plan<'view, 'plan, 'image>(
    source: &'view MaterializedSource<'plan, 'image>,
    pc: u64,
    bytes: &[u8],
    placement: CachePlacement,
) -> MaterializedBlockPlan<'view, 'plan, 'image> {
    plan_materialized_block(
        source,
        0,
        pc,
        bytes,
        placement,
        &TranslationPolicy::x64_sse2(),
        &Default::default(),
    )
    .unwrap()
}

#[test]
fn actual_paired_ways_retain_two_hot_calls_and_evict_the_least_recent_validated_way() {
    let code = original();
    for slots in [1, 2, 4] {
        with_source(Some(&code), |source, arena, stack, state| {
            let limits = NativeDispatchLimits {
                cache_slots: slots,
                ..Default::default()
            };
            let mut engine =
                NativeBlockStepper::new(source, &mut arena.pause().unwrap(), state, stack, &limits)
                    .unwrap();
            let primary = direct_index(state.guest_pc, slots);
            for offset in OFFSETS {
                assert_eq!(direct_index(state.guest_pc + offset as u64, slots), primary);
            }
            let mut placements = Vec::new();
            for (call, function) in [0, 1, 0, 2, 1].into_iter().enumerate() {
                let offset = OFFSETS[function];
                let mut current = state;
                current.guest_pc += offset as u64;
                if call != 0 {
                    engine
                        .begin_call(&mut arena.pause().unwrap(), current)
                        .unwrap();
                }
                let placement = engine.placement_for(current.guest_pc);
                placements.push(placement);
                let body = plan(
                    source,
                    current.guest_pc,
                    &code[offset..offset + 6],
                    placement,
                );
                // SAFETY: Original same-owner MOV/RET calls and real checked
                // source, stack, CPU profile and cache publication every time.
                let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &body) };
                assert_eq!(
                    stop.unwrap(),
                    NativeBlockStop::Returned {
                        value: function as u64 + 1
                    }
                );
                let slot = ((placement.address - engine.machine.cache.address())
                    / (limits.cache_bytes / slots) as u64) as usize;
                let hint = engine.machine.published_slots[slot].resident.unwrap();
                assert_eq!(hint.key.guest_pc, current.guest_pc);
                assert_eq!(hint.last_use, engine.usage().blocks);
            }
            assert_eq!(engine.usage().native_entries, 5);
            assert_eq!(engine.usage().native_instructions, 5);
            assert_eq!(engine.machine.published_slots.len(), slots);
            assert_eq!(engine.machine.published_slots.capacity(), slots);
            if slots == 1 {
                assert!(
                    placements
                        .iter()
                        .all(|placement| *placement == engine.placement())
                );
                assert_eq!(engine.usage().cache_publications, 5);
                assert_eq!(engine.usage().cache_reuses, 0);
            } else {
                assert_ne!(placements[0], placements[1]);
                assert_eq!(placements[0], placements[2]); // A was just reused.
                assert_eq!(placements[1], placements[3]); // C evicts older B.
                assert_eq!(placements[0], placements[4]); // B next evicts older A.
                assert_eq!(engine.usage().cache_publications, 4);
                assert_eq!(engine.usage().cache_reuses, 1);
                assert_eq!(engine.usage().cache_replacements, 2);
            }
        });
    }
}

#[test]
fn validated_byte_reuse_renames_only_the_resident_hint_and_selection_has_no_effects() {
    let mut code = original();
    code[OFFSETS[1] + 1] = 1; // Distinct original PCs, identical MOV/RET emissions.
    with_source(Some(&code), |source, arena, stack, state| {
        let limits = NativeDispatchLimits {
            cache_slots: 2,
            ..Default::default()
        };
        let mut engine =
            NativeBlockStepper::new(source, &mut arena.pause().unwrap(), state, stack, &limits)
                .unwrap();
        let placement = engine.placement_for(state.guest_pc);
        let first = plan(source, state.guest_pc, &code[..6], placement);
        // SAFETY: Actual checked original same-owner source and stack.
        let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &first) };
        assert_eq!(stop.unwrap(), NativeBlockStop::Returned { value: 1 });
        let mut next = state;
        next.guest_pc += OFFSETS[1] as u64;
        engine
            .begin_call(&mut arena.pause().unwrap(), next)
            .unwrap();
        // Explicitly keep the same existing slot to exercise byte-identical
        // reuse across PCs independently of the two-way placement heuristic.
        let second = plan(
            source,
            next.guest_pc,
            &code[OFFSETS[1]..OFFSETS[1] + 6],
            placement,
        );
        // SAFETY: The exact bytes/tail/cell and live source are checked again.
        let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &second) };
        assert_eq!(stop.unwrap(), NativeBlockStop::Returned { value: 1 });
        let slot = direct_index(state.guest_pc, 2);
        let metadata = engine.machine.published_slots[slot];
        assert_eq!(metadata.current.unwrap().guest_pc, state.guest_pc); // Historical producer.
        assert_eq!(metadata.resident.unwrap().key.guest_pc, next.guest_pc); // Current hint.
        assert_eq!(metadata.resident.unwrap().last_use, 2);
        assert_eq!(engine.usage().cache_publications, 1);
        assert_eq!(engine.usage().cache_reuses, 1);
        assert_eq!(engine.usage().cache_replacements, 0);
        assert_eq!(engine.usage().cache_evicted_key_revisits, 0);
        let usage = engine.usage();
        for _ in 0..4 {
            assert_eq!(engine.placement_for(next.guest_pc), placement);
            assert_ne!(engine.placement_for(state.guest_pc), placement); // Other way is vacant.
        }
        assert!(engine.machine.published_slots[slot] == metadata);
        assert_eq!(engine.usage(), usage);
    });
}

#[test]
fn validated_cache_events_remain_committed_when_later_native_entry_admission_refuses() {
    for reuse in [false, true] {
        let mut code = original();
        if reuse {
            code[OFFSETS[1] + 1] = 1;
        }
        for held_guard in [false, true] {
            with_source(Some(&code), |source, arena, stack, state| {
                let limits = NativeDispatchLimits {
                    cache_slots: 2,
                    ..Default::default()
                };
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    state,
                    stack,
                    &limits,
                )
                .unwrap();
                let placement = engine.placement_for(state.guest_pc);
                let first = plan(source, state.guest_pc, &code[..6], placement);
                // SAFETY: Warm the slot through a genuine checked original call.
                let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &first) };
                assert_eq!(stop.unwrap(), NativeBlockStop::Returned { value: 1 });
                let mut next = state;
                next.guest_pc += OFFSETS[1] as u64;
                next.gpr[RAX] = 0xa11c_5eed_dead_beef; // Either native MOV would change this.
                engine
                    .begin_call(&mut arena.pause().unwrap(), next)
                    .unwrap();
                let second = plan(
                    source,
                    next.guest_pc,
                    &code[OFFSETS[1]..OFFSETS[1] + 6],
                    placement,
                );
                if !held_guard {
                    engine.machine.usage.native_entries = u64::MAX;
                }
                let before = engine.usage();
                let slot = direct_index(state.guest_pc, limits.cache_slots);
                let previous = engine.machine.published_slots[slot];
                let guard = held_guard.then(|| DispatchGuard::acquire().unwrap());
                // SAFETY: The cache event passes all checked source/operand
                // admission; the deliberately later entry gate must refuse.
                let result = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &second) };
                let expected = if held_guard {
                    NativeDispatchError::Platform(PlatformError::TrapScopeUnavailable)
                } else {
                    NativeDispatchError::Budget("native entries")
                };
                assert_eq!(result, Err(expected));
                assert_eq!(*engine.state(), next);
                assert!(engine.machine.phase == Phase::Ready);
                assert_eq!(engine.usage().native_entries, before.native_entries);
                assert_eq!(
                    engine.usage().native_instructions,
                    before.native_instructions
                );
                let current = engine.machine.published_slots[slot];
                assert_eq!(current.resident.unwrap().key.guest_pc, next.guest_pc);
                assert_eq!(current.resident.unwrap().last_use, engine.usage().blocks);
                assert!(current.resident.unwrap().last_use > previous.resident.unwrap().last_use);
                assert_eq!(
                    engine.usage().cache_publications,
                    before.cache_publications + u64::from(!reuse)
                );
                assert_eq!(
                    engine.usage().cache_reuses,
                    before.cache_reuses + u64::from(reuse)
                );
                if reuse {
                    assert!(current.current == previous.current);
                    assert!(current.previous_evicted == previous.previous_evicted);
                } else {
                    assert_eq!(current.current.unwrap().guest_pc, next.guest_pc);
                    assert!(current.previous_evicted == previous.current);
                }
                drop(guard);
            });
        }
    }
}

#[test]
fn resident_domains_follow_exact_full_mode_and_checked_source_reattachment() {
    let mut code = original();
    code[OFFSETS[2]..OFFSETS[2] + 3].copy_from_slice(&[0x0f, 0x05, 0xc3]);
    with_source(Some(&code), |source, arena, stack, state| {
        let limits = NativeDispatchLimits {
            cache_slots: 2,
            ..Default::default()
        };
        let mut engine =
            NativeBlockStepper::new(source, &mut arena.pause().unwrap(), state, stack, &limits)
                .unwrap();
        for (function, &offset) in OFFSETS.iter().enumerate().take(2) {
            let mut current = state;
            current.guest_pc += offset as u64;
            if function != 0 {
                engine
                    .begin_call(&mut arena.pause().unwrap(), current)
                    .unwrap();
            }
            let body = plan(
                source,
                current.guest_pc,
                &code[offset..offset + 6],
                engine.placement_for(current.guest_pc),
            );
            // SAFETY: Original MOV/RET functions under the same checked owner.
            let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &body) };
            assert_eq!(
                stop.unwrap(),
                NativeBlockStop::Returned {
                    value: function as u64 + 1
                }
            );
        }
        let primary = direct_index(state.guest_pc, 2);
        let b_pc = state.guest_pc + OFFSETS[1] as u64;
        assert_eq!(
            engine.placement_for(b_pc),
            engine.slot_placement(primary ^ 1)
        );
        let original_mode = engine.machine.vector_mode;
        let original_slots = engine.machine.published_slots.clone();
        for mode in [
            VectorMode::Sse2,
            VectorMode::Avx {
                avx2: false,
                fma: false,
            },
            VectorMode::Avx {
                avx2: true,
                fma: false,
            },
            VectorMode::Avx {
                avx2: false,
                fma: true,
            },
            VectorMode::Avx {
                avx2: true,
                fma: true,
            },
        ] {
            if mode == original_mode {
                continue;
            }
            engine.machine.vector_mode = mode; // Selection only; never enter a fabricated mode.
            assert_eq!(engine.placement_for(b_pc), engine.slot_placement(primary));
            assert!(engine.machine.published_slots == original_slots);
        }
        engine.machine.vector_mode = original_mode;
        let mut current = state;
        current.guest_pc += OFFSETS[2] as u64;
        engine
            .begin_call(&mut arena.pause().unwrap(), current)
            .unwrap();
        let pending = plan(
            source,
            current.guest_pc,
            &code[OFFSETS[2]..OFFSETS[2] + 3],
            engine.placement_for(current.guest_pc),
        );
        // SAFETY: Genuine original pre-SYSCALL exit, with no native body.
        let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &pending) };
        let token = match stop.unwrap() {
            NativeBlockStop::PreSyscall(token) => token,
            other => panic!("unexpected original stop: {other:?}"),
        };
        let added = build_elf(&[0xc3]);
        let next = pentax_linker::append_symbolic_module(
            source.source_plan(),
            ModuleInput::new(
                open_executable(&added).unwrap(),
                source.source_plan().modules()[0].load_bias() + 4 * 1024 * 1024,
            ),
            &Default::default(),
        )
        .unwrap();
        let next_source = declare_materialized_source(&next, &[], &BlockLimits::default()).unwrap();
        let mut retained = Some(engine.into_machine());
        let mut engine = NativeBlockStepper::from_machine(
            &next_source,
            &mut arena.pause().unwrap(),
            &mut retained,
        )
        .unwrap();
        assert!(engine.machine.published_slots == original_slots);
        assert_eq!(engine.placement_for(b_pc), engine.slot_placement(primary)); // Old epoch is stale.
        engine.complete_syscall(token, 42).unwrap();
        let ret = plan(
            &next_source,
            current.guest_pc + 2,
            &[0xc3],
            engine.placement_for(current.guest_pc + 2),
        );
        // SAFETY: Resume only the mapped old module, never the unapplied append.
        let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &ret) };
        assert_eq!(stop.unwrap(), NativeBlockStop::Returned { value: 42 });
        engine
            .begin_call(&mut arena.pause().unwrap(), state)
            .unwrap();
        let body = plan(
            &next_source,
            state.guest_pc,
            &code[..6],
            engine.placement_for(state.guest_pc),
        );
        // SAFETY: New-epoch source proof still performs every actual live check.
        let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &body) };
        assert_eq!(stop.unwrap(), NativeBlockStop::Returned { value: 1 });
        let slot = &engine.machine.published_slots[primary];
        assert_eq!(
            slot.current.unwrap().source,
            source.source_plan().identity()
        );
        assert_eq!(slot.resident.unwrap().key.source, next.identity());
        assert_eq!(engine.usage().cache_publications, 2);
        assert_eq!(engine.usage().cache_reuses, 1);
        assert_eq!(engine.usage().cache_replacements, 0);
    });
}

#[test]
fn cached_hints_cannot_bypass_current_source_owner_operand_permissions_or_work() {
    let code = [0x8b, 0x07, 0xc3]; // Original MOV EAX,[RDI]; RET.
    for case in 0..6 {
        with_source(Some(&code), |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            state.gpr[RDI] = data;
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let body = plan(
                source,
                state.guest_pc,
                &code,
                engine.placement_for(state.guest_pc),
            );
            // SAFETY: Genuine checked original read from the owned data page.
            let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &body) };
            assert_eq!(stop.unwrap(), NativeBlockStop::Returned { value: 100 });
            engine
                .begin_call(&mut arena.pause().unwrap(), state)
                .unwrap();
            let history = engine.machine.published_slots.clone();
            let before = engine.usage();
            let maximum = engine.machine.limits.max_work_units;
            let page = state.guest_pc & !(PAGE - 1);
            match case {
                0 => arena.protect(page, PAGE, NativePermissions::READ).unwrap(),
                1 => arena
                    .protect(page, PAGE, NativePermissions::new(true, true, true))
                    .unwrap(),
                2 => {
                    arena
                        .protect(page, PAGE, NativePermissions::READ_WRITE)
                        .unwrap();
                    arena
                        .pause()
                        .unwrap()
                        .write(state.guest_pc, &[0x90])
                        .unwrap();
                    arena
                        .protect(page, PAGE, NativePermissions::READ_EXECUTE)
                        .unwrap();
                }
                3 => arena
                    .protect(data, PAGE, NativePermissions::new(false, true, false))
                    .unwrap(),
                5 => {
                    engine.machine.limits.max_work_units =
                        before.work_units + RESIDENT_HINT_WORK - 1
                }
                _ => {}
            }
            let refused = if case == 4 {
                let mut other =
                    NativeAddressSpace::reserve_any(Default::default(), 65_536).unwrap();
                // SAFETY: Deliberate wrong-owner refusal, never actual entry.
                unsafe { engine.step_checked(&mut other.pause().unwrap(), &body) }.map(|_| true)
            } else if case == 5 {
                engine.publish(&arena.pause().unwrap(), &body) // Exact reuse, one hint-work unit short.
            } else {
                // SAFETY: Cached hint is metadata; current live checks must refuse.
                unsafe { engine.step_checked(&mut arena.pause().unwrap(), &body) }.map(|_| true)
            };
            assert!(refused.is_err());
            assert_eq!(*engine.state(), state);
            assert!(engine.machine.published_slots == history);
            assert_eq!(engine.usage().cache_publications, before.cache_publications);
            assert_eq!(engine.usage().cache_reuses, before.cache_reuses);
            assert_eq!(engine.usage().cache_replacements, before.cache_replacements);
            assert_eq!(engine.usage().native_entries, before.native_entries);
            engine.machine.limits.max_work_units = maximum;
            if case == 3 {
                arena
                    .protect(data, PAGE, NativePermissions::READ_WRITE)
                    .unwrap();
                arena
                    .pause()
                    .unwrap()
                    .write(data, &123_u32.to_le_bytes())
                    .unwrap();
                // SAFETY: Same hinted body, restored actual rights and fresh bytes.
                let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &body) };
                assert_eq!(stop.unwrap(), NativeBlockStop::Returned { value: 123 });
                assert_eq!(engine.usage().cache_reuses, before.cache_reuses + 1);
            }
        });
    }
}

fn comparison(calls: u64) {
    let code = original();
    for paired in [false, true] {
        with_source(Some(&code), |source, arena, stack, state| {
            let limits = NativeDispatchLimits {
                cache_slots: 4096,
                cache_bytes: 2 * 1024 * 1024,
                max_blocks: calls,
                max_work_units: 1024 * 1024 * 1024,
                ..Default::default()
            };
            let mut engine =
                NativeBlockStepper::new(source, &mut arena.pause().unwrap(), state, stack, &limits)
                    .unwrap();
            let primary = direct_index(state.guest_pc, limits.cache_slots);
            assert_eq!(
                primary,
                direct_index(state.guest_pc + OFFSETS[1] as u64, limits.cache_slots)
            );
            let ways = [
                engine.slot_placement(primary),
                engine.slot_placement(primary ^ 1),
            ];
            let plans: Vec<_> = (0..2)
                .map(|function| {
                    ways.map(|placement| {
                        plan(
                            source,
                            state.guest_pc + OFFSETS[function] as u64,
                            &code[OFFSETS[function]..OFFSETS[function] + 6],
                            placement,
                        )
                    })
                })
                .collect();
            let start = std::time::Instant::now();
            for call in 0..calls {
                let function = call as usize & 1;
                let mut current = state;
                current.guest_pc += OFFSETS[function] as u64;
                if call != 0 {
                    engine
                        .begin_call(&mut arena.pause().unwrap(), current)
                        .unwrap();
                }
                let placement = if paired {
                    engine.placement_for(current.guest_pc)
                } else {
                    ways[0]
                };
                let way = usize::from(placement == ways[1]);
                assert_eq!(placement, ways[way]);
                // SAFETY: Identical original calls/limits/live checks in both
                // cases. Only the controlled storage-selection strategy differs.
                let stop = unsafe {
                    engine.step_checked(&mut arena.pause().unwrap(), &plans[function][way])
                };
                assert_eq!(
                    stop.unwrap(),
                    NativeBlockStop::Returned {
                        value: function as u64 + 1
                    }
                );
            }
            let elapsed = start.elapsed().as_nanos();
            let usage = engine.usage();
            assert_eq!(usage.blocks, calls);
            assert_eq!(usage.native_entries, calls);
            assert_eq!(usage.native_instructions, calls);
            assert_eq!(usage.vector_operations, 0);
            assert_eq!(usage.cache_publications, if paired { 2 } else { calls });
            assert_eq!(usage.cache_reuses, if paired { calls - 2 } else { 0 });
            assert_eq!(usage.cache_replacements, if paired { 0 } else { calls - 1 });
            assert_eq!(
                usage.cache_evicted_key_revisits,
                if paired { 0 } else { calls - 2 }
            );
            println!(
                "two_way_native_cache_available=true mode={} calls={calls} elapsed_ns={elapsed} publications={} reuses={} replacements={} revisits={} work={}",
                if paired { "paired" } else { "direct" },
                usage.cache_publications,
                usage.cache_reuses,
                usage.cache_replacements,
                usage.cache_evicted_key_revisits,
                usage.work_units
            );
        });
    }
}

#[test]
fn bounded_original_comparison_keeps_results_and_native_counts_with_unchanged_capacities() {
    comparison(8);
}

#[test]
#[ignore = "finite original direct-placement versus paired-placement measurement"]
fn original_two_way_native_cache_finite_measurement() {
    comparison(50_000);
}
