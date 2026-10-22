// SPDX-License-Identifier: GPL-2.0-only
//! Original context-exchange tests. These tests execute no guest instructions.
use super::*;
fn frame(arena: &mut NativeAddressSpace, root: NativeStackFrame) -> NativeStackFrame {
    let address = root.guards.unwrap()[1].address + 3 * PAGE;
    arena
        .map_at(address, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let frame = NativeStackFrame {
        stack: NativeRange {
            address,
            size: PAGE,
        },
        guards: None,
        return_slot: address + PAGE - 8,
        return_marker: root.return_marker,
    };
    arena
        .pause()
        .unwrap()
        .write(frame.return_slot, &frame.return_marker.to_le_bytes())
        .unwrap();
    frame
}
#[test]
fn owned_fiber_exchange_restores_complete_cpu_and_keeps_current_thread_tls() {
    with_source(None, |source, arena, root, mut state| {
        state.xmm.fill(0x19);
        state.ymm_upper.fill(0x37);
        state.mxcsr = 0x5fa5;
        state.x87.control_word = 0x0f7f;
        state.x87.registers.fill([0x45; 10]);
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            root,
            &Default::default(),
        )
        .unwrap();
        let frame = frame(arena, root);
        let mut other = state;
        other.guest_pc = state.guest_pc;
        other.gpr[RSP] = frame.return_slot;
        other.gpr[3] = 0xabcdef;
        other.rflags = 0x246;
        other.xmm.fill(0xa3);
        other.ymm_upper.fill(0xb7);
        other.mxcsr = 0x3f80;
        other.x87.control_word = 0x077f;
        other.x87.registers.fill([0x67; 10]);
        let mut parked = engine
            .prepare_fiber(&mut arena.pause().unwrap(), other, frame)
            .unwrap();
        let origin = parked.origin().unwrap();
        assert_eq!(origin.entry(), other.guest_pc);
        assert_eq!(*origin.stack(), frame);
        assert_ne!(origin.call_sequence(), engine.call_sequence());
        engine
            .exchange_fiber(&mut arena.pause().unwrap(), &mut parked)
            .unwrap();
        assert_eq!(*engine.state(), other);
        assert_eq!(*parked.state(), state);
        assert!(parked.origin().is_none());
        assert_eq!(engine.thread_index(), 0);
        engine
            .exchange_fiber(&mut arena.pause().unwrap(), &mut parked)
            .unwrap();
        assert_eq!(*engine.state(), state);
        assert_eq!(parked.origin(), Some(origin));
        // Model selection of another admitted pthread's TLS. The fiber's owned
        // SIMD/control/GPR continuation moves, its old TLS does not replace it.
        engine.machine.state.fs_base = Some(root.stack.address);
        engine.machine.state.gs_base = Some(root.stack.address + 8);
        engine
            .exchange_fiber(&mut arena.pause().unwrap(), &mut parked)
            .unwrap();
        other.fs_base = Some(root.stack.address);
        other.gs_base = Some(root.stack.address + 8);
        assert_eq!(*engine.state(), other);
        assert_eq!(engine.thread_index(), 0);
        assert_eq!(engine.machine.fiber_origin, Some(origin));
    });
}
#[test]
fn live_context_slots_recycle_and_failed_stack_or_executor_admission_has_no_exchange() {
    with_source(None, |source, arena, root, state| {
        let limits = NativeDispatchLimits {
            max_thread_contexts: 1,
            ..Default::default()
        };
        let mut engine =
            NativeBlockStepper::new(source, &mut arena.pause().unwrap(), state, root, &limits)
                .unwrap();
        let frame = frame(arena, root);
        let other = NativeCpuState::new(state.guest_pc, frame.return_slot);
        let mut parked = engine
            .prepare_fiber(&mut arena.pause().unwrap(), other, frame)
            .unwrap();
        assert!(matches!(
            engine.prepare_fiber(&mut arena.pause().unwrap(), other, frame),
            Err(NativeDispatchError::Budget("fiber contexts"))
        ));
        arena
            .protect(frame.stack.address, PAGE, NativePermissions::READ)
            .unwrap();
        assert!(
            engine
                .exchange_fiber(&mut arena.pause().unwrap(), &mut parked)
                .is_err()
        );
        assert_eq!(*engine.state(), state);
        assert_eq!(*parked.state(), other);
        arena
            .protect(frame.stack.address, PAGE, NativePermissions::READ_WRITE)
            .unwrap();
        let mut foreign =
            NativeBlockStepper::new(source, &mut arena.pause().unwrap(), state, root, &limits)
                .unwrap();
        assert_eq!(
            foreign.exchange_fiber(&mut arena.pause().unwrap(), &mut parked),
            Err(NativeDispatchError::WrongOwner)
        );
        drop(parked);
        let replacement = engine
            .prepare_fiber(&mut arena.pause().unwrap(), other, frame)
            .unwrap();
        assert_eq!(replacement.owner_id(), engine.owner_id());
        assert!(
            engine
                .prepare_fiber(&mut arena.pause().unwrap(), state, root)
                .is_err()
        );
        assert_eq!(*engine.state(), state);
    });
}
