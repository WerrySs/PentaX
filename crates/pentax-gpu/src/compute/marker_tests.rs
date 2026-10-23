// SPDX-License-Identifier: GPL-2.0-only
//! Original bounded annotation position, binary payload and refusal consumers.
use super::*;
struct Source(Vec<u8>);
impl Memory for Source {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
        if address != 0x1000 || output.len() != self.0.len() {
            return Err(MemoryError::Unavailable);
        }
        output.copy_from_slice(&self.0);
        Ok(())
    }
}
fn source(words: &[u32]) -> Source {
    Source(words.iter().flat_map(|v| v.to_le_bytes()).collect())
}
fn range(input: &Source) -> CommandRange {
    CommandRange {
        address: 0x1000,
        size: input.0.len() as u64,
    }
}
#[test]
fn marker_annotations_are_explicit_ordered_binary_payloads_without_register_or_memory_effects() {
    // No matching depth is fabricated: Pop is an independent source diagnostic.
    // A callback may remove the original NUL; only packet bounds matter here.
    let input = source(&[
        0xc000_1030,
        0,
        0xc001_102c,
        0xffff_ffff,
        0x6875_0777,
        0xc000_1030,
        0,
    ]);
    let plan = plan_commands(
        &ComputeState::empty(),
        range(&input),
        &input,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(plan.commands().count(), 3);
    assert_eq!(plan.final_state(), ComputeState::empty());
    assert_eq!(plan.usage().write_bytes, 0);
    let Command::Annotation(marker) = plan.command(1).unwrap() else {
        panic!("explicit annotation")
    };
    assert_eq!(marker.kind(), MarkerKind::Push);
    assert_eq!(marker.payload_words(), 2);
    let mut live = OrderedCommands::prepare(
        &input.0,
        range(&input),
        ComputeState::empty(),
        &Default::default(),
    )
    .unwrap();
    for index in 0..3 {
        assert!(
            matches!(live.next_step().unwrap(), Some(OrderedStep::Memory(actual)) if actual == index)
        );
        assert!(matches!(
            live.memory_command(index).unwrap(),
            Command::Annotation(_)
        ));
    }
    assert!(live.next_step().unwrap().is_none());
    assert_eq!(live.final_state().unwrap(), ComputeState::empty());
    let used = plan.usage().work;
    let limits = CommandLimits {
        max_work: used - 1,
        ..Default::default()
    };
    assert!(matches!(
        plan_commands(&ComputeState::empty(), range(&input), &input, &limits),
        Err(Error::Limit(_))
    ));
}

#[test]
fn malformed_or_unbounded_marker_packets_and_action_exhaustion_refuse_whole_plan() {
    for words in [
        vec![0xc001_102c, 1],
        vec![0xc000_1030, 1],
        vec![0xffff_102c],
        vec![0xc001_1030, 0, 0],
        vec![0xc000_102d, 0],
    ] {
        let input = source(&words);
        assert!(
            plan_commands(
                &ComputeState::empty(),
                range(&input),
                &input,
                &Default::default()
            )
            .is_err()
        );
        assert!(
            OrderedCommands::prepare(
                &input.0,
                range(&input),
                ComputeState::empty(),
                &Default::default()
            )
            .is_err()
        );
    }
    let input = source(&[0xc000_102c, 0]);
    let limits = CommandLimits {
        max_actions: 0,
        ..Default::default()
    };
    assert!(matches!(
        plan_commands(&ComputeState::empty(), range(&input), &input, &limits),
        Err(Error::Limit(_))
    ));
}
