// SPDX-License-Identifier: GPL-2.0-only
//! Original owned-memory asynchronous consumers, without shaders or Vulkan.
use super::*;
use pentax_platform::native_memory::{
    NativeAddressSpace, NativeMemoryEffect, NativeMemoryTransaction, NativePermissions,
};

fn owner() -> NativeAddressSpace {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x20000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x20000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    owner
}
fn write(address: u64, value: u32) -> Vec<u32> {
    vec![
        0xc003_3700,
        0x100,
        address as u32,
        (address >> 32) as u32,
        value,
    ]
}
fn wait(address: u64) -> Vec<u32> {
    vec![
        0xc005_3c00,
        0x13,
        address as u32,
        (address >> 32) as u32,
        7,
        u32::MAX,
        0,
    ]
}
fn encode(memory: &mut PausedNativeMemory<'_>, root: u64, words: &[u32]) -> NativeRange {
    let bytes: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    memory.write(root, &bytes).unwrap();
    NativeRange {
        address: root,
        size: bytes.len() as u64,
    }
}
fn read(memory: &PausedNativeMemory<'_>, address: u64) -> u32 {
    let mut bytes = [0; 4];
    memory.read(address, &mut bytes).unwrap();
    u32::from_le_bytes(bytes)
}
fn expected_state(initial: compute::ComputeState, words: &[u32]) -> compute::ComputeState {
    let bytes: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    struct Source<'a>(&'a [u8]);
    impl compute::Memory for Source<'_> {
        fn read(&self, address: u64, output: &mut [u8]) -> Result<(), compute::MemoryError> {
            assert_eq!(address, 0x1000);
            assert_eq!(output.len(), self.0.len());
            output.copy_from_slice(self.0);
            Ok(())
        }
    }
    compute::plan_commands(
        &initial,
        compute::CommandRange {
            address: 0x1000,
            size: bytes.len() as u64,
        },
        &Source(&bytes),
        &Default::default(),
    )
    .unwrap()
    .final_state()
}
fn pump(driver: &mut Driver, memory: &mut PausedNativeMemory<'_>) {
    for _ in 0..256 {
        if driver.pump_compute_queues(memory, None).unwrap().pending == 0 {
            return;
        }
    }
    panic!("original compute queues did not retire");
}

#[test]
fn waits_retain_prefix_current_payload_fifo_capture_and_retired_queue_state() {
    let mut space = owner();
    let base = space.reserved_ranges().next().unwrap().address;
    let mut memory = space.pause().unwrap();
    let prefix = base + 0x8000;
    let gate = prefix + 4;
    let out = prefix + 8;
    let tail = prefix + 12;
    let mut words = write(prefix, 11);
    words.extend(wait(gate));
    words.extend([0xc001_7600, 0x215, 23]);
    words.extend([
        0xc005_5000,
        0,
        prefix as u32,
        (prefix >> 32) as u32,
        out as u32,
        (out >> 32) as u32,
        4,
    ]);
    let root = encode(&mut memory, base + 0x1000, &words);
    let mut driver = Driver::new(Default::default());
    let initial = driver.queues[0];
    assert_eq!(
        driver.execute_compute_ranges(&mut memory, 0x20, vec![root], None),
        Ok(0)
    );
    assert_eq!(read(&memory, prefix), 11);
    assert_eq!(driver.queues[0], initial); // Future SH state is not retired yet.
    assert_eq!(driver.pending_compute_queues(), 1);
    let mut next = vec![0xc001_7600, 0x22a, 29];
    next.extend(write(tail, 41));
    let queued = encode(&mut memory, base + 0x2000, &next);
    assert_eq!(
        driver.execute_compute_ranges(&mut memory, 0x20, vec![queued], None),
        Ok(0)
    );
    assert_eq!(read(&memory, tail), 0);
    memory.write(queued.address, &[0; 12]).unwrap(); // Commands are already sealed.
    memory.write(prefix, &99u32.to_le_bytes()).unwrap();
    let peer = encode(&mut memory, base + 0x3000, &write(gate, 7));
    assert_eq!(
        driver.execute_compute_ranges(&mut memory, 0x21, vec![peer], None),
        Ok(0)
    );
    pump(&mut driver, &mut memory);
    assert_eq!(read(&memory, out), 99); // No prefix replay or stale payload snapshot.
    assert_eq!(read(&memory, tail), 41);
    let expected = expected_state(initial, &[0xc001_7600, 0x215, 23, 0xc001_7600, 0x22a, 29]);
    assert_eq!(driver.queues[0], expected);
    assert_eq!(driver.usage.command_buffers, 3);
    assert_eq!(driver.usage.memory_written_bytes, 16);
    assert!(!driver.poisoned);

    // A later root of the SAME submit cannot erase a retired state or memory
    // prefix. Exercise eager failure and failure after accepted0 separately.
    for memory_prefix in [false, true] {
        let mut space = owner();
        let base = space.reserved_ranges().next().unwrap().address;
        let mut memory = space.pause().unwrap();
        let label = base + 0x8000;
        memory.write(label, &0u32.to_le_bytes()).unwrap();
        let gate = label + 4;
        memory.write(gate, &0u32.to_le_bytes()).unwrap();
        let mut prefix_words = if memory_prefix {
            write(label, 17)
        } else {
            vec![0xc001_7600, 0x215, 123]
        };
        if memory_prefix {
            prefix_words.extend(wait(gate));
        }
        let prefix = encode(&mut memory, base + 0x1000, &prefix_words);
        let refused = encode(&mut memory, base + 0x2000, &write(base + 0x30000, 19));
        let mut driver = Driver::new(Default::default());
        let expected_state = compute::plan_commands(
            &driver.queues[0],
            compute::CommandRange {
                address: prefix.address,
                size: prefix.size,
            },
            &crate::gpu::Memory(&memory),
            &driver.limits.commands,
        )
        .unwrap()
        .final_state();
        let mut result =
            driver.execute_compute_ranges(&mut memory, 0x20, vec![prefix, refused], None);
        if memory_prefix {
            assert_eq!(result, Ok(0));
            assert_eq!(read(&memory, label), 17);
            // A real CPU store releases the first root's retained false wait;
            // the refused second root belongs to already accepted work.
            memory.write(gate, &7u32.to_le_bytes()).unwrap();
        } else {
            assert_eq!(result, Err(Failure::Memory));
        }
        for _ in 0..64 {
            if result.is_err() {
                break;
            }
            result = driver.pump_compute_queues(&mut memory, None).map(|_| 0);
        }
        assert_eq!(result, Err(Failure::Memory));
        assert!(driver.poisoned);
        assert_eq!(driver.queues[0], expected_state);
        assert_eq!(read(&memory, label), if memory_prefix { 17 } else { 0 });
        let written = driver.usage.written_bytes;
        memory.write(label, &99u32.to_le_bytes()).unwrap();
        assert_eq!(
            driver.execute_compute_ranges(&mut memory, 0x20, vec![prefix, refused], None,),
            Err(Failure::Poisoned)
        );
        assert_eq!(read(&memory, label), 99);
        assert_eq!(driver.usage.written_bytes, written);
    }
}

#[test]
fn repeated_false_waits_spend_finite_work_and_cleanup_authenticates_even_after_poison() {
    let mut space = owner();
    let base = space.reserved_ranges().next().unwrap().address;
    let mut memory = space.pause().unwrap();
    let root = encode(&mut memory, base + 0x1000, &wait(base + 0x8000));
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.execute_compute_ranges(&mut memory, 0x20, vec![root], None),
        Ok(0)
    );
    let mut foreign = owner();
    let mut other = foreign.pause().unwrap();
    let before = driver.usage;
    assert!(matches!(
        driver.pump_compute_queues(&mut other, None),
        Err(Failure::Memory)
    ));
    assert_eq!(
        driver.revoke_compute_owner(other.owner_id()),
        Err(Failure::Memory)
    );
    assert_eq!(driver.usage, before);
    assert!(!driver.poisoned);
    driver.limits.max_preparation_work = driver.usage.preparation_work + 8;
    let mut exhausted = false;
    for _ in 0..4 {
        if matches!(
            driver.pump_compute_queues(&mut memory, None),
            Err(Failure::Limit)
        ) {
            exhausted = true;
            break;
        }
    }
    assert!(exhausted);
    assert!(driver.poisoned);
    assert_eq!(driver.pending_compute_queues(), 1);
    assert_eq!(
        driver.revoke_compute_owner(other.owner_id()),
        Err(Failure::Memory)
    );
    driver.revoke_compute_owner(memory.owner_id()).unwrap();
    assert_eq!(driver.pending_compute_queues(), 0);
    assert_eq!(driver.compute_retained_bytes().unwrap(), 0);
    assert!(driver.poisoned);
}

#[test]
fn compute_and_graphics_writes_both_exclude_other_retained_command_backing() {
    for graphics_writer in [false, true] {
        let mut space = owner();
        let base = space.reserved_ranges().next().unwrap().address;
        let mut memory = space.pause().unwrap();
        let root = encode(&mut memory, base + 0x1000, &wait(base + 0x8000));
        let mut driver = Driver::new(Default::default());
        driver
            .execute_compute_ranges(&mut memory, 0x20, vec![root], None)
            .unwrap();
        let overwrite = encode(&mut memory, base + 0x2000, &write(root.address, 55));
        let result = if graphics_writer {
            driver.submit_graphics(&mut memory, &[overwrite])
        } else {
            driver.execute_compute_ranges(&mut memory, 0x21, vec![overwrite], None)
        };
        assert_eq!(result, Err(Failure::Memory));
        assert_eq!(read(&memory, root.address), 0xc005_3c00);
        assert_eq!(driver.usage.memory_written_bytes, 0);
        // Neither rejected writer committed an effect or returned accepted0.
        // Its refusal must preserve the other queue's already accepted wait.
        assert!(!driver.poisoned);
        assert_eq!(driver.pending_compute_queues(), 1);
        memory.write(base + 0x8000, &7u32.to_le_bytes()).unwrap();
        pump(&mut driver, &mut memory);
        assert_eq!(driver.pending_compute_queues(), 0);
        assert_eq!(read(&memory, root.address), 0xc005_3c00);
        assert_eq!(driver.usage.memory_written_bytes, 0);
    }
    // A prepared actual graphics source is equally protected from compute.
    let mut space = owner();
    let base = space.reserved_ranges().next().unwrap().address;
    let mut memory = space.pause().unwrap();
    let graphics = encode(&mut memory, base + 0x1000, &write(base + 0x8000, 3));
    let mut driver = Driver::new(Default::default());
    driver.graphics_execution = driver
        .prepare_graphics_execution(&mut memory, &[graphics], None, None)
        .unwrap();
    let overwrite = encode(&mut memory, base + 0x2000, &write(graphics.address, 55));
    assert_eq!(
        driver.execute_compute_ranges(&mut memory, 0x20, vec![overwrite], None),
        Err(Failure::Memory)
    );
    assert_eq!(read(&memory, graphics.address), 0xc003_3700);
    assert_eq!(driver.usage.memory_written_bytes, 0);

    // Different virtual ranges can still be the same pending command backing.
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x40000).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(
            base + 0x30000,
            0x4000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut memory = space.pause().unwrap();
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
    let root = encode(&mut memory, base + 0x1000, &wait(base + 0x30000));
    let overwrite = encode(
        &mut memory,
        base + 0x31000,
        &write(root.address + 0x10000, 55),
    );
    let mut driver = Driver::new(Default::default());
    driver
        .execute_compute_ranges(&mut memory, 0x20, vec![root], None)
        .unwrap();
    assert_eq!(
        driver.execute_compute_ranges(&mut memory, 0x21, vec![overwrite], None),
        Err(Failure::Memory)
    );
    assert_eq!(read(&memory, root.address), 0xc005_3c00);
    assert_eq!(driver.usage.memory_written_bytes, 0);
}

#[test]
fn failed_pending_admission_releases_all_compute_storage_and_keeps_terminal_failure() {
    let mut space = owner();
    let base = space.reserved_ranges().next().unwrap().address;
    let mut memory = space.pause().unwrap();
    let root = encode(&mut memory, base + 0x1000, &wait(base + 0x8000));
    let mut driver = Driver::new(Default::default());
    driver
        .execute_compute_ranges(&mut memory, 0x20, vec![root], None)
        .unwrap();
    driver.limits.commands.max_host_bytes = 1;
    assert!(
        driver
            .execute_compute_ranges(&mut memory, 0x21, vec![root], None)
            .is_err()
    );
    assert_eq!(driver.pending_compute_queues(), 0);
    assert_eq!(driver.compute_retained_bytes().unwrap(), 0);
    assert!(driver.poisoned);
}

#[test]
fn local_graphics_plan_and_shared_exclusion_scratch_must_fit_together_before_effects() {
    let mut space = owner();
    let base = space.reserved_ranges().next().unwrap().address;
    let mut memory = space.pause().unwrap();
    let root = encode(&mut memory, base + 0x1000, &write(base + 0x8000, 37));
    let mut driver = Driver::new(Default::default());
    let execution = driver
        .prepare_graphics_execution(&mut memory, &[root], None, None)
        .unwrap()
        .unwrap();
    assert!(driver.graphics_execution.is_none()); // Actual advance owns it locally.
    driver.limits.graphics.max_host_bytes =
        execution.retained + size_of::<compute::CommandRange>() - 1;
    let result: Result<(), Failure> = driver.with_command_exclusions(
        &[compute::CommandRange {
            address: root.address,
            size: root.size,
        }],
        execution.retained,
        |_, _| panic!("over-budget graphics effect was reached"),
    );
    assert_eq!(result, Err(Failure::Limit));
    assert_eq!(read(&memory, base + 0x8000), 0);
    assert_eq!(driver.usage.memory_written_bytes, 0);
}
