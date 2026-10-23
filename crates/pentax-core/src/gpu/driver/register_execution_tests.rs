// SPDX-License-Identifier: GPL-2.0-only
//! Original paused-owner ordered array execution; no Vulkan device is used.
use super::super::{Driver, Failure, Function, Limits, compute, draw};
use pentax_platform::native_memory::{
    NativeAddressSpace, NativeMemoryEffect, NativeMemoryTransaction, NativePermissions,
    PausedNativeMemory,
};
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn descriptor(memory: &mut PausedNativeMemory<'_>, at: u64, command: u64, words: &[u32]) {
    memory.write(command, &bytes(words)).unwrap();
    memory.write(at, &command.to_le_bytes()).unwrap();
    memory
        .write(at + 8, &(words.len() as u32).to_le_bytes())
        .unwrap();
}
fn original_stream(array: u64, opcode: u32, register: u32, dispatch: bool) -> Vec<u32> {
    let mut words = vec![
        0xc004_3700,
        0x100,
        array as u32,
        (array >> 32) as u32,
        register,
        17,
        0xc003_0000 | opcode << 8,
        array as u32,
        (array >> 32) as u32,
        0x8000_0000,
        1,
    ];
    if dispatch {
        words.extend([0xc003_1500, 0, 1, 1, 0x8041]);
    }
    words
}
#[test]
fn driver_write_then_array_publishes_actual_compute_and_graphics_banks() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x20000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x10000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let stream = original_stream(base + 0x8000, 0x63, 0x20c, true);
    descriptor(&mut memory, base + 0x20, base + 0x1000, &stream);
    let mut driver = Driver::new(Limits::default());
    assert_eq!(
        driver.submit(&mut memory, Function::Acb, [0x20, base + 0x20, 0, 0, 0, 0]),
        Ok(0)
    );
    let direct = bytes(&[0xc001_7600, 0x20c, 17]);
    struct Snapshot<'a>(&'a [u8]);
    impl compute::Memory for Snapshot<'_> {
        fn read(&self, address: u64, output: &mut [u8]) -> Result<(), compute::MemoryError> {
            assert_eq!(address, 0x1000);
            assert_eq!(output.len(), self.0.len());
            output.copy_from_slice(self.0);
            Ok(())
        }
    }
    let expected = compute::plan_commands(
        &compute::ComputeState::for_new_queue(),
        compute::CommandRange {
            address: 0x1000,
            size: direct.len() as u64,
        },
        &Snapshot(&direct),
        &Default::default(),
    )
    .unwrap()
    .final_state();
    assert_eq!(driver.queues[0], expected);
    assert_eq!(driver.usage.dispatches, 1); // Real zero-group dispatch retires without a GPU job.
    assert_eq!(driver.usage.written_bytes, 8);
    assert!(!driver.poisoned);
    memory.write(base + 4, &0u32.to_le_bytes()).unwrap();
    let stream = original_stream(base + 0x8100, 0x9f, 0x318, false);
    descriptor(&mut memory, base + 0x20, base + 0x1000, &stream);
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::CommandBuffer,
            [base, base + 0x20, 0, 0, 0, 0]
        ),
        Ok(0)
    );
    let direct = bytes(&[0xc001_6900, 0x318, 17]);
    let expected = draw::plan_commands(
        &[&direct],
        draw::GraphicsState::for_new_processor(),
        &Default::default(),
    )
    .unwrap()
    .final_state();
    assert_eq!(driver.graphics, expected);
    assert_eq!(driver.usage.draws, 0);
}
#[test]
fn captured_array_physical_alias_is_a_future_write_exclusion_and_partial_prefix_is_poisoned() {
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
    let array = base + 0x800;
    let mut stream = original_stream(array, 0x63, 0x20c, false);
    let alias = array + 0x10000;
    stream.extend([0xc003_3700, 0x100, alias as u32, (alias >> 32) as u32, 99]);
    descriptor(&mut memory, base + 0x18020, base + 0x19000, &stream);
    let mut driver = Driver::new(Limits::default());
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::Acb,
            [0x20, base + 0x18020, 0, 0, 0, 0]
        ),
        Err(Failure::Memory)
    );
    let mut actual = [0; 8];
    memory.read(array, &mut actual).unwrap();
    assert_eq!(actual.as_slice(), bytes(&[0x20c, 17]));
    assert_eq!(driver.usage.written_bytes, 8);
    assert!(driver.poisoned);
    assert_eq!(driver.queues[0], compute::ComputeState::for_new_queue());
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::Acb,
            [0x20, base + 0x18020, 0, 0, 0, 0]
        ),
        Err(Failure::Poisoned)
    );
}
#[test]
fn complete_action_admission_precedes_writes_and_late_unknown_array_retains_only_real_prefix() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x20000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x10000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let array = base + 0x8000;
    memory.write(array, &[0xa5; 8]).unwrap();
    let stream = original_stream(array, 0x63, 0x20c, true);
    descriptor(&mut memory, base + 0x20, base + 0x1000, &stream);
    let mut limits = Limits::default();
    limits.commands.max_actions = 1;
    let mut limited = Driver::new(limits);
    assert!(matches!(
        limited.submit(&mut memory, Function::Acb, [0x20, base + 0x20, 0, 0, 0, 0]),
        Err(Failure::Preparation(compute::Error::Limit(_)))
    ));
    let mut actual = [0; 8];
    memory.read(array, &mut actual).unwrap();
    assert_eq!(actual, [0xa5; 8]);
    assert!(!limited.poisoned);
    let stream = original_stream(array, 0x63, 0x8000_020c, false);
    descriptor(&mut memory, base + 0x20, base + 0x1000, &stream);
    let mut driver = Driver::new(Limits::default());
    assert_eq!(
        driver.submit(&mut memory, Function::Acb, [0x20, base + 0x20, 0, 0, 0, 0]),
        Err(Failure::Preparation(compute::Error::Unsupported(
            "compute SH register"
        )))
    );
    memory.read(array, &mut actual).unwrap();
    assert_eq!(actual.as_slice(), bytes(&[0x8000_020c, 17]));
    assert!(driver.poisoned);
    assert_eq!(driver.queues[0], compute::ComputeState::for_new_queue());
}
