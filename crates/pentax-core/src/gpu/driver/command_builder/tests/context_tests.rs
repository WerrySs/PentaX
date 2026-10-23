// SPDX-License-Identifier: GPL-2.0-only
//! Original fragmented context command and owned failure-prefix consumers.
use super::*;
const FUNCTION: Function = Function::AgcDcbContextStateOp;
fn encoded(op: u32) -> Vec<u8> {
    let segments: &[u32] = match op {
        0 => &[5],
        1 => &[5, 8, 9, 3, 2],
        2 => &[3, 5, 8, 9, 2],
        _ => &[5, 8, 9, 3, 2, 5],
    };
    let mut words = Vec::new();
    for (index, &size) in segments.iter().enumerate() {
        words.push(0xc000_1000 | ((size - 2) << 16) | if index == 0 { 0x68 } else { 0 });
        words.push(if index == 0 { op } else { 0 });
        words.resize(words.len() + size as usize - 2, 0);
    }
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}
#[test]
fn context_encoder_publishes_exact_fragmented_zeroed_segments_without_immediate_state_effects() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(base, 0x8000, NativePermissions::READ_WRITE)
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let up = base + 0x4000;
    let mut driver = Driver::new(Default::default());
    let initial = driver.graphics;
    for operation in 0..4 {
        setup(&mut memory, base, up, 64, 0);
        memory.write(up, &[0xa5; 160]).unwrap();
        assert_eq!(
            returned(
                driver
                    .begin_command_builder(
                        &mut memory,
                        FUNCTION,
                        [base, 0xffff_ffff_0000_0000 | operation, 0, 0, 0, 0],
                        None
                    )
                    .unwrap()
            ),
            up
        );
        let expected = encoded(operation as u32);
        assert_eq!(read(&memory, up, expected.len()), expected);
        assert_eq!(
            read(&memory, base + 16, 8),
            (up + expected.len() as u64).to_le_bytes()
        );
        assert_eq!(read(&memory, up + expected.len() as u64, 8), [0xa5; 8]);
        assert_eq!(driver.graphics, initial);
    }
    assert_eq!((driver.usage().draws, driver.usage().dispatches), (0, 0));
    for args in [[0, u64::MAX, 0, 0, 0, 0], [base + 0xc000, 4, 0, 0, 0, 0]] {
        assert_eq!(
            returned(
                driver
                    .begin_command_builder(&mut memory, FUNCTION, args, None)
                    .unwrap()
            ),
            0
        );
    }
}
#[test]
fn context_reserve_and_later_growth_preserve_first_pointer_and_each_source_prefix() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(base, 0x8000, NativePermissions::READ_WRITE)
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let first = base + 0x4000;
    let second = base + 0x5000;
    let third = base + 0x6000;
    setup(&mut memory, base, first, 3, base + 0x100);
    let mut driver = Driver::new(Default::default());
    let CommandBuilderAction::Grow(pending) = driver
        .begin_command_builder(&mut memory, FUNCTION, [base, 3, 0, 0, 0, 0], None)
        .unwrap()
    else {
        panic!("reserve22")
    };
    assert_eq!(pending.args(), [base, 25, 0x7123]);
    assert_eq!(read(&memory, base + 16, 8), first.to_le_bytes());
    setup(&mut memory, base, first, 25, base + 0x100);
    let CommandBuilderAction::Grow(pending) = driver
        .complete_command_builder(&mut memory, pending, 1)
        .unwrap()
    else {
        panic!("append3")
    };
    assert_eq!(pending.args(), [base, 6, 0x7123]);
    assert_eq!(read(&memory, first, 88), encoded(3)[..88]);
    assert_eq!(read(&memory, base + 16, 8), (first + 88).to_le_bytes());
    setup(&mut memory, base, second, 8, base + 0x100);
    let CommandBuilderAction::Grow(pending) = driver
        .complete_command_builder(&mut memory, pending, 0x101)
        .unwrap()
    else {
        panic!("tail5")
    };
    assert_eq!(pending.args(), [base, 8, 0x7123]);
    assert_eq!(read(&memory, second, 20), encoded(3)[88..108]);
    setup(&mut memory, base, third, 8, 0);
    assert_eq!(
        returned(
            driver
                .complete_command_builder(&mut memory, pending, 1)
                .unwrap()
        ),
        first
    );
    assert_eq!(read(&memory, third, 20), encoded(3)[108..128]);
    assert!(!driver.poisoned);
    // Pop's first3 is already visible before the standalone Reserve22 fails.
    for value in [0, 0x100, 1] {
        setup(&mut memory, base, first, 6, base + 0x100);
        let mut driver = Driver::new(Default::default());
        let CommandBuilderAction::Grow(pending) = driver
            .begin_command_builder(&mut memory, FUNCTION, [base, 2, 0, 0, 0, 0], None)
            .unwrap()
        else {
            panic!("pop reserve")
        };
        assert_eq!(pending.args(), [base, 25, 0x7123]);
        assert_eq!(read(&memory, first, 12), encoded(2)[..12]);
        assert_eq!(read(&memory, base + 16, 8), (first + 12).to_le_bytes());
        assert_eq!(
            returned(
                driver
                    .complete_command_builder(&mut memory, pending, value)
                    .unwrap()
            ),
            0
        );
        assert_eq!(read(&memory, first, 12), encoded(2)[..12]);
        assert!(!driver.poisoned);
    }
}
#[test]
fn context_builder_budget_owner_and_late_output_faults_keep_precise_admission_boundary() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(base, 0x8000, NativePermissions::READ_WRITE)
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let up = base + 0x4000;
    setup(&mut memory, base, up, 64, 0);
    let before = read(&memory, base, 56);
    for limits in [
        Limits {
            max_memory_write_bytes: 35,
            ..Default::default()
        },
        Limits {
            max_preparation_work: 8447,
            ..Default::default()
        },
    ] {
        let mut driver = Driver::new(limits);
        assert!(matches!(
            driver.begin_command_builder(&mut memory, FUNCTION, [base, 0, 0, 0, 0, 0], None),
            Err(Failure::Limit)
        ));
        assert_eq!(read(&memory, base, 56), before);
    }
    setup(&mut memory, base, up, 3, base + 0x100);
    let mut driver = Driver::new(Default::default());
    let CommandBuilderAction::Grow(pending) = driver
        .begin_command_builder(&mut memory, FUNCTION, [base, 1, 0, 0, 0, 0], None)
        .unwrap()
    else {
        panic!("grow")
    };
    let mut other = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let mut foreign = other.pause().unwrap();
    assert!(matches!(
        driver.complete_command_builder(&mut foreign, pending, 1),
        Err(Failure::Memory)
    ));
    assert_eq!(read(&memory, base + 16, 8), up.to_le_bytes());
    // The cursor is actual, but a whole-segment zero crossing a hole refuses
    // before a first header. It cannot be replayed through the poisoned driver.
    let end = base + 0x7ffc;
    setup(&mut memory, base, end, 8, 0);
    memory.write(end, &[0xa5; 4]).unwrap();
    let mut driver = Driver::new(Default::default());
    assert!(matches!(
        driver.begin_command_builder(&mut memory, FUNCTION, [base, 0, 0, 0, 0, 0], None),
        Err(Failure::Memory)
    ));
    assert_eq!(read(&memory, base + 16, 8), (end + 20).to_le_bytes());
    assert_eq!(read(&memory, end, 4), [0xa5; 4]);
    assert!(matches!(
        driver.begin_command_builder(&mut memory, FUNCTION, [base, 0, 0, 0, 0, 0], None),
        Err(Failure::Poisoned)
    ));
}

#[test]
fn submitted_context_commands_retain_owned_saved_state_between_real_encoded_submissions() {
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
    let command = base + 0x1000;
    let buffer = base + 0x100;
    let up = base + 0x4000;
    let mut driver = Driver::new(Default::default());
    memory
        .write(
            command,
            &[
                0xc001_6900u32.to_le_bytes(),
                0x318u32.to_le_bytes(),
                0x456u32.to_le_bytes(),
            ]
            .concat(),
        )
        .unwrap();
    driver
        .submit_graphics(
            &mut memory,
            &[NativeRange {
                address: command,
                size: 12,
            }],
        )
        .unwrap();
    let initial = driver.graphics;
    setup(&mut memory, buffer, up, 64, 0);
    assert_eq!(
        returned(
            driver
                .begin_command_builder(&mut memory, FUNCTION, [buffer, 1, 0, 0, 0, 0], None)
                .unwrap()
        ),
        up
    );
    assert_eq!(driver.graphics, initial); // Emission has no immediate CP effects.
    driver
        .submit_graphics(
            &mut memory,
            &[NativeRange {
                address: up,
                size: 108,
            }],
        )
        .unwrap();
    let saved = driver.graphics;
    // Change + source DCBResetQueue(Clear) must not erase the saved context.
    memory
        .write(
            command,
            &[0xc001_6900u32, 0x318, 0x987, 0xc000_1200, 15]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    driver
        .submit_graphics(
            &mut memory,
            &[NativeRange {
                address: command,
                size: 20,
            }],
        )
        .unwrap();
    setup(&mut memory, buffer, up, 64, 0);
    assert_eq!(
        returned(
            driver
                .begin_command_builder(&mut memory, FUNCTION, [buffer, 2, 0, 0, 0, 0], None)
                .unwrap()
        ),
        up
    );
    driver
        .submit_graphics(
            &mut memory,
            &[NativeRange {
                address: up,
                size: 108,
            }],
        )
        .unwrap();
    assert_eq!(driver.graphics, initial);
    assert_ne!(saved, initial);
    let mut independent = Driver::new(Default::default());
    assert!(
        independent
            .submit_graphics(
                &mut memory,
                &[NativeRange {
                    address: up,
                    size: 108
                }]
            )
            .is_err()
    );
    assert_eq!(driver.graphics, initial);
    let mut other = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let mut foreign = other.pause().unwrap();
    assert!(matches!(
        driver.submit_graphics(&mut foreign, &[]),
        Err(Failure::Memory)
    ));
    assert_eq!((driver.usage().draws, driver.usage().dispatches), (0, 0));
}
