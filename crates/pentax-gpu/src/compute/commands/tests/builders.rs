// SPDX-License-Identifier: GPL-2.0-only
//! Original tagged compounds and exact ordered packet effects.
use super::*;

fn tagged(size: usize, cache: u32) -> Vec<u32> {
    let mut words = vec![
        0xc002_7904,
        0x342,
        if size == 4 { 0xc801_0000 } else { 0xc802_0000 },
        0x3003,
    ];
    if size == 4 {
        words.extend([
            0xc005_3c00,
            0x13 | (cache << 25),
            0x3000,
            0,
            17,
            u32::MAX,
            0xffff,
        ]);
    } else {
        words.extend([
            0xc007_9300,
            0x13 | (cache << 25),
            0x3000,
            0,
            17,
            1,
            u32::MAX,
            u32::MAX,
            0xffff,
        ]);
    }
    words.extend([0xc001_7904, 0x342, 0xc800_0000]);
    words
}

#[test]
fn tagged_waits_bind_all_three_packets_and_actual_comparison_widths_without_polling() {
    for size in [4, 8] {
        for cache in 0..4 {
            let words = tagged(size, cache);
            let memory = source(&[(0x1000, &words), (0x3000, &[17, 1])]);
            let plan = plan_commands(
                &ComputeState::empty(),
                root(&words),
                &memory,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(plan.usage().packets, 3);
            assert_eq!(memory.reads.get(), 1); // Planner reads no wait operand.
            let Command::Wait(wait) = plan.commands().next().unwrap() else {
                panic!("wait")
            };
            assert_eq!(wait.byte_count(), size);
            assert!(wait.satisfied(&memory).unwrap());
            let changed = source(&[(0x3000, &[18, 1])]);
            assert!(!wait.satisfied(&changed).unwrap());
            assert!(wait.satisfied(&source(&[])).is_err());
            assert!(matches!(
                plan_commands(
                    &ComputeState::empty(),
                    root(&words),
                    &memory,
                    &CommandLimits {
                        max_packets: 2,
                        ..Default::default()
                    }
                ),
                Err(Error::Limit(_))
            ));
            let mut malformed = Vec::new();
            for (index, value) in [
                (0, 0xc002_7900),
                (1, 0x343),
                (2, 0xc803_0000),
                (3, 0x4000),
                (4, 0xc005_3c02),
                (words.len() - 3, 0xc001_7900),
                (words.len() - 2, 0x343),
                (words.len() - 1, 0xc801_0000),
            ] {
                let mut bad = words.clone();
                bad[index] = value;
                malformed.push(bad);
            }
            malformed.push(words[..words.len() - 1].to_vec());
            malformed.push(words[words.len() - 3..].to_vec());
            for bad in malformed {
                let memory = source(&[(0x1000, &bad)]);
                assert!(
                    plan_commands(
                        &ComputeState::empty(),
                        root(&bad),
                        &memory,
                        &Default::default()
                    )
                    .is_err()
                );
            }
        }
    }
}

#[test]
fn write_cache_profiles_zero_payload_constant_address_and_drain_preserve_real_actions() {
    for data in 1..=3 {
        for interrupt in 0..=4 {
            let words = [
                0xc006_1060,
                0x62f,
                (data << 29) | (interrupt << 24),
                0x3000,
                0,
                17,
                29,
                37,
            ];
            let memory = source(&[(0x1000, &words)]);
            let compute = plan_commands(
                &ComputeState::empty(),
                root(&words),
                &memory,
                &Default::default(),
            )
            .unwrap();
            let Command::Release(acb) = compute.commands().next().unwrap() else {
                panic!("ACB release")
            };
            let (graphics, _) = memory_packet(&bytes(&words), 4096, 4096).unwrap();
            let MemoryAction::Release(dcb) = graphics else {
                panic!("DCB release")
            };
            assert_eq!(acb.write().is_some(), interrupt != 4);
            assert_eq!(dcb.write().is_some(), !matches!(interrupt, 1 | 4));
            assert_eq!(acb.reference_clock(), data == 3 && interrupt != 4);
            assert_eq!(
                dcb.reference_clock(),
                data == 3 && !matches!(interrupt, 1 | 4)
            );
            assert_eq!(acb.interrupt_context(), dcb.interrupt_context());
        }
    }
    for cache in 0..4 {
        for constant in [false, true] {
            let mut words = vec![
                0xc004_3700,
                0x4000_0200 | (cache << 25) | (u32::from(constant) << 16),
                0x3000,
                0,
                17,
                29,
            ];
            words.extend([0xc000_4200, 0]);
            words.extend([0xc002_3700, 0x4000_0200, 0, 0]);
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
            let Command::Write(write) = actions.next().unwrap() else {
                panic!("write")
            };
            assert_eq!(write.address(), 0x3000);
            assert_eq!(
                write.bytes(),
                bytes(if constant { &[29] } else { &[17, 29] })
            );
            assert!(matches!(actions.next(), Some(Command::Drain)));
            let Command::Write(empty) = actions.next().unwrap() else {
                panic!("empty write")
            };
            assert!(empty.bytes().is_empty());
            assert!(actions.next().is_none());
            assert_eq!(plan.usage().write_bytes, if constant { 4 } else { 8 });
        }
    }
    for words in [
        vec![0xc000_4200, 1],
        vec![0xc000_4202, 0],
        vec![0xc003_3700, 0x4000_0200 | (1 << 24), 0x3000, 0, 17],
        vec![0xc006_1060, 0x62f, 5 << 29, 0x3000, 0, 17, 0, 0],
    ] {
        let memory = source(&[(0x1000, &words)]);
        assert!(
            plan_commands(
                &ComputeState::empty(),
                root(&words),
                &memory,
                &Default::default()
            )
            .is_err()
        );
    }
}
