// SPDX-License-Identifier: GPL-2.0-only
//! Original entry/RET continuation proofs, not libc export authorization.

use super::*;

#[test]
fn builtin_entry_call_and_tail_jump_preserve_normal_ret_context_and_observation() {
    for opcode in [0xe8, 0xe9] {
        // CALL/JMP function; RET; padding; MOV EAX,deadbeef; RET.
        // The replacement must never execute that MOV or manufacture a SYSCALL.
        let mut code = vec![opcode, 11, 0, 0, 0, 0xc3];
        code.resize(16, 0x90);
        code.extend([0xb8, 0xef, 0xbe, 0xad, 0xde, 0xc3]);
        with_source(Some(&code), |source, arena, stack, mut state| {
            for (index, value) in state.gpr.iter_mut().enumerate() {
                if index != RSP {
                    *value = 0x1234_0000_0000_0000 + index as u64;
                }
            }
            state.xmm = std::array::from_fn(|i| (i as u8).wrapping_mul(13));
            state.ymm_upper = std::array::from_fn(|i| (i as u8).wrapping_mul(17));
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let first = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            // SAFETY: This exact original plan has no native body. Its direct
            // branch uses the real owned stack and initialized source target.
            let resumed = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &first) };
            assert!(matches!(resumed, Ok(NativeBlockStop::Resume { .. })));
            assert_eq!(engine.state().guest_pc, state.guest_pc + 16);
            let entry = *engine.state();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                entry.guest_pc,
                engine.placement(),
            );
            assert!(planned.native_admission().emitted_instructions() > 0);
            let token = engine
                .begin_libc_builtin(&arena.pause().unwrap(), &planned)
                .unwrap();
            assert_eq!(token.entry(), entry.guest_pc);
            assert_eq!(token.return_slot(), entry.gpr[RSP]);
            assert_eq!(token.source_identity(), source.source_plan().identity());
            assert_eq!(
                token.args(),
                [
                    entry.gpr[RDI],
                    entry.gpr[RSI],
                    entry.gpr[RDX],
                    entry.gpr[RCX],
                    entry.gpr[8],
                    entry.gpr[9]
                ]
            );
            assert_eq!(
                token.return_target(),
                (opcode == 0xe8).then_some(state.guest_pc + 5)
            );
            engine
                .validate_libc_builtin(&arena.pause().unwrap(), &token)
                .unwrap();
            let before = engine.usage();
            // A successful completion needs no new budget, even at exact limits.
            engine.machine.limits.max_work_units = before.work_units;
            let result = engine
                .complete_libc_builtin(&arena.pause().unwrap(), token, 42)
                .unwrap();
            let mut expected = entry;
            expected.gpr[RAX] = 42;
            expected.gpr[RSP] += 8;
            expected.guest_pc = if opcode == 0xe8 {
                state.guest_pc + 5
            } else {
                stack.return_marker
            };
            assert_eq!(*engine.state(), expected); // Includes RCX/R11, flags and all SIMD/control bits.
            assert_eq!(engine.usage(), before);
            assert_eq!(before.native_entries, 0);
            assert_eq!(before.native_instructions, 0);
            assert_eq!(before.cache_publications, 0);
            if opcode == 0xe8 {
                assert_eq!(
                    result,
                    NativeBlockStop::Resume {
                        guest_pc: state.guest_pc + 5
                    }
                );
                assert!(engine.observed_return().is_none());
                engine.machine.limits.max_work_units =
                    NativeDispatchLimits::default().max_work_units;
                let last = block(
                    source,
                    &arena.pause().unwrap(),
                    engine.state().guest_pc,
                    engine.placement(),
                );
                assert_eq!(
                    engine
                        .step_return_exit(&mut arena.pause().unwrap(), &last)
                        .unwrap(),
                    NativeBlockStop::Returned { value: 42 }
                );
            } else {
                assert_eq!(result, NativeBlockStop::Returned { value: 42 });
            }
            let observed = engine.observed_return().unwrap();
            engine.validate_call_return(&observed).unwrap();
            assert_eq!(observed.initial_state(), &state);
            assert_eq!(observed.value(), 42);
            assert_eq!(engine.usage().blocks, if opcode == 0xe8 { 3 } else { 2 });
        });
    }
}

#[test]
fn builtin_entry_refuses_live_source_return_and_budget_failures_before_effects() {
    for failure in 0..6 {
        let code = [0xb8, 1, 0, 0, 0, 0xc3];
        with_source(Some(&code), |source, arena, stack, mut state| {
            if failure == 5 {
                state.gpr[RSP] -= 8;
            }
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            let page = source.source_plan().modules()[0].load_bias();
            match failure {
                0 => engine.machine.limits.max_work_units = engine.usage().work_units,
                1 => engine.machine.limits.max_blocks = engine.usage().blocks,
                2 => arena
                    .protect(page, PAGE, NativePermissions::new(true, true, true))
                    .unwrap(),
                3 => {
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
                4 => arena
                    .pause()
                    .unwrap()
                    .write(state.gpr[RSP], &u64::MAX.to_le_bytes())
                    .unwrap(),
                5 => arena
                    .pause()
                    .unwrap()
                    .write(state.gpr[RSP], &stack.return_marker.to_le_bytes())
                    .unwrap(),
                _ => unreachable!(),
            }
            let result = engine.begin_libc_builtin(&arena.pause().unwrap(), &planned);
            let expected = match failure {
                0 => NativeDispatchError::Budget("stack work"),
                1 => NativeDispatchError::Budget("blocks"),
                2 => NativeDispatchError::WritableSource,
                3 => NativeDispatchError::SourceMismatch,
                4 => NativeDispatchError::InvalidTarget(u64::MAX),
                5 => NativeDispatchError::InvalidTarget(stack.return_marker),
                _ => unreachable!(),
            };
            assert_eq!(result.unwrap_err(), expected);
            assert_eq!(*engine.state(), state);
            assert!(engine.machine.phase == Phase::Ready);
            assert_eq!(engine.usage().native_entries, 0);
            assert_eq!(engine.usage().cache_publications, 0);
        });
    }
}

#[test]
fn builtin_completion_late_faults_keep_helper_writes_and_rax_without_replay() {
    for failure in 0..4 {
        with_source(
            Some(&[0xb8, 1, 0, 0, 0, 0xc3]),
            |source, arena, stack, mut state| {
                let return_target = state.guest_pc + 5;
                if failure == 3 {
                    state.gpr[RSP] -= 8;
                    arena
                        .pause()
                        .unwrap()
                        .write(state.gpr[RSP], &return_target.to_le_bytes())
                        .unwrap();
                }
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    state,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                let planned = block(
                    source,
                    &arena.pause().unwrap(),
                    state.guest_pc,
                    engine.placement(),
                );
                let token = engine
                    .begin_libc_builtin(&arena.pause().unwrap(), &planned)
                    .unwrap();
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                // Direct checked-memory effect stands in for a separately admitted
                // helper in this platform-only continuation proof.
                arena.pause().unwrap().write(data, &[0x71; 8]).unwrap();
                match failure {
                    0 => arena
                        .pause()
                        .unwrap()
                        .write(state.gpr[RSP], &u64::MAX.to_le_bytes())
                        .unwrap(),
                    1 => arena
                        .protect(
                            stack.stack.address,
                            stack.stack.size,
                            NativePermissions::READ,
                        )
                        .unwrap(),
                    2 => arena
                        .protect(
                            stack.stack.address,
                            stack.stack.size,
                            NativePermissions::new(true, true, true),
                        )
                        .unwrap(),
                    3 => arena
                        .protect(
                            source.source_plan().modules()[0].load_bias(),
                            PAGE,
                            NativePermissions::READ,
                        )
                        .unwrap(),
                    _ => unreachable!(),
                }
                let before = engine.usage();
                engine.machine.limits.max_work_units = before.work_units;
                let result = engine.complete_libc_builtin(&arena.pause().unwrap(), token, data);
                match failure {
                    0 => assert_eq!(result, Err(NativeDispatchError::InvalidTarget(u64::MAX))),
                    2 => assert_eq!(result, Err(NativeDispatchError::InvalidStack)),
                    _ => assert!(matches!(
                        result,
                        Err(NativeDispatchError::Memory(
                            NativeMemoryError::PermissionDenied(_)
                        ))
                    )),
                }
                let mut expected = state;
                expected.gpr[RAX] = data;
                assert_eq!(*engine.state(), expected);
                assert!(engine.machine.phase == Phase::Terminal);
                assert!(engine.observed_return().is_none());
                assert_eq!(engine.usage(), before);
                let mut actual = [0; 8];
                arena.pause().unwrap().read(data, &mut actual).unwrap();
                assert_eq!(actual, [0x71; 8]);
                assert_eq!(
                    engine
                        .begin_libc_builtin(&arena.pause().unwrap(), &planned)
                        .unwrap_err(),
                    NativeDispatchError::WrongPhase
                );
            },
        );
    }
}

#[test]
fn builtin_pending_phase_blocks_context_replacement_and_cancel_retains_admission() {
    with_source(Some(&[0xc3]), |source, arena, stack, state| {
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        let address = stack.guards.unwrap()[1].address + 3 * PAGE;
        arena
            .map_at(address, PAGE, NativePermissions::READ_WRITE)
            .unwrap();
        let other_stack = NativeStackFrame {
            stack: NativeRange {
                address,
                size: PAGE,
            },
            guards: None,
            return_slot: address + PAGE - 8,
            return_marker: stack.return_marker,
        };
        arena
            .pause()
            .unwrap()
            .write(other_stack.return_slot, &stack.return_marker.to_le_bytes())
            .unwrap();
        let other = NativeCpuState::new(state.guest_pc, other_stack.return_slot);
        let mut parked = engine
            .prepare_thread(&mut arena.pause().unwrap(), other, other_stack)
            .unwrap();
        let planned = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc,
            engine.placement(),
        );
        let token = engine
            .begin_libc_builtin(&arena.pause().unwrap(), &planned)
            .unwrap();
        let first_pending = engine.machine.phase;
        let before = engine.usage();
        assert_eq!(
            engine.exchange_thread(&mut arena.pause().unwrap(), &mut parked),
            Err(NativeDispatchError::WrongPhase)
        );
        assert_eq!(
            engine
                .prepare_thread(&mut arena.pause().unwrap(), other, other_stack)
                .unwrap_err(),
            NativeDispatchError::WrongPhase
        );
        assert_eq!(
            engine
                .begin_libc_builtin(&arena.pause().unwrap(), &planned)
                .unwrap_err(),
            NativeDispatchError::WrongPhase
        );
        engine.cancel_libc_builtin(token).unwrap();
        assert_eq!(engine.usage(), before);
        assert_eq!(*engine.state(), state);
        let token = engine
            .begin_libc_builtin(&arena.pause().unwrap(), &planned)
            .unwrap();
        assert_eq!(engine.usage().blocks, before.blocks + 1);
        assert!(engine.machine.phase != first_pending); // Same entry/state/call, distinct nonce.
        let mut suspended = Some(engine.into_machine());
        assert!(matches!(
            NativeBlockStepper::from_machine(source, &mut arena.pause().unwrap(), &mut suspended),
            Err(NativeDispatchError::WrongPhase)
        ));
        assert!(suspended.is_some());
        // Dropping the unavailable executor cannot turn its token into a new
        // engine's pending entry, even with identical source and guest state.
        drop(suspended);
        let mut fresh = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        let fresh_plan = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc,
            fresh.placement(),
        );
        let current = fresh
            .begin_libc_builtin(&arena.pause().unwrap(), &fresh_plan)
            .unwrap();
        assert_eq!(
            fresh.validate_libc_builtin(&arena.pause().unwrap(), &token),
            Err(NativeDispatchError::WrongPhase)
        );
        assert_eq!(
            fresh.complete_libc_builtin(&arena.pause().unwrap(), token, 99),
            Err(NativeDispatchError::WrongPhase)
        );
        fresh
            .validate_libc_builtin(&arena.pause().unwrap(), &current)
            .unwrap();
        fresh.fail_libc_builtin(current).unwrap();
        assert!(fresh.machine.phase == Phase::Terminal);
    });
}

#[test]
fn builtin_entry_exclusion_preserves_original_partial_rep_moves_and_stores() {
    for opcode in [0xa4, 0xaa] {
        // REP MOVSB / REP STOSB at the named entry.
        with_source(
            Some(&[0xf3, opcode, 0xc3]),
            |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                state.gpr[RSI] = data;
                state.gpr[RDI] = data + 1024;
                state.gpr[RCX] = 512;
                state.gpr[RAX] = 0x75;
                let original: [u8; 512] = std::array::from_fn(|i| (i as u8).wrapping_mul(19));
                arena.pause().unwrap().write(data, &original).unwrap();
                arena
                    .pause()
                    .unwrap()
                    .write(data + 1024, &[0x37; 512])
                    .unwrap();
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    state,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                let planned = block(
                    source,
                    &arena.pause().unwrap(),
                    state.guest_pc,
                    engine.placement(),
                );
                for completed in [0, 256] {
                    // None comparison state is NOT proof that an entry is fresh.
                    assert!(engine.libc_builtin_entry_ready());
                    assert!(engine.machine.string_repeat.is_none());
                    let before = *engine.state();
                    assert_eq!(
                        engine
                            .begin_libc_builtin(&arena.pause().unwrap(), &planned)
                            .unwrap_err(),
                        NativeDispatchError::WrongPhase
                    );
                    assert_eq!(*engine.state(), before);
                    // SAFETY: This original REP-only plan has no native body. All
                    // 512 bytes reside in the original owner's RW data mapping.
                    let resumed =
                        unsafe { engine.step_checked(&mut arena.pause().unwrap(), &planned) };
                    assert!(matches!(resumed, Ok(NativeBlockStop::Resume { .. })));
                    assert_eq!(engine.state().gpr[RCX], 512 - completed - 256);
                    assert_eq!(engine.state().gpr[RDI], data + 1024 + completed + 256);
                    assert_eq!(
                        engine.state().guest_pc,
                        if completed == 0 {
                            state.guest_pc
                        } else {
                            state.guest_pc + 2
                        }
                    );
                }
                let mut actual = [0; 512];
                arena
                    .pause()
                    .unwrap()
                    .read(data + 1024, &mut actual)
                    .unwrap();
                assert_eq!(
                    actual,
                    if opcode == 0xa4 {
                        original
                    } else {
                        [0x75; 512]
                    }
                );
                assert_eq!(engine.usage().string_elements, 512);
                assert_eq!(engine.usage().native_entries, 0);
                assert_eq!(engine.usage().native_instructions, 0);
            },
        );
    }
}
