// SPDX-License-Identifier: GPL-2.0-only
//! Original checked native hint execution, complete context and live refusal.
use super::*;

#[test]
fn native_pause_preserves_complete_context_before_the_existing_block_continuation() {
    assert!(super::super::super::host_supports(CpuidFeature::PAUSE));
    for flags in [0x202, 0xed7] {
        // PAUSE; JMP next; RET. The jump supplies an observation boundary
        // without introducing any additional native instruction effects.
        with_source(
            Some(&[0xf3, 0x90, 0xeb, 0, 0xc3]),
            |source, arena, stack, mut state| {
                for (index, value) in state.gpr.iter_mut().enumerate() {
                    if index != RSP {
                        *value = 0xa5a5_9876_0000_0000 | index as u64;
                    }
                }
                state.rflags = flags;
                state.xmm.fill(0x97);
                state.ymm_upper.fill(0xa6);
                state.mxcsr = 0x1f84;
                state.x87.control_word = 0x0f7f;
                state.fs_base = Some(0x1234);
                state.gs_base = Some(0x5678);
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    state,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                let body = block(
                    source,
                    &arena.pause().unwrap(),
                    state.guest_pc,
                    engine.placement_for(state.guest_pc),
                );
                assert_eq!(&body.bytes()[..2], &[0xf3, 0x90]);
                // SAFETY: Only original PAUSE bytes execute on this admitted
                // owner/source/stack; the conditional-free exit is checked normally.
                let stop =
                    unsafe { engine.step_checked(&mut arena.pause().unwrap(), &body) }.unwrap();
                assert!(matches!(stop, NativeBlockStop::Resume { .. }));
                let mut expected = state;
                expected.guest_pc += 4;
                assert_eq!(*engine.state(), expected);
                assert_eq!(engine.usage().native_entries, 1);
                assert_eq!(engine.usage().native_instructions, 1);
                assert_eq!(engine.usage().blocks, 1);
                assert!(engine.usage().work_units > 0);
            },
        );
    }
}

#[test]
fn native_pause_keeps_live_source_owner_and_budget_refusals_before_any_entry() {
    for case in 0..4 {
        with_source(Some(&[0xf3, 0x90, 0xc3]), |source, arena, stack, state| {
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let body = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement_for(state.guest_pc),
            );
            let before = engine.usage();
            if case == 0 {
                let page = state.guest_pc & !(PAGE - 1);
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
            } else if case == 1 {
                engine.machine.limits.max_work_units = before.work_units;
            } else if case == 3 {
                engine.machine.limits.max_blocks = before.blocks;
            }
            let result = if case == 2 {
                let mut other =
                    NativeAddressSpace::reserve_any(Default::default(), 65_536).unwrap();
                // SAFETY: Deliberately foreign owner must refuse before entry.
                unsafe { engine.step_checked(&mut other.pause().unwrap(), &body) }
            } else {
                // SAFETY: Deliberate source/work refusal must precede entry.
                unsafe { engine.step_checked(&mut arena.pause().unwrap(), &body) }
            };
            assert!(result.is_err());
            assert_eq!(*engine.state(), state);
            assert_eq!(engine.usage().native_entries, before.native_entries);
            assert_eq!(
                engine.usage().native_instructions,
                before.native_instructions
            );
            assert_eq!(engine.usage().cache_publications, before.cache_publications);
        });
    }
}
