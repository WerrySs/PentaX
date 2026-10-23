// SPDX-License-Identifier: GPL-2.0-only
//! Original packet metadata values, fresh header reads and atomic boundaries.
use super::*;
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};
fn backing() -> (NativeAddressSpace, u64) {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x40000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(base, 0x4000, NativePermissions::READ_WRITE)
        .unwrap();
    (owner, base)
}
fn read<const N: usize>(memory: &PausedNativeMemory<'_>, address: u64) -> [u8; N] {
    let mut bytes = [0; N];
    memory.read(address, &mut bytes).unwrap();
    bytes
}
fn invoke(
    driver: &mut Driver,
    memory: &mut PausedNativeMemory<'_>,
    output: u64,
    command: u64,
    kind: u64,
) -> Result<u64, Failure> {
    driver.submit(
        memory,
        Function::AgcGetDataPacketPayloadAddress,
        [output, command, kind, 0, 0, 0],
    )
}
#[test]
fn packet_payload_type_width_and_literal_header_bits_return_only_plain_pointer_data() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let output = base + 0x103;
    let command = base + 0x300;
    let mut driver = Driver::new(Default::default());
    for (kind, command, expected) in [
        (1, 0, 8),
        (u64::MAX, base + 0x30000, base + 0x30008),
        (0x100000001, 1, 9),
        (0x80000000, u64::MAX - 8, u64::MAX),
    ] {
        memory.write(output - 2, &[0xa5; 12]).unwrap();
        assert_eq!(
            invoke(&mut driver, &mut memory, output, command, kind),
            Ok(0)
        );
        let actual = read::<12>(&memory, output - 2);
        assert_eq!(&actual[..2], &[0xa5; 2]);
        assert_eq!(&actual[2..10], &expected.to_le_bytes());
        assert_eq!(&actual[10..], &[0xa5; 2]);
    }
    for (header, expected) in [
        (0u32, command + 4),
        (0xffff, command + 4),
        (0xc0001024, command + 4),
        (0x3fff0000, 0),
        (u32::MAX, 0),
        (0xbfff1234, 0),
        (0x3ffe0000, command + 4),
    ] {
        memory.write(command, &header.to_le_bytes()).unwrap();
        assert_eq!(
            invoke(
                &mut driver,
                &mut memory,
                output,
                command,
                0xffff_ffff_0000_0000
            ),
            Ok(0)
        );
        assert_eq!(u64::from_le_bytes(read(&memory, output)), expected);
    }
    assert_eq!(driver.usage().command_buffers, 0);
    assert_eq!(driver.usage().dispatches, 0);
    assert_eq!(driver.usage().draws, 0);
    assert_eq!(driver.frame_state(), FrameState::default());
}
#[test]
fn packet_payload_reads_exactly_current_header_before_same_or_physical_alias_output() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    let edge = base + 0x3ffc;
    let output = base + 0x100;
    memory.write(edge, &0u32.to_le_bytes()).unwrap();
    assert_eq!(invoke(&mut driver, &mut memory, output, edge, 0), Ok(0));
    // Returned address is on the unmapped next page: it is data, not access.
    assert_eq!(u64::from_le_bytes(read(&memory, output)), base + 0x4000);
    memory.write(edge, &u32::MAX.to_le_bytes()).unwrap();
    assert_eq!(invoke(&mut driver, &mut memory, output, edge, 0), Ok(0));
    assert_eq!(read::<8>(&memory, output), [0; 8]);
    let alias = base + 0x200;
    memory.write(alias, &[0xff; 8]).unwrap();
    assert_eq!(invoke(&mut driver, &mut memory, alias, alias, 0), Ok(0));
    assert_eq!(read::<8>(&memory, alias), [0; 8]);
    assert_eq!(invoke(&mut driver, &mut memory, alias, alias, 0), Ok(0));
    assert_eq!(u64::from_le_bytes(read(&memory, alias)), alias + 4);
    memory
        .apply_transaction(
            NativeMemoryTransaction {
                effect: NativeMemoryEffect::AllocatePhysical {
                    physical: 0,
                    size: 0x10000,
                },
                writes: &[],
            },
            &Default::default(),
        )
        .unwrap();
    for address in [base + 0x10000, base + 0x20000] {
        memory
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::MapPhysical {
                        address,
                        size: 0x10000,
                        physical: 0,
                        permissions: NativePermissions::READ_WRITE,
                    },
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
    }
    memory.write(base + 0x10000, &[0xff; 8]).unwrap();
    assert_eq!(
        invoke(&mut driver, &mut memory, base + 0x10000, base + 0x20000, 0),
        Ok(0)
    );
    assert_eq!(read::<8>(&memory, base + 0x20000), [0; 8]);
    assert_eq!(
        invoke(&mut driver, &mut memory, base + 0x10000, base + 0x20000, 0),
        Ok(0)
    );
    assert_eq!(
        u64::from_le_bytes(read(&memory, base + 0x20000)),
        base + 0x20004
    );
}
#[test]
fn packet_payload_full_output_refusals_limits_and_poison_keep_header_and_canaries() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    let output = base + 0x100;
    memory.write(output, &[0xa5; 16]).unwrap();
    let before = read::<16>(&memory, output);
    for (out, command, kind) in [(0, u64::MAX, 1), (output, 0, 0), (output, u64::MAX - 4, 1)] {
        assert_eq!(
            invoke(&mut driver, &mut memory, out, command, kind),
            Err(Failure::Memory)
        );
        assert_eq!(read::<16>(&memory, output), before);
    }
    memory.write(base + 0x3ffc, &[0x96; 4]).unwrap();
    assert_eq!(
        invoke(&mut driver, &mut memory, base + 0x3ffc, 0, 1),
        Err(Failure::Memory)
    );
    assert_eq!(read::<4>(&memory, base + 0x3ffc), [0x96; 4]);
    for limits in [
        Limits {
            max_preparation_work: 63,
            ..Default::default()
        },
        Limits {
            max_memory_write_bytes: 7,
            ..Default::default()
        },
        Limits {
            max_calls: 0,
            ..Default::default()
        },
    ] {
        let mut limited = Driver::new(limits);
        assert_eq!(
            invoke(&mut limited, &mut memory, output, 0, 1),
            Err(Failure::Limit)
        );
        assert_eq!(read::<16>(&memory, output), before);
    }
    driver.poisoned = true;
    assert_eq!(
        invoke(&mut driver, &mut memory, output, 0, 1),
        Err(Failure::Poisoned)
    );
    assert_eq!(read::<16>(&memory, output), before);
    owner
        .protect(base, 0x4000, NativePermissions::READ)
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        invoke(&mut driver, &mut memory, output, 0, 1),
        Err(Failure::Memory)
    );
    assert_eq!(read::<16>(&memory, output), before);
}
