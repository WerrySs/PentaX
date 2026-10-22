// SPDX-License-Identifier: GPL-2.0-only
//! Original nested graphics admission and current-owner execution, without Vulkan.
use super::*;
use crate::gpu::driver::{Function, Limits};
use pentax_platform::native_memory::{
    NativeAddressSpace, NativeMemoryEffect, NativeMemoryTransaction, NativePermissions,
};

fn owner() -> (NativeAddressSpace, u64) {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x20000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x10000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    (owner, base)
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn put(memory: &mut PausedNativeMemory<'_>, address: u64, words: &[u32]) {
    memory.write(address, &bytes(words)).unwrap();
}
fn ib(address: u64, dwords: usize, chain: bool) -> [u32; 4] {
    [
        0xc002_3f00,
        address as u32,
        (address >> 32) as u32,
        0x0f20_0000 | dwords as u32 | if chain { 1 << 20 } else { 0 },
    ]
}
fn write(address: u64, payload: &[u32]) -> Vec<u32> {
    let mut result = vec![
        0xc000_3700 | ((payload.len() as u32 + 2) << 16),
        0x100,
        address as u32,
        (address >> 32) as u32,
    ];
    result.extend(payload);
    result
}
fn range(address: u64, words: &[u32]) -> NativeRange {
    NativeRange {
        address,
        size: words.len() as u64 * 4,
    }
}
fn state(words: &[u32]) -> draw::GraphicsState {
    let original = bytes(words);
    draw::plan_commands(
        &[&original],
        draw::GraphicsState::empty(),
        &Default::default(),
    )
    .unwrap()
    .final_state()
}

#[test]
fn nested_calls_chains_and_fresh_submissions_publish_only_the_actual_final_register_state() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let mut leaf = vec![0xc000_2f00, 3, 0xc001_6900, 0x318, 17, 0xc001_2d00, 0, 2];
    let mut middle = vec![0xc000_2f00, 2];
    middle.extend(ib(base + 0x3000, leaf.len(), true));
    middle.push(0); // A real CHAIN skips this malformed tail, but captures its bytes.
    let mut root = vec![0xc000_2f00, 1];
    root.extend(ib(base + 0x2000, middle.len(), false));
    root.extend([0xc000_2f00, 4, 0xc001_2d00, 0, 2]);
    put(&mut memory, base + 0x1000, &root);
    put(&mut memory, base + 0x2000, &middle);
    let mut driver = Driver::new(Limits::default());
    for (iteration, value) in [17, 31].into_iter().enumerate() {
        leaf[4] = value;
        put(&mut memory, base + 0x3000, &leaf);
        assert_eq!(
            driver.submit_graphics(&mut memory, &[range(base + 0x1000, &root)]),
            Ok(0)
        );
        assert_eq!(
            driver.graphics,
            state(&[0xc001_6900, 0x318, value, 0xc000_2f00, 4])
        );
        assert_eq!(driver.usage.command_buffers, (iteration as u64 + 1) * 3);
        assert_eq!(driver.usage.indirect_buffers, (iteration as u64 + 1) * 2);
        assert_eq!(
            driver.usage.command_bytes,
            (iteration as u64 + 1) * (root.len() + middle.len() + leaf.len()) as u64 * 4
        );
        assert_eq!(driver.graphics_owner, Some(memory.owner_id()));
        assert!(!driver.frame.reset_before_graphics);
        assert!(!driver.poisoned);
        assert_eq!((driver.usage.draws, driver.usage.written_bytes), (0, 0));
    }
}

#[test]
fn a_late_nested_grammar_or_gpu_read_failure_precedes_every_prefix_write_and_state_publication() {
    for refusal in 0..3 {
        let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x20000).unwrap();
        let base = owner.reserved_ranges().next().unwrap().address;
        owner
            .map_at(
                base,
                0x4000,
                NativePermissions::READ_WRITE.with_gpu(true, true),
            )
            .unwrap();
        if refusal != 1 {
            owner
                .map_at(
                    base + 0x8000,
                    0x4000,
                    NativePermissions::READ_WRITE.with_gpu(refusal != 2, true),
                )
                .unwrap();
        }
        let mut memory = owner.pause().unwrap();
        let leaf = [0xc000_2f00, 2, 0];
        if refusal != 1 {
            put(&mut memory, base + 0x8000, &leaf);
        }
        let mut root = write(base + 0x3000, &[17]);
        root.extend(ib(base + 0x8000, leaf.len(), false));
        put(&mut memory, base + 0x1000, &root);
        memory.write(base + 0x3000, &[0xa5; 4]).unwrap();
        let mut driver = Driver::new(Limits::default());
        let failure = driver.submit_graphics(&mut memory, &[range(base + 0x1000, &root)]);
        if refusal == 0 {
            assert!(
                matches!(failure, Err(Failure::GraphicsPreparation(draw::Error::SourcePacket {
                address, dword: 2, .. })) if address == base + 0x8000)
            );
        } else {
            assert!(
                matches!(failure, Err(Failure::GraphicsPreparation(draw::Error::SourceMemory {
                address, .. })) if address == base + 0x8000)
            );
        }
        let mut actual = [0; 4];
        memory.read(base + 0x3000, &mut actual).unwrap();
        assert_eq!(actual, [0xa5; 4]);
        assert_eq!(driver.graphics, draw::GraphicsState::empty());
        assert_eq!(driver.graphics_owner, None);
        assert!(driver.frame.reset_before_graphics);
        assert!(!driver.poisoned);
        assert_eq!((driver.usage.draws, driver.usage.written_bytes), (0, 0));
    }
}

#[test]
fn nested_closure_and_combined_capture_planning_capacities_share_the_existing_session_limits() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let leaf = [0xc001_6900, 0x318, 23];
    let mut root = write(base + 0x7000, &[17]);
    root.extend(ib(base + 0x3000, leaf.len(), false));
    put(&mut memory, base + 0x1000, &root);
    put(&mut memory, base + 0x3000, &leaf);
    let roots = [compute::CommandRange {
        address: base + 0x1000,
        size: root.len() as u64 * 4,
    }];
    let mut captured = draw::CapturedCommands::capture(
        &roots,
        &super::super::super::Memory(&memory),
        &Default::default(),
    )
    .unwrap();
    let capture = captured.usage();
    let plan = captured
        .plan(draw::GraphicsState::empty(), &Default::default())
        .unwrap();
    let prepared = plan.usage();
    assert!(
        capture.host_bytes > 0 && capture.work > 0 && prepared.host_bytes > 0 && prepared.work > 0
    );
    for refusal in 0..8 {
        let mut limits = Limits::default();
        match refusal {
            0 => limits.graphics_commands.max_command_bytes = root.len() * 4,
            1 => limits.max_command_bytes = (root.len() + leaf.len()) as u64 * 4 - 1,
            2 => limits.max_command_buffers = 1,
            3 => limits.graphics_commands.max_buffers = 1,
            4 => {
                limits.graphics_commands.max_host_bytes =
                    capture.host_bytes.max(prepared.host_bytes)
            }
            5 => limits.graphics_commands.max_work = capture.work.max(prepared.work),
            6 => limits.max_preparation_work = 0,
            _ => limits.graphics.max_host_bytes = 0,
        }
        memory.write(base + 0x7000, &[0xa5; 4]).unwrap();
        let mut driver = Driver::new(limits);
        assert!(
            driver
                .submit_graphics(&mut memory, &[range(base + 0x1000, &root)])
                .is_err()
        );
        let mut actual = [0; 4];
        memory.read(base + 0x7000, &mut actual).unwrap();
        assert_eq!(actual, [0xa5; 4]);
        assert_eq!(driver.graphics, draw::GraphicsState::empty());
        assert_eq!(driver.graphics_owner, None);
        assert!(!driver.poisoned);
        assert_eq!(driver.usage.written_bytes, 0);
    }
}

#[test]
fn nested_register_arrays_read_after_actual_prefix_writes_and_late_alias_failure_poison_is_not_replayed()
 {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let array = base + 0x6000;
    let leaf = [
        0xc003_9f00,
        array as u32,
        (array >> 32) as u32,
        0x8000_0000,
        1,
        0xc000_2f00,
        1,
        0xc001_2d00,
        0,
        2,
    ];
    put(&mut memory, base + 0x3000, &leaf);
    for late_alias in [false, true] {
        let mut root = write(array, &[0x318, 17]);
        root.extend(ib(base + 0x3000, leaf.len(), false));
        if late_alias {
            root.extend(write(array, &[99]));
        }
        put(&mut memory, base + 0x1000, &root);
        put(&mut memory, array, &[0x318, 0]);
        let mut driver = Driver::new(Limits::default());
        let result = driver.submit_graphics(&mut memory, &[range(base + 0x1000, &root)]);
        if late_alias {
            assert_eq!(result, Err(Failure::Memory));
            assert_eq!(driver.graphics, draw::GraphicsState::empty());
            assert!(driver.poisoned);
            let before = driver.usage();
            assert_eq!(
                driver.submit(&mut memory, Function::CommandBuffer, [0; 6]),
                Err(Failure::Poisoned)
            );
            assert_eq!(driver.usage(), before);
        } else {
            assert_eq!(result, Ok(0));
            assert_eq!(
                driver.graphics,
                state(&[0xc001_6900, 0x318, 17, 0xc000_2f00, 1])
            );
            assert!(!driver.poisoned);
        }
        let mut actual = [0; 8];
        memory.read(array, &mut actual).unwrap();
        assert_eq!(actual.as_slice(), bytes(&[0x318, 17]));
        assert_eq!((driver.usage.draws, driver.usage.written_bytes), (0, 8));
    }
}

#[test]
fn nested_full_source_including_skipped_chain_tail_excludes_a_real_physical_alias_write() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x30000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base + 0x18000,
            0x4000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    memory
        .apply_transaction(
            NativeMemoryTransaction {
                effect: NativeMemoryEffect::AllocatePhysical {
                    physical: 0,
                    size: 0x4000,
                },
                writes: &[],
            },
            &Default::default(),
        )
        .unwrap();
    for address in [base, base + 0x10000] {
        memory
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::MapPhysical {
                        address,
                        size: 0x4000,
                        physical: 0,
                        permissions: NativePermissions::READ_WRITE.with_gpu(true, true),
                    },
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
    }
    let leaf = [0xc000_2f00, 1];
    let mut child = ib(base + 0x19200, leaf.len(), true).to_vec();
    child.push(0xa5a5_a5a5);
    let mut root = write(base + 0x10810, &[99]);
    root.extend(ib(base + 0x800, child.len(), false));
    put(&mut memory, base + 0x19000, &root);
    put(&mut memory, base + 0x19200, &leaf);
    put(&mut memory, base + 0x800, &child);
    let mut driver = Driver::new(Limits::default());
    assert_eq!(
        driver.submit_graphics(&mut memory, &[range(base + 0x19000, &root)]),
        Err(Failure::Memory)
    );
    let mut actual = [0; 4];
    memory.read(base + 0x810, &mut actual).unwrap();
    assert_eq!(actual, [0xa5; 4]);
    assert_eq!(driver.graphics, draw::GraphicsState::empty());
    assert_eq!(driver.graphics_owner, None);
    assert!(!driver.poisoned);
    assert_eq!((driver.usage.draws, driver.usage.written_bytes), (0, 0));
}

#[test]
fn repeated_refused_nested_captures_consume_session_attempt_budgets_before_any_effect() {
    for denied in [false, true] {
        let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x20000).unwrap();
        let base = owner.reserved_ranges().next().unwrap().address;
        owner
            .map_at(
                base,
                0x4000,
                NativePermissions::READ_WRITE.with_gpu(true, true),
            )
            .unwrap();
        owner
            .map_at(
                base + 0x8000,
                0x4000,
                NativePermissions::READ_WRITE.with_gpu(!denied, true),
            )
            .unwrap();
        let mut memory = owner.pause().unwrap();
        let child = [0xc000_2f00, 2, 0];
        let mut root = write(base + 0x3000, &[17]);
        root.extend(ib(base + 0x8000, child.len(), false));
        put(&mut memory, base + 0x1000, &root);
        put(&mut memory, base + 0x8000, &child);
        memory.write(base + 0x3000, &[0xa5; 4]).unwrap();
        let limits = Limits {
            max_command_buffers: 4,
            ..Default::default()
        };
        let mut driver = Driver::new(limits);
        let mut previous_work = 0;
        for attempt in 1..=2 {
            let result = driver.submit_graphics(&mut memory, &[range(base + 0x1000, &root)]);
            if denied {
                assert!(matches!(
                    result,
                    Err(Failure::GraphicsPreparation(
                        draw::Error::SourceMemory { .. }
                    ))
                ));
            } else {
                assert!(matches!(
                    result,
                    Err(Failure::GraphicsPreparation(
                        draw::Error::SourcePacket { .. }
                    ))
                ));
            }
            assert_eq!(driver.usage.command_buffers, attempt * 2);
            assert_eq!(driver.usage.indirect_buffers, attempt);
            assert_eq!(
                driver.usage.command_bytes,
                attempt * (root.len() + child.len()) as u64 * 4
            );
            assert!(driver.usage.preparation_work > previous_work);
            previous_work = driver.usage.preparation_work;
        }
        assert_eq!(
            driver.submit_graphics(&mut memory, &[range(base + 0x1000, &root)]),
            Err(Failure::Limit)
        );
        assert_eq!(driver.usage.command_buffers, 4);
        let mut actual = [0; 4];
        memory.read(base + 0x3000, &mut actual).unwrap();
        assert_eq!(actual, [0xa5; 4]);
        assert_eq!(driver.graphics, draw::GraphicsState::empty());
        assert_eq!(driver.graphics_owner, None);
        assert!(!driver.poisoned);
        assert_eq!((driver.usage.draws, driver.usage.written_bytes), (0, 0));
    }
}
