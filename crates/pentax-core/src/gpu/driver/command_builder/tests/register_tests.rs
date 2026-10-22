// SPDX-License-Identifier: GPL-2.0-only
//! Original checked-memory register packet, allocation and continuation proofs.
use super::*;

fn words(memory: &PausedNativeMemory<'_>, at: u64, count: usize) -> Vec<u32> {
    read(memory, at, count * 4)
        .chunks_exact(4)
        .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
        .collect()
}
fn pairs(memory: &mut PausedNativeMemory<'_>, at: u64, values: &[(u32, u32)]) {
    for (index, (offset, value)) in values.iter().enumerate() {
        memory
            .write(at + index as u64 * 8, &offset.to_le_bytes())
            .unwrap();
        memory
            .write(at + index as u64 * 8 + 4, &value.to_le_bytes())
            .unwrap();
    }
}
fn grow(action: CommandBuilderAction) -> PendingCommandBuilder {
    let CommandBuilderAction::Grow(pending) = action else {
        panic!("expected real allocator request")
    };
    pending
}

#[test]
fn register_null_and_zero_count_paths_preserve_allocator_and_payload_order() {
    use Function::*;
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base, 0x8000, NativePermissions::READ_WRITE)
        .unwrap();
    let mut memory = space.pause().unwrap();
    for function in [AgcCbSetShRegistersDirect, AgcCbSetUcRegistersDirect] {
        let mut driver = Driver::new(Default::default());
        // Low32 zero is tested before either pointer; high argument bits do
        // not turn the source uint32 count into a host-sized allocation.
        assert_eq!(
            returned(
                driver
                    .begin_command_builder(&mut memory, function, [0, 0, 1u64 << 32, 0, 0, 0], None)
                    .unwrap()
            ),
            0
        );
        assert_eq!(driver.usage().memory_written_bytes, 0);
    }
    for function in [
        AgcCbSetShRegisterRangeDirect,
        AgcDcbSetShRegisterDirect,
        AgcDcbSetCxRegisterDirect,
        AgcDcbSetUcRegisterDirect,
    ] {
        setup(&mut memory, base, 0, 16, 0);
        let mut driver = Driver::new(Default::default());
        assert_eq!(
            returned(
                driver
                    .begin_command_builder(
                        &mut memory,
                        function,
                        [base, 7, u64::MAX, 1, 0, 0],
                        None
                    )
                    .unwrap()
            ),
            0
        );
        assert_eq!(pointer(&memory, base, 16).unwrap(), 12);
        // The inaccessible range source is never read when AllocateDW has
        // returned nullptr, despite its already committed cursor store.
        assert!(!driver.poisoned);
    }
    for function in [AgcCbSetShRegistersDirect, AgcCbSetUcRegistersDirect] {
        setup(&mut memory, base, 0, 16, 0);
        let source = base + 0x200;
        pairs(&mut memory, source, &[(0x100, 11)]);
        let mut driver = Driver::new(Default::default());
        assert_eq!(
            returned(
                driver
                    .begin_command_builder(&mut memory, function, [base, source, 1, 0, 0, 0], None)
                    .unwrap()
            ),
            0
        );
        assert_eq!(pointer(&memory, base, 16).unwrap(), 12);
        assert!(!driver.poisoned);
    }
    for function in [
        AgcDcbSetCxRegistersIndirect,
        AgcDcbSetShRegistersIndirect,
        AgcDcbSetUcRegistersIndirect,
    ] {
        setup(&mut memory, base, 0, 16, 0);
        let mut driver = Driver::new(Default::default());
        assert!(matches!(
            driver.begin_command_builder(&mut memory, function, [base, u64::MAX, 1, 0, 0, 0], None),
            Err(Failure::UnsupportedQueue)
        ));
        assert_eq!(pointer(&memory, base, 16).unwrap(), 20);
        assert!(driver.poisoned);
    }
}

#[test]
fn register_single_range_indirect_and_sizes_publish_exact_source_words() {
    use Function::*;
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base, 0x8000, NativePermissions::READ_WRITE)
        .unwrap();
    let mut memory = space.pause().unwrap();
    let up = base + 0x4000;
    for (function, opcode) in [
        (AgcDcbSetShRegisterDirect, 0x76u32),
        (AgcDcbSetCxRegisterDirect, 0x69),
        (AgcDcbSetUcRegisterDirect, 0x79),
    ] {
        setup(&mut memory, base, up, 16, 0);
        let mut driver = Driver::new(Default::default());
        assert_eq!(
            returned(
                driver
                    .begin_command_builder(
                        &mut memory,
                        function,
                        [base, 0x1234_5678_abcd_00e1, 0, 0, 0, 0],
                        None
                    )
                    .unwrap()
            ),
            up
        );
        assert_eq!(
            words(&memory, up, 3),
            [0xc001_0000 | opcode << 8, 0xe1, 0x1234_5678]
        );
        assert_eq!(pointer(&memory, base, 16).unwrap(), up + 12);
    }
    for (function, header) in [
        (AgcDcbSetCxRegistersIndirect, 0xc003_9f00),
        (AgcDcbSetShRegistersIndirect, 0xc003_6300),
        (AgcDcbSetUcRegistersIndirect, 0xc003_6400),
    ] {
        setup(&mut memory, base, up, 16, 0);
        let mut driver = Driver::new(Default::default());
        assert_eq!(
            returned(
                driver
                    .begin_command_builder(
                        &mut memory,
                        function,
                        [base, 0x1234_5678_9abc_def3, 0xffff_8001, 0, 0, 0],
                        None
                    )
                    .unwrap()
            ),
            up
        );
        assert_eq!(
            words(&memory, up, 5),
            [header, 0x9abc_def0, 0x1234_5678, 0x8000_0000, 1]
        );
    }
    let mut driver = Driver::new(Default::default());
    for (function, count, bytes) in [
        (AgcCbSetShRegisterRangeDirectGetSize, u32::MAX as u64, 4),
        (AgcDcbSetCxRegisterDirectGetSize, 0, 12),
        (AgcDcbSetCxRegistersIndirectGetSize, 0, 20),
        (AgcDcbSetShRegistersIndirectGetSize, 0, 20),
    ] {
        assert_eq!(
            returned(
                driver
                    .begin_command_builder(&mut memory, function, [count, 0, 0, 0, 0, 0], None)
                    .unwrap()
            ),
            bytes
        );
    }
    // The cursor publication aliases the source: the subsequent memcpy sees
    // its new eight-byte value, after both source header stores.
    setup(&mut memory, base, up, 16, 0);
    assert_eq!(
        returned(
            driver
                .begin_command_builder(
                    &mut memory,
                    AgcCbSetShRegisterRangeDirect,
                    [base, 0xffff_0031, base + 16, 2, 0, 0],
                    None
                )
                .unwrap()
        ),
        up
    );
    assert_eq!(words(&memory, up, 2), [0xc002_7600, 0x31]);
    assert_eq!(read(&memory, up + 8, 8), (up + 16).to_le_bytes());
    setup(&mut memory, base, up, 16, 0);
    memory.write(up, &[0x5a; 16]).unwrap();
    returned(
        driver
            .begin_command_builder(
                &mut memory,
                AgcCbSetShRegisterRangeDirect,
                [base, 7, 0, 2, 0, 0],
                None,
            )
            .unwrap(),
    );
    assert_eq!(read(&memory, up + 8, 8), [0x5a; 8]);
}

#[test]
fn grouped_register_runs_coalesce_wrapping_offsets_and_keep_packet_boundaries() {
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base, 0x8000, NativePermissions::READ_WRITE)
        .unwrap();
    let mut memory = space.pause().unwrap();
    let (up, source) = (base + 0x4000, base + 0x200);
    for (function, opcode) in [
        (Function::AgcCbSetShRegistersDirect, 0x76),
        (Function::AgcCbSetUcRegistersDirect, 0x79),
    ] {
        setup(&mut memory, base, up, 16, 0);
        pairs(&mut memory, source, &[(u32::MAX, 11), (0, 22), (0x200, 33)]);
        let mut driver = Driver::new(Default::default());
        assert_eq!(
            returned(
                driver
                    .begin_command_builder(&mut memory, function, [base, source, 3, 0, 0, 0], None)
                    .unwrap()
            ),
            up
        );
        assert_eq!(
            words(&memory, up, 7),
            [
                0xc002_0000 | opcode << 8,
                0xffff,
                11,
                22,
                0xc001_0000 | opcode << 8,
                0x200,
                33
            ]
        );
        assert_eq!(pointer(&memory, base, 16).unwrap(), up + 28);
    }
}

#[test]
fn grouped_sh_snapshots_all_pairs_but_uc_rereads_only_future_runs_across_growth() {
    for sh in [true, false] {
        let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
        let base = space.reserved_ranges().next().unwrap().address;
        space
            .map_at(base, 0x8000, NativePermissions::READ_WRITE)
            .unwrap();
        let mut memory = space.pause().unwrap();
        let (up, source) = (base + 0x4000, base + 0x200);
        setup(&mut memory, base, up, 3, base + 0x80);
        pairs(
            &mut memory,
            source,
            &[(0x100, 11), (0x200, 22), (0x300, 33)],
        );
        let mut driver = Driver::new(Default::default());
        let function = if sh {
            Function::AgcCbSetShRegistersDirect
        } else {
            Function::AgcCbSetUcRegistersDirect
        };
        let first = grow(
            driver
                .begin_command_builder(&mut memory, function, [base, source, 3, 0, 0, 0], None)
                .unwrap(),
        );
        let usage = driver.usage();
        pairs(
            &mut memory,
            source,
            &[(0x110, 111), (0x210, 222), (0x300, 333)],
        );
        setup(&mut memory, base, up + 0x100, 6, base + 0x80);
        let second = grow(
            driver
                .complete_command_builder(&mut memory, first, 1)
                .unwrap(),
        );
        pairs(
            &mut memory,
            source,
            &[(0x120, 1111), (0x220, 2222), (0x310, 3333)],
        );
        setup(&mut memory, base, up + 0x200, 6, base + 0x80);
        let third = grow(
            driver
                .complete_command_builder(&mut memory, second, 1)
                .unwrap(),
        );
        pairs(
            &mut memory,
            source,
            &[(0x130, 11111), (0x230, 22222), (0x320, 33333)],
        );
        setup(&mut memory, base, up + 0x300, 6, base + 0x80);
        assert_eq!(
            returned(
                driver
                    .complete_command_builder(&mut memory, third, 1)
                    .unwrap()
            ),
            up + 0x100
        );
        let expected: [(u32, u32); 3] = if sh {
            [(0x100, 11), (0x200, 22), (0x300, 33)]
        } else {
            [(0x100, 11), (0x210, 222), (0x310, 3333)]
        };
        for (index, (offset, value)) in expected.into_iter().enumerate() {
            assert_eq!(
                words(&memory, up + (index as u64 + 1) * 0x100, 3),
                [if sh { 0xc001_7600 } else { 0xc001_7900 }, offset, value]
            );
        }
        assert_eq!(driver.usage(), usage);
        assert_eq!(driver.function_profile().count(function), 1);
    }
}

#[test]
fn grouped_failed_allocation_stops_sh_but_uc_continues_without_refunding_the_call() {
    for sh in [true, false] {
        let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
        let base = space.reserved_ranges().next().unwrap().address;
        space
            .map_at(base, 0x8000, NativePermissions::READ_WRITE)
            .unwrap();
        let mut memory = space.pause().unwrap();
        let (up, source) = (base + 0x4000, base + 0x200);
        setup(&mut memory, base, up, 3, base + 0x80);
        pairs(
            &mut memory,
            source,
            &[(0x100, 11), (0x200, 22), (0x300, 33)],
        );
        let mut driver = Driver::new(Default::default());
        let function = if sh {
            Function::AgcCbSetShRegistersDirect
        } else {
            Function::AgcCbSetUcRegistersDirect
        };
        let first = grow(
            driver
                .begin_command_builder(&mut memory, function, [base, source, 3, 0, 0, 0], None)
                .unwrap(),
        );
        let usage = driver.usage();
        let action = driver
            .complete_command_builder(&mut memory, first, 0x100)
            .unwrap();
        if sh {
            assert_eq!(returned(action), 0);
        } else {
            let second = grow(action);
            // Source UC skips this failed run too, then can emit a later one.
            let third = grow(
                driver
                    .complete_command_builder(&mut memory, second, 0)
                    .unwrap(),
            );
            setup(&mut memory, base, up, 6, 0);
            assert_eq!(
                returned(
                    driver
                        .complete_command_builder(&mut memory, third, 1)
                        .unwrap()
                ),
                up
            );
            assert_eq!(words(&memory, up, 3), [0xc001_7900, 0x300, 33]);
        }
        assert_eq!(driver.usage(), usage);
    }
}

#[test]
fn register_late_reads_keep_exact_prefix_but_snapshot_and_quota_refusals_publish_nothing() {
    for sh in [true, false] {
        let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
        let base = space.reserved_ranges().next().unwrap().address;
        space
            .map_at(base, 0x8000, NativePermissions::READ_WRITE)
            .unwrap();
        let mut memory = space.pause().unwrap();
        let (up, source) = (base + 0x4000, base + 0x8000 - 16);
        setup(&mut memory, base, up, 32, 0);
        pairs(&mut memory, source, &[(0x100, 11), (0x200, 22)]);
        memory.write(up, &[0x5a; 32]).unwrap();
        let mut driver = Driver::new(Default::default());
        let function = if sh {
            Function::AgcCbSetShRegistersDirect
        } else {
            Function::AgcCbSetUcRegistersDirect
        };
        assert!(matches!(
            driver.begin_command_builder(&mut memory, function, [base, source, 3, 0, 0, 0], None),
            Err(Failure::Memory)
        ));
        if sh {
            assert_eq!(pointer(&memory, base, 16).unwrap(), up);
            assert_eq!(read(&memory, up, 32), [0x5a; 32]);
        } else {
            assert_eq!(pointer(&memory, base, 16).unwrap(), up + 12);
            assert_eq!(words(&memory, up, 3), [0xc001_7900, 0x100, 11]);
            assert!(matches!(
                driver.begin_command_builder(
                    &mut memory,
                    Function::AgcCbNop,
                    [base, 1, 0, 0, 0, 0],
                    None
                ),
                Err(Failure::Poisoned)
            ));
        }
        setup(&mut memory, base, up, 32, 0);
        let mut limits = Limits::default();
        limits.commands.max_host_bytes = 1;
        let mut limited = Driver::new(limits);
        assert!(matches!(
            limited.begin_command_builder(&mut memory, function, [base, source, 3, 0, 0, 0], None),
            Err(Failure::Limit)
        ));
        assert_eq!(pointer(&memory, base, 16).unwrap(), up);
        assert_eq!(limited.usage().memory_written_bytes, 0);
    }
}

#[test]
fn register_group_admits_whole_call_limits_before_source_reads_or_publication() {
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base, 0x8000, NativePermissions::READ_WRITE)
        .unwrap();
    let mut memory = space.pause().unwrap();
    let up = base + 0x4000;
    for function in [
        Function::AgcCbSetShRegistersDirect,
        Function::AgcCbSetUcRegistersDirect,
    ] {
        for policy in 0..5 {
            setup(&mut memory, base, up, 32, base + 0x80);
            memory.write(up, &[0x5a; 32]).unwrap();
            let mut limits = Limits::default();
            match policy {
                0 => limits.commands.max_command_bytes = 35, // Three worst-case runs need36.
                1 => limits.commands.max_host_bytes = 1,
                2 => limits.commands.max_work = 1,
                3 => limits.max_preparation_work = 256,
                _ => limits.max_memory_write_bytes = 59, // Three runs retain60.
            }
            let mut driver = Driver::new(limits);
            // An invalid pair pointer distinguishes a quota admission refusal
            // from an accidental early source read or callback request.
            assert!(matches!(
                driver.begin_command_builder(
                    &mut memory,
                    function,
                    [base, u64::MAX, 3, 0, 0, 0],
                    None
                ),
                Err(Failure::Limit)
            ));
            assert_eq!(pointer(&memory, base, 16).unwrap(), up);
            assert_eq!(read(&memory, up, 32), [0x5a; 32]);
            assert_eq!(driver.usage().memory_written_bytes, 0);
        }
    }
}

#[test]
fn indirect_patches_keep_flags_wrap_counts_and_preserve_cross_page_failure_prefix() {
    use Function::*;
    let roles = [
        (
            0x9fu32,
            AgcSetCxRegIndirectPatchSetAddress,
            AgcSetCxRegIndirectPatchSetNumRegisters,
            AgcSetCxRegIndirectPatchAddRegisters,
        ),
        (
            0x63,
            AgcSetShRegIndirectPatchSetAddress,
            AgcSetShRegIndirectPatchSetNumRegisters,
            AgcSetShRegIndirectPatchAddRegisters,
        ),
        (
            0x64,
            AgcSetUcRegIndirectPatchSetAddress,
            AgcSetUcRegIndirectPatchSetNumRegisters,
            AgcSetUcRegIndirectPatchAddRegisters,
        ),
    ];
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base, 0x8000, NativePermissions::READ_WRITE)
        .unwrap();
    let mut memory = space.pause().unwrap();
    let at = base + 0x200;
    for (opcode, address, set, add_count) in roles {
        for function in [address, set, add_count] {
            memory.write(at, &0xc003_1000u32.to_le_bytes()).unwrap();
            memory.write(at + 4, &[0x5a; 16]).unwrap();
            let mut rejected = Driver::new(Default::default());
            assert_eq!(
                returned(
                    rejected
                        .begin_command_builder(
                            &mut memory,
                            function,
                            [at, 0x1234_5678_9abc_def1, 0, 0, 0, 0],
                            None
                        )
                        .unwrap()
                ),
                INVALID_PACKET
            );
            assert_eq!(read(&memory, at + 4, 16), [0x5a; 16]);
            assert_eq!(rejected.usage().memory_written_bytes, 0);
        }
        for (index, value) in [0xff00_0003 | opcode << 8, 3, 0, 0x8000_0000, 0xc000_3fff]
            .into_iter()
            .enumerate()
        {
            memory
                .write(at + index as u64 * 4, &value.to_le_bytes())
                .unwrap();
        }
        let mut driver = Driver::new(Default::default());
        assert_eq!(
            returned(
                driver
                    .begin_command_builder(
                        &mut memory,
                        address,
                        [at, 0x1234_5678_9abc_def1, 0, 0, 0, 0],
                        None
                    )
                    .unwrap()
            ),
            0
        );
        assert_eq!(words(&memory, at + 4, 2), [0x9abc_def3, 0x1234_5678]);
        returned(
            driver
                .begin_command_builder(&mut memory, set, [at, 0xffff_ffff, 0, 0, 0, 0], None)
                .unwrap(),
        );
        returned(
            driver
                .begin_command_builder(&mut memory, add_count, [at, 2, 0, 0, 0, 0], None)
                .unwrap(),
        );
        assert_eq!(words(&memory, at + 16, 1), [0xc000_0001]);
        let boundary = base + 0x8000 - 8;
        memory
            .write(boundary, &(opcode << 8).to_le_bytes())
            .unwrap();
        memory.write(boundary + 4, &3u32.to_le_bytes()).unwrap();
        assert!(matches!(
            driver.begin_command_builder(
                &mut memory,
                address,
                [boundary, 0x1234_5678_9abc_def1, 0, 0, 0, 0],
                None
            ),
            Err(Failure::Memory)
        ));
        assert_eq!(words(&memory, boundary + 4, 1), [0x9abc_def3]);
        assert!(matches!(
            driver.begin_command_builder(&mut memory, set, [at, 1, 0, 0, 0, 0], None),
            Err(Failure::Poisoned)
        ));
    }
}
