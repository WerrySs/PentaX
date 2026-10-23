// SPDX-License-Identifier: GPL-2.0-only
//! Original draw/synchronization/flip action ordering and exact admission.
use super::*;
fn stream() -> Vec<u32> {
    let mut words = commands(&[]);
    words.extend([
        0xc006_1050,
        0x8000_0000,
        0x100,
        0,
        0xffff_ffff,
        0,
        1,
        0x1234,
        0xc000_4600,
        0x407,
        0xc004_105c,
        1,
        0,
        1,
        77,
        0,
    ]);
    words
}
#[test]
fn draw_sync_flip_actions_keep_current_pixels_eligibility_in_static_and_ordered_plans() {
    let words = stream();
    let data = bytes(&words);
    let planned = plan_commands(&[&data], GraphicsState::empty(), &Default::default()).unwrap();
    assert_eq!((planned.draws().len(), planned.usage().flips), (1, 1));
    assert_eq!(planned.actions().len(), 4);
    assert!(matches!(planned.actions()[0], Action::Draw(0)));
    for index in [1, 2] {
        assert!(matches!(
            planned.memory_action(index - 1),
            MemoryAction::Synchronize(_)
        ));
    }
    assert!(matches!(planned.actions()[3], Action::Flip(_)));
    let roots = [CommandRange {
        address: 0x4000,
        size: data.len() as u64,
    }];
    let mut live = OrderedCommands::prepare(
        &[&data],
        &roots,
        GraphicsState::empty(),
        &Default::default(),
    )
    .unwrap();
    assert!(matches!(
        live.next_step().unwrap(),
        Some(OrderedStep::Draw(_))
    ));
    for index in [0, 1] {
        assert!(
            matches!(live.next_step().unwrap(), Some(OrderedStep::Memory(actual)) if actual == index)
        );
        assert!(matches!(
            live.memory_action(index),
            MemoryAction::Synchronize(_)
        ));
    }
    assert!(matches!(
        live.next_step().unwrap(),
        Some(OrderedStep::Flip(_))
    ));
    assert!(live.next_step().unwrap().is_none());
    // An opaque cache-control payload can equal an old untagged flip magic.
    // Exact tagged Acquire must keep its own grammar rather than interpreting it.
    let mut collision = commands(&[]);
    collision.extend([0xc006_1050, 0x6875_0777, 0, 0, 0, 0, 0, 0]);
    let collision_bytes = bytes(&collision);
    let collision_plan = plan_commands(
        &[&collision_bytes],
        GraphicsState::empty(),
        &Default::default(),
    )
    .unwrap();
    assert!(matches!(
        collision_plan.memory_action(0),
        MemoryAction::Synchronize(_)
    ));
    let collision_roots = [CommandRange {
        address: 0x4000,
        size: collision_bytes.len() as u64,
    }];
    let mut collision_live = OrderedCommands::prepare(
        &[&collision_bytes],
        &collision_roots,
        GraphicsState::empty(),
        &Default::default(),
    )
    .unwrap();
    assert!(matches!(
        collision_live.next_step().unwrap(),
        Some(OrderedStep::Draw(_))
    ));
    assert!(matches!(
        collision_live.next_step().unwrap(),
        Some(OrderedStep::Memory(0))
    ));
    assert!(collision_live.next_step().unwrap().is_none());
    // A barrier may preserve an actual draw, never create one or repair a write.
    let barrier_flip = bytes(&[0xc000_4600, 0x407, 0xc004_105c, 1, 0, 1, 77, 0]);
    assert!(
        plan_commands(
            &[&barrier_flip],
            GraphicsState::empty(),
            &Default::default()
        )
        .is_err()
    );
    let mut invalid = commands(&[]);
    invalid.extend([
        0xc003_3700,
        0x100,
        0x3000,
        0,
        1,
        0xc000_4600,
        0x407,
        0xc004_105c,
        1,
        0,
        1,
        77,
        0,
    ]);
    assert!(
        plan_commands(
            &[&bytes(&invalid)],
            GraphicsState::empty(),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn nested_sync_is_immutable_and_unsupported_late_event_refuses_whole_draw_stream() {
    struct Source {
        root: Vec<u8>,
        child: Vec<u8>,
    }
    impl Memory for Source {
        fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
            let data = match address {
                0x4000 => &self.root,
                0x8000 => &self.child,
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
        child: bytes(&[0xc000_4600, 0x407]),
    };
    let roots = [CommandRange {
        address: 0x4000,
        size: source.root.len() as u64,
    }];
    let mut capture = CapturedCommands::capture(&roots, &source, &Default::default()).unwrap();
    source.child = bytes(&[0xc000_4600, 0x139]);
    let plan = capture
        .plan(GraphicsState::empty(), &Default::default())
        .unwrap();
    let MemoryAction::Synchronize(sync) = plan.memory_action(0) else {
        panic!("sealed sync")
    };
    assert_eq!(sync.payload(), &[0x407]);
    assert!(CapturedCommands::capture(&roots, &source, &Default::default()).is_err());
    let mut refused = commands(&[]);
    refused.extend([0xc000_4600, 0x39]);
    assert!(
        plan_commands(
            &[&bytes(&refused)],
            GraphicsState::empty(),
            &Default::default()
        )
        .is_err()
    );
    let complete = stream();
    let data = bytes(&complete);
    let used = plan_commands(&[&data], GraphicsState::empty(), &Default::default())
        .unwrap()
        .usage()
        .work;
    let limits = CommandLimits {
        max_work: used - 1,
        ..Default::default()
    };
    assert!(plan_commands(&[&data], GraphicsState::empty(), &limits).is_err());
}
