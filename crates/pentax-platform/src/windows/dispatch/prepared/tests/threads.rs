// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn caller_stack_keeps_exact_bounds_and_defers_marker_until_selection() {
    with_source(None, |source, arena, stack, state| {
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        // A byte-granular caller stack inside a larger fully mapped allocation
        // has neither an invented reservation hole nor automatic outer guards.
        let address = stack.guards.unwrap()[1].address + 3 * PAGE;
        arena
            .map_at(address, 2 * PAGE, NativePermissions::READ_WRITE)
            .unwrap();
        let borrowed = NativeStackFrame {
            stack: NativeRange {
                address: address + 13,
                size: PAGE + 23,
            },
            guards: None,
            return_slot: (address + PAGE + 36) & !15_u64,
            return_marker: stack.return_marker,
        };
        let borrowed = NativeStackFrame {
            return_slot: borrowed.return_slot - 8,
            ..borrowed
        };
        let other = NativeCpuState::new(state.guest_pc, borrowed.return_slot);
        arena
            .pause()
            .unwrap()
            .write(
                borrowed.return_slot,
                &(!borrowed.return_marker).to_le_bytes(),
            )
            .unwrap();
        let mut parked = engine
            .prepare_thread(&mut arena.pause().unwrap(), other, borrowed)
            .unwrap();
        assert_eq!(
            engine.exchange_thread(&mut arena.pause().unwrap(), &mut parked),
            Err(NativeDispatchError::InvalidStack)
        );
        assert_eq!(*engine.state(), state);
        assert_eq!(*parked.state(), other);
        arena
            .pause()
            .unwrap()
            .write(borrowed.return_slot, &borrowed.return_marker.to_le_bytes())
            .unwrap();
        engine
            .exchange_thread(&mut arena.pause().unwrap(), &mut parked)
            .unwrap();
        assert_eq!(*engine.state(), other);
        assert_eq!(*parked.state(), state);
        // Even a borrowed range may not overlap any admitted stack or guard.
        let mut alias = borrowed;
        alias.stack.address += 1;
        alias.stack.size -= 1;
        assert_eq!(
            engine
                .prepare_thread(&mut arena.pause().unwrap(), other, alias)
                .unwrap_err(),
            NativeDispatchError::InvalidStack
        );
        // Red-zone bounds and NX/write permissions still apply without guards.
        let mut usage = NativeDispatchUsage::default();
        assert_eq!(
            validate_stack(
                &arena.pause().unwrap(),
                &borrowed,
                borrowed.stack.address + 127,
                &mut usage,
                &Default::default()
            ),
            Err(NativeDispatchError::InvalidStack)
        );
        for permissions in [NativePermissions::READ, NativePermissions::READ_EXECUTE] {
            arena.protect(address, 2 * PAGE, permissions).unwrap();
            assert!(
                validate_stack(
                    &arena.pause().unwrap(),
                    &borrowed,
                    borrowed.return_slot,
                    &mut usage,
                    &Default::default()
                )
                .is_err()
            );
        }
    });
}

fn second_stack(arena: &mut NativeAddressSpace, first: NativeStackFrame) -> NativeStackFrame {
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
fn thread_exchange_preserves_full_context_and_refuses_stale_stack_or_foreign_executor() {
    with_source(None, |source, arena, stack, mut state| {
        state.xmm.fill(0x23);
        state.ymm_upper.fill(0x45);
        state.x87.control_word = 0x0f7f;
        state.x87.status_word = 0x4500;
        state.x87.registers.fill([0x19; 10]);
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        assert!(
            engine
                .prepare_thread(&mut arena.pause().unwrap(), state, stack)
                .is_err()
        );
        let other_stack = second_stack(arena, stack);
        let mut other = NativeCpuState::new(state.guest_pc, other_stack.return_slot);
        other.xmm.fill(0x67);
        other.ymm_upper.fill(0x89);
        other.mxcsr = 0x3f80;
        other.x87.control_word = 0x027f;
        other.x87.status_word = 0x2800;
        other.x87.tag_word = 0xfffc;
        other.x87.registers.fill([0x81; 10]);
        other.rflags |= 1 << 10;
        other.fs_base = Some(other_stack.stack.address);
        let mut parked = engine
            .prepare_thread(&mut arena.pause().unwrap(), other, other_stack)
            .unwrap();
        assert_eq!(parked.index(), 1);
        assert_eq!(engine.thread_index(), 0);
        engine
            .exchange_thread(&mut arena.pause().unwrap(), &mut parked)
            .unwrap();
        assert_eq!(*engine.state(), other);
        assert_eq!(*parked.state(), state);
        assert_eq!(engine.thread_index(), 1);
        let mut foreign = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            foreign.exchange_thread(&mut arena.pause().unwrap(), &mut parked),
            Err(NativeDispatchError::WrongOwner)
        );
        assert_eq!(*parked.state(), state);
        arena
            .pause()
            .unwrap()
            .write(stack.return_slot, &(!stack.return_marker).to_le_bytes())
            .unwrap();
        assert_eq!(
            engine.exchange_thread(&mut arena.pause().unwrap(), &mut parked),
            Err(NativeDispatchError::InvalidStack)
        );
        assert_eq!(*engine.state(), other);
        assert_eq!(*parked.state(), state);
        arena
            .pause()
            .unwrap()
            .write(stack.return_slot, &stack.return_marker.to_le_bytes())
            .unwrap();
        let blocks = engine.usage().blocks;
        engine
            .exchange_thread(&mut arena.pause().unwrap(), &mut parked)
            .unwrap();
        assert_eq!(*engine.state(), state);
        assert_eq!(*parked.state(), other);
        assert_eq!(engine.usage().blocks, blocks);
    });
}

#[test]
fn thread_registration_limit_and_pending_syscall_survive_exchange() {
    with_source(Some(&[0x0f, 0x05, 0xc3]), |source, arena, stack, state| {
        let limits = NativeDispatchLimits {
            max_thread_contexts: 2,
            ..Default::default()
        };
        let mut engine =
            NativeBlockStepper::new(source, &mut arena.pause().unwrap(), state, stack, &limits)
                .unwrap();
        let other_stack = second_stack(arena, stack);
        let other = NativeCpuState::new(state.guest_pc, other_stack.return_slot);
        let mut parked = engine
            .prepare_thread(&mut arena.pause().unwrap(), other, other_stack)
            .unwrap();
        assert!(matches!(
            engine.prepare_thread(&mut arena.pause().unwrap(), other, other_stack),
            Err(NativeDispatchError::Budget("thread contexts"))
        ));
        let planned = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc,
            engine.placement(),
        );
        let NativeBlockStop::PreSyscall(token) = engine
            .resolve(&mut arena.pause().unwrap(), &planned)
            .unwrap()
        else {
            panic!("expected syscall");
        };
        engine
            .exchange_thread(&mut arena.pause().unwrap(), &mut parked)
            .unwrap();
        assert_eq!(
            engine.complete_syscall(token, 42),
            Err(NativeDispatchError::WrongPhase)
        );
        engine
            .exchange_thread(&mut arena.pause().unwrap(), &mut parked)
            .unwrap();
        engine.complete_syscall(token, 42).unwrap();
        assert_eq!(engine.state().gpr[RAX], 42);
        assert_eq!(
            engine.complete_syscall(token, 42),
            Err(NativeDispatchError::WrongPhase)
        );
    });
}
