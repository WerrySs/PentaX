// SPDX-License-Identifier: GPL-2.0-only
//! Original source-key histories. No identity is exposed through observations.
use super::*;

const FUNCTIONS: [u8; 18] = [
    0xb8, 1, 0, 0, 0, 0xc3, 0xb8, 2, 0, 0, 0, 0xc3, 0xb8, 3, 0, 0, 0, 0xc3,
];

#[test]
fn committed_publications_distinguish_immediate_evicted_revisits_from_new_frontiers() {
    for (sequence, publications, reuses, replacements, revisits) in
        [([0, 1, 0, 0], 3, 1, 2, 1), ([0, 1, 2, 0], 4, 0, 3, 0)]
    {
        with_source(Some(&FUNCTIONS), |source, arena, stack, state| {
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let capacity = engine.machine.published_slots.capacity();
            for (call, function) in sequence.into_iter().enumerate() {
                let mut current = state;
                current.guest_pc += function * 6;
                if call != 0 {
                    engine
                        .begin_call(&mut arena.pause().unwrap(), current)
                        .unwrap();
                }
                let plan = block(
                    source,
                    &arena.pause().unwrap(),
                    current.guest_pc,
                    engine.placement(),
                );
                assert_eq!(
                    // SAFETY: Owned original MOV/RET source, live checked stack
                    // and ordinary native admission are retained for this call.
                    unsafe { engine.step_checked(&mut arena.pause().unwrap(), &plan) }.unwrap(),
                    NativeBlockStop::Returned {
                        value: function + 1
                    }
                );
                assert_eq!(engine.machine.published_slots.capacity(), capacity);
                assert_eq!(engine.machine.published_slots.len(), 1);
            }
            let usage = engine.usage();
            assert_eq!(usage.cache_publications, publications);
            assert_eq!(usage.cache_reuses, reuses);
            assert_eq!(usage.cache_replacements, replacements);
            assert_eq!(usage.cache_evicted_key_revisits, revisits);
            assert!(!usage.cache_history_saturated);
            assert_eq!(usage.native_entries, 4);
            assert_eq!(usage.native_instructions, 4);
        });
    }
    with_source(Some(&FUNCTIONS), |source, arena, stack, state| {
        let limits = NativeDispatchLimits {
            cache_slots: 4,
            ..Default::default()
        };
        let mut engine =
            NativeBlockStepper::new(source, &mut arena.pause().unwrap(), state, stack, &limits)
                .unwrap();
        for index in 0..3 {
            let plan = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc + index * 6,
                engine.slot_placement(index as usize),
            );
            assert!(engine.publish(&arena.pause().unwrap(), &plan).unwrap());
        }
        assert_eq!(engine.usage().cache_publications, 3);
        assert_eq!(engine.usage().cache_replacements, 0);
        assert_eq!(engine.usage().cache_evicted_key_revisits, 0);
        assert_eq!(engine.usage().native_entries, 0); // Publication is not execution.
    });
}

#[test]
fn byte_reuse_repairs_and_saturated_observations_never_rename_or_authorize_a_slot() {
    // A and B have distinct source PCs but identical emissions. Their reuse
    // cannot rename the historical producer; these are publication statistics.
    let code = [
        0xb8, 1, 0, 0, 0, 0xc3, 0xb8, 1, 0, 0, 0, 0xc3, 0xb8, 2, 0, 0, 0, 0xc3,
    ];
    with_source(Some(&code), |source, arena, stack, state| {
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        let a = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc,
            engine.placement(),
        );
        let b = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc + 6,
            engine.placement(),
        );
        let c = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc + 12,
            engine.placement(),
        );
        assert!(engine.publish(&arena.pause().unwrap(), &a).unwrap());
        let first = engine.machine.published_slots[0];
        assert!(engine.publish(&arena.pause().unwrap(), &b).unwrap());
        assert!(engine.machine.published_slots[0] == first);
        assert_eq!(engine.usage().cache_publications, 1);
        assert_eq!(engine.usage().cache_reuses, 1);
        assert!(engine.publish(&arena.pause().unwrap(), &c).unwrap());
        assert!(engine.publish(&arena.pause().unwrap(), &b).unwrap());
        assert_eq!(engine.usage().cache_replacements, 2);
        assert_eq!(engine.usage().cache_evicted_key_revisits, 0); // B != historical A.
        let history = engine.machine.published_slots[0];
        engine.machine.cache.arm_writable().unwrap();
        engine.machine.cache.write(0, &[0x90]).unwrap();
        engine.machine.cache.seal_executable().unwrap();
        assert!(engine.publish(&arena.pause().unwrap(), &b).unwrap());
        assert_eq!(engine.usage().cache_replacements, 3);
        assert_eq!(engine.usage().cache_evicted_key_revisits, 0);
        assert!(engine.machine.published_slots[0].current == history.current);
        assert!(engine.machine.published_slots[0].previous_evicted == history.previous_evicted);
        // Diagnostic overflow is sticky and saturating; it cannot stop a real
        // successful publication or prevent the next historical-key update.
        engine.machine.usage.cache_replacements = u64::MAX;
        engine.machine.usage.cache_evicted_key_revisits = u64::MAX;
        assert!(engine.publish(&arena.pause().unwrap(), &c).unwrap());
        assert!(engine.publish(&arena.pause().unwrap(), &b).unwrap());
        assert_eq!(engine.usage().cache_replacements, u64::MAX);
        assert_eq!(engine.usage().cache_evicted_key_revisits, u64::MAX);
        assert!(engine.usage().cache_history_saturated);
        assert_eq!(engine.usage().cache_publications, 6);
        assert_eq!(engine.usage().native_entries, 0);
        assert_eq!(*engine.state(), state);
    });
}

#[test]
fn historical_keys_keep_exact_epoch_and_full_mode_without_storing_execution_authority() {
    with_source(Some(&FUNCTIONS), |source, _, _, state| {
        let bytes = build_elf(&FUNCTIONS);
        let other = plan_symbolic(
            &[ModuleInput::new(
                open_executable(&bytes).unwrap(),
                source.source_plan().modules()[0].load_bias(),
            )],
            &SymbolicLimits::default(),
        )
        .unwrap();
        assert_ne!(source.source_plan().identity(), other.identity());
        let original = PublicationKey {
            source: source.source_plan().identity(),
            guest_pc: state.guest_pc,
            vector_mode: VectorMode::Sse2,
        };
        for incoming in [
            PublicationKey {
                source: other.identity(),
                ..original
            },
            PublicationKey {
                vector_mode: VectorMode::Avx {
                    avx2: false,
                    fma: false,
                },
                ..original
            },
            PublicationKey {
                vector_mode: VectorMode::Avx {
                    avx2: true,
                    fma: false,
                },
                ..original
            },
            PublicationKey {
                vector_mode: VectorMode::Avx {
                    avx2: false,
                    fma: true,
                },
                ..original
            },
            PublicationKey {
                vector_mode: VectorMode::Avx {
                    avx2: true,
                    fma: true,
                },
                ..original
            },
        ] {
            let mut slot = PublicationSlot::default();
            let mut usage = NativeDispatchUsage::default();
            slot.commit(original, 11, &mut usage);
            slot.commit(
                PublicationKey {
                    guest_pc: state.guest_pc + 6,
                    ..original
                },
                11,
                &mut usage,
            );
            slot.commit(incoming, 11, &mut usage);
            assert_eq!(usage.cache_replacements, 2);
            assert_eq!(usage.cache_evicted_key_revisits, 0);
            assert!(!usage.cache_history_saturated);
        }
    });
    let mut code = FUNCTIONS[..12].to_vec();
    code.extend([0x0f, 0x05, 0xc3]);
    with_source(Some(&code), |source, arena, stack, state| {
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        for (call, offset) in [0, 6, 12].into_iter().enumerate() {
            let mut current = state;
            current.guest_pc += offset;
            if call != 0 {
                engine
                    .begin_call(&mut arena.pause().unwrap(), current)
                    .unwrap();
            }
            let plan = block(
                source,
                &arena.pause().unwrap(),
                current.guest_pc,
                engine.placement(),
            );
            // SAFETY: Only original MOV/RET or the genuine pre-SYSCALL exit.
            let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &plan) }.unwrap();
            if call < 2 {
                assert_eq!(
                    stop,
                    NativeBlockStop::Returned {
                        value: call as u64 + 1
                    }
                );
            } else {
                assert!(matches!(stop, NativeBlockStop::PreSyscall(_)));
            }
        }
        let old_history = engine.machine.published_slots[0];
        let token = match engine.machine.phase {
            Phase::Syscall(token) => token,
            _ => panic!("missing original pending syscall"),
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
        assert!(engine.machine.published_slots[0] == old_history);
        engine.complete_syscall(token, 42).unwrap();
        let plan = block(
            &next_source,
            &arena.pause().unwrap(),
            state.guest_pc + 14,
            engine.placement(),
        );
        assert_eq!(
            // SAFETY: Resume the original old-module return with live checked
            // ownership and stack; the appended module is not executed here.
            unsafe { engine.step_checked(&mut arena.pause().unwrap(), &plan) }.unwrap(),
            NativeBlockStop::Returned { value: 42 }
        );
        engine
            .begin_call(&mut arena.pause().unwrap(), state)
            .unwrap();
        let plan = block(
            &next_source,
            &arena.pause().unwrap(),
            state.guest_pc,
            engine.placement(),
        );
        assert_eq!(
            // SAFETY: Same owned original code and checked stack in the
            // admitted appended source epoch.
            unsafe { engine.step_checked(&mut arena.pause().unwrap(), &plan) }.unwrap(),
            NativeBlockStop::Returned { value: 1 }
        );
        assert_eq!(engine.usage().cache_publications, 3);
        assert_eq!(engine.usage().cache_replacements, 2);
        assert_eq!(engine.usage().cache_evicted_key_revisits, 0); // New epoch != old A.
        assert_eq!(
            engine.machine.published_slots[0].current.unwrap().source,
            next.identity()
        );
    });
}

#[test]
fn refused_current_source_rights_placement_and_work_leave_committed_history_unchanged() {
    for case in 0..5 {
        with_source(Some(&FUNCTIONS), |source, arena, stack, state| {
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let a = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            assert!(engine.publish(&arena.pause().unwrap(), &a).unwrap());
            let placement = if case == 3 {
                CachePlacement {
                    address: engine.placement().address + 1,
                    ..engine.placement()
                }
            } else {
                engine.placement()
            };
            let b = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc + 6,
                placement,
            );
            let history = engine.machine.published_slots[0];
            let usage = engine.usage();
            let page = state.guest_pc & !(PAGE - 1);
            match case {
                0 => arena
                    .protect(page, PAGE, NativePermissions::new(true, true, true))
                    .unwrap(),
                1 => {
                    arena
                        .protect(page, PAGE, NativePermissions::READ_WRITE)
                        .unwrap();
                    arena
                        .pause()
                        .unwrap()
                        .write(state.guest_pc + 6, &[0x90])
                        .unwrap();
                    arena
                        .protect(page, PAGE, NativePermissions::READ_EXECUTE)
                        .unwrap();
                }
                2 => {
                    engine.machine.limits.max_work_units =
                        engine.usage().work_units + PUBLICATION_HISTORY_WORK - 1
                }
                4 => arena.protect(page, PAGE, NativePermissions::READ).unwrap(),
                _ => {}
            }
            let result = if case == 4 {
                let mut current = state;
                current.guest_pc += 6;
                engine.machine.state = current;
                // SAFETY: Revoked execute rights must refuse before entry.
                unsafe { engine.step_checked(&mut arena.pause().unwrap(), &b) }.map(|_| true)
            } else {
                engine.publish(&arena.pause().unwrap(), &b)
            };
            assert!(result.is_err());
            assert!(engine.machine.published_slots[0] == history);
            assert_eq!(engine.usage().cache_publications, usage.cache_publications);
            assert_eq!(engine.usage().cache_replacements, usage.cache_replacements);
            assert_eq!(
                engine.usage().cache_evicted_key_revisits,
                usage.cache_evicted_key_revisits
            );
            assert_eq!(
                engine.usage().cache_history_saturated,
                usage.cache_history_saturated
            );
            assert_eq!(engine.usage().native_entries, 0);
            let mut expected = state;
            if case == 4 {
                expected.guest_pc += 6;
            }
            assert_eq!(*engine.state(), expected);
        });
    }
}

#[test]
fn publication_history_capacity_and_fixed_work_are_admitted_without_slot_growth() {
    with_source(Some(&FUNCTIONS), |source, arena, stack, state| {
        let mut initial_work = 0;
        for slots in [1, 4] {
            let limits = NativeDispatchLimits {
                cache_slots: slots,
                ..Default::default()
            };
            let mut engine =
                NativeBlockStepper::new(source, &mut arena.pause().unwrap(), state, stack, &limits)
                    .unwrap();
            assert_eq!(engine.machine.published_slots.capacity(), slots);
            assert_eq!(engine.machine.published_slots.len(), slots);
            let work = engine.usage().work_units;
            if slots == 1 {
                initial_work = work;
            } else {
                assert_eq!(
                    work - initial_work,
                    3 * std::mem::size_of::<PublicationSlot>() as u64
                );
            }
            let plan = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            assert!(engine.publish(&arena.pause().unwrap(), &plan).unwrap());
            assert_eq!(engine.usage().work_units - work, PUBLICATION_HISTORY_WORK);
            let work = engine.usage().work_units;
            assert!(engine.publish(&arena.pause().unwrap(), &plan).unwrap());
            assert_eq!(engine.usage().work_units, work); // Reuse adds no history work.
        }
        let limits = NativeDispatchLimits {
            cache_slots: 4096,
            cache_bytes: 4096 * 128,
            ..Default::default()
        };
        let engine =
            NativeBlockStepper::new(source, &mut arena.pause().unwrap(), state, stack, &limits)
                .unwrap();
        assert_eq!(engine.machine.published_slots.capacity(), 4096);
        assert!(std::mem::size_of::<PublicationSlot>() <= 80);
        assert!(
            engine.machine.published_slots.capacity() * std::mem::size_of::<PublicationSlot>()
                <= 320 * 1024
        );
        drop(engine);
        let limits = NativeDispatchLimits {
            max_work_units: initial_work - 129,
            ..Default::default()
        };
        assert!(matches!(
            NativeBlockStepper::new(source, &mut arena.pause().unwrap(), state, stack, &limits),
            Err(NativeDispatchError::Budget("cache metadata"))
        ));
    });
}
