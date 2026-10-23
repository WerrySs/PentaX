// SPDX-License-Identifier: GPL-2.0-only
//! Original command-stream reset, live-array and queue-history consumers.
use super::*;
use std::cell::{Cell, RefCell};

struct Source {
    bytes: Vec<u8>,
    array: RefCell<Vec<u8>>,
    reads: Cell<usize>,
}
impl Memory for Source {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
        self.reads.set(self.reads.get() + 1);
        let array = self.array.borrow();
        let bytes = match address {
            0x1000 => &self.bytes,
            0x8000 => &*array,
            _ => return Err(MemoryError::Unavailable),
        };
        if bytes.len() != output.len() {
            return Err(MemoryError::Unavailable);
        }
        output.copy_from_slice(bytes);
        Ok(())
    }
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn source(words: &[u32]) -> Source {
    Source {
        bytes: bytes(words),
        array: RefCell::new(bytes(&[0x240, 0x1122])),
        reads: Cell::new(0),
    }
}
fn range(source: &Source) -> CommandRange {
    CommandRange {
        address: 0x1000,
        size: source.bytes.len() as u64,
    }
}
fn seeded() -> ComputeState {
    let mut state = ComputeState::empty();
    for register in [
        0x204, 0x207, 0x208, 0x209, 0x20c, 0x20d, 0x212, 0x213, 0x228, 0x240,
    ] {
        state.write(register, 0x1234).unwrap();
    }
    state.indirect_base = Some(0x9000);
    state
}

#[test]
fn queue_reset_replaces_only_selected_history_and_restores_proven_compute_defaults() {
    let original = seeded();
    let other_queue = original;
    let input = source(&[0xc000_1024, 0]);
    let plan = plan_commands(&original, range(&input), &input, &Default::default()).unwrap();
    let reset = plan.final_state();
    assert!(plan.commands().next().is_none());
    assert_eq!(input.reads.get(), 1);
    for register in [
        0x204, 0x205, 0x206, 0x207, 0x208, 0x209, 0x20c, 0x20d, 0x240,
    ] {
        assert_eq!(reset.get(register).unwrap(), 0);
    }
    for register in [0x212, 0x213, 0x228] {
        assert!(reset.get(register).is_err());
    }
    assert_eq!(reset.indirect_base, Some(0));
    assert_eq!(other_queue, original);
    // A context-only clear must not erase a compute processor's SH/indirect state.
    let clear = source(&[0xc000_1200, 15]);
    assert_eq!(
        plan_commands(&original, range(&clear), &clear, &Default::default())
            .unwrap()
            .final_state(),
        original
    );
    // Stale dispatch-indirect addresses cannot survive reset.
    let before_indirect = source(&[0xc001_1600, 0, 0x41]);
    assert!(
        plan_commands(
            &original,
            range(&before_indirect),
            &before_indirect,
            &Default::default()
        )
        .is_ok()
    );
    let indirect = source(&[0xc000_1024, 0, 0xc001_1600, 0, 0x41]);
    assert!(plan_commands(&original, range(&indirect), &indirect, &Default::default()).is_err());
    // A fresh program write after reset supplies the dispatch snapshot.
    let fresh = source(&[
        0xc000_1024,
        0,
        0xc002_7600,
        0x20c,
        0x456,
        0,
        0xc003_1500,
        1,
        1,
        1,
        0x41,
    ]);
    let plan = plan_commands(&original, range(&fresh), &fresh, &Default::default()).unwrap();
    assert_eq!(plan.dispatches()[0].shader_address().unwrap(), 0x45600);
    assert_eq!(plan.dispatches()[0].state.get(0x240).unwrap(), 0);
    // Queue plans retain the earlier dispatch snapshot when a later reset
    // changes final history. The separate one-dispatch API refuses that shape.
    let tail_reset = source(&[
        0xc002_7600,
        0x20c,
        0x789,
        0,
        0xc003_1500,
        1,
        1,
        1,
        0x41,
        0xc000_1024,
        0,
    ]);
    let history = plan_commands(
        &original,
        range(&tail_reset),
        &tail_reset,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(history.dispatches()[0].shader_address().unwrap(), 0x78900);
    assert_eq!(history.final_state().get(0x20c).unwrap(), 0);
    let limits = Limits::default();
    let mut budget = Budget {
        limits: &limits,
        usage: Usage::default(),
    };
    assert!(matches!(
        packets(&tail_reset.bytes, &mut budget),
        Err(Error::Packet {
            reason: "queue reset after single dispatch",
            ..
        })
    ));
}

#[test]
fn ordered_queue_reset_occurs_after_actual_array_read_and_before_later_dispatch() {
    let input = source(&[
        0xc003_6300,
        0x8000,
        0,
        0x8000_0000,
        1,
        0xc000_1024,
        0,
        0xc002_7600,
        0x20c,
        0x456,
        0,
        0xc003_1500,
        1,
        1,
        1,
        0x41,
    ]);
    let mut plan =
        OrderedCommands::prepare(&input.bytes, range(&input), seeded(), &Default::default())
            .unwrap();
    assert_eq!(input.reads.get(), 0);
    assert!(matches!(
        plan.next_step().unwrap(),
        Some(OrderedStep::Array)
    ));
    *input.array.borrow_mut() = bytes(&[0x240, 0x5566]);
    plan.resolve_array(&input).unwrap();
    assert_eq!(input.reads.get(), 1);
    let Some(OrderedStep::Dispatch(dispatch)) = plan.next_step().unwrap() else {
        panic!("dispatch")
    };
    assert_eq!(dispatch.shader_address().unwrap(), 0x45600);
    assert_eq!(dispatch.state.get(0x240).unwrap(), 0);
    assert_eq!(dispatch.state.indirect_base, Some(0));
    assert!(plan.next_step().unwrap().is_none());
    assert_eq!(plan.final_state().unwrap().get(0x240).unwrap(), 0);
}

#[test]
fn reset_packets_have_exact_grammar_truncation_and_precharged_work_limits() {
    for words in [
        vec![0xc000_1024],
        vec![0xc000_1024, 1],
        vec![0xc001_1024, 0, 0],
        vec![0xc000_1028, 0],
        vec![0xc000_1200, 16],
        vec![0xc001_1200, 0, 0],
    ] {
        let input = source(&words);
        assert!(plan_commands(&seeded(), range(&input), &input, &Default::default()).is_err());
        assert!(
            OrderedCommands::prepare(&input.bytes, range(&input), seeded(), &Default::default())
                .is_err()
        );
    }
    let input = source(&[0xc000_1024, 0]);
    let exact = plan_commands(&seeded(), range(&input), &input, &Default::default())
        .unwrap()
        .usage()
        .work;
    let limits = CommandLimits {
        max_work: exact - 1,
        ..Default::default()
    };
    assert!(matches!(
        plan_commands(&seeded(), range(&input), &input, &limits),
        Err(Error::Limit(_))
    ));
    assert_eq!(seeded().get(0x240).unwrap(), 0x1234);
    assert!(!crate::contains_indirect_registers(&[&input.bytes]).unwrap());
}

#[test]
fn fresh_queue_start_defaults_preserve_detached_unknown_and_explicit_nonzero_refusals() {
    let empty = ComputeState::empty();
    let fresh = ComputeState::for_new_queue();
    assert_eq!(ComputeState::default(), empty);
    for register in 0x204..=0x206 {
        assert_eq!(empty.get(register), Err(Error::MissingRegister(register)));
        assert_eq!(fresh.get(register).unwrap(), 0);
    }
    assert_eq!(
        fresh.values.iter().filter(|value| value.is_some()).count(),
        3
    );
    assert_eq!(fresh.indirect_base, None);
    let input = source(&[]);
    let limits = Limits::default();
    let make_budget = || Budget {
        limits: &limits,
        usage: Usage::default(),
    };
    assert!(matches!(
        prepare_state(empty, [1; 3], 0x41, &input, make_budget()),
        Err(Error::MissingRegister(0x204))
    ));
    assert!(matches!(
        prepare_state(fresh, [1; 3], 0x41, &input, make_budget()),
        Err(Error::MissingRegister(0x207))
    ));
    for register in 0x204..=0x206 {
        let mut explicit = fresh;
        explicit.write(register, 1).unwrap();
        assert!(matches!(
            prepare_state(explicit, [1; 3], 0x41, &input, make_budget()),
            Err(Error::Unsupported("compute start coordinates"))
        ));
    }
    assert_eq!(input.reads.get(), 0);
}

#[test]
fn queue_start_defaults_survive_context_clear_and_processor_reset_replaces_them() {
    let mut initial = ComputeState::for_new_queue();
    initial.write(0x204, 7).unwrap();
    initial.write(0x205, 9).unwrap();
    initial.write(0x206, 11).unwrap();
    for (words, expected) in [
        (vec![0xc000_1200, 15], [7, 9, 11]),
        (vec![0xc000_1024, 0], [0, 0, 0]),
    ] {
        let input = source(&words);
        let static_plan =
            plan_commands(&initial, range(&input), &input, &Default::default()).unwrap();
        let mut ordered =
            OrderedCommands::prepare(&input.bytes, range(&input), initial, &Default::default())
                .unwrap();
        assert!(ordered.next_step().unwrap().is_none());
        let ordered_final = ordered.final_state().unwrap();
        assert_eq!(static_plan.final_state(), ordered_final);
        for (index, value) in expected.into_iter().enumerate() {
            assert_eq!(ordered_final.get(0x204 + index as u32).unwrap(), value);
        }
    }
}
