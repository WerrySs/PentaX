// SPDX-License-Identifier: GPL-2.0-only
//! Original context-only/full-processor reset and completed-draw eligibility.
use super::*;

fn initial() -> GraphicsState {
    let mut state = plan(&commands(&[])).final_state();
    state.index_base = Some(0x18000);
    state.index_count = Some(7);
    state.index_type = Some(IndexType::U32);
    state.instances = Some(3);
    state
}
fn ordered(words: &[u32], state: GraphicsState) -> OrderedCommands {
    let data = bytes(words);
    OrderedCommands::prepare(
        &[&data],
        &[CommandRange {
            address: 0x4000,
            size: data.len() as u64,
        }],
        state,
        &Default::default(),
    )
    .unwrap()
}

#[test]
fn clear_state_is_context_only_for_every_payload_and_both_planners() {
    let initial = initial();
    for payload in 0..16 {
        let words = [0xc000_1200, payload];
        let static_plan = plan_commands(&[&bytes(&words)], initial, &Default::default()).unwrap();
        let mut live = ordered(&words, initial);
        assert!(live.next_step().unwrap().is_none());
        let after = static_plan.final_state();
        assert_eq!(after, live.final_state().unwrap());
        assert!(after.get(RegisterSpace::Context, 0x202).is_err());
        for (space, register) in [
            (RegisterSpace::Sh, 0x08),
            (RegisterSpace::Sh, 0x0c),
            (RegisterSpace::Uconfig, 0x242),
        ] {
            assert_eq!(after.get(space, register), initial.get(space, register));
        }
        assert_eq!(
            (
                after.index_base,
                after.index_count,
                after.index_type,
                after.instances
            ),
            (
                initial.index_base,
                initial.index_count,
                initial.index_type,
                initial.instances
            )
        );
    }
}

#[test]
fn processor_reset_clears_stale_graphics_state_and_invalidates_old_draw_for_flip() {
    let initial = initial();
    let words = [0xc000_1024, 0];
    let static_plan = plan_commands(&[&bytes(&words)], initial, &Default::default()).unwrap();
    let after = static_plan.final_state();
    assert!(after.get(RegisterSpace::Context, 0x202).is_err());
    assert!(after.get(RegisterSpace::Sh, 0x08).is_err());
    assert_eq!(after.get(RegisterSpace::Uconfig, 0x242).unwrap(), 0);
    assert_eq!(
        (after.index_type, after.index_count),
        (Some(IndexType::U16), Some(0))
    );
    // The exact pinned209dffe Reset leaves these two independent fields alone.
    assert_eq!(
        (after.index_base, after.instances),
        (initial.index_base, initial.instances)
    );
    let mut live = ordered(&words, initial);
    assert!(live.next_step().unwrap().is_none());
    assert_eq!(live.final_state().unwrap(), after);
    for reset in [[0xc000_1024, 0], [0xc000_1200, 15]] {
        let mut stream = commands(&[]);
        stream.extend(reset);
        stream.extend([0xc004_105c, 1, 0, 1, 0, 0]);
        assert!(matches!(
            plan_commands(
                &[&bytes(&stream)],
                GraphicsState::empty(),
                &Default::default()
            ),
            Err(Error::Unsupported("GPU flip needs a fresh draw"))
        ));
        // A complete fresh setup and draw after the reset restores eligibility.
        stream.truncate(stream.len() - 6);
        stream.extend(commands(&[]));
        stream.extend([0xc004_105c, 1, 0, 1, 0, 0]);
        let data = bytes(&stream);
        let valid = plan_commands(&[&data], GraphicsState::empty(), &Default::default()).unwrap();
        assert_eq!((valid.draws().len(), valid.usage().flips), (2, 1));
    }
}

#[test]
fn captured_nested_reset_retains_source_and_refuses_nonproducer_payload() {
    struct Source {
        root: Vec<u8>,
        nested: Vec<u8>,
    }
    impl Memory for Source {
        fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
            let data = match address {
                0x4000 => &self.root,
                0x8000 => &self.nested,
                _ => return Err(MemoryError::Unavailable),
            };
            if data.len() != output.len() {
                return Err(MemoryError::Unavailable);
            }
            output.copy_from_slice(data);
            Ok(())
        }
    }
    let mut source = Source {
        root: bytes(&[0xc002_3f00, 0x8000, 0, 0x0f20_0002]),
        nested: bytes(&[0xc000_1024, 0]),
    };
    let roots = [CommandRange {
        address: 0x4000,
        size: source.root.len() as u64,
    }];
    let mut captured = CapturedCommands::capture(&roots, &source, &Default::default()).unwrap();
    assert_eq!(captured.command_ranges().len(), 2);
    let reset = captured
        .plan(initial(), &Default::default())
        .unwrap()
        .final_state();
    assert!(reset.get(RegisterSpace::Sh, 0x08).is_err());
    source.nested = bytes(&[0xc000_1024, 1]);
    assert!(CapturedCommands::capture(&roots, &source, &Default::default()).is_err());
}
