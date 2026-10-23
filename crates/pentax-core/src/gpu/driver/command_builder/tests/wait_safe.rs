// SPDX-License-Identifier: GPL-2.0-only
//! Original exact producer/grow consumers; producing a wait never waits.
use super::*;

#[test]
fn wait_safe_producer_preserves_seven_words_across_grow_and_keeps_fatal_null_boundaries() {
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
    let old = base + 0x4000;
    let moved = base + 0x5000;
    let function = Function::AgcDcbWaitUntilSafeForRendering;
    assert!(supports(function));
    assert_eq!(EXPORTS.len(), 52);
    for grow in [false, true] {
        let mut driver = Driver::new(Default::default());
        setup(
            &mut memory,
            base,
            old,
            if grow { 3 } else { 16 },
            if grow { base + 0x80 } else { 0 },
        );
        memory.write(old - 4, &[0x69; 36]).unwrap();
        memory.write(moved - 4, &[0x96; 36]).unwrap();
        let action = driver
            .begin_command_builder(
                &mut memory,
                function,
                [base, 0xffff_ffff_8123_4567, 0x1234_5678_ffff_fffe, 0, 0, 0],
                None,
            )
            .unwrap();
        let (destination, value) = if grow {
            let CommandBuilderAction::Grow(pending) = action else {
                panic!("real seven-word grow missing")
            };
            assert_eq!(pending.args(), [base, 10, 0x7123]);
            setup(&mut memory, base, moved, 16, 0);
            (
                moved,
                returned(
                    driver
                        .complete_command_builder(&mut memory, pending, 0x101)
                        .unwrap(),
                ),
            )
        } else {
            (old, returned(action))
        };
        assert_eq!(value, destination);
        let expected: Vec<_> = [0xc005_1018_u32, 0x8123_4567, 0xffff_fffe, 0, 0, 0, 0]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        assert_eq!(read(&memory, destination, 28), expected);
        let canary = if grow { 0x96 } else { 0x69 };
        assert_eq!(read(&memory, destination - 4, 4), [canary; 4]);
        assert_eq!(read(&memory, destination + 28, 4), [canary; 4]);
        assert_eq!(
            read(&memory, base + 16, 8),
            (destination + 28).to_le_bytes()
        );
        assert_eq!(driver.video_flip_observations(), Default::default());
        assert_eq!(driver.usage.draws, 0);
    }
    for null in [false, true] {
        let mut driver = Driver::new(Default::default());
        setup(&mut memory, base, old, 3, 0);
        let before = read(&memory, old, 28);
        assert!(matches!(
            driver.begin_command_builder(
                &mut memory,
                function,
                [if null { 0 } else { base }, 1, 0, 0, 0, 0],
                None
            ),
            Err(Failure::UnsupportedQueue)
        ));
        assert_eq!(read(&memory, old, 28), before);
    }
}
