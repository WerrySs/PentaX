// SPDX-License-Identifier: GPL-2.0-only
//! Original five-word producer consumers; encoding never publishes statistics.
use super::*;

#[test]
fn lod_stats_captures_both_stack_scalars_before_real_grow_and_publishes_literal_words() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x30000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x10000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let stack = base + 0x800;
    for grow in [false, true] {
        let mut driver = Driver::new(Default::default());
        let destination = base + if grow { 0x5000 } else { 0x4000 };
        setup(
            &mut memory,
            base,
            base + 0x4000,
            if grow { 1 } else { 16 },
            if grow { base + 0x80 } else { 0 },
        );
        memory.write(destination - 4, &[0x69; 28]).unwrap();
        memory
            .write(stack + 8, &0xffff_ffff_ffff_ff03u64.to_le_bytes())
            .unwrap();
        memory
            .write(stack + 16, &0x8765_4321_1234_5678u64.to_le_bytes())
            .unwrap();
        let action = driver
            .begin_command_builder(
                &mut memory,
                Function::AgcDcbGetLodStats,
                [
                    base,
                    0xffff_ffff_ffff_ff02,
                    0x1234_5678_9abc_def7,
                    0x1234_5678_0000_0037,
                    0xffff_ffff_1234_1234,
                    0xffff_ffff_ffff_ff02,
                ],
                Some(stack),
            )
            .unwrap();
        let result = if grow {
            let CommandBuilderAction::Grow(pending) = action else {
                panic!("real five-word grow missing")
            };
            assert_eq!(pending.args(), [base, 8, 0x7123]);
            // A callback may change the caller's stack after its scalar snapshot.
            memory.write(stack + 8, &[0; 16]).unwrap();
            setup(&mut memory, base, destination, 16, 0);
            returned(
                driver
                    .complete_command_builder(&mut memory, pending, 0x101)
                    .unwrap(),
            )
        } else {
            returned(action)
        };
        assert_eq!(result, destination);
        let expected: Vec<_> = [0xc003_8e00u32, 0x37, 0x9abc_dec0, 0x1234_5678, 0x2008_d1e0]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        assert_eq!(read(&memory, destination, 20), expected);
        assert_eq!(read(&memory, destination - 4, 4), [0x69; 4]);
        assert_eq!(read(&memory, destination + 20, 4), [0x69; 4]);
        assert_eq!(
            read(&memory, base + 16, 8),
            (destination + 20).to_le_bytes()
        );
        assert_eq!(driver.video_flip_observations(), Default::default());
        assert_eq!(driver.usage.draws, 0);
    }
}

#[test]
fn lod_stats_null_allocation_and_guarded_stack_keep_original_return_and_failure_order() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x30000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x10000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let function = Function::AgcDcbGetLodStats;
    let stack = base + 0x800;
    memory.write(stack + 8, &[0xff; 16]).unwrap();
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        returned(
            driver
                .begin_command_builder(&mut memory, function, [0; 6], None)
                .unwrap()
        ),
        0
    );
    setup(&mut memory, base, base + 0x4000, 1, 0);
    memory.write(base + 0x4000, &[0x96; 24]).unwrap();
    assert_eq!(
        returned(
            driver
                .begin_command_builder(&mut memory, function, [base, 0, 0, 0, 0, 0], Some(stack))
                .unwrap()
        ),
        0
    );
    assert_eq!(read(&memory, base + 0x4000, 24), [0x96; 24]);
    // A null allocated cursor still advances by exactly five words, then returns0.
    setup(&mut memory, base, 0, 16, 0);
    assert_eq!(
        returned(
            driver
                .begin_command_builder(&mut memory, function, [base, 0, 0, 0, 0, 0], Some(stack))
                .unwrap()
        ),
        0
    );
    assert_eq!(read(&memory, base + 16, 8), 20u64.to_le_bytes());
    for stack in [None, Some(base + 0xfff8), Some(base + 0xfff0)] {
        let mut driver = Driver::new(Default::default());
        setup(&mut memory, base, base + 0x4000, 16, base + 0x80);
        let before = read(&memory, base, 56);
        assert!(
            driver
                .begin_command_builder(&mut memory, function, [base, 0, 0, 0, 0, 0], stack)
                .is_err()
        );
        assert_eq!(read(&memory, base, 56), before);
        assert_eq!(read(&memory, base + 0x4000, 24), [0x96; 24]);
    }
}
