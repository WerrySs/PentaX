// SPDX-License-Identifier: GPL-2.0-only
//! Original versioned index-size packets and real allocator continuations.
use super::*;
const FUNCTION: Function = Function::AgcDcbSetIndexSize0600;
#[test]
fn index_size_0600_masks_exact_low_arguments_and_preserves_old_builder_and_no_draw_state() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(base, 0x8000, NativePermissions::READ_WRITE)
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let up = base + 0x4000;
    let mut driver = Driver::new(Default::default());
    let initial = driver.graphics;
    for kind in 0..4u64 {
        for policy in 0..4u64 {
            for flag in 0..2u64 {
                setup(&mut memory, base, up, 6, 0);
                memory.write(up, &[0xa5; 20]).unwrap();
                let args = [
                    base,
                    0xffff_ffff_0000_0000 | kind,
                    0xffff_ffff_0000_0000 | policy,
                    0xffff_ffff_0000_0002 | flag,
                    0,
                    0,
                ];
                assert_eq!(
                    returned(
                        driver
                            .begin_command_builder(&mut memory, FUNCTION, args, None)
                            .unwrap()
                    ),
                    up
                );
                let expected = [
                    0xc0017a00u32,
                    0x20000243,
                    0x400 | kind as u32 | ((policy as u32) << 6) | ((flag as u32) << 14),
                ];
                assert_eq!(
                    read(&memory, up, 12),
                    expected
                        .into_iter()
                        .flat_map(u32::to_le_bytes)
                        .collect::<Vec<_>>()
                );
                assert_eq!(read(&memory, up + 12, 8), [0xa5; 8]);
                assert_eq!(read(&memory, base + 16, 8), (up + 12).to_le_bytes());
            }
        }
    }
    setup(&mut memory, base, up, 6, 0);
    assert_eq!(
        returned(
            driver
                .begin_command_builder(
                    &mut memory,
                    Function::AgcDcbSetIndexSize,
                    [base, 1, 2, u64::MAX, 0, 0],
                    None
                )
                .unwrap()
        ),
        up
    );
    assert_eq!(
        read(&memory, up, 12),
        [0xc0017a00u32, 0x20000243, 0x481]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        returned(
            driver
                .begin_command_builder(&mut memory, FUNCTION, [0; 6], None)
                .unwrap()
        ),
        0
    );
    assert_eq!(driver.graphics, initial);
    assert_eq!(
        (
            driver.usage().draws,
            driver.usage().dispatches,
            driver.usage().command_buffers
        ),
        (0, 0, 0)
    );
}
#[test]
fn index_size_0600_grow_al_current_descriptor_and_output_failure_keep_real_prefixes() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(base, 0x8000, NativePermissions::READ_WRITE)
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let first = base + 0x4000;
    let moved = base + 0x5000;
    setup(&mut memory, base, first, 3, base + 0x100);
    let mut driver = Driver::new(Default::default());
    let CommandBuilderAction::Grow(pending) = driver
        .begin_command_builder(&mut memory, FUNCTION, [base, 1, 3, 1, 0, 0], None)
        .unwrap()
    else {
        panic!("actual grow")
    };
    assert_eq!(pending.args(), [base, 6, 0x7123]);
    setup(&mut memory, base, moved, 6, 0);
    assert_eq!(
        returned(
            driver
                .complete_command_builder(&mut memory, pending, 0x101)
                .unwrap()
        ),
        moved
    );
    assert_eq!(
        read(&memory, moved, 12),
        [0xc0017a00u32, 0x20000243, 0x44c1]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>()
    );
    for value in [0, 0x100, 1] {
        setup(&mut memory, base, first, 3, base + 0x100);
        memory.write(first, &[0x96; 12]).unwrap();
        let mut driver = Driver::new(Default::default());
        let CommandBuilderAction::Grow(pending) = driver
            .begin_command_builder(&mut memory, FUNCTION, [base, 1, 0, 1, 0, 0], None)
            .unwrap()
        else {
            panic!("grow")
        };
        setup(&mut memory, base, first, 5, 0);
        assert_eq!(
            returned(
                driver
                    .complete_command_builder(&mut memory, pending, value)
                    .unwrap()
            ),
            0
        );
        assert_eq!(read(&memory, first, 12), [0x96; 12]);
        assert_eq!(read(&memory, base + 16, 8), first.to_le_bytes());
    }
    setup(&mut memory, base, first, 3, 0);
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        returned(
            driver
                .begin_command_builder(&mut memory, FUNCTION, [base, 0, 0, 0, 0, 0], None)
                .unwrap()
        ),
        0
    );
    setup(&mut memory, base, 0, 6, 0);
    assert_eq!(
        returned(
            driver
                .begin_command_builder(&mut memory, FUNCTION, [base, 0, 0, 0, 0, 0], None)
                .unwrap()
        ),
        0
    );
    assert_eq!(read(&memory, base + 16, 8), 12u64.to_le_bytes());
    setup(&mut memory, base, base + 0xc000, 6, 0);
    let mut driver = Driver::new(Default::default());
    assert!(matches!(
        driver.begin_command_builder(&mut memory, FUNCTION, [base, 0, 0, 1, 0, 0], None),
        Err(Failure::Memory)
    ));
    assert_eq!(read(&memory, base + 16, 8), (base + 0xc00c).to_le_bytes());
    assert!(driver.poisoned);
    assert!(matches!(
        driver.begin_command_builder(&mut memory, FUNCTION, [0; 6], None),
        Err(Failure::Poisoned)
    ));
}
