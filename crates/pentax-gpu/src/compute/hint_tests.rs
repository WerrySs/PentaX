// SPDX-License-Identifier: GPL-2.0-only
//! Original closed-bank scheduling hint and live register-array consumers.
use super::*;
use std::cell::{Cell, RefCell};

struct Source {
    commands: Vec<u8>,
    array: RefCell<Vec<u8>>,
    array_reads: Cell<usize>,
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn source(words: &[u32], pairs: &[u32]) -> Source {
    Source {
        commands: bytes(words),
        array: RefCell::new(bytes(pairs)),
        array_reads: Cell::new(0),
    }
}
impl Memory for Source {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
        let array = self.array.borrow();
        let input = match address {
            0x1000 => &self.commands,
            0x8000 => {
                self.array_reads.set(self.array_reads.get() + 1);
                &*array
            }
            _ => return Err(MemoryError::Unavailable),
        };
        if input.len() != output.len() {
            return Err(MemoryError::Unavailable);
        }
        output.copy_from_slice(input);
        Ok(())
    }
}
fn range(input: &Source) -> CommandRange {
    CommandRange {
        address: 0x1000,
        size: input.commands.len() as u64,
    }
}
fn initial() -> ComputeState {
    let mut state = ComputeState::empty();
    state.write(0x20c, 0x321).unwrap();
    state.write(0x240, 0x5678).unwrap();
    state.indirect_base = Some(0x9000);
    state
}

#[test]
fn static_hints_preserve_dispatch_snapshots_and_unrelated_queue_state() {
    let input = source(
        &[
            0xc001_7600,
            0x215,
            0x1357_9bdf,
            0xc001_7602,
            0x22a,
            0x2468_ace0,
            0xc003_1500,
            1,
            2,
            3,
            0x8005,
            0xc001_7600,
            0x215,
            u32::MAX,
            0xc001_7600,
            0x22a,
            0,
            0xc003_1502,
            3,
            2,
            1,
            0x8005,
        ],
        &[],
    );
    let original = initial();
    let plan = plan_commands(&original, range(&input), &input, &Default::default()).unwrap();
    assert_eq!(plan.dispatches().len(), 2);
    let first = plan.dispatches()[0].state;
    let last = plan.dispatches()[1].state;
    assert_eq!(first.get(0x215).unwrap(), 0x1357_9bdf);
    assert_eq!(first.get(0x22a).unwrap(), 0x2468_ace0);
    assert_eq!(last.get(0x215).unwrap(), u32::MAX);
    assert_eq!(last.get(0x22a).unwrap(), 0);
    assert_eq!(plan.final_state(), last);
    for state in [first, last] {
        assert_eq!(state.get(0x20c).unwrap(), 0x321);
        assert_eq!(state.get(0x240).unwrap(), 0x5678);
        assert_eq!(state.indirect_base, Some(0x9000));
    }
    assert!(matches!(
        original.get(0x215),
        Err(Error::MissingRegister(0x215))
    ));
    assert!(matches!(
        original.get(0x22a),
        Err(Error::MissingRegister(0x22a))
    ));
    assert_eq!(input.array_reads.get(), 0);
}

#[test]
fn ordered_hints_read_current_pairs_at_the_array_position_before_dispatch() {
    let input = source(
        &[
            0xc003_6300,
            0x8000,
            0,
            0x8000_0000,
            2,
            0xc003_1500,
            1,
            1,
            1,
            0x8005,
            0xc001_7600,
            0x22a,
            0x55aa_00ff,
            0xc003_1500,
            1,
            1,
            1,
            0x8005,
        ],
        &[0x215, 0, 0x22a, 0],
    );
    let mut plan = OrderedCommands::prepare(
        &input.commands,
        range(&input),
        initial(),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(input.array_reads.get(), 0);
    assert!(matches!(
        plan.next_step().unwrap(),
        Some(OrderedStep::Array)
    ));
    *input.array.borrow_mut() = bytes(&[0x215, 0xa5a5_5a5a, 0x22a, 0x0102_0304]);
    plan.resolve_array(&input).unwrap();
    let Some(OrderedStep::Dispatch(first)) = plan.next_step().unwrap() else {
        panic!("first dispatch");
    };
    assert_eq!(first.state.get(0x215).unwrap(), 0xa5a5_5a5a);
    assert_eq!(first.state.get(0x22a).unwrap(), 0x0102_0304);
    *input.array.borrow_mut() = bytes(&[0x215, 0, 0x22a, 0]);
    let Some(OrderedStep::Dispatch(second)) = plan.next_step().unwrap() else {
        panic!("second dispatch");
    };
    assert_eq!(second.state.get(0x215).unwrap(), 0xa5a5_5a5a);
    assert_eq!(second.state.get(0x22a).unwrap(), 0x55aa_00ff);
    assert_eq!(second.state.get(0x240).unwrap(), 0x5678);
    assert!(plan.next_step().unwrap().is_none());
    assert_eq!(plan.final_state().unwrap(), second.state);
    assert_eq!(input.array_reads.get(), 1);
}

#[test]
fn reset_keeps_only_proven_defaults_and_context_reset_retains_hint_history() {
    for (header, payload, processor) in [(0xc000_1024, 0, true), (0xc000_1200, 0, false)] {
        let mut seeded = initial();
        seeded.write(0x215, 0x1111_2222).unwrap();
        seeded.write(0x22a, 0x3333_4444).unwrap();
        let input = source(&[header, payload], &[]);
        let plan = plan_commands(&seeded, range(&input), &input, &Default::default()).unwrap();
        let reset = plan.final_state();
        if processor {
            assert!(matches!(
                reset.get(0x215),
                Err(Error::MissingRegister(0x215))
            ));
            assert!(matches!(
                reset.get(0x22a),
                Err(Error::MissingRegister(0x22a))
            ));
            assert_eq!(reset.get(0x20c).unwrap(), 0);
            assert!(matches!(
                reset.get(0x212),
                Err(Error::MissingRegister(0x212))
            ));
        } else {
            assert_eq!(reset, seeded);
        }
        let ordered = source(
            &[
                0xc003_6300,
                0x8000,
                0,
                0x8000_0000,
                2,
                header,
                payload,
                0xc003_1500,
                1,
                1,
                1,
                0x8005,
            ],
            &[0x215, 0x1111_2222, 0x22a, 0x3333_4444],
        );
        let mut plan = OrderedCommands::prepare(
            &ordered.commands,
            range(&ordered),
            seeded,
            &Default::default(),
        )
        .unwrap();
        assert!(matches!(
            plan.next_step().unwrap(),
            Some(OrderedStep::Array)
        ));
        plan.resolve_array(&ordered).unwrap();
        let Some(OrderedStep::Dispatch(dispatch)) = plan.next_step().unwrap() else {
            panic!("dispatch after reset");
        };
        assert_eq!(dispatch.state, reset);
        assert!(plan.next_step().unwrap().is_none());
        assert_eq!(plan.final_state().unwrap(), reset);
        assert_eq!(ordered.array_reads.get(), 1);
        assert_eq!(seeded.get(0x215).unwrap(), 0x1111_2222);
    }
}

#[test]
fn unrelated_registers_and_scratch_ring_stay_refused_in_static_and_live_arrays() {
    for register in [0x214, 0x218, 0x219, 0x229, 0x22b, 0x250, 0xffff] {
        let original = initial();
        let input = source(&[0xc001_7600, register, 0], &[]);
        assert!(matches!(
            plan_commands(&original, range(&input), &input, &Default::default()),
            Err(Error::Unsupported("compute SH register"))
        ));
        let input = source(&[0xc003_6300, 0x8000, 0, 0x8000_0000, 1], &[register, 0]);
        let mut plan = OrderedCommands::prepare(
            &input.commands,
            range(&input),
            original,
            &Default::default(),
        )
        .unwrap();
        assert!(matches!(
            plan.next_step().unwrap(),
            Some(OrderedStep::Array)
        ));
        assert!(matches!(
            plan.resolve_array(&input),
            Err(Error::Unsupported("compute SH register"))
        ));
        assert!(plan.next_step().is_err());
        assert_eq!(original, initial());
    }
}
