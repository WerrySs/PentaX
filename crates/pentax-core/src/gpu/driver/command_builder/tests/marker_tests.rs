// SPDX-License-Identifier: GPL-2.0-only
//! Original real-owner marker growth, byte ordering and refusal consumers.
use super::*;
fn source() -> (NativeAddressSpace, u64) {
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
#[test]
fn marker_growth_captures_length_then_copies_current_bytes_and_preserves_zero_padding() {
    let (mut owner, base) = source();
    let mut memory = owner.pause().unwrap();
    let up = base + 0x4000;
    let text = base + 0x8000;
    for (function, original, changed, expected) in [
        (
            Function::AgcAcbPushMarker,
            &b"abcd\0"[..],
            &b"0123456\0"[..],
            &b"01234\0\0\0"[..],
        ),
        (
            Function::AgcDcbPushMarker,
            &b"abcd\0"[..],
            &b"Z\0pqrst\0"[..],
            &b"Z\0pqr\0\0\0"[..],
        ),
    ] {
        setup(&mut memory, base, up, 0, 1);
        memory.write(text, original).unwrap();
        memory.write(up, &[0xa5; 12]).unwrap();
        let mut driver = Driver::new(Default::default());
        let action = driver
            .begin_command_builder(&mut memory, function, [base, text, u64::MAX, 0, 0, 0], None)
            .unwrap();
        let CommandBuilderAction::Grow(pending) = action else {
            panic!("actual growth continuation")
        };
        assert_eq!(pending.args()[1], 6); // Three packet words plus the source reserve.
        memory.write(text, changed).unwrap();
        setup(&mut memory, base, up, 6, 1);
        assert_eq!(
            returned(
                driver
                    .complete_command_builder(&mut memory, pending, 1)
                    .unwrap()
            ),
            up
        );
        assert_eq!(read(&memory, up, 4), 0xc001_102cu32.to_le_bytes());
        assert_eq!(read(&memory, up + 4, 8), expected);
        assert_eq!(read(&memory, base + 16, 8), (up + 12).to_le_bytes());
        assert!(!driver.poisoned);
    }
    // The terminator occupies the final mapped byte: no speculative next read.
    memory.write(base + 0xfffe, b"x\0").unwrap();
    setup(&mut memory, base, up, 5, 0);
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        returned(
            driver
                .begin_command_builder(
                    &mut memory,
                    Function::AgcAcbPushMarker,
                    [base, base + 0xfffe, 0, 0, 0, 0],
                    None
                )
                .unwrap()
        ),
        up
    );
    assert_eq!(read(&memory, up + 4, 4), b"x\0\0\0");
}

#[test]
fn marker_zero_store_and_late_source_fault_preserve_real_cursor_header_and_poison() {
    let (mut owner, base) = source();
    let mut memory = owner.pause().unwrap();
    let up = base + 0x4000;
    // memcpy overlap is refused only after the source's complete memset.
    setup(&mut memory, base, up, 6, 0);
    memory.write(up + 4, b"abcdef\0!").unwrap();
    let mut driver = Driver::new(Default::default());
    assert!(matches!(
        driver.begin_command_builder(
            &mut memory,
            Function::AgcDcbPushMarker,
            [base, up + 4, 0, 0, 0, 0],
            None
        ),
        Err(Failure::Memory)
    ));
    assert_eq!(read(&memory, base + 16, 8), (up + 12).to_le_bytes());
    assert_eq!(
        read(&memory, up, 12),
        [0xc001_102cu32.to_le_bytes().as_slice(), &[0; 8]].concat()
    );
    assert!(driver.poisoned);
    // A real callback may revoke the source after strlen, before publication.
    setup(&mut memory, base, up, 0, 1);
    memory.write(base + 0xc000, b"abc\0").unwrap();
    let mut driver = Driver::new(Default::default());
    let CommandBuilderAction::Grow(pending) = driver
        .begin_command_builder(
            &mut memory,
            Function::AgcAcbPushMarker,
            [base, base + 0xc000, 0, 0, 0, 0],
            None,
        )
        .unwrap()
    else {
        panic!("grow")
    };
    // End the pause before the actual owner permission change.
    owner
        .protect(
            base + 0xc000,
            0x4000,
            NativePermissions::new(false, false, false),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    setup(&mut memory, base, up, 5, 1);
    memory.write(up, &[0xa5; 8]).unwrap();
    assert!(matches!(
        driver.complete_command_builder(&mut memory, pending, 1),
        Err(Failure::Memory)
    ));
    assert_eq!(
        read(&memory, up, 8),
        [0xc000_102cu32.to_le_bytes(), [0; 4]].concat()
    );
    assert!(driver.poisoned);
}

#[test]
fn marker_scan_admission_is_bounded_and_null_buffer_precedes_unreadable_text() {
    let (mut owner, base) = source();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        returned(
            driver
                .begin_command_builder(
                    &mut memory,
                    Function::AgcAcbPushMarker,
                    [0, u64::MAX, 0, 0, 0, 0],
                    None
                )
                .unwrap()
        ),
        0
    );
    setup(&mut memory, base, base + 0x4000, 2, 0);
    memory.write(base + 0x8000, b"abc\0").unwrap();
    let mut limited = Driver::new(Limits {
        max_preparation_work: 260,
        ..Default::default()
    });
    assert!(matches!(
        limited.begin_command_builder(
            &mut memory,
            Function::AgcDcbPushMarker,
            [base, base + 0x8000, 0, 0, 0, 0],
            None
        ),
        Err(Failure::Limit)
    ));
    assert_eq!(read(&memory, base + 16, 8), (base + 0x4000).to_le_bytes());
    assert_eq!(limited.usage().preparation_work, 260);
    assert!(!limited.poisoned);
}

#[test]
fn submitted_annotations_preserve_actual_write_array_wait_order_without_fake_memory_or_frame() {
    let (mut owner, base) = source();
    let mut memory = owner.pause().unwrap();
    let array = base + 0x8000;
    let words = [
        0xc004_3700,
        0x100,
        array as u32,
        (array >> 32) as u32,
        0x20c,
        17,
        0xc000_102c,
        0xff,
        0xc003_6300,
        array as u32,
        (array >> 32) as u32,
        0x8000_0000,
        1,
        0xc000_1030,
        0,
        0xc005_3c00,
        0x13,
        (array + 4) as u32,
        ((array + 4) >> 32) as u32,
        17,
        u32::MAX,
        0,
    ];
    let command = base + 0x1000;
    let descriptor = base + 0x100;
    let bytes: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
    memory.write(command, &bytes).unwrap();
    memory.write(descriptor, &command.to_le_bytes()).unwrap();
    memory
        .write(descriptor + 8, &(words.len() as u32).to_le_bytes())
        .unwrap();
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(&mut memory, Function::Acb, [0x20, descriptor, 0, 0, 0, 0]),
        Ok(0)
    );
    assert_eq!(
        read(&memory, array, 8),
        [0x20cu32.to_le_bytes(), 17u32.to_le_bytes()].concat()
    );
    assert_ne!(driver.queues[0], compute::ComputeState::empty());
    assert!(
        driver.queues[1..]
            .iter()
            .all(|q| *q == compute::ComputeState::empty())
    );
    assert_eq!(
        (driver.usage().memory_commands, driver.usage().written_bytes),
        (2, 8)
    );
    assert_eq!(
        (
            driver.usage().draws,
            driver.usage().dispatches,
            driver.usage().interrupts
        ),
        (0, 0, 0)
    );
    assert_eq!(driver.frame_state(), Default::default());
    // Invalid Pop after a write must fail complete admission before the write.
    memory.write(array, &[0xa5; 8]).unwrap();
    let mut invalid = bytes.clone();
    invalid[14 * 4..15 * 4].copy_from_slice(&1u32.to_le_bytes());
    memory.write(command, &invalid).unwrap();
    let mut driver = Driver::new(Default::default());
    assert!(matches!(
        driver.submit(&mut memory, Function::Acb, [0x20, descriptor, 0, 0, 0, 0]),
        Err(Failure::Preparation(_))
    ));
    assert_eq!(read(&memory, array, 8), [0xa5; 8]);
    assert_eq!(driver.usage().written_bytes, 0);
}
