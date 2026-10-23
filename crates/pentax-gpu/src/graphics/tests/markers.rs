// SPDX-License-Identifier: GPL-2.0-only
//! Original draw eligibility, immutable nested annotations and binary labels.
use super::*;
#[test]
fn submitted_binary_markers_preserve_only_existing_draw_eligibility_and_order() {
    let mut words = commands(&[]);
    // This payload resembles an old unsupported untagged flip marker. Tagged
    // diagnostic bytes must never be interpreted as a flip or host C string.
    words.extend([
        0xc001_102c,
        0x6875_0777,
        0xffff_ffff,
        0xc000_1030,
        0,
        0xc004_105c,
        1,
        0,
        1,
        77,
        0,
    ]);
    let data = bytes(&words);
    let plan = plan_commands(&[&data], GraphicsState::empty(), &Default::default()).unwrap();
    assert_eq!((plan.draws().len(), plan.usage().flips), (1, 1));
    assert_eq!(plan.actions().len(), 4);
    assert!(matches!(plan.memory_action(0), MemoryAction::Annotation(_)));
    assert!(matches!(plan.memory_action(1), MemoryAction::Annotation(_)));
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
    for index in 0..2 {
        assert!(
            matches!(live.next_step().unwrap(), Some(OrderedStep::Memory(actual)) if actual == index)
        );
    }
    assert!(matches!(
        live.next_step().unwrap(),
        Some(OrderedStep::Flip(_))
    ));
    assert!(live.next_step().unwrap().is_none());
    let no_draw = bytes(&[0xc000_102c, 0, 0xc004_105c, 1, 0, 1, 77, 0]);
    assert!(plan_commands(&[&no_draw], GraphicsState::empty(), &Default::default()).is_err());
    let mut stale = commands(&[]);
    stale.extend([
        0xc003_3700,
        0x100,
        0x3000,
        0,
        1,
        0xc000_1030,
        0,
        0xc004_105c,
        1,
        0,
        1,
        77,
        0,
    ]);
    assert!(
        plan_commands(
            &[&bytes(&stale)],
            GraphicsState::empty(),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn nested_annotation_capture_is_immutable_and_late_invalid_pop_cannot_admit_a_draw() {
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
        child: bytes(&[0xc000_1030, 0]),
    };
    let roots = [CommandRange {
        address: 0x4000,
        size: source.root.len() as u64,
    }];
    let mut capture = CapturedCommands::capture(&roots, &source, &Default::default()).unwrap();
    source.child = bytes(&[0xc000_1030, 1]);
    let plan = capture
        .plan(GraphicsState::empty(), &Default::default())
        .unwrap();
    let MemoryAction::Annotation(marker) = plan.memory_action(0) else {
        panic!("captured marker")
    };
    assert_eq!(marker.kind(), crate::compute::MarkerKind::Pop);
    assert!(CapturedCommands::capture(&roots, &source, &Default::default()).is_err());
    let mut words = commands(&[]);
    words.extend([0xc000_1030, 1]);
    assert!(
        plan_commands(
            &[&bytes(&words)],
            GraphicsState::empty(),
            &Default::default()
        )
        .is_err()
    );
}
