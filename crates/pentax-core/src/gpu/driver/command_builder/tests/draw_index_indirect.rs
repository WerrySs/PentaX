// SPDX-License-Identifier: GPL-2.0-only
//! Original real producer -> submitted live argument/state/prefix consumers.
use super::*;
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn encode(
    driver: &mut Driver,
    memory: &mut PausedNativeMemory<'_>,
    base: u64,
    function: Function,
    arg: u64,
    modifier: u64,
) -> Vec<u8> {
    let count = if function == Function::AgcDcbDrawIndexIndirect {
        5
    } else {
        4
    };
    setup(memory, base, base + 0x4000, count + 3, 0);
    assert_eq!(
        returned(
            driver
                .begin_command_builder(memory, function, [base, arg, modifier, 0, 0, 0], None)
                .unwrap()
        ),
        base + 0x4000
    );
    read(memory, base + 0x4000, count as usize * 4)
}
#[test]
fn submitted_indirect_zero_keeps_actual_state_order_and_late_fault_keeps_only_real_prefix() {
    for denied in [false, true] {
        let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x20000).unwrap();
        let base = owner.reserved_ranges().next().unwrap().address;
        owner
            .map_at(
                base,
                0x10000,
                NativePermissions::READ_WRITE.with_gpu(true, true),
            )
            .unwrap();
        let mut memory = owner.pause().unwrap();
        let args = if denied { base + 0xfff0 } else { base + 0x8000 };
        let marker = base + 0x9000;
        memory.write(marker, &0u32.to_le_bytes()).unwrap();
        let mut driver = Driver::new(Default::default());
        let mut stream = bytes(&[
            0xc000_2a00,
            0,
            0xc001_2600,
            (base + 0xa000) as u32,
            ((base + 0xa000) >> 32) as u32,
            0xc000_1300,
            3,
            0xc000_2f00,
            99,
            0xc003_3700,
            0x100,
            marker as u32,
            (marker >> 32) as u32,
            17,
        ]);
        if !denied {
            stream.extend(bytes(&[
                0xc007_3700,
                0x100,
                args as u32,
                (args >> 32) as u32,
                3,
                0,
                1,
                (-3i32) as u32,
                5,
            ]));
        }
        stream.extend(encode(
            &mut driver,
            &mut memory,
            base,
            Function::AgcDcbSetBaseIndirectArgs,
            0,
            args,
        ));
        stream.extend(encode(
            &mut driver,
            &mut memory,
            base,
            Function::AgcDcbDrawIndexIndirect,
            0,
            7 | (1 << 19) | (2 << 14),
        ));
        stream.extend(bytes(&[
            0xc001_2d00,
            3,
            2,
            0xc003_3700,
            0x100,
            marker as u32,
            (marker >> 32) as u32,
            23,
        ]));
        memory.write(base + 0x1000, &stream).unwrap();
        memory
            .write(base + 0x100, &(base + 0x1000).to_le_bytes())
            .unwrap();
        memory
            .write(base + 0x108, &(stream.len() as u32 / 4).to_le_bytes())
            .unwrap();
        memory.write(base + 0x204, &0u32.to_le_bytes()).unwrap();
        let result = driver.submit(
            &mut memory,
            Function::CommandBuffer,
            [base + 0x200, base + 0x100, 0, 0, 0, 0],
        );
        if denied {
            assert!(matches!(
                result,
                Err(Failure::GraphicsPreparation(draw::Error::Memory(_)))
            ));
            assert_eq!(read(&memory, marker, 4), 17u32.to_le_bytes());
            assert!(driver.poisoned);
            assert_eq!(
                driver.submit(
                    &mut memory,
                    Function::CommandBuffer,
                    [base + 0x200, base + 0x100, 0, 0, 0, 0]
                ),
                Err(Failure::Poisoned)
            );
        } else {
            assert_eq!(result, Ok(0));
            assert_eq!(read(&memory, marker, 4), 23u32.to_le_bytes());
            assert_eq!(
                read(&memory, args, 20),
                bytes(&[3, 0, 1, (-3i32) as u32, 5])
            );
            assert!(!driver.poisoned);
        }
        // Stale NUM_INSTANCES99 must not survive the actual zero-instance
        // indirect parameters and provoke the later AUTO into a GPU operation.
        assert_eq!(driver.usage().draws, 0);
    }
}
