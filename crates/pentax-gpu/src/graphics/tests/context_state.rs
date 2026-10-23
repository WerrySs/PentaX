// SPDX-License-Identifier: GPL-2.0-only
//! Original one-level context history, actual array ordering and packet guards.
use super::*;
fn op(operation: u32) -> Vec<u32> {
    if operation == 2 {
        vec![0xc001_1068, 2, 0]
    } else {
        vec![0xc003_1068, operation, 0, 0, 0]
    }
}
fn static_plan(words: &[u32], initial: GraphicsState) -> CommandPlan {
    plan_commands(&[&bytes(words)], initial, &Default::default()).unwrap()
}
fn live(words: &[u32], initial: GraphicsState) -> OrderedCommands {
    let data = bytes(words);
    OrderedCommands::prepare(
        &[&data],
        &[CommandRange {
            address: 0x4000,
            size: data.len() as u64,
        }],
        initial,
        &Default::default(),
    )
    .unwrap()
}
#[test]
fn context_history_crosses_submissions_and_clear_preserves_saved_context_only() {
    let mut initial = plan(&commands(&[])).final_state();
    initial.index_base = Some(0x8000);
    initial.index_count = Some(17);
    initial.index_type = Some(IndexType::U32);
    initial.instances = Some(5);
    for push in [1, 3] {
        let pushed = static_plan(&op(push), initial).final_state();
        if push == 3 {
            assert!(pushed.get(RegisterSpace::Context, 0x318).is_err());
        }
        let mut change = vec![
            0xc001_6900,
            0x318,
            0x9876,
            0xc001_7600,
            0x08,
            0x1234,
            0xc001_7900,
            0x242,
            3,
        ];
        change.extend([0xc000_1200, 15]); // DCB ResetQueue clears current only.
        change.extend(op(0)); // Explicit Clear also preserves saved history.
        change.extend(op(2));
        let restored = static_plan(&change, pushed).final_state();
        assert_eq!(
            restored.get(RegisterSpace::Context, 0x318),
            initial.get(RegisterSpace::Context, 0x318)
        );
        assert_eq!(
            restored.get(RegisterSpace::Context, 0x390),
            initial.get(RegisterSpace::Context, 0x390)
        );
        assert_eq!(restored.get(RegisterSpace::Sh, 0x08), Ok(0x1234));
        assert_eq!(restored.get(RegisterSpace::Uconfig, 0x242), Ok(3));
        assert_eq!(
            (
                restored.index_base,
                restored.index_count,
                restored.index_type,
                restored.instances
            ),
            (
                initial.index_base,
                initial.index_count,
                initial.index_type,
                initial.instances
            )
        );
        let mut replay = live(&change, pushed);
        assert!(replay.next_step().unwrap().is_none());
        assert_eq!(replay.final_state().unwrap(), restored);
        assert!(plan_commands(&[&bytes(&op(2))], restored, &Default::default()).is_err());
        let mut reset = vec![0xc000_1024, 0];
        reset.extend(op(2));
        assert!(plan_commands(&[&bytes(&reset)], pushed, &Default::default()).is_err());
        // A second Push refuses without replacing the retained original slot.
        assert!(plan_commands(&[&bytes(&op(1))], pushed, &Default::default()).is_err());
        assert_eq!(
            static_plan(&op(2), pushed)
                .final_state()
                .get(RegisterSpace::Context, 0x318),
            initial.get(RegisterSpace::Context, 0x318)
        );
    }
}
#[test]
fn context_operations_keep_earlier_draw_snapshot_and_require_a_new_draw_for_flip() {
    let mut words = commands(&[]);
    let before = static_plan(&words, GraphicsState::empty()).draws()[0].state;
    words.extend(op(3));
    let planned = static_plan(&words, GraphicsState::empty());
    assert_eq!(planned.draws()[0].state, before);
    assert!(
        planned
            .final_state()
            .get(RegisterSpace::Context, 0x318)
            .is_err()
    );
    let mut replay = live(&words, GraphicsState::empty());
    let Some(OrderedStep::Draw(draw)) = replay.next_step().unwrap() else {
        panic!("original draw")
    };
    assert_eq!(draw.state, before);
    assert!(replay.next_step().unwrap().is_none());
    words.extend([0xc004_105c, 1, 0, 1, 0, 0]);
    assert!(matches!(
        plan_commands(
            &[&bytes(&words)],
            GraphicsState::empty(),
            &Default::default()
        ),
        Err(Error::Unsupported("GPU flip needs a fresh draw"))
    ));
    words.truncate(words.len() - 6);
    words.extend(op(2));
    words.extend([0xc001_2d00, 3, 2, 0xc004_105c, 1, 0, 1, 0, 0]);
    let fresh = static_plan(&words, GraphicsState::empty());
    assert_eq!((fresh.draws().len(), fresh.usage().flips), (2, 1));
}
#[test]
fn context_push_snapshots_live_array_values_before_later_array_overrides_and_pop() {
    struct Array(RefCell<Vec<u8>>);
    impl Memory for Array {
        fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
            if address != 0x8000 || output.len() != 8 {
                return Err(MemoryError::Unavailable);
            }
            output.copy_from_slice(&self.0.borrow());
            Ok(())
        }
    }
    let array = Array(RefCell::new(bytes(&[0x318, 0xabc])));
    let mut words = vec![0xc000_2f00, 1, 0xc003_9f00, 0x8000, 0, 0x8000_0000, 1];
    words.extend(op(1));
    words.extend([0xc003_9f00, 0x8000, 0, 0x8000_0000, 1]);
    words.extend(op(2));
    words.extend([0xc001_2d00, 3, 2]);
    let mut replay = live(&words, GraphicsState::empty());
    assert!(matches!(
        replay.next_step().unwrap(),
        Some(OrderedStep::Array)
    ));
    replay.resolve_array(&array).unwrap();
    assert!(matches!(
        replay.next_step().unwrap(),
        Some(OrderedStep::Array)
    ));
    *array.0.borrow_mut() = bytes(&[0x318, 0xdef]);
    replay.resolve_array(&array).unwrap();
    let Some(OrderedStep::Draw(draw)) = replay.next_step().unwrap() else {
        panic!("restored draw")
    };
    assert_eq!(draw.state.get(RegisterSpace::Context, 0x318), Ok(0xabc));
    assert!(replay.next_step().unwrap().is_none());
    assert_eq!(
        replay
            .final_state()
            .unwrap()
            .get(RegisterSpace::Context, 0x318),
        Ok(0xabc)
    );
}
#[test]
fn nested_context_capture_and_budget_guards_reject_unrelated_or_malformed_tagged_packets() {
    struct Nested {
        root: Vec<u8>,
        child: Vec<u8>,
    }
    impl Memory for Nested {
        fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
            let input = match address {
                0x4000 => &self.root,
                0x8000 => &self.child,
                _ => return Err(MemoryError::Unavailable),
            };
            if input.len() != output.len() {
                return Err(MemoryError::Unavailable);
            }
            output.copy_from_slice(input);
            Ok(())
        }
    }
    let mut child = op(1);
    child.extend([0xc001_6900, 0x318, 77]);
    let mut root = vec![0xc002_3f00, 0x8000, 0, 0x0f20_0008];
    root.extend(op(2));
    let source = Nested {
        root: bytes(&root),
        child: bytes(&child),
    };
    let roots = [CommandRange {
        address: 0x4000,
        size: source.root.len() as u64,
    }];
    let initial = plan(&commands(&[])).final_state();
    let mut captured = CapturedCommands::capture(&roots, &source, &Default::default()).unwrap();
    assert_eq!(captured.command_ranges().len(), 2);
    assert_eq!(
        captured
            .plan(initial, &Default::default())
            .unwrap()
            .final_state(),
        initial
    );
    for invalid in [
        vec![0xc001_1068, 0, 0],
        vec![0xc003_1068, 2, 0, 0, 0],
        vec![0xc003_1068, 4, 0, 0, 0],
        vec![0xc003_1068, 1, 0, 1, 0],
        vec![0xc003_106c, 1, 0, 0, 0],
        vec![0xc003_1068, 1],
    ] {
        let data = bytes(&invalid);
        assert!(plan_commands(&[&data], initial, &Default::default()).is_err());
        assert!(
            OrderedCommands::prepare(
                &[&data],
                &[CommandRange {
                    address: 0x4000,
                    size: data.len() as u64
                }],
                initial,
                &Default::default()
            )
            .is_err()
        );
    }
    let good = bytes(&op(1));
    let plan = plan_commands(&[&good], initial, &Default::default()).unwrap();
    for limits in [
        CommandLimits {
            max_work: plan.usage().work - 1,
            ..Default::default()
        },
        CommandLimits {
            max_host_bytes: plan.usage().host_bytes - 1,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            plan_commands(&[&good], initial, &limits),
            Err(Error::Limit(_))
        ));
    }
    // Graphics context operations cannot become opaque compute NOP success.
    struct Flat<'a>(&'a [u8]);
    impl Memory for Flat<'_> {
        fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
            if address != 0x4000 || output.len() != self.0.len() {
                return Err(MemoryError::Unavailable);
            }
            output.copy_from_slice(self.0);
            Ok(())
        }
    }
    assert!(matches!(
        crate::compute::plan_commands(
            &crate::compute::ComputeState::empty(),
            CommandRange {
                address: 0x4000,
                size: good.len() as u64
            },
            &Flat(&good),
            &Default::default()
        ),
        Err(crate::compute::Error::Packet {
            reason: "packet header profile",
            ..
        })
    ));
}

#[test]
fn restored_context_drives_real_shader_target_preparation_with_current_sh_values() {
    let source = Source::new();
    let initial = plan(&commands(&[])).final_state();
    let pushed = static_plan(&op(1), initial).final_state();
    let changed = [
        0xc001_6900,
        0x318,
        (VS >> 8) as u32,
        0xc001_7600,
        0x0c,
        0.25f32.to_bits(),
    ];
    let mut bad = changed.to_vec();
    bad.extend([0xc001_2d00, 3, 2]);
    let bad_plan = static_plan(&bad, pushed);
    assert!(matches!(
        prepare_draw(&bad_plan.draws()[0], &source, &Default::default()),
        Err(Error::Unsupported("render target overlaps shader source"))
    ));
    let mut restored = changed.to_vec();
    restored.extend(op(2));
    restored.extend([0xc001_2d00, 3, 2]);
    let restored_plan = static_plan(&restored, pushed);
    let draw = &restored_plan.draws()[0];
    assert_eq!(
        draw.state.get(RegisterSpace::Sh, 0x0c),
        Ok(0.25f32.to_bits())
    );
    let prepared = prepare_draw(draw, &source, &Default::default()).unwrap();
    assert_eq!(prepared.target_address(), TARGET);
    assert_eq!(
        (prepared.width(), prepared.height(), prepared.vertex_count()),
        (64, 64, 3)
    );
    assert_eq!(prepared.input(), source.target.as_slice());
    assert!(!prepared.vertex().spirv().is_empty());
    assert!(!prepared.pixel().spirv().is_empty());
    let mut replay = live(&restored, pushed);
    let Some(OrderedStep::Draw(draw)) = replay.next_step().unwrap() else {
        panic!("restored draw")
    };
    let live_prepared = prepare_draw(&draw, &source, &Default::default()).unwrap();
    assert_eq!(live_prepared.target_address(), TARGET);
    assert_eq!(live_prepared.pixel_push_data(), prepared.pixel_push_data());
}
