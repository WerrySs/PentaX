// SPDX-License-Identifier: GPL-2.0-only
//! Original retained-payload and ordered synchronization consumers.
use super::*;
use std::cell::Cell;
struct Source {
    bytes: Vec<u8>,
    reads: Cell<usize>,
}
impl Memory for Source {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
        self.reads.set(self.reads.get() + 1);
        if address != 0x1000 || output.len() != self.bytes.len() {
            return Err(MemoryError::Unavailable);
        }
        output.copy_from_slice(&self.bytes);
        Ok(())
    }
}
fn source(words: &[u32]) -> Source {
    Source {
        bytes: words.iter().flat_map(|v| v.to_le_bytes()).collect(),
        reads: Cell::new(0),
    }
}
fn range(input: &Source) -> CommandRange {
    CommandRange {
        address: 0x1000,
        size: input.bytes.len() as u64,
    }
}
fn plan(input: &Source) -> CommandPlan {
    plan_commands(
        &ComputeState::empty(),
        range(input),
        input,
        &Default::default(),
    )
    .unwrap()
}

#[test]
fn explicit_acquire_and_event_actions_retain_payload_without_reading_cache_coordinates() {
    let words = [
        0xc006_1050,
        0xffff_ffff,
        0xffff_ffff,
        0,
        0xffff_ffff,
        0,
        0x1234,
        0xabcd_1234,
        0xc000_4600,
        0x407,
        0xc000_4600,
        0x31,
    ];
    let input = source(&words);
    let plan = plan(&input);
    assert_eq!(input.reads.get(), 1); // Only immutable command capture, no range read.
    assert_eq!(plan.usage().write_bytes, 0);
    let actions: Vec<_> = plan.commands().collect();
    assert_eq!(actions.len(), 3);
    for (index, expected) in [
        SynchronizationKind::Acquire,
        SynchronizationKind::Event,
        SynchronizationKind::Event,
    ]
    .into_iter()
    .enumerate()
    {
        let Command::Synchronize(sync) = &actions[index] else {
            panic!("explicit synchronization")
        };
        assert_eq!(sync.kind(), expected);
        if index == 0 {
            assert_eq!(sync.payload(), &words[1..8]);
        }
    }
    assert!(plan.dispatches().is_empty());
    assert_eq!(plan.final_state(), ComputeState::empty());
    // The single-flat API has no ordered receiver and must not swallow barriers.
    let limits = Limits::default();
    let mut budget = Budget {
        limits: &limits,
        usage: Usage::default(),
    };
    assert!(packets(&input.bytes, &mut budget).is_err());
}

#[test]
fn ordered_barriers_keep_dispatch_position_and_do_not_bypass_array_or_action_admission() {
    let input = source(&[
        0xc000_4600,
        7,
        0xc003_6300,
        0x9000,
        0,
        0x8000_0000,
        1,
        0xc000_4600,
        0x10,
        0xc003_1500,
        0,
        1,
        1,
        0x8041,
    ]);
    let mut ordered = OrderedCommands::prepare(
        &input.bytes,
        range(&input),
        ComputeState::empty(),
        &Default::default(),
    )
    .unwrap();
    assert!(matches!(
        ordered.next_step().unwrap(),
        Some(OrderedStep::Memory(0))
    ));
    assert!(matches!(
        ordered.memory_command(0).unwrap(),
        Command::Synchronize(_)
    ));
    assert!(matches!(
        ordered.next_step().unwrap(),
        Some(OrderedStep::Array)
    ));
    assert!(ordered.next_step().is_err()); // No skipped live read or later dispatch.
    let limits = CommandLimits {
        max_actions: 0,
        ..Default::default()
    };
    let one = source(&[0xc000_4600, 0x407]);
    assert!(matches!(
        plan_commands(&ComputeState::empty(), range(&one), &one, &limits),
        Err(Error::Limit(_))
    ));
    let used = plan(&one).usage().work;
    let limits = CommandLimits {
        max_work: used - 1,
        ..Default::default()
    };
    assert!(matches!(
        plan_commands(&ComputeState::empty(), range(&one), &one, &limits),
        Err(Error::Limit(_))
    ));
    assert_eq!(plan(&one).usage().write_bytes, 0);
}

#[test]
fn unsupported_queries_ignored_events_reserved_words_and_wrong_headers_never_become_sync() {
    for event in [7, 15, 16] {
        for index in [0, 4] {
            assert_eq!(
                plan(&source(&[0xc000_4600, event | index << 8]))
                    .commands()
                    .count(),
                1
            );
        }
    }
    for event in [0x16, 0x31, 0x2a, 0x2c, 0x2e] {
        assert_eq!(plan(&source(&[0xc000_4600, event])).commands().count(), 1);
    }
    for words in [
        vec![0xc006_1050, 0, 0, 1, 0, 0, 0, 0],
        vec![0xc006_1050, 0, 0, 0, 0, 1, 0, 0],
        vec![0xc006_1050, 0],
        vec![0xc005_1050, 0, 0, 0, 0, 0, 0],
        vec![0xc006_5800, 0, 0, 0, 0, 0, 0, 0],
        vec![0xc002_4600, 0x139, 0x4000, 0],
        vec![0xc000_4600, 0x39],
        vec![0xc000_4600, 0x38],
        vec![0xc000_4600, 0x0d],
        vec![0xc000_4600, 0x716],
        vec![0xc000_4600, 0x207],
        vec![0xc000_4600, 0x8000_0407],
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
                &input.bytes,
                range(&input),
                ComputeState::empty(),
                &Default::default()
            )
            .is_err()
        );
    }
}
