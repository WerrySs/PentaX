// SPDX-License-Identifier: GPL-2.0-only

//! Original immutable-membership and actual live-owner continuation proofs.

use super::*;

#[test]
fn target_interval_memo_checks_each_live_target_and_excludes_noninitialized_source() {
    with_source(Some(&[0x90, 0x90, 0xc3]), |source, arena, stack, state| {
        let engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(engine.target_interval.get(), None);
        let usage = engine.usage();
        let bias = source.source_plan().modules()[0].load_bias();
        arena.protect(bias, PAGE, NativePermissions::READ).unwrap();
        assert!(matches!(
            engine.check_target(&arena.pause().unwrap(), state.guest_pc),
            Err(NativeDispatchError::Memory(
                NativeMemoryError::PermissionDenied(_)
            ))
        ));
        assert_eq!(engine.target_interval.get(), None);
        arena
            .protect(bias, PAGE, NativePermissions::READ_EXECUTE)
            .unwrap();
        engine
            .check_target(&arena.pause().unwrap(), state.guest_pc)
            .unwrap();
        let interval = engine.target_interval.get().unwrap();
        assert_eq!(interval.end, state.guest_pc + 3);
        // This second target is a different byte of the retained interval.
        engine
            .check_target(&arena.pause().unwrap(), state.guest_pc + 1)
            .unwrap();
        assert_eq!(engine.target_interval.get(), Some(interval));
        assert_eq!(engine.usage(), usage);

        // A memo hit cannot retain permission or backing authority. Execute-only
        // source works; removing X fails at the NEW target byte, then retry works.
        arena
            .protect(
                interval.start,
                PAGE,
                NativePermissions::new(false, false, true),
            )
            .unwrap();
        engine
            .check_target(&arena.pause().unwrap(), state.guest_pc + 2)
            .unwrap();
        arena
            .protect(interval.start, PAGE, NativePermissions::READ)
            .unwrap();
        assert_eq!(
            engine.check_target(&arena.pause().unwrap(), state.guest_pc + 1),
            Err(NativeDispatchError::Memory(
                NativeMemoryError::PermissionDenied(state.guest_pc + 1)
            ))
        );
        arena
            .protect(interval.start, PAGE, NativePermissions::READ_EXECUTE)
            .unwrap();
        engine
            .check_target(&arena.pause().unwrap(), state.guest_pc + 1)
            .unwrap();

        // The executable BSS tail, a reservation gap and initialized RW data
        // are not immutable initialized executable source. Granting live X to
        // data does not change that immutable metadata decision.
        arena
            .protect(bias + DATA_OFF, PAGE, NativePermissions::READ_EXECUTE)
            .unwrap();
        for target in [
            interval.end,
            bias + PAGE,
            bias + DATA_OFF,
            NATIVE_ADDRESS_LIMIT,
        ] {
            assert_eq!(
                engine.check_target(&arena.pause().unwrap(), target),
                Err(NativeDispatchError::InvalidTarget(target))
            );
            assert_eq!(engine.target_interval.get(), Some(interval));
        }
    });
}

fn segment(bias: u64, vaddr: u64, initialized_bytes: u64, eligible: bool) -> TargetSegment {
    TargetSegment {
        bias,
        vaddr,
        initialized_bytes,
        eligible,
    }
}

#[test]
fn target_interval_scanner_preserves_overlap_empty_and_arithmetic_error_order() {
    let target = 0x180;
    let expected = TargetInterval {
        start: 0x100,
        end: 0x200,
    };
    // The exact production iterator is lazy: a successful earlier match does
    // not evaluate a later overflow. An overlap still chooses the earlier one.
    let mut visits = 0;
    let result = scan_target_segments(
        target,
        [
            segment(u64::MAX, 1, 1, false), // Ineligible arithmetic is never checked.
            segment(0, 0x100, 0x100, true),
            segment(0, 0x170, 0x100, true),
            segment(u64::MAX, 1, 1, true),
        ]
        .into_iter()
        .inspect(|_| visits += 1),
    );
    assert_eq!(result, Ok(expected));
    assert_eq!(visits, 2);
    for bad in [segment(u64::MAX, 1, 1, true), segment(0, u64::MAX, 1, true)] {
        assert_eq!(
            scan_target_segments(target, [bad, segment(0, 0x100, 0x100, true)]),
            Err(NativeDispatchError::InvalidTarget(target))
        );
    }
    assert_eq!(
        scan_target_segments(
            target,
            [segment(0, target, 0, true), segment(0, 0x181, 1, true)],
        ),
        Err(NativeDispatchError::InvalidTarget(target))
    );
    let mut visited = false;
    assert_eq!(
        scan_target_segments(
            NATIVE_ADDRESS_LIMIT,
            std::iter::once_with(|| {
                visited = true;
                segment(0, 0, u64::MAX, true)
            }),
        ),
        Err(NativeDispatchError::InvalidTarget(NATIVE_ADDRESS_LIMIT))
    );
    assert!(!visited);
}

#[test]
fn source_reattachment_discards_interval_for_same_and_appended_source_epochs() {
    with_source(Some(&[0x0f, 0x05, 0xc3]), |source, arena, stack, state| {
        let engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        engine
            .check_target(&arena.pause().unwrap(), state.guest_pc)
            .unwrap();
        assert!(engine.target_interval.get().is_some());
        let mut machine = Some(engine.into_machine());
        let mut engine =
            NativeBlockStepper::from_machine(source, &mut arena.pause().unwrap(), &mut machine)
                .unwrap();
        assert_eq!(engine.target_interval.get(), None);
        engine
            .check_target(&arena.pause().unwrap(), state.guest_pc + 1)
            .unwrap();
        let syscall = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc,
            engine.placement(),
        );
        // SAFETY: Original staged source, exact owned stack and paused owner;
        // the checked syscall-only plan executes no guest native body.
        let token =
            match unsafe { engine.step_checked(&mut arena.pause().unwrap(), &syscall) }.unwrap() {
                NativeBlockStop::PreSyscall(token) => token,
                other => panic!("unexpected original syscall stop: {other:?}"),
            };
        let identity = source.source_plan().identity();
        let added_image = build_elf(&[0xc3]);
        let next = pentax_linker::append_symbolic_module(
            source.source_plan(),
            ModuleInput::new(
                open_executable(&added_image).unwrap(),
                source.source_plan().modules()[0].load_bias() + 4 * 1024 * 1024,
            ),
            &Default::default(),
        )
        .unwrap();
        assert_ne!(next.identity(), identity);
        let next_source = declare_materialized_source(&next, &[], &BlockLimits::default()).unwrap();
        let mut machine = Some(engine.into_machine());
        let mut engine = NativeBlockStepper::from_machine(
            &next_source,
            &mut arena.pause().unwrap(),
            &mut machine,
        )
        .unwrap();
        assert_eq!(engine.target_interval.get(), None);
        // No new-module execution or publication is asserted here. The original
        // pending continuation remains bound to the existing live owner.
        engine.complete_syscall(token, 42).unwrap();
        engine
            .check_target(&arena.pause().unwrap(), state.guest_pc + 2)
            .unwrap();
        assert!(engine.target_interval.get().is_some());
    });
}

#[test]
fn actual_native_conditional_and_jump_continuations_share_only_source_membership() {
    // MOV/CMP; untaken JNE; JMP; failure RET; ADD/RET. Literal result is 43.
    let code = [
        0xb8, 42, 0, 0, 0, 0x83, 0xf8, 42, 0x75, 2, 0xeb, 3, 0x31, 0xc0, 0xc3, 0x83, 0xc0, 1, 0xc3,
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
        for offset in [10, 15] {
            let planned = block(
                source,
                &arena.pause().unwrap(),
                engine.state().guest_pc,
                engine.placement(),
            );
            assert_eq!(
                // SAFETY: Original source, stack and live staged owner; full
                // checked admission precedes this actual bridge entry.
                unsafe { engine.step_checked(&mut arena.pause().unwrap(), &planned) }.unwrap(),
                NativeBlockStop::Resume {
                    guest_pc: state.guest_pc + offset
                }
            );
            assert!(
                engine
                    .target_interval
                    .get()
                    .unwrap()
                    .contains(state.guest_pc + offset)
            );
        }
        let planned = block(
            source,
            &arena.pause().unwrap(),
            engine.state().guest_pc,
            engine.placement(),
        );
        assert_eq!(
            // SAFETY: Same exact original source, stack and checked live owner.
            unsafe { engine.step_checked(&mut arena.pause().unwrap(), &planned) }.unwrap(),
            NativeBlockStop::Returned { value: 43 }
        );
        assert_eq!(engine.usage().blocks, 3);
        assert_eq!(engine.usage().native_entries, 2);
        assert_eq!(engine.usage().native_instructions, 3);
        assert!(engine.observed_return().is_some());
    });
}
