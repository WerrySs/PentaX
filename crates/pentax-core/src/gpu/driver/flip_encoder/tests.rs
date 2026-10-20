// SPDX-License-Identifier: GPL-2.0-only
//! Original source CommandBuffer fit/growth/fault transactions.
use super::*;
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

fn setup(memory: &mut PausedNativeMemory<'_>, buf: u64, target: u64, available: u64) {
    let mut header = [0xa5; 56];
    header[16..24].copy_from_slice(&target.to_le_bytes());
    header[24..32].copy_from_slice(&(target + available * 4).to_le_bytes());
    header[32..40].copy_from_slice(&u64::MAX.to_le_bytes()); // Opaque callback must not run on fit.
    header[48..52].copy_from_slice(&2u32.to_le_bytes());
    memory.write(buf, &header).unwrap();
}
fn read(memory: &PausedNativeMemory<'_>, address: u64, len: usize) -> Vec<u8> {
    let mut bytes = vec![0; len];
    memory.read(address, &mut bytes).unwrap();
    bytes
}
#[test]
fn flip_encoder_writes_exact_six_words_and_advances_original_cursor_once_without_callback() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(base, 0x8000, NativePermissions::READ_WRITE)
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let target = base + 0x4000;
    setup(&mut memory, base, target, 8);
    memory.write(target - 1, &[0x9b; 26]).unwrap();
    let before = read(&memory, base, 56);
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::AgcDcbSetFlip,
            [
                base,
                0x1234_5678_0000_0001,
                0xffff_ffff,
                2,
                0xfedc_ba98_7654_3210,
                u64::MAX
            ]
        ),
        Ok(target)
    );
    let encoded = read(&memory, target, 24);
    assert_eq!(
        encoded
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect::<Vec<_>>(),
        [0xc004_105c, 1, u32::MAX, 2, 0x7654_3210, 0xfedc_ba98]
    );
    let mut expected = before;
    expected[16..24].copy_from_slice(&(target + 24).to_le_bytes());
    assert_eq!(read(&memory, base, 56), expected);
    assert_eq!(read(&memory, target - 1, 1), [0x9b]);
    assert_eq!(read(&memory, target + 24, 1), [0x9b]);
    assert_eq!(driver.usage.memory_written_bytes, 32);
    setup(&mut memory, base, target, 7);
    let before = read(&memory, base, 56);
    let packet = read(&memory, target, 24);
    assert!(
        driver
            .submit(&mut memory, Function::AgcDcbSetFlip, [base, 1, 0, 1, 0, 0])
            .is_err()
    );
    assert_eq!(read(&memory, base, 56), before);
    assert_eq!(read(&memory, target, 24), packet);
    assert_eq!(driver.usage.memory_written_bytes, 32);
}
#[test]
fn flip_encoder_late_permissions_alias_and_byte_quota_refuse_both_outputs_atomically() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(base, 0x8000, NativePermissions::READ_WRITE)
        .unwrap();
    {
        let mut memory = owner.pause().unwrap();
        setup(&mut memory, base, base + 0x4000, 8);
    }
    owner
        .protect(base + 0x4000, 0x4000, NativePermissions::READ)
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let before = read(&memory, base, 56);
    let mut driver = Driver::new(Default::default());
    assert!(
        driver
            .submit(&mut memory, Function::AgcDcbSetFlip, [base, 1, 0, 1, 0, 0])
            .is_err()
    );
    assert_eq!(read(&memory, base, 56), before);
    assert_eq!(driver.usage.memory_written_bytes, 0);
    setup(&mut memory, base, base + 16, 8);
    let before = read(&memory, base, 56);
    assert!(
        driver
            .submit(&mut memory, Function::AgcDcbSetFlip, [base, 1, 0, 1, 0, 0])
            .is_err()
    );
    assert_eq!(read(&memory, base, 56), before);
    let mut limited = Driver::new(super::super::Limits {
        max_memory_write_bytes: 31,
        ..Default::default()
    });
    setup(&mut memory, base, base + 0x100, 8);
    let before = read(&memory, base, 56);
    assert_eq!(
        limited.submit(&mut memory, Function::AgcDcbSetFlip, [base, 1, 0, 1, 0, 0]),
        Err(Failure::Limit)
    );
    assert_eq!(read(&memory, base, 56), before);
}
