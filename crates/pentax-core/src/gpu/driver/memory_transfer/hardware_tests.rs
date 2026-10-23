// SPDX-License-Identifier: GPL-2.0-only
//! Original actual Vulkan writeback followed by ordered current-data consumers.
use super::*;
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn read(memory: &PausedNativeMemory<'_>, at: u64, size: usize) -> Vec<u8> {
    let mut value = vec![0; size];
    memory.read(at, &mut value).unwrap();
    value
}

#[test]
#[ignore = "requires a local Vulkan device and an outer owning timeout gate"]
fn actual_compute_writeback_then_dma_copy_copydata_fill_and_wait_consume_real_shader_values() {
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
    let shader = base + 0x4000;
    let output = base + 0x8000;
    let copied = base + 0x9000;
    let item = base + 0xa000;
    let filled = base + 0xb000;
    // Original eight-thread seed+local-id shader, no host SPIR-V or fake result.
    memory
        .write(
            shader,
            &bytes(&[
                (0x25 << 25) | (1 << 17) | 4,
                (0x1a << 25) | (2 << 17) | 130,
                0xe070_1000,
                (128 << 24) | (1 << 8) | 2,
                0xbf81_0000,
            ]),
        )
        .unwrap();
    for (address, size) in [(output, 40), (copied, 40), (item, 16), (filled, 24)] {
        memory.write(address, &vec![0xa5; size]).unwrap();
    }
    let mut words = Vec::new();
    let mut set = |first: u32, values: &[u32]| {
        words.push(0xc000_7602 | (values.len() as u32) << 16);
        words.push(first);
        words.extend(values);
    };
    set(0x207, &[8, 1, 1]);
    set(0x20c, &[(shader >> 8) as u32, (shader >> 40) as u32]);
    set(0x212, &[0x2c_0000, 10]);
    set(0x228, &[0]);
    set(0x240, &[output as u32, (output >> 32) as u32, 32, 0, 17]);
    words.extend([
        0xc003_1502,
        1,
        1,
        1,
        0x8005,
        0xc005_5000,
        0,
        output as u32,
        (output >> 32) as u32,
        copied as u32,
        (copied >> 32) as u32,
        32,
        0xc004_4000,
        0x10101,
        (copied + 8) as u32,
        ((copied + 8) >> 32) as u32,
        item as u32,
        (item >> 32) as u32,
        0xc005_5000,
        0x4000_0000,
        0x0123_4567,
        u32::MAX,
        filled as u32,
        (filled >> 32) as u32,
        16,
        0xc005_3c00,
        0x13,
        item as u32,
        (item >> 32) as u32,
        19,
        u32::MAX,
        0,
    ]);
    memory.write(base + 0x1000, &bytes(&words)).unwrap();
    memory
        .write(base + 0x100, &(base + 0x1000).to_le_bytes())
        .unwrap();
    memory
        .write(base + 0x108, &(words.len() as u32).to_le_bytes())
        .unwrap();
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(&mut memory, Function::Acb, [0x20, base + 0x100, 0, 0, 0, 0]),
        Ok(0)
    );
    let expected = bytes(&[17, 18, 19, 20, 21, 22, 23, 24]);
    assert_eq!(read(&memory, output, 32), expected);
    assert_eq!(read(&memory, copied, 32), expected);
    assert_eq!(read(&memory, item, 8), bytes(&[19, 20]));
    assert_eq!(read(&memory, filled, 16), bytes(&[0x0123_4567; 4]));
    for (address, size) in [(output, 32), (copied, 32), (item, 8), (filled, 16)] {
        assert_eq!(read(&memory, address + size, 8), [0xa5; 8]);
    }
    assert_eq!(driver.usage().dispatches, 1);
    assert_eq!(driver.usage().memory_commands, 4);
    assert_eq!(driver.usage().written_bytes, 88);
    assert!(!driver.poisoned);
    println!(
        "agc_memory_transfer_gpu_available=true agc_memory_transfer_gpu_dispatches=1 agc_memory_transfer_gpu_copies=2 agc_memory_transfer_gpu_fills=1"
    );
}
