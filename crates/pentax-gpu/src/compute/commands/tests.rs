// SPDX-License-Identifier: GPL-2.0-only
use super::*;
use std::cell::Cell;
mod builders;

#[test]
fn release_packets_preserve_exact_data_and_interrupt_context_without_early_effects() {
    for header in [0xc006_1060, 0xc006_4900, 0xc006_4902] {
        for data in 0..=3 {
            for interrupt in 0..=4 {
                let words = [
                    header,
                    0x62f,
                    (data << 29) | (interrupt << 24),
                    0x2000,
                    0,
                    0x89ab_cdef,
                    0x1234_5678,
                    0xf123_4567,
                ];
                let memory = source(&[(0x1000, &words)]);
                let plan = plan_commands(
                    &ComputeState::empty(),
                    root(&words),
                    &memory,
                    &Default::default(),
                )
                .unwrap();
                assert_eq!(memory.reads.get(), 1);
                let mut actions = plan.commands();
                let Command::Release(release) = actions.next().unwrap() else {
                    panic!("release action")
                };
                assert_eq!(
                    release.interrupt_context(),
                    matches!(interrupt, 1 | 2 | 4).then_some(0x0123_4567)
                );
                assert_eq!(release.reference_clock(), data == 3 && interrupt != 4);
                if data == 0 || interrupt == 4 {
                    assert!(release.write().is_none());
                } else {
                    let write = release.write().unwrap();
                    assert_eq!(write.address(), 0x2000);
                    let expected = match data {
                        1 => 0x89ab_cdef_u32.to_le_bytes().to_vec(),
                        2 => 0x1234_5678_89ab_cdef_u64.to_le_bytes().to_vec(),
                        _ => vec![0; 8],
                    };
                    assert_eq!(write.bytes(), expected);
                }
                assert!(actions.next().is_none());
                let limits = CommandLimits {
                    max_actions: 0,
                    ..Default::default()
                };
                assert!(matches!(
                    plan_commands(&ComputeState::empty(), root(&words), &memory, &limits),
                    Err(Error::Limit(_))
                ));
            }
        }
    }
    for (selection, size) in [(0xa200_0000, 8), (0x2100_0000, 7), (0x2102_0000, 8)] {
        let words = [0xc006_1060, 0x62f, selection, 0x2000, 0, 1, 0, 0];
        let memory = source(&[(0x1000, &words[..size])]);
        assert!(
            plan_commands(
                &ComputeState::empty(),
                root(&words[..size]),
                &memory,
                &Default::default()
            )
            .is_err()
        );
    }
    for (event, interrupt, allowed) in [
        (0x504, 2, true),
        (0x604, 2, false),
        (0x614, 2, true),
        (0x628, 2, true),
        (0x62b, 0, true),
        (0x62b, 2, false),
        (0x62d, 3, true),
        (0x630, 0, true),
        (0x630, 1, false),
        (0x600, 0, false),
        (0x20_062f, 0, true),
        (0x20_062f, 2, false),
        (0x20_052f, 0, false),
        (0x20_0628, 2, true),
        (0x20_0630, 0, false),
        (0x600, 4, true),
    ] {
        let words = [
            0xc006_4900,
            event,
            0x4000_0000 | (interrupt << 24),
            0x2000,
            0,
            1,
            0,
            0,
        ];
        let memory = source(&[(0x1000, &words)]);
        assert_eq!(
            plan_commands(
                &ComputeState::empty(),
                root(&words),
                &memory,
                &Default::default()
            )
            .is_ok(),
            allowed
        );
    }
}
struct Source {
    regions: Vec<(u64, Vec<u8>)>,
    reads: Cell<usize>,
}
impl Memory for Source {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), super::super::MemoryError> {
        self.reads.set(self.reads.get() + 1);
        for (base, bytes) in &self.regions {
            if let Some(at) = address
                .checked_sub(*base)
                .and_then(|v| usize::try_from(v).ok())
                && let Some(input) = at
                    .checked_add(output.len())
                    .and_then(|end| bytes.get(at..end))
            {
                output.copy_from_slice(input);
                return Ok(());
            }
        }
        Err(super::super::MemoryError::Unavailable)
    }
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn seed(value: u32) -> [u32; 3] {
    [0xc001_7602, 0x244, value]
}
fn dispatch() -> [u32; 5] {
    [0xc003_1502, 1, 1, 1, 0x8005]
}
fn ib(address: u64, dwords: u32, chain: bool) -> [u32; 4] {
    [
        0xc002_3f02,
        address as u32,
        (address >> 32) as u32,
        0x0f20_0000 | dwords | if chain { 1 << 20 } else { 0 },
    ]
}
fn source(regions: &[(u64, &[u32])]) -> Source {
    Source {
        regions: regions.iter().map(|(a, w)| (*a, bytes(w))).collect(),
        reads: Cell::new(0),
    }
}
fn root(words: &[u32]) -> CommandRange {
    CommandRange {
        address: 0x1000,
        size: words.len() as u64 * 4,
    }
}
#[test]
fn queue_state_only_updates_and_multi_dispatch_snapshots_preserve_order() {
    let setup = seed(17);
    let memory = source(&[(0x1000, &setup)]);
    let first = plan_commands(
        &ComputeState::empty(),
        root(&setup),
        &memory,
        &Default::default(),
    )
    .unwrap();
    assert!(first.dispatches().is_empty());
    let initial = first.final_state();
    assert_eq!(initial.get(0x244), Ok(17));
    let mut stream = dispatch().to_vec();
    stream.extend(seed(23));
    stream.extend(dispatch());
    stream.extend(seed(31));
    let memory = source(&[(0x1000, &stream)]);
    let next = plan_commands(&initial, root(&stream), &memory, &Default::default()).unwrap();
    assert_eq!(next.dispatches().len(), 2);
    assert_eq!(next.dispatches()[0].state.get(0x244), Ok(17));
    assert_eq!(next.dispatches()[1].state.get(0x244), Ok(23));
    assert_eq!(next.final_state().get(0x244), Ok(31));
    assert_eq!(initial.get(0x244), Ok(17));
    assert!(matches!(initial.get(0x240), Err(Error::MissingRegister(_))));
    assert_eq!(memory.reads.get(), 1);
}
#[test]
fn indirect_calls_return_chains_replace_and_completed_targets_can_repeat() {
    let mut leaf = seed(33).to_vec();
    leaf.extend(dispatch());
    let mut middle = seed(22).to_vec();
    middle.extend(ib(0x3000, leaf.len() as u32, true));
    middle.push(0xc000_2d00);
    let mut stream = ib(0x2000, middle.len() as u32, false).to_vec();
    stream.extend(seed(44));
    stream.extend(dispatch());
    stream.extend(ib(0, 0, true));
    let memory = source(&[(0x1000, &stream), (0x2000, &middle), (0x3000, &leaf)]);
    let plan = plan_commands(
        &ComputeState::empty(),
        root(&stream),
        &memory,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(plan.dispatches().len(), 2);
    assert_eq!(plan.dispatches()[0].state.get(0x244), Ok(33));
    assert_eq!(plan.dispatches()[1].state.get(0x244), Ok(44));
    assert_eq!(plan.usage().indirect_buffers, 2);
    assert_eq!(plan.command_ranges().len(), 3);
    assert_eq!(memory.reads.get(), 3);
    let mut repeated = ib(0x3000, leaf.len() as u32, false).to_vec();
    repeated.extend(ib(0x3000, leaf.len() as u32, false));
    let memory = source(&[(0x1000, &repeated), (0x3000, &leaf)]);
    assert_eq!(
        plan_commands(
            &ComputeState::empty(),
            root(&repeated),
            &memory,
            &Default::default()
        )
        .unwrap()
        .dispatches()
        .len(),
        2
    );
}
#[test]
fn indirect_cycles_limits_and_late_unsupported_packets_publish_no_plan() {
    let a = ib(0x2000, 4, true);
    let b = ib(0x1000, 4, true);
    let memory = source(&[(0x1000, &a), (0x2000, &b)]);
    assert!(matches!(
        plan_commands(
            &ComputeState::empty(),
            root(&a),
            &memory,
            &Default::default()
        ),
        Err(Error::Unsupported("indirect buffer cycle"))
    ));
    assert_eq!(memory.reads.get(), 2);
    let call = ib(0x2000, 5, false);
    let leaf = dispatch();
    for limits in [
        CommandLimits {
            max_depth: 1,
            ..Default::default()
        },
        CommandLimits {
            max_command_bytes: 16,
            ..Default::default()
        },
        CommandLimits {
            max_indirect_buffers: 0,
            ..Default::default()
        },
    ] {
        let memory = source(&[(0x1000, &call), (0x2000, &leaf)]);
        assert!(matches!(
            plan_commands(&ComputeState::empty(), root(&call), &memory, &limits),
            Err(Error::Limit(_))
        ));
        assert_eq!(memory.reads.get(), 1);
    }
    for limits in [
        CommandLimits {
            max_host_bytes: 0,
            ..Default::default()
        },
        CommandLimits {
            max_work: 0,
            ..Default::default()
        },
    ] {
        let memory = source(&[(0x1000, &call)]);
        assert!(matches!(
            plan_commands(&ComputeState::empty(), root(&call), &memory, &limits),
            Err(Error::Limit(_))
        ));
        assert_eq!(memory.reads.get(), 0);
    }
    let mut late = dispatch().to_vec();
    late.extend([0xc000_2d00, 0]);
    let memory = source(&[(0x1000, &late)]);
    assert!(matches!(
        plan_commands(
            &ComputeState::empty(),
            root(&late),
            &memory,
            &Default::default()
        ),
        Err(Error::Packet { .. })
    ));
    assert_eq!(memory.reads.get(), 1); // No shader or buffer reads.
}

#[test]
fn memory_actions_preserve_order_payloads_and_live_wait_comparisons() {
    let mut stream = vec![0xc005_3700, 0x10100, 0x2000, 0, 7, 11, 17];
    stream.extend([0xc005_3c00, 0x13, 0x2000, 0, 17, 0xffff_ffff, 0]);
    stream.extend([
        0xc006_1060,
        0x62f,
        0x4000_0000,
        0x2008,
        0,
        0x89ab_cdef,
        0x0123_4567,
        0,
    ]);
    stream.extend([
        0xc007_9300,
        0x113,
        0x2008,
        0,
        0x89ab_cdef,
        0x0123_4567,
        0xffff_ffff,
        0xffff_ffff,
        0,
    ]);
    let memory = source(&[(0x1000, &stream)]);
    let plan = plan_commands(
        &ComputeState::empty(),
        root(&stream),
        &memory,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(memory.reads.get(), 1);
    assert_eq!(plan.usage().write_bytes, 12);
    let mut actions = plan.commands();
    let Command::Write(write) = actions.next().unwrap() else {
        panic!()
    };
    assert_eq!(write.address(), 0x2000);
    assert_eq!(write.bytes(), &17_u32.to_le_bytes());
    let Command::Wait(wait) = actions.next().unwrap() else {
        panic!()
    };
    assert!(wait.satisfied(&source(&[(0x2000, &[17])])).unwrap());
    assert!(!wait.satisfied(&source(&[(0x2000, &[18])])).unwrap());
    let Command::Release(release) = actions.next().unwrap() else {
        panic!()
    };
    assert_eq!(release.interrupt_context(), None);
    assert!(!release.reference_clock());
    let write = release.write().unwrap();
    assert_eq!(write.bytes(), &0x0123_4567_89ab_cdef_u64.to_le_bytes());
    let Command::Wait(wait) = actions.next().unwrap() else {
        panic!()
    };
    assert!(
        wait.satisfied(&source(&[(0x2008, &[0x89ab_cdef, 0x0123_4567])]))
            .unwrap()
    );
    assert!(actions.next().is_none());
}

#[test]
fn indirect_dispatch_reads_live_dimensions_and_base_survives_state_only_submission() {
    let setup = [0xc002_1102, 1, 0x2000, 0];
    let memory = source(&[(0x1000, &setup)]);
    let base = plan_commands(
        &ComputeState::empty(),
        root(&setup),
        &memory,
        &Default::default(),
    )
    .unwrap()
    .final_state();
    let stream = [0xc001_1600, 8, 0x8005, 0xc002_1600, 0x2008, 0, 0x8005];
    let memory = source(&[(0x1000, &stream)]);
    let plan = plan_commands(&base, root(&stream), &memory, &Default::default()).unwrap();
    assert_eq!(plan.dispatches().len(), 2);
    assert_eq!(memory.reads.get(), 1);
    let dimensions = source(&[(0x2008, &[0, 1, 1])]);
    for dispatch in plan.dispatches() {
        assert!(
            super::super::prepare_dispatch(dispatch, &dimensions, &Default::default())
                .unwrap()
                .into_parts()
                .0
                .is_none()
        );
    }
    assert_eq!(dimensions.reads.get(), 2);
    assert!(matches!(
        plan_commands(
            &ComputeState::empty(),
            root(&stream),
            &memory,
            &Default::default()
        ),
        Err(Error::Unsupported("missing indirect args base"))
    ));
    let mut unsupported = stream.to_vec();
    unsupported.extend([0xc006_1060, 0x62f, 0x2500_0000, 0x2000, 0, 1, 0, 0]);
    let memory = source(&[(0x1000, &unsupported)]);
    assert!(matches!(
        plan_commands(&base, root(&unsupported), &memory, &Default::default()),
        Err(Error::Unsupported("release memory profile"))
    ));
    assert_eq!(memory.reads.get(), 1);
}
