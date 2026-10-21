// SPDX-License-Identifier: GPL-2.0-only
//! Original bounded builder packets, source ordering and real paused backing.
use super::*;
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

fn setup(memory: &mut PausedNativeMemory<'_>, buffer: u64, up: u64, available: u64, callback: u64) {
    let mut bytes = [0; 56];
    bytes[16..24].copy_from_slice(&up.to_le_bytes());
    bytes[24..32].copy_from_slice(&(up + available * 4).to_le_bytes());
    bytes[32..40].copy_from_slice(&callback.to_le_bytes());
    bytes[40..48].copy_from_slice(&0x7123u64.to_le_bytes());
    bytes[48..52].copy_from_slice(&3u32.to_le_bytes());
    memory.write(buffer, &bytes).unwrap();
}
fn read(memory: &PausedNativeMemory<'_>, address: u64, size: usize) -> Vec<u8> {
    let mut bytes = vec![0; size];
    memory.read(address, &mut bytes).unwrap();
    bytes
}
fn returned(action: CommandBuilderAction) -> u64 {
    let CommandBuilderAction::Returned(value) = action else {
        panic!("unexpected callback")
    };
    value
}
fn full(args: &[u64]) -> [u64; 12] {
    let mut result = [0; 12];
    result[..args.len()].copy_from_slice(args);
    result
}

#[test]
fn source_packet_words_sizes_narrow_arguments_and_all_selected_roles_are_closed() {
    use Function::*;
    let cases: &[(Function, &[u64], &[u32])] = &[
        (AgcCbNop, &[1, 1], &[0xffff_1000]),
        (
            AgcCbDispatch,
            &[1, 2, 3, 4, u64::MAX],
            &[0xc003_1500, 2, 3, 4, 0xa079],
        ),
        (
            AgcDcbSetIndexSize,
            &[1, 1, 2],
            &[0xc001_7a00, 0x2000_0243, 0x481],
        ),
        (
            AgcDcbSetIndexBuffer,
            &[1, 0x1234_5678_0000_2222],
            &[0xc001_2600, 0x2222, 0x1234_5678],
        ),
        (AgcDcbSetIndexCount, &[1, 19], &[0xc000_1300, 19]),
        (AgcDcbSetNumInstances, &[1, 7], &[0xc000_2f00, 7]),
        (
            AgcDcbDrawIndex,
            &[1, 0, 0x2340, 0x100],
            &[0xc004_2700, 1, 0x2340, 0, 0, 0x20],
        ),
        (AgcDcbDrawIndexAuto, &[1, 3, 1 << 32], &[0xc001_2d00, 3, 2]),
        (
            AgcDcbDrawIndexOffset,
            &[1, 9, 3, 0x100],
            &[0xc003_3500, 3, 9, 3, 0x20],
        ),
        (AgcDcbStallCommandBufferParser, &[1], &[0xc000_4200, 0]),
        (
            AgcDcbWriteData,
            &[1, 1, 2, 0x1234_5678_0000_1237, 0x4000, 2, 1, 1],
            &[0xc004_3700, 0x4411_0000, 0x1234, 0x1234_5678],
        ),
        (
            AgcAcbWriteData,
            &[1, 1, 2, 0x1234_5678_0000_1237, 0x4000, 2, 1, 1],
            &[0xc004_3700, 0x0411_0100, 0x1237, 0x1234_5678],
        ),
        (
            AgcCbReleaseMem,
            &[
                1,
                0x14,
                0x100,
                1,
                2,
                0x1237,
                2,
                0x1122_3344_5566_7788,
                0,
                0,
                3,
                7,
            ],
            &[
                0xc006_1060,
                0x0430_0514,
                0x4301_0000,
                0x1234,
                0,
                0x5566_7788,
                0x1122_3344,
                7,
            ],
        ),
        (
            AgcDcbWaitRegMem,
            &[1, 0, 3, 0, 0, 0x1237, 7, u32::MAX as u64, 0x100],
            &[
                0xc002_7904,
                0x342,
                BEGIN32,
                0x1237,
                0xc005_3c00,
                0x13,
                0x1234,
                0,
                7,
                u32::MAX,
                16,
                0xc001_7904,
                0x342,
                END_WAIT,
            ],
        ),
        (
            AgcAcbWaitRegMem,
            &[1, 1, 3, 0, 0x1237, 0x1122_3344_5566_7788, u64::MAX, 0x100],
            &[
                0xc002_7904,
                0x342,
                BEGIN64,
                0x1237,
                0xc007_9300,
                0x13,
                0x1230,
                0,
                0x5566_7788,
                0x1122_3344,
                u32::MAX,
                u32::MAX,
                16,
                0xc001_7904,
                0x342,
                END_WAIT,
            ],
        ),
    ];
    for (function, args, expected) in cases {
        let packet = packet(*function, full(args)).unwrap().unwrap();
        assert_eq!(&packet.words[..packet.written], *expected, "{function:?}");
    }
    for (function, arg, expected) in [
        (AgcCbNopGetSize, 0x4000_0001, 4),
        (AgcCbDispatchGetSize, 0, 20),
        (AgcDcbSetNumInstancesGetSize, 0, 8),
        (AgcDcbDrawIndexGetSize, 0, 24),
        (AgcDcbDrawIndexAutoGetSize, 0, 12),
        (AgcDcbDrawIndexOffsetGetSize, 0, 20),
        (AgcDcbWriteDataGetSize, u32::MAX as u64, 12),
        (AgcCbQueueEndOfPipeActionGetSize, 0, 32),
        (AgcDcbWaitOnAddressGetSize, 0x100, 0),
        (AgcAcbWaitOnAddressGetSize, 0x100, 56),
        (AgcDcbWaitOnAddressGetSize, 1, 64),
    ] {
        assert_eq!(size(function, &[arg, 0, 0, 0, 0, 0]), Some(expected));
    }
    assert_eq!(EXPORTS.len(), 30);
    assert!(packet(AgcDcbSetIndexBuffer, full(&[1, 3])).is_err());
    assert!(packet(AgcCbReleaseMem, full(&[1, 0, 0, 2])).is_err());
    assert!(
        packet(AgcAcbWriteData, full(&[1, 0, 0, 0, 1, 0]))
            .unwrap()
            .is_none()
    );
}

#[test]
fn allocation_prefix_precedes_descriptor_and_header_payload_reads_and_fresh_growth() {
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base, 0x8000, NativePermissions::READ_WRITE)
        .unwrap();
    let mut memory = space.pause().unwrap();
    let up = base + 0x4000;
    for source in [base + 16, up] {
        setup(&mut memory, base, up, 9, u64::MAX);
        memory.write(up, &[0xa5; 32]).unwrap();
        let mut driver = Driver::new(Default::default());
        let action = driver
            .begin_command_builder(
                &mut memory,
                Function::AgcDcbWriteData,
                [base, 1, 0, 0, source, 2],
                Some(base + 0x100),
            )
            .unwrap();
        assert_eq!(returned(action), up);
        let expected = if source == up {
            [0xc004_3700u32, 0x4000_0000]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>()
        } else {
            (up + 24).to_le_bytes().to_vec()
        };
        assert_eq!(read(&memory, up + 16, 8), expected);
        assert_eq!(read(&memory, up + 24, 8), [0xa5; 8]);
        assert_eq!(driver.usage().calls, 1);
    }
    // Only callback arguments and scalar call values are retained before growth.
    // The callback changes the descriptor and payload; completion reads both.
    setup(&mut memory, base, up, 3, base + 0x80);
    let data = base + 0x200;
    memory.write(data, &[1; 8]).unwrap();
    let mut driver = Driver::new(Default::default());
    let CommandBuilderAction::Grow(pending) = driver
        .begin_command_builder(
            &mut memory,
            Function::AgcDcbWriteData,
            [base, 1, 0, 0, data, 2],
            Some(base + 0x100),
        )
        .unwrap()
    else {
        panic!("missing growth")
    };
    assert_eq!(pending.args(), [base, 9, 0x7123]);
    setup(&mut memory, base, up + 0x100, 9, base + 0x80);
    memory.write(data, &[0x5c; 8]).unwrap();
    assert_eq!(
        driver.complete_command_builder(&mut memory, pending, 0x10001),
        Ok(up + 0x100)
    );
    assert_eq!(read(&memory, up + 0x110, 8), [0x5c; 8]);
    assert_eq!(driver.usage().calls, 1);
}

#[test]
fn source_ordered_late_fault_preserves_cursor_and_header_but_quota_refuses_before_effects() {
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base, 0x8000, NativePermissions::READ_WRITE)
        .unwrap();
    let mut memory = space.pause().unwrap();
    let up = base + 0x4000;
    setup(&mut memory, base, up, 9, 0);
    memory.write(up, &[0xa5; 32]).unwrap();
    let args = [base, 1, 0, 0, 1, 2]; // Non-null unreadable payload.
    let mut driver = Driver::new(Limits {
        max_memory_write_bytes: 31,
        ..Default::default()
    });
    assert!(matches!(
        driver.begin_command_builder(
            &mut memory,
            Function::AgcDcbWriteData,
            args,
            Some(base + 0x100)
        ),
        Err(Failure::Limit)
    ));
    assert_eq!(pointer(&memory, base, 16).unwrap(), up);
    assert_eq!(read(&memory, up, 32), [0xa5; 32]);
    let mut driver = Driver::new(Default::default());
    assert!(matches!(
        driver.begin_command_builder(
            &mut memory,
            Function::AgcDcbWriteData,
            args,
            Some(base + 0x100)
        ),
        Err(Failure::Memory)
    ));
    assert_eq!(pointer(&memory, base, 16).unwrap(), up + 24);
    assert_eq!(word(&memory, up, 0).unwrap(), 0xc004_3700);
    assert_eq!(read(&memory, up + 16, 8), [0xa5; 8]);
    assert!(driver.poisoned);
    assert!(matches!(
        driver.begin_command_builder(&mut memory, Function::AgcCbNop, [base, 1, 0, 0, 0, 0], None),
        Err(Failure::Poisoned)
    ));
}

#[test]
fn patch_helpers_preserve_compound_masks_low_reference_and_source_error_domains() {
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base, 0x4000, NativePermissions::READ_WRITE)
        .unwrap();
    let mut memory = space.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    let wait = packet(
        Function::AgcDcbWaitRegMem,
        full(&[base, 1, 3, 0, 0, 0x1237, 0x1122_3344_5566_7788, u64::MAX, 0]),
    )
    .unwrap()
    .unwrap();
    for (index, value) in wait.words[..wait.written].iter().enumerate() {
        memory
            .write(base + index as u64 * 4, &value.to_le_bytes())
            .unwrap();
    }
    let call = |driver: &mut Driver, memory: &mut PausedNativeMemory<'_>, function, value| {
        returned(
            driver
                .begin_command_builder(memory, function, [base, value, 0, 0, 0, 0], None)
                .unwrap(),
        )
    };
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            Function::AgcWaitRegMemPatchReference,
            0xaaaa_bbbb_9999_8888
        ),
        0
    );
    assert_eq!(word(&memory, base, 8).unwrap(), 0x9999_8888);
    assert_eq!(word(&memory, base, 9).unwrap(), 0x1122_3344);
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            Function::AgcWaitRegMemPatchAddress,
            0x1234_5678_9abc_def7
        ),
        0
    );
    assert_eq!(word(&memory, base, 2).unwrap(), BEGIN64 | 0x5678);
    assert_eq!(word(&memory, base, 3).unwrap(), 0x9abc_def7);
    assert_eq!(word(&memory, base, 6).unwrap(), 0x9abc_def0);
    assert_eq!(word(&memory, base, 7).unwrap(), 0x5678);
    memory.write(base, &0xc002_3700u32.to_le_bytes()).unwrap();
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            Function::AgcWriteDataPatchSetAddressOrOffset,
            0x1234_5678_9abc_def0
        ),
        0
    );
    assert_eq!(word(&memory, base, 2).unwrap(), 0x9abc_def0);
    assert_eq!(word(&memory, base, 3).unwrap(), 0x1234_5678);
    memory.write(base, &0u32.to_le_bytes()).unwrap();
    let before = read(&memory, base, 64);
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            Function::AgcWaitRegMemPatchAddress,
            0
        ),
        INVALID_PACKET
    );
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            Function::AgcWriteDataPatchSetAddressOrOffset,
            0
        ),
        INVALID_PACKET
    );
    assert_eq!(read(&memory, base, 64), before);
    let release = packet(
        Function::AgcCbReleaseMem,
        full(&[base, 0x14, 0, 0, 0, 0, 5, 0, 2, 3, 0, 0]),
    )
    .unwrap()
    .unwrap();
    for (index, value) in release.words[..release.written].iter().enumerate() {
        memory
            .write(base + index as u64 * 4, &value.to_le_bytes())
            .unwrap();
    }
    let before = read(&memory, base, 32);
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            Function::AgcQueueEndOfPipeActionPatchData,
            u64::MAX
        ),
        0
    );
    assert_eq!(read(&memory, base, 32), before); // GDS selector has no data patch.
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            Function::AgcQueueEndOfPipeActionPatchAddress,
            0x1234_5678
        ),
        0
    );
    assert_eq!(word(&memory, base, 3).unwrap(), 0x1234_5678);
    memory
        .write(base + 8, &0x4000_0000u32.to_le_bytes())
        .unwrap();
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            Function::AgcQueueEndOfPipeActionPatchData,
            0x1234_5678_9abc_def0
        ),
        0
    );
    assert_eq!(word(&memory, base, 5).unwrap(), 0x9abc_def0);
    assert_eq!(word(&memory, base, 6).unwrap(), 0x1234_5678);
    memory.write(base, &0xc004_4700u32.to_le_bytes()).unwrap();
    memory
        .write(base + 12, &0xdead_0000u32.to_le_bytes())
        .unwrap();
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            Function::AgcQueueEndOfPipeActionPatchAddress,
            0x1234_5678_9abc_def0
        ),
        0
    );
    assert_eq!(word(&memory, base, 2).unwrap(), 0x9abc_def0);
    assert_eq!(word(&memory, base, 3).unwrap(), 0xdead_5678);
}

#[test]
fn built_graphics_memory_stream_writes_then_waits_and_drains_without_inventing_gpu_jobs() {
    for waiting in [false, true] {
        let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
        let base = space.reserved_ranges().next().unwrap().address;
        space
            .map_at(
                base,
                0x8000,
                NativePermissions::READ_WRITE.with_gpu(true, true),
            )
            .unwrap();
        let mut memory = space.pause().unwrap();
        let up = base + 0x4000;
        let (data, label, stack) = (base + 0x200, base + 0x300, base + 0x400);
        setup(&mut memory, base, up, 256, 0);
        memory.write(data, &37u32.to_le_bytes()).unwrap();
        memory.write(stack + 8, &0u64.to_le_bytes()).unwrap();
        memory.write(stack + 16, &1u64.to_le_bytes()).unwrap();
        let mut driver = Driver::new(Default::default());
        assert_eq!(
            returned(
                driver
                    .begin_command_builder(
                        &mut memory,
                        Function::AgcDcbWriteData,
                        [base, 2, 0, label, data, 1],
                        Some(stack)
                    )
                    .unwrap()
            ),
            up
        );
        // Wait has reference/mask/poll as real stack arguments seven to nine.
        for (index, value) in [if waiting { 38u64 } else { 37 }, u32::MAX as u64, 0]
            .into_iter()
            .enumerate()
        {
            memory
                .write(stack + 8 + index as u64 * 8, &value.to_le_bytes())
                .unwrap();
        }
        returned(
            driver
                .begin_command_builder(
                    &mut memory,
                    Function::AgcDcbWaitRegMem,
                    [base, 0, 3, 0, 0, label],
                    Some(stack),
                )
                .unwrap(),
        );
        returned(
            driver
                .begin_command_builder(
                    &mut memory,
                    Function::AgcDcbStallCommandBufferParser,
                    [base, 0, 0, 0, 0, 0],
                    None,
                )
                .unwrap(),
        );
        returned(
            driver
                .begin_command_builder(
                    &mut memory,
                    Function::AgcDcbWriteData,
                    [base, 2, 0, 0, data, 0],
                    Some(stack),
                )
                .unwrap(),
        );
        let end = pointer(&memory, base, 16).unwrap();
        let result = driver.submit_graphics(
            &mut memory,
            &[NativeRange {
                address: up,
                size: end - up,
            }],
        );
        assert_eq!(
            result,
            if waiting {
                Err(Failure::Waiting)
            } else {
                Ok(0)
            }
        );
        assert_eq!(word(&memory, label, 0).unwrap(), 37);
        assert_eq!(driver.usage().memory_commands, if waiting { 2 } else { 4 });
        assert_eq!(driver.usage().written_bytes, 4);
        assert_eq!(
            (
                driver.usage().draws,
                driver.usage().dispatches,
                driver.usage().interrupts
            ),
            (0, 0, 0)
        );
        assert_eq!(driver.poisoned, waiting);
    }
}

#[test]
fn late_patch_rmw_fault_keeps_actual_source_prefix_and_prevents_replay() {
    for wait in [false, true] {
        let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
        let base = space.reserved_ranges().next().unwrap().address;
        space
            .map_at(base, 0x4000, NativePermissions::READ_WRITE)
            .unwrap();
        let mut memory = space.pause().unwrap();
        let (at, words, function) = if wait {
            (
                base + 0x4000 - 24,
                vec![0xc002_7904, 0x342, BEGIN32, 0x1100, 0xc005_3c00, 0x13],
                Function::AgcWaitRegMemPatchAddress,
            )
        } else {
            (
                base + 0x4000 - 12,
                vec![0xc004_4700, 0x14, 0x1100],
                Function::AgcQueueEndOfPipeActionPatchAddress,
            )
        };
        for (index, value) in words.iter().enumerate() {
            memory
                .write(at + index as u64 * 4, &value.to_le_bytes())
                .unwrap();
        }
        let value = 0x1234_5678_9abc_def0;
        let mut driver = Driver::new(Default::default());
        assert!(matches!(
            driver.begin_command_builder(&mut memory, function, [at, value, 0, 0, 0, 0], None),
            Err(Failure::Memory)
        ));
        assert_eq!(
            word(&memory, at, 2).unwrap(),
            if wait { BEGIN32 | 0x5678 } else { 0x9abc_def0 }
        );
        if wait {
            assert_eq!(word(&memory, at, 3).unwrap(), 0x9abc_def0);
        }
        assert!(driver.poisoned);
        let snapshot = read(&memory, at, words.len() * 4);
        assert!(matches!(
            driver.begin_command_builder(&mut memory, function, [at, 0, 0, 0, 0, 0], None),
            Err(Failure::Poisoned)
        ));
        assert_eq!(read(&memory, at, words.len() * 4), snapshot);
    }
}
