// SPDX-License-Identifier: GPL-2.0-only
//! Original real-owned-memory submissions; no shader or physical GPU claim.
use super::*;
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
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
fn dma(out: &mut Vec<u32>, src: u32, dst: u32, from: u64, to: u64, count: u32) {
    out.extend([
        0xc005_5000,
        (src << 29) | (dst << 20),
        from as u32,
        (from >> 32) as u32,
        to as u32,
        (to >> 32) as u32,
        count,
    ]);
}
fn write(out: &mut Vec<u32>, address: u64, words: &[u32]) {
    out.extend([
        0xc000_3700 | ((words.len() as u32 + 2) << 16),
        0x100,
        address as u32,
        (address >> 32) as u32,
    ]);
    out.extend(words);
}
fn submit(
    driver: &mut Driver,
    memory: &mut PausedNativeMemory<'_>,
    base: u64,
    queue: u64,
    words: &[u32],
) -> Result<u64, Failure> {
    memory.write(base + 0x1000, &bytes(words)).unwrap();
    memory
        .write(base + 0x100, &(base + 0x1000).to_le_bytes())
        .unwrap();
    memory
        .write(base + 0x108, &(words.len() as u32).to_le_bytes())
        .unwrap();
    driver.submit(memory, Function::Acb, [queue, base + 0x100, 0, 0, 0, 0])
}
fn read(memory: &PausedNativeMemory<'_>, address: u64, count: usize) -> Vec<u8> {
    let mut out = vec![0; count];
    memory.read(address, &mut out).unwrap();
    out
}
#[test]
fn gds_dma_roundtrip_reads_prior_real_write_and_preserves_ram_canaries() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    memory.write(base + 0x8000, &[0xa5; 32]).unwrap();
    memory.write(base + 0x9000, &[0xa5; 32]).unwrap();
    let mut words = Vec::new();
    write(&mut words, base + 0x8004, &[0x1122_3344, 0x5566_7788]);
    dma(&mut words, 3, 1, base + 0x8004, 0x20, 8);
    dma(&mut words, 1, 3, 0x20, base + 0x9004, 8);
    dma(&mut words, 2, 1, 0xdead_beef, 0x100, 4);
    dma(&mut words, 1, 3, 0x100, base + 0x9010, 4);
    assert_eq!(submit(&mut driver, &mut memory, base, 0x20, &words), Ok(0));
    assert_eq!(
        read(&memory, base + 0x9004, 8),
        bytes(&[0x1122_3344, 0x5566_7788])
    );
    assert_eq!(read(&memory, base + 0x9010, 4), bytes(&[0xdead_beef]));
    for (at, count) in [
        (0x9000, 4),
        (0x900c, 4),
        (0x9014, 12),
        (0x8000, 4),
        (0x800c, 20),
    ] {
        assert_eq!(read(&memory, base + at, count), vec![0xa5; count]);
    }
    assert_eq!(driver.usage().memory_commands, 5);
    assert_eq!(driver.usage().memory_written_bytes, 32);
    assert_eq!(driver.usage().written_bytes, 20);
    assert_eq!(driver.usage().dispatches, 0);
    assert!(!driver.poisoned);
    assert!(driver.graphics_retained_bytes().unwrap() >= compute::Transfer::GDS_BYTES);
}
#[test]
fn gds_lifetime_spans_queues_and_processor_reset_but_not_another_driver() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    let mut fill = Vec::new();
    dma(&mut fill, 2, 1, 0xcafe_babe, 65532, 4);
    assert_eq!(submit(&mut driver, &mut memory, base, 0x20, &fill), Ok(0));
    let mut copy = vec![0xc000_1024, 0];
    dma(&mut copy, 1, 3, 65532, base + 0x9000, 4);
    dma(&mut copy, 1, 3, 0, base + 0x9004, 4);
    assert_eq!(submit(&mut driver, &mut memory, base, 0x21, &copy), Ok(0));
    assert_eq!(read(&memory, base + 0x9000, 8), bytes(&[0xcafe_babe, 0]));
    let retained = driver.gds_retained_bytes();
    assert_eq!(submit(&mut driver, &mut memory, base, 0x20, &copy), Ok(0));
    assert_eq!(driver.gds_retained_bytes(), retained);
    let mut isolated = Driver::new(Default::default());
    assert_eq!(submit(&mut isolated, &mut memory, base, 0x20, &copy), Ok(0));
    assert_eq!(read(&memory, base + 0x9000, 8), bytes(&[0, 0]));
}
#[test]
fn gds_late_ram_permission_failure_keeps_real_prefix_and_poison_without_partial_output() {
    let (mut owner, base) = owner();
    let mut driver = Driver::new(Default::default());
    {
        let mut memory = owner.pause().unwrap();
        let mut fill = Vec::new();
        dma(&mut fill, 2, 1, 0x1234_5678, 0x20, 8);
        assert_eq!(submit(&mut driver, &mut memory, base, 0x20, &fill), Ok(0));
        memory.write(base + 0x3ffc, &[0xa5; 8]).unwrap();
    }
    owner
        .protect(
            base + 0x4000,
            0x4000,
            NativePermissions::READ.with_gpu(true, false),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let mut words = Vec::new();
    write(&mut words, base + 0x200, &[0x9876_5432]);
    dma(&mut words, 1, 3, 0x20, base + 0x3ffc, 8);
    assert_eq!(
        submit(&mut driver, &mut memory, base, 0x20, &words),
        Err(Failure::Memory)
    );
    assert_eq!(read(&memory, base + 0x200, 4), bytes(&[0x9876_5432]));
    assert_eq!(read(&memory, base + 0x3ffc, 8), vec![0xa5; 8]);
    assert_eq!(
        &driver.gds.as_ref().unwrap().bytes[0x20..0x28],
        bytes(&[0x1234_5678; 2]).as_slice()
    );
    assert!(driver.poisoned);
    assert_eq!(
        submit(&mut driver, &mut memory, base, 0x20, &words),
        Err(Failure::Poisoned)
    );
}
#[test]
fn gds_host_output_and_owner_refusals_allocate_or_publish_no_fake_ram() {
    let (mut first, base) = owner();
    let (mut second, other) = owner();
    let mut words = Vec::new();
    dma(&mut words, 2, 1, 0x8765_4321, 0, 4);
    for kind in 0..2 {
        let mut driver = Driver::new(Default::default());
        if kind == 0 {
            driver.limits.compute.max_host_bytes = 65535;
        } else {
            driver.limits.max_memory_write_bytes = 3;
        }
        let mut memory = first.pause().unwrap();
        assert_eq!(
            submit(&mut driver, &mut memory, base, 0x20, &words),
            Err(Failure::Limit)
        );
        assert!(driver.gds.is_none());
        assert_eq!(driver.usage().written_bytes, 0);
    }
    let mut driver = Driver::new(Default::default());
    {
        let mut memory = first.pause().unwrap();
        assert_eq!(submit(&mut driver, &mut memory, base, 0x20, &words), Ok(0));
    }
    let mut copy = Vec::new();
    dma(&mut copy, 1, 3, 0, other + 0x9000, 4);
    {
        let mut memory = second.pause().unwrap();
        memory.write(other + 0x9000, &[0xa5; 4]).unwrap();
        assert_eq!(
            submit(&mut driver, &mut memory, other, 0x20, &copy),
            Err(Failure::Memory)
        );
        assert_eq!(read(&memory, other + 0x9000, 4), vec![0xa5; 4]);
    }
    let mut copy = Vec::new();
    dma(&mut copy, 1, 3, 0, base + 0x9000, 4);
    let mut memory = first.pause().unwrap();
    assert_eq!(submit(&mut driver, &mut memory, base, 0x20, &copy), Ok(0));
    assert_eq!(read(&memory, base + 0x9000, 4), bytes(&[0x8765_4321]));
}

#[test]
fn retained_gds_tightens_next_plan_before_allocation_without_double_charging_staging() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    for with_staging in [false, true] {
        let mut expected_ceiling = None;
        for below in [true, false] {
            // Fresh equivalent driver states keep a rejected attempt's metadata
            // and work from changing the successful boundary case.
            let mut driver = Driver::new(Default::default());
            let mut fill = Vec::new();
            dma(&mut fill, 2, 1, 0x55aa_33cc, 0x20, 4);
            assert_eq!(submit(&mut driver, &mut memory, base, 0x20, &fill), Ok(0));
            let retained = driver.gds_retained_bytes();
            let initial = driver.queues[0];
            let output = base + 0x9004;
            memory.write(output - 4, &[0xa5; 16]).unwrap();
            let mut stream = Vec::new();
            if with_staging {
                dma(&mut stream, 1, 3, 0x20, output, 4);
            } else {
                stream.extend([0xc000_4200, 0]);
            }
            memory.write(base + 0x1000, &bytes(&stream)).unwrap();
            let range = compute::CommandRange {
                address: base + 0x1000,
                size: (stream.len() * 4) as u64,
            };
            // These are the real immutable capture and command producers, not a
            // guessed larger allowance or a second implementation of planning.
            let captured = draw::CapturedComputeCommands::capture_attempt(
                range,
                &crate::gpu::Memory(&memory),
                &driver.limits.commands,
            )
            .unwrap();
            assert_eq!(captured.root(), range);
            let plan = register_execution::ComputeCommands::prepare_source(
                &captured,
                range,
                initial,
                &driver.limits.commands,
            )
            .unwrap();
            let usage = plan.usage();
            assert_eq!(usage.staging_bytes, if with_staging { 4 } else { 0 });
            assert_eq!(usage.write_bytes, if with_staging { 4 } else { 0 });
            let required = retained
                .checked_add(captured.usage().host_bytes)
                .and_then(|n| n.checked_add(usage.host_bytes))
                .and_then(|n| n.checked_add(usage.staging_bytes))
                .unwrap();
            // Queue/range/exclusion pools have their independent shared-host
            // ceiling; the command-local bound is exactly GDS + C + P + S.
            assert!(required <= driver.limits.commands.max_host_bytes);
            if let Some(expected) = expected_ceiling {
                assert_eq!(required, expected);
            } else {
                expected_ceiling = Some(required);
            }
            drop(plan);
            drop(captured);
            let before = driver.usage();
            let work_limit = driver.limits.max_preparation_work;
            driver.limits.commands.max_host_bytes = required - usize::from(below);
            let result = submit(&mut driver, &mut memory, base, 0x20, &stream);
            if below {
                if with_staging {
                    // The plan fits but its real four-byte staging does not.
                    assert_eq!(result, Err(Failure::Limit));
                } else {
                    // Even a no-output Drain must admit its complete plan.
                    assert!(
                        matches!(result, Err(Failure::Preparation(compute::Error::Limit(_)))),
                        "{result:?}"
                    );
                }
            } else {
                assert_eq!(result, Ok(0));
            }
            assert_eq!(driver.pending_compute_queues(), 0);
            assert!(!driver.poisoned);
            assert_eq!(driver.queues[0], initial);
            assert_eq!(driver.gds_retained_bytes(), retained);
            assert_eq!(
                &driver.gds.as_ref().unwrap().bytes[0x20..0x24],
                0x55aa_33ccu32.to_le_bytes().as_slice()
            );
            let mut expected = [0xa5; 16];
            let committed = usize::from(with_staging && !below);
            if committed != 0 {
                expected[4..8].copy_from_slice(&0x55aa_33ccu32.to_le_bytes());
            }
            assert_eq!(read(&memory, output - 4, 16), expected);
            let after = driver.usage();
            assert_eq!(after.calls, before.calls + 1);
            assert_eq!(after.command_buffers, before.command_buffers + 1);
            assert_eq!(after.command_bytes, before.command_bytes + range.size);
            assert_eq!(
                after.written_bytes,
                before.written_bytes + committed as u64 * 4
            );
            assert_eq!(
                after.memory_written_bytes,
                before.memory_written_bytes + committed as u64 * 4
            );
            assert_eq!(
                after.memory_commands,
                before.memory_commands + u64::from(!below)
            );
            // Captured bytes and admitted producer allocations cost real work,
            // including a refused attempt. No additional quota is introduced.
            assert!(after.preparation_work > before.preparation_work);
            assert!(after.preparation_work <= work_limit);
            assert_eq!(driver.limits.max_preparation_work, work_limit);
        }
    }
}
