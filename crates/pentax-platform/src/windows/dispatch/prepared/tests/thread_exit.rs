// SPDX-License-Identifier: GPL-2.0-only
//! Original attested return paths; every plan here has zero native body.

use super::*;

fn return_now(engine: &mut NativeBlockStepper<'_, '_, '_>, arena: &mut NativeAddressSpace) {
    let planned = block(
        engine.source,
        &arena.pause().unwrap(),
        engine.state().guest_pc,
        engine.placement(),
    );
    assert!(matches!(
        planned.exit().kind(),
        ExitKind::Return { adjust: 0 }
    ));
    assert_eq!(planned.bytes(), &[0x0f, 0x0b]);
    assert!(
        planned
            .instructions()
            .iter()
            .all(|instruction| instruction.emitted_len() == 0)
    );
    // Use the existing exact single-RET entry. It retains normal preflight and
    // real observed-return accounting without entering the assembly bridge.
    assert!(matches!(
        engine.step_return_exit(&mut arena.pause().unwrap(), &planned),
        Ok(NativeBlockStop::Returned { .. })
    ));
    assert_eq!(engine.usage().native_entries, 0);
    assert_eq!(engine.usage().native_instructions, 0);
}

fn other_stack(arena: &mut NativeAddressSpace, first: NativeStackFrame) -> NativeStackFrame {
    let address = first.guards.unwrap()[1].address + 3 * PAGE;
    arena
        .map_at(address, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let slot = address + PAGE - 8;
    arena
        .pause()
        .unwrap()
        .write(slot, &first.return_marker.to_le_bytes())
        .unwrap();
    NativeStackFrame {
        stack: NativeRange {
            address,
            size: PAGE,
        },
        guards: Some([
            NativeRange {
                address: address - PAGE,
                size: PAGE,
            },
            NativeRange {
                address: address + PAGE,
                size: PAGE,
            },
        ]),
        return_slot: slot,
        return_marker: first.return_marker,
    }
}

#[test]
fn current_outer_return_requires_actual_observation_and_exact_owner_executor_source() {
    with_source(Some(&[0xc3]), |source, arena, stack, mut state| {
        state.gpr[RAX] = 42;
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(engine.call_sequence(), 0);
        return_now(&mut engine, arena);
        let returned = engine.observed_return().unwrap();
        assert_eq!(
            (engine.executor_id(), engine.call_sequence()),
            (returned.executor_id(), returned.call_sequence())
        );
        assert_eq!(returned.value(), 42);
        let before_state = *engine.state();
        let before_usage = engine.usage();
        assert_eq!(engine.validate_call_return(&returned), Ok(()));
        assert_eq!(engine.validate_call_return(&returned), Ok(())); // Read-only, not consumption.
        assert_eq!(
            (*engine.state(), engine.usage()),
            (before_state, before_usage)
        );

        let mut data_only = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        // Direct data-only resolve may reach Returned, but cannot mint the
        // observation that outer-call cleanup requires.
        let planned = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc,
            data_only.placement(),
        );
        assert_eq!(
            data_only.resolve(&mut arena.pause().unwrap(), &planned),
            Ok(NativeBlockStop::Returned { value: 42 })
        );
        assert!(data_only.observed_return().is_none());
        assert_eq!(
            data_only.validate_call_return(&returned),
            Err(NativeDispatchError::WrongPhase)
        );
        let mut foreign = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        assert_ne!(engine.executor_id(), foreign.executor_id());
        return_now(&mut foreign, arena);
        let foreign_return = foreign.observed_return().unwrap();
        assert_eq!(
            (
                returned.owner_id(),
                returned.call_sequence(),
                returned.initial_state(),
                returned.stack(),
                returned.value()
            ),
            (
                foreign_return.owner_id(),
                foreign_return.call_sequence(),
                foreign_return.initial_state(),
                foreign_return.stack(),
                foreign_return.value()
            )
        );
        assert_eq!(
            engine.validate_call_return(&foreign_return),
            Err(NativeDispatchError::WrongPhase)
        );
        assert_eq!(
            foreign.validate_call_return(&returned),
            Err(NativeDispatchError::WrongPhase)
        );

        with_source(
            Some(&[0xc3]),
            |other_source, other_arena, other_stack, other_state| {
                let mut other = NativeBlockStepper::new(
                    other_source,
                    &mut other_arena.pause().unwrap(),
                    other_state,
                    other_stack,
                    &Default::default(),
                )
                .unwrap();
                return_now(&mut other, other_arena);
                let other_return = other.observed_return().unwrap();
                assert_ne!(returned.owner_id(), other_return.owner_id());
                assert!(!std::ptr::eq(
                    returned.source_plan(),
                    other_return.source_plan()
                ));
                assert_eq!(
                    engine.validate_call_return(&other_return),
                    Err(NativeDispatchError::WrongPhase)
                );
                assert_eq!(
                    other.validate_call_return(&returned),
                    Err(NativeDispatchError::WrongPhase)
                );
            },
        );
        assert_eq!(
            (*engine.state(), engine.usage()),
            (before_state, before_usage)
        );
        assert_eq!(engine.validate_call_return(&returned), Ok(()));
    });
}

#[test]
fn outer_return_validation_follows_selected_context_and_refuses_nested_or_historical_returns() {
    with_source(Some(&[0x0f, 0x05, 0xc3]), |source, arena, stack, state| {
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        let executor = engine.executor_id();
        let child_stack = other_stack(arena, stack);
        let mut child = NativeCpuState::new(state.guest_pc + 2, child_stack.return_slot);
        child.gpr[RAX] = 73;
        let mut parked = engine
            .prepare_thread(&mut arena.pause().unwrap(), child, child_stack)
            .unwrap();
        let child_sequence = parked.call_sequence();
        assert_ne!(engine.call_sequence(), child_sequence);
        assert_eq!(engine.call_sequence(), 0);
        engine
            .exchange_thread(&mut arena.pause().unwrap(), &mut parked)
            .unwrap();
        assert_eq!(
            (
                engine.executor_id(),
                engine.call_sequence(),
                parked.call_sequence()
            ),
            (executor, child_sequence, 0)
        );
        return_now(&mut engine, arena);
        let child_return = engine.observed_return().unwrap();
        assert_eq!(engine.validate_call_return(&child_return), Ok(()));
        engine
            .exchange_thread(&mut arena.pause().unwrap(), &mut parked)
            .unwrap();
        assert_eq!(
            (engine.executor_id(), engine.call_sequence()),
            (executor, 0)
        );
        assert_eq!(
            engine.validate_call_return(&child_return),
            Err(NativeDispatchError::WrongPhase)
        );

        let syscall = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc,
            engine.placement(),
        );
        let NativeBlockStop::PreSyscall(token) = engine
            .step_syscall_exit(&mut arena.pause().unwrap(), &syscall)
            .unwrap()
        else {
            panic!("original syscall did not stop");
        };
        let nested = engine
            .begin_syscall_call(
                &mut arena.pause().unwrap(),
                token,
                child.guest_pc,
                [1, 2, 3],
            )
            .unwrap();
        assert_eq!(
            (engine.executor_id(), engine.call_sequence()),
            (nested.executor_id(), nested.call_sequence())
        );
        return_now(&mut engine, arena);
        let nested_return = engine.observed_return().unwrap();
        assert_eq!(
            engine.validate_syscall_call_return(nested, &nested_return),
            Ok(())
        );
        assert_eq!(
            engine.validate_call_return(&nested_return),
            Err(NativeDispatchError::WrongPhase)
        );
        engine
            .complete_syscall_call(&mut arena.pause().unwrap(), nested, 99)
            .unwrap();
        assert_eq!(engine.call_sequence(), 0);
        assert_eq!(
            engine.validate_call_return(&nested_return),
            Err(NativeDispatchError::WrongPhase)
        );
        return_now(&mut engine, arena);
        let outer_return = engine.observed_return().unwrap();
        assert_eq!(outer_return.value(), 99);
        assert_eq!(engine.validate_call_return(&outer_return), Ok(()));
        assert_eq!(
            engine.validate_call_return(&child_return),
            Err(NativeDispatchError::WrongPhase)
        );

        engine
            .exchange_thread(&mut arena.pause().unwrap(), &mut parked)
            .unwrap();
        assert_eq!(
            (engine.executor_id(), engine.call_sequence()),
            (executor, child_sequence)
        );
        assert_eq!(engine.validate_call_return(&child_return), Ok(()));
        assert_eq!(
            engine.validate_call_return(&outer_return),
            Err(NativeDispatchError::WrongPhase)
        );
        engine
            .begin_call(&mut arena.pause().unwrap(), child)
            .unwrap();
        assert!(engine.call_sequence() > nested.call_sequence());
        assert!(engine.observed_return().is_none());
        assert_eq!(
            engine.validate_call_return(&child_return),
            Err(NativeDispatchError::WrongPhase)
        );
        return_now(&mut engine, arena);
        let next_return = engine.observed_return().unwrap();
        assert_eq!(engine.validate_call_return(&next_return), Ok(()));
        assert_eq!(
            engine.validate_call_return(&child_return),
            Err(NativeDispatchError::WrongPhase)
        );
    });
}
