// SPDX-License-Identifier: GPL-2.0-only
//! Original paused-owner synchronization, live-array and failure-prefix cases.
use super::*;
fn descriptor(memory: &mut PausedNativeMemory<'_>, at: u64, command: u64, words: &[u32]) {
    memory
        .write(
            command,
            &words
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
        )
        .unwrap();
    memory.write(at, &command.to_le_bytes()).unwrap();
    memory
        .write(at + 8, &(words.len() as u32).to_le_bytes())
        .unwrap();
}
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
fn owner_write_sync_array_event_wait_retires_real_prefix_without_fake_signal() {
    let (mut owner, base) = source();
    let mut memory = owner.pause().unwrap();
    let array = base + 0x8000;
    memory.write(array, &[0xa5; 8]).unwrap();
    let words = [
        0xc004_3700,
        0x100,
        array as u32,
        (array >> 32) as u32,
        0x20c,
        17,
        0xc006_1050,
        0x8000_0000,
        0xffff_ffff,
        0,
        0xffff_ffff,
        0,
        0,
        0xffff_ffff,
        0xc003_6300,
        array as u32,
        (array >> 32) as u32,
        0x8000_0000,
        1,
        0xc000_4600,
        0x407,
        0xc005_3c00,
        0x13,
        (array + 4) as u32,
        ((array + 4) >> 32) as u32,
        17,
        u32::MAX,
        0,
    ];
    descriptor(&mut memory, base + 0x100, base + 0x1000, &words);
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(&mut memory, Function::Acb, [0x20, base + 0x100, 0, 0, 0, 0]),
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
            .all(|state| *state == compute::ComputeState::empty())
    );
    assert_eq!(
        (driver.usage().memory_commands, driver.usage().written_bytes),
        (4, 8)
    );
    assert_eq!(
        (
            driver.usage().dispatches,
            driver.usage().draws,
            driver.usage().interrupts
        ),
        (0, 0, 0)
    );
    assert_eq!(driver.frame_state(), Default::default());
    assert!(!driver.poisoned);
}

#[test]
fn unsupported_sync_or_work_admission_refuses_before_any_earlier_write() {
    let (mut owner, base) = source();
    let mut memory = owner.pause().unwrap();
    let label = base + 0x8000;
    for event in [0x39, 0x38, 0x0d, 0x716] {
        memory.write(label, &[0xa5; 4]).unwrap();
        let words = [
            0xc003_3700,
            0x100,
            label as u32,
            (label >> 32) as u32,
            17,
            0xc000_4600,
            event,
        ];
        descriptor(&mut memory, base + 0x100, base + 0x1000, &words);
        let mut driver = Driver::new(Default::default());
        assert!(matches!(
            driver.submit(&mut memory, Function::Acb, [0x20, base + 0x100, 0, 0, 0, 0]),
            Err(Failure::Preparation(_))
        ));
        assert_eq!(read(&memory, label, 4), [0xa5; 4]);
        assert_eq!(driver.usage().written_bytes, 0);
        assert_eq!(driver.usage().memory_commands, 0);
    }
    let words = [
        0xc003_3700,
        0x100,
        label as u32,
        (label >> 32) as u32,
        17,
        0xc000_4600,
        0x407,
    ];
    descriptor(&mut memory, base + 0x100, base + 0x1000, &words);
    let mut driver = Driver::new(Limits {
        max_preparation_work: 0,
        ..Default::default()
    });
    assert!(matches!(
        driver.submit(&mut memory, Function::Acb, [0x20, base + 0x100, 0, 0, 0, 0]),
        Err(Failure::Limit)
    ));
    assert_eq!(read(&memory, label, 4), [0xa5; 4]);
}

#[test]
fn dcb_stack_poll_and_wide_cache_coordinates_encode_without_payload_dereference() {
    let (mut owner, base) = source();
    let mut memory = owner.pause().unwrap();
    let up = base + 0x4000;
    let stack = base + 0x300;
    setup(&mut memory, base, up, 11, 0);
    memory
        .write(stack + 8, &0xffff_ffff_0000_00a0u64.to_le_bytes())
        .unwrap();
    let mut driver = Driver::new(Default::default());
    let args = [
        base,
        0xffff_ffff_0000_0103,
        0x063f_a55a,
        0xdead_beef,
        u64::MAX,
        u64::MAX,
    ];
    assert_eq!(
        returned(
            driver
                .begin_command_builder(&mut memory, Function::AgcDcbAcquireMem, args, Some(stack))
                .unwrap()
        ),
        up
    );
    let expected = [
        0xc006_1050u32,
        0x863f_a55a,
        0,
        0,
        0xffff_ffff,
        0,
        4,
        0xdead_beef,
    ];
    assert_eq!(
        read(&memory, up, 32),
        expected
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>()
    );
    assert_eq!(read(&memory, base + 16, 8), (up + 32).to_le_bytes());
    // A late payload fault preserves the real advanced cursor/header prefix.
    setup(&mut memory, base, base + 0xfffc, 11, 0);
    let mut driver = Driver::new(Default::default());
    assert!(matches!(
        driver.begin_command_builder(
            &mut memory,
            Function::AgcAcbAcquireMem,
            [base, 0, u64::MAX, u64::MAX, 40, 0],
            None
        ),
        Err(Failure::Memory)
    ));
    assert_eq!(read(&memory, base + 16, 8), (base + 0x1001c).to_le_bytes());
    assert_eq!(
        read(&memory, base + 0xfffc, 4),
        0xc006_1050u32.to_le_bytes()
    );
    assert!(driver.poisoned);
}
