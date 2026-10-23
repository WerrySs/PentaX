// SPDX-License-Identifier: GPL-2.0-only
//! Original literal packets and complete admission; payload readers are denied.
use super::*;
use std::cell::Cell;
struct Source {
    bytes: Vec<u8>,
    reads: Cell<usize>,
}
impl Memory for Source {
    fn read(&self, at: u64, out: &mut [u8]) -> Result<(), MemoryError> {
        self.reads.set(self.reads.get() + 1);
        if at != 0x1000 || out.len() != self.bytes.len() {
            return Err(MemoryError::Unavailable);
        }
        out.copy_from_slice(&self.bytes);
        Ok(())
    }
}
fn source(words: &[u32]) -> Source {
    Source {
        bytes: words.iter().flat_map(|word| word.to_le_bytes()).collect(),
        reads: Cell::new(0),
    }
}
fn root(input: &Source) -> CommandRange {
    CommandRange {
        address: 0x1000,
        size: input.bytes.len() as u64,
    }
}

#[test]
fn transfer_packets_seal_real_lengths_normalized_selectors_and_no_source_payload_snapshot() {
    for (words, bytes, source_address, immediate) in [
        (
            vec![0xc004_4000, 0x101, 0x8000, 0, 0x9000, 0],
            4,
            Some(0x8000),
            None,
        ),
        (
            vec![0xc004_4000, 0x10102, 0x8000, 0, 0x9000, 0],
            8,
            Some(0x8000),
            None,
        ),
        (
            vec![0xc004_4000, 0x4000_0102, 0x8000, 0, 0x9000, 0],
            4,
            Some(0x8000),
            None,
        ),
        (
            vec![0xc004_4000, 0x105, 0x89ab_cdef, u32::MAX, 0x9000, 0],
            4,
            None,
            Some(0x89ab_cdef),
        ),
        (
            vec![0xc004_4000, 0x4000_0105, 17, 1, 0x9000, 0],
            4,
            None,
            Some(17),
        ),
        (
            vec![0xc005_5000, 0x6030_0001, 0x8000, 0, 0x9000, 0, 64],
            64,
            Some(0x8000),
            None,
        ),
        (
            vec![
                0xc005_5000,
                0xc000_0000,
                0x0123_4567,
                u32::MAX,
                0x9000,
                0,
                0xc000_1000,
            ],
            4096,
            None,
            Some(0x0123_4567),
        ),
        // Exact zero DMA: opaque unsupported selectors/control/addresses stay unread.
        (
            vec![
                0xc005_5000,
                u32::MAX,
                u32::MAX,
                u32::MAX,
                1,
                u32::MAX,
                0xfc00_0000,
            ],
            0,
            None,
            None,
        ),
    ] {
        let input = source(&words);
        let plan = plan_commands(
            &ComputeState::empty(),
            root(&input),
            &input,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(input.reads.get(), 1);
        assert_eq!(plan.usage().write_bytes, bytes);
        assert_eq!(plan.usage().staging_bytes, bytes);
        let Command::Transfer(value) = plan.command(0).unwrap() else {
            panic!("sealed transfer");
        };
        assert_eq!(value.packet(), words);
        assert_eq!(value.byte_count(), bytes);
        assert_eq!(value.source_address(), source_address);
        assert_eq!(value.fill_word(), immediate);
        assert!(plan.command(1).is_none());
    }
}

#[test]
fn transfer_whole_plan_rejects_late_invalid_domains_or_full_output_budgets_before_effects() {
    for tail in [
        vec![0xc004_4002, 0x101, 0x8000, 0, 0x9000, 0], // nonexact header
        vec![0xc004_4000, 0x141, 0x8000, 0, 0x9000, 0], // reserved bit
        vec![0xc004_4000, 0x104, 0x8000, 0, 0x9000, 0], // unsupported register source8
        vec![0xc004_4000, 0x10105, 1, 0, 0x9000, 0],    // immediate64
        vec![0xc004_4000, 0x4000_0104, 1, 0, 0x9000, 0], // reference clock9
        vec![0xc005_5000, 0x2000_0000, 0x8000, 0, 0x9000, 0, 16], // GDS
        vec![0xc005_5000, 0, 0x8000, 0, 0x9000, 0, 6],  // partial word
        vec![0xc005_5000, 0, 0x8000, 0, 0x9001, 0, 4],
        vec![0xc005_5000, 0, 0x8000, 0, 0x3022c, 0, 4], // source discard profile unavailable
    ] {
        let mut words = vec![0xc003_3700, 0x100, 0xa000, 0, 17];
        words.extend(tail);
        let input = source(&words);
        assert!(
            plan_commands(
                &ComputeState::empty(),
                root(&input),
                &input,
                &Default::default()
            )
            .is_err()
        );
        assert_eq!(input.reads.get(), 1);
    }
    let input = source(&[0xc005_5000, 0x4000_0000, 17, 0, 0x9000, 0, 4096]);
    let generous = CommandLimits {
        max_actions: 1,
        max_dispatches: 0,
        max_indirect_buffers: 0,
        max_depth: 1,
        ..Default::default()
    };
    let plan = plan_commands(&ComputeState::empty(), root(&input), &input, &generous).unwrap();
    assert!(plan.usage().work >= 8192);
    for limits in [
        CommandLimits {
            max_write_bytes: 4095,
            ..generous
        },
        CommandLimits {
            max_work: plan.usage().work - 1,
            ..generous
        },
        CommandLimits {
            max_host_bytes: plan.usage().host_bytes + 4095,
            ..generous
        },
        CommandLimits {
            max_actions: 0,
            ..generous
        },
    ] {
        assert!(matches!(
            plan_commands(&ComputeState::empty(), root(&input), &input, &limits),
            Err(Error::Limit(_))
        ));
    }
}

#[test]
fn ordered_transfers_keep_action_indices_across_arrays_and_distinguish_total_from_peak() {
    let input = source(&[
        0xc005_5000,
        0x4000_0000,
        0x0123_4567,
        0,
        0x9000,
        0,
        64,
        0xc003_6300,
        0x8000,
        0,
        0x8000_0000,
        1,
        0xc004_4000,
        0x10101,
        0x9000,
        0,
        0xa000,
        0,
    ]);
    let mut ordered = OrderedCommands::prepare(
        &input.bytes,
        root(&input),
        ComputeState::empty(),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        (ordered.usage().write_bytes, ordered.usage().staging_bytes),
        (72, 64)
    );
    assert!(matches!(
        ordered.next_step().unwrap(),
        Some(OrderedStep::Memory(0))
    ));
    assert!(matches!(
        ordered.memory_command(0).unwrap(),
        Command::Transfer(_)
    ));
    assert!(matches!(
        ordered.next_step().unwrap(),
        Some(OrderedStep::Array)
    ));
    struct Array;
    impl Memory for Array {
        fn read(&self, at: u64, out: &mut [u8]) -> Result<(), MemoryError> {
            if at != 0x8000 || out.len() != 8 {
                return Err(MemoryError::Unavailable);
            }
            out.copy_from_slice(&[0x20c_u32.to_le_bytes(), 17_u32.to_le_bytes()].concat());
            Ok(())
        }
    }
    ordered.resolve_array(&Array).unwrap();
    assert!(matches!(
        ordered.next_step().unwrap(),
        Some(OrderedStep::Memory(1))
    ));
    assert!(matches!(
        ordered.memory_command(1).unwrap(),
        Command::Transfer(_)
    ));
    assert!(ordered.next_step().unwrap().is_none());
    ordered.final_state().unwrap();
}
