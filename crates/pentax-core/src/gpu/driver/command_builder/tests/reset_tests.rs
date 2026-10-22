// SPDX-License-Identifier: GPL-2.0-only
//! Original owner-backed reset encoders and source-ordered refusal prefixes.
use super::*;

#[test]
fn reset_encoders_narrow_masks_append_real_packets_and_leave_unsubmitted_queue_state() {
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base, 0x8000, NativePermissions::READ_WRITE)
        .unwrap();
    let mut memory = space.pause().unwrap();
    let up = base + 0x4000;
    let mut driver = Driver::new(Default::default());
    let queues = driver.queues;
    for mask in 0u32..16 {
        let op = ((mask & 1) << 1) | ((mask & 14) << 5);
        setup(&mut memory, base, up, 5, 0);
        memory.write(up, &[0xa5; 16]).unwrap();
        assert_eq!(
            returned(
                driver
                    .begin_command_builder(
                        &mut memory,
                        Function::AgcAcbResetQueue,
                        [base, 0xffff_ffff_0000_0000 | u64::from(op), 0, 0, 0, 0],
                        None
                    )
                    .unwrap()
            ),
            up
        );
        assert_eq!(read(&memory, base + 16, 8), (up + 8).to_le_bytes());
        assert_eq!(
            read(&memory, up, 8),
            [0xc000_1024u32.to_le_bytes(), 0u32.to_le_bytes()].concat()
        );
        assert_eq!(read(&memory, up + 8, 8), [0xa5; 8]);
    }
    setup(&mut memory, base, up, 5, 0);
    assert_eq!(
        returned(
            driver
                .begin_command_builder(
                    &mut memory,
                    Function::AgcDcbResetQueue,
                    [base, 0xffff_ffff_0000_0fff, 0x1234_5678_ffff_fffe, 0, 0, 0],
                    None
                )
                .unwrap()
        ),
        up
    );
    assert_eq!(
        read(&memory, up, 8),
        [0xc000_1200u32.to_le_bytes(), 14u32.to_le_bytes()].concat()
    );
    assert_eq!(driver.queues, queues);
    assert_eq!(driver.usage().dispatches, 0);
    for function in [Function::AgcAcbResetQueue, Function::AgcDcbResetQueue] {
        let mut rejected = Driver::new(Default::default());
        assert!(matches!(
            rejected.begin_command_builder(&mut memory, function, [0, u64::MAX, 0, 0, 0, 0], None),
            Err(Failure::UnsupportedQueue)
        ));
        let invalid = if function == Function::AgcAcbResetQueue {
            1
        } else {
            0x1000
        };
        assert!(matches!(
            rejected.begin_command_builder(
                &mut memory,
                function,
                [base + 0xc000, invalid, 0, 0, 0, 0],
                None
            ),
            Err(Failure::UnsupportedQueue)
        ));
        assert!(matches!(
            rejected.begin_command_builder(
                &mut memory,
                function,
                [base + 0xc000, 0, 0, 0, 0, 0],
                None
            ),
            Err(Failure::Memory)
        ));
    }
}

#[test]
fn reset_grow_body_requires_space_and_late_payload_fault_keeps_source_prefix_nonreplayable() {
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base, 0x8000, NativePermissions::READ_WRITE)
        .unwrap();
    let mut memory = space.pause().unwrap();
    for function in [Function::AgcAcbResetQueue, Function::AgcDcbResetQueue] {
        for returned_bool in [0, 0x100] {
            setup(&mut memory, base, base + 0x4000, 3, base + 0x80);
            let mut driver = Driver::new(Default::default());
            let CommandBuilderAction::Grow(pending) = driver
                .begin_command_builder(&mut memory, function, [base, 0, 0, 0, 0, 0], None)
                .unwrap()
            else {
                panic!("grow")
            };
            assert_eq!(pending.args(), [base, 5, 0x7123]);
            assert!(matches!(
                driver.complete_command_builder(&mut memory, pending, returned_bool),
                Err(Failure::UnsupportedQueue)
            ));
            assert_eq!(read(&memory, base + 16, 8), (base + 0x4000).to_le_bytes());
        }
        // The pure body test does not authenticate callbacks: real native
        // continuation/receipt fixtures independently exercise the transport.
        setup(&mut memory, base, base + 0x4000, 3, base + 0x80);
        let mut driver = Driver::new(Default::default());
        let CommandBuilderAction::Grow(pending) = driver
            .begin_command_builder(&mut memory, function, [base, 0, 0, 0, 0, 0], None)
            .unwrap()
        else {
            panic!("grow")
        };
        assert!(matches!(
            driver.complete_command_builder(&mut memory, pending, 1),
            Err(Failure::UnsupportedQueue)
        ));
        let up = base + 0x7ffc;
        setup(&mut memory, base, up, 5, 0);
        let mut driver = Driver::new(Default::default());
        assert!(matches!(
            driver.begin_command_builder(&mut memory, function, [base, 0, 0, 0, 0, 0], None),
            Err(Failure::Memory)
        ));
        assert_eq!(read(&memory, base + 16, 8), (up + 8).to_le_bytes());
        let header = if function == Function::AgcAcbResetQueue {
            0xc000_1024u32
        } else {
            0xc000_1200
        };
        assert_eq!(read(&memory, up, 4), header.to_le_bytes());
        assert!(matches!(
            driver.begin_command_builder(&mut memory, function, [base, 0, 0, 0, 0, 0], None),
            Err(Failure::Poisoned)
        ));
    }
}

#[test]
fn submitted_reset_changes_only_its_owned_acb_queue_after_the_real_encoder_result() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x8000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let descriptor = base + 0x100;
    let command = base + 0x1000;
    let seed = [0xc002_7600u32, 0x20c, 0x456, 0];
    memory
        .write(
            command,
            &seed
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    memory.write(descriptor, &command.to_le_bytes()).unwrap();
    memory.write(descriptor + 8, &4u32.to_le_bytes()).unwrap();
    let mut driver = Driver::new(Default::default());
    for queue in [0x20, 0x21] {
        assert_eq!(
            driver.submit(&mut memory, Function::Acb, [queue, descriptor, 0, 0, 0, 0]),
            Ok(0)
        );
    }
    let before = driver.queues;
    let buffer = base + 0x200;
    let up = base + 0x4000;
    setup(&mut memory, buffer, up, 5, 0);
    assert_eq!(
        returned(
            driver
                .begin_command_builder(
                    &mut memory,
                    Function::AgcAcbResetQueue,
                    [buffer, 0, 0, 0, 0, 0],
                    None
                )
                .unwrap()
        ),
        up
    );
    assert_eq!(driver.queues, before);
    memory.write(descriptor, &up.to_le_bytes()).unwrap();
    memory.write(descriptor + 8, &2u32.to_le_bytes()).unwrap();
    assert_eq!(
        driver.submit(&mut memory, Function::Acb, [0x20, descriptor, 0, 0, 0, 0]),
        Ok(0)
    );
    assert_ne!(driver.queues[0], before[0]);
    assert_eq!(driver.queues[1], before[1]);
    assert_eq!(&driver.queues[2..], &before[2..]);
    assert_eq!(driver.usage().dispatches, 0);
    assert_eq!(driver.usage().draws, 0);
    assert!(!driver.poisoned);
}
