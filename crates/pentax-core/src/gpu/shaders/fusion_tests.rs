// SPDX-License-Identifier: GPL-2.0-only
use super::*;
use crate::gpu::driver::{Driver, Function, Limits};
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

const PAGE: u64 = 16384;
const FRONT: u64 = 0x100;
const BACK: u64 = 0x200;
const OUTPUT: u64 = 0x300;
const FRONT_REGS: u64 = 0x1000;
const BACK_REGS: u64 = 0x1800;
const SCRATCH: u64 = 0x2000;
const FRONT_CODE: u64 = 0x9999_ab12_3456_78ff;

fn owner() -> (NativeAddressSpace, u64) {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 4 * PAGE).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(base, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    (owner, base)
}
fn write64(memory: &mut PausedNativeMemory<'_>, address: u64, value: u64) {
    memory.write(address, &value.to_le_bytes()).unwrap();
}
fn read(memory: &PausedNativeMemory<'_>, address: u64, size: usize) -> Vec<u8> {
    let mut bytes = vec![0; size];
    memory.read(address, &mut bytes).unwrap();
    bytes
}
fn seed(memory: &mut PausedNativeMemory<'_>, base: u64, geometry: bool, count: u8) -> [u64; 6] {
    let mut front = [0x5a; 96];
    let mut back = [0xa5; 96];
    for (bytes, registers, specials, code, kind, user) in [
        (
            &mut front,
            FRONT_REGS,
            0x400,
            FRONT_CODE,
            if geometry { 4 } else { 5 },
            0x1122_3344_5566_7788_u64,
        ),
        (
            &mut back,
            BACK_REGS,
            0x420,
            0x5555_8899_aabb_ccdd,
            if geometry { 6 } else { 7 },
            0x8877_6655_4433_2211,
        ),
    ] {
        bytes[8..16].copy_from_slice(&user.to_le_bytes());
        bytes[16..24].copy_from_slice(&code.to_le_bytes());
        bytes[32..40].copy_from_slice(&(base + registers).to_le_bytes());
        bytes[40..48].copy_from_slice(&(base + specials).to_le_bytes());
        bytes[90] = kind;
        bytes[92] = count;
    }
    memory.write(base + FRONT, &front).unwrap();
    memory.write(base + BACK, &back).unwrap();
    memory.write(base + OUTPUT, &[0x33; 112]).unwrap();
    memory.write(base + SCRATCH - 8, &[0x66; 2064]).unwrap();
    for (registers, values) in [
        (
            FRONT_REGS,
            [
                0x1122_3344,
                0x5566_7788,
                0xa000_0013,
                0xf805_003e,
                0x999,
                0x888,
            ],
        ),
        (
            BACK_REGS,
            [
                0xdead_beef,
                0xfedc_ba98,
                0x5100_0007,
                0x1a02_0001,
                0x111,
                0xa5a5_a500,
            ],
        ),
    ] {
        // Place required registers at the end for the maximum-count case.
        for index in 0..usize::from(count) {
            let needed = index.checked_sub(usize::from(count.saturating_sub(6)));
            let checksum = if geometry { 0x80_u32 } else { 0x100 };
            let offsets = [
                checksum,
                checksum,
                checksum + 10,
                checksum + 11,
                if geometry { 0xc8 } else { 0x148 },
                if geometry { 0xc9 } else { 0x149 },
            ];
            let (offset, value): (u32, u32) = if let Some(i) = needed.filter(|i| *i < 6) {
                (offsets[i], values[i])
            } else {
                (0x777_u32, 0xcafe_babe)
            };
            let mut record = [0; 8];
            record[..4].copy_from_slice(&offset.to_le_bytes());
            record[4..].copy_from_slice(&value.to_le_bytes());
            memory
                .write(base + registers + index as u64 * 8, &record)
                .unwrap();
        }
    }
    write64(memory, base + 0x40c, 0);
    write64(memory, base + 0x42c, 0);
    [
        base + OUTPUT,
        base + FRONT,
        base + BACK,
        base + SCRATCH,
        0,
        0,
    ]
}

#[test]
fn fused_size_uses_conditional_byte_fields_and_exact_atomic_size_align_output() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let front = base + PAGE - 91;
    let back = base + 0x100;
    let output = base + 0x300;
    let mut driver = Driver::new(Default::default());
    for (front_kind, back_kind, count) in [(4, 6, 0), (5, 7, 255), (4, 7, 123)] {
        memory.write(front + 90, &[front_kind]).unwrap();
        memory.write(back + 90, &[back_kind, 0xee, count]).unwrap();
        memory.write(output - 1, &[0xa5; 18]).unwrap();
        let result = driver.submit(
            &mut memory,
            Function::AgcGetFusedShaderSize,
            [output, front, back, 0, 0, 0],
        );
        if front_kind == 4 && back_kind == 7 {
            assert_eq!(result, Ok(0x8a6c_0008));
            assert_eq!(read(&memory, output - 1, 18), [0xa5; 18]);
        } else {
            assert_eq!(result, Ok(0));
            let mut expected = vec![0xa5];
            expected.extend((u64::from(count) * 8).to_le_bytes());
            expected.extend(4_u64.to_le_bytes());
            expected.push(0xa5);
            assert_eq!(read(&memory, output - 1, 18), expected);
        }
    }
    // Invalid kinds do not touch an output or a missing count byte.
    memory.write(front + 90, &[3]).unwrap();
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::AgcGetFusedShaderSize,
            [u64::MAX, front, front, 0, 0, 0]
        ),
        Ok(0x8a6c_0008)
    );
    // Unlike fuse, size reads both kind bytes before checking the pair.
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::AgcGetFusedShaderSize,
            [output, front, base + PAGE, 0, 0, 0]
        ),
        Err(Failure::Memory)
    );
    // The destination may overwrite the input fields after their capture.
    memory.write(back + 90, &[5, 0, 255]).unwrap();
    memory.write(back + 128 + 90, &[7, 0, 13]).unwrap();
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::AgcGetFusedShaderSize,
            [back + 88, back, back + 128, 0, 0, 0]
        ),
        Ok(0)
    );
    assert_eq!(
        read(&memory, back + 88, 16),
        [104_u64.to_le_bytes(), 4_u64.to_le_bytes()].concat()
    );
}

#[test]
fn fused_halves_keep_source_gs_hs_register_profiles_scratch_and_header_bytes() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    for geometry in [false, true] {
        for scratch in [false, true] {
            for modern in [false, true] {
                let mut args = seed(&mut memory, base, geometry, 6);
                if !scratch {
                    args[3] = 0;
                }
                let before_front = read(&memory, base + FRONT, 96);
                let before_registers = read(&memory, base + BACK_REGS, 48);
                let mut expected_header = read(&memory, base + BACK, 96);
                expected_header[90] = if geometry { 2 } else { 3 };
                expected_header[8..16].copy_from_slice(
                    &if modern { 0_u64 } else { 0x1122_3344_5566_7788 }.to_le_bytes(),
                );
                let registers = base + if scratch { SCRATCH } else { BACK_REGS };
                expected_header[32..40].copy_from_slice(&registers.to_le_bytes());
                let mut driver = Driver::new(Default::default());
                let function = if modern {
                    Function::AgcFuseShaderHalves
                } else {
                    Function::AgcFuseShaderHalvesLegacy
                };
                assert_eq!(driver.submit(&mut memory, function, args), Ok(0));
                assert_eq!(read(&memory, base + OUTPUT, 96), expected_header);
                assert_eq!(read(&memory, base + FRONT, 96), before_front);
                let expected = [
                    0x1122_3344_u32,
                    0x5566_7788,
                    if geometry { 0x5100_0013 } else { 0x6100_0013 },
                    match (geometry, modern) {
                        (true, true) => 0x2a06_003f,
                        (true, false) => 0xfa06_003f,
                        (false, true) => 0x2a02_003f,
                        (false, false) => 0xfa02_003f,
                    },
                    0x1234_5678,
                    0xa5a5_a5ab,
                ];
                for (i, value) in expected.into_iter().enumerate() {
                    assert_eq!(
                        read(&memory, registers + i as u64 * 8 + 4, 4),
                        value.to_le_bytes()
                    );
                    assert_eq!(
                        read(&memory, registers + i as u64 * 8, 4),
                        before_registers[i * 8..i * 8 + 4]
                    );
                }
                if scratch {
                    assert_eq!(read(&memory, base + BACK_REGS, 48), before_registers);
                }
                assert_eq!(read(&memory, base + OUTPUT + 96, 16), [0x33; 16]);
                assert_eq!(read(&memory, base + SCRATCH - 8, 8), [0x66; 8]);
                assert_eq!(read(&memory, base + SCRATCH + 48, 8), [0x66; 8]);
                assert_eq!(driver.usage().dispatches, 0);
            }
        }
    }
}

#[test]
fn fusion_retains_guest_error_prefix_but_hard_refusals_publish_nothing() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let mut args = seed(&mut memory, base, true, 6);
    let mut driver = Driver::new(Default::default());
    memory.write(base + FRONT + 90, &[0]).unwrap();
    args[2] = base + PAGE;
    assert_eq!(
        driver.submit(&mut memory, Function::AgcFuseShaderHalves, args),
        Ok(0x8a6c_0008)
    );
    assert_eq!(read(&memory, base + OUTPUT, 96), [0x33; 96]);
    args = seed(&mut memory, base, true, 6);
    let mut expected = read(&memory, base + BACK, 96);
    expected[90] = 2;
    memory
        .write(base + 0x42c, &(1_u32 << 22).to_le_bytes())
        .unwrap();
    assert_eq!(
        driver.submit(&mut memory, Function::AgcFuseShaderHalves, args),
        Ok(0x8a6c_0008)
    );
    assert_eq!(read(&memory, base + OUTPUT, 96), expected);
    assert_eq!(read(&memory, base + SCRATCH, 48), [0x66; 48]);
    args = seed(&mut memory, base, true, 6);
    // Missing second checksum is a source-aborting path, not a successful fuse.
    memory
        .write(base + BACK_REGS + 8, &0x777_u32.to_le_bytes())
        .unwrap();
    let before = read(&memory, base, PAGE as usize);
    assert_eq!(
        driver.submit(&mut memory, Function::AgcFuseShaderHalves, args),
        Err(Failure::Shader(ShaderFailure::Registers))
    );
    assert_eq!(read(&memory, base, PAGE as usize), before);
    args = seed(&mut memory, base, true, 6);
    args[3] = base + PAGE - 47;
    let before = read(&memory, base, PAGE as usize);
    assert_eq!(
        driver.submit(&mut memory, Function::AgcFuseShaderHalves, args),
        Err(Failure::Memory)
    );
    assert_eq!(read(&memory, base, PAGE as usize), before);
    let mut limited = Driver::new(Limits {
        max_preparation_work: 0,
        ..Default::default()
    });
    assert_eq!(
        limited.submit(
            &mut memory,
            Function::AgcGetFusedShaderSize,
            [base + OUTPUT, base + FRONT, base + BACK, 0, 0, 0]
        ),
        Err(Failure::Limit)
    );
    assert_eq!(read(&memory, base, PAGE as usize), before);
    for function in [
        Function::AgcGetFusedShaderSize,
        Function::AgcFuseShaderHalves,
        Function::AgcFuseShaderHalvesLegacy,
    ] {
        assert_eq!(
            driver.submit(&mut memory, function, [0; 6]),
            Err(Failure::Shader(ShaderFailure::NullArgument))
        );
    }
}

#[test]
fn fusion_handles_maximum_register_count_and_in_place_back_header() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let mut args = seed(&mut memory, base, true, 255);
    args[0] = base + BACK;
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(&mut memory, Function::AgcFuseShaderHalves, args),
        Ok(0)
    );
    assert_eq!(read(&memory, base + BACK + 90, 3), [2, 0xa5, 255]);
    assert_eq!(read(&memory, base + BACK + 8, 8), [0; 8]);
    assert_eq!(
        read(&memory, base + BACK + 32, 8),
        (base + SCRATCH).to_le_bytes()
    );
    assert_eq!(
        read(&memory, base + SCRATCH + 249 * 8 + 4, 4),
        0x1122_3344_u32.to_le_bytes()
    );
    assert_eq!(read(&memory, base + SCRATCH + 255 * 8, 8), [0x66; 8]);
}

#[test]
fn fusion_shared_vgpr_rounding_and_optional_address_pair_follow_source_branches() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    for (front_vgprs, back_vgprs, front_shared, back_shared, expected) in [
        (19_u32, 7_u32, 0_u32, 0_u32, 0_u32),
        (0, 0, 7, 0, 0),
        (0, 0, 8, 0, 1),
        (0, 0, 15, 0, 1),
    ] {
        let args = seed(&mut memory, base, true, 6);
        for (address, value) in [
            (base + FRONT_REGS + 20, front_vgprs),
            (base + BACK_REGS + 20, back_vgprs),
            (base + FRONT_REGS + 28, front_shared << 28),
            (base + BACK_REGS + 28, back_shared << 28),
        ] {
            memory.write(address, &value.to_le_bytes()).unwrap();
        }
        let mut driver = Driver::new(Default::default());
        assert_eq!(
            driver.submit(&mut memory, Function::AgcFuseShaderHalves, args),
            Ok(0)
        );
        let bytes = read(&memory, base + SCRATCH + 28, 4);
        assert_eq!(
            u32::from_le_bytes(bytes.try_into().unwrap()) >> 28,
            expected
        );
    }
    for malformed in 0..3 {
        let args = seed(&mut memory, base, true, 6);
        if malformed == 2 {
            memory.write(base + BACK + 92, &[5]).unwrap();
        } else {
            memory
                .write(
                    base + BACK_REGS + 32 + malformed * 8,
                    &0x777_u32.to_le_bytes(),
                )
                .unwrap();
        }
        let mut driver = Driver::new(Default::default());
        assert_eq!(
            driver.submit(&mut memory, Function::AgcFuseShaderHalves, args),
            Ok(0)
        );
        assert_eq!(
            read(&memory, base + SCRATCH + 36, 4),
            0x111_u32.to_le_bytes()
        );
    }
    // An output fault after the first staged word must publish neither word.
    memory.write(base + FRONT + 90, &[4]).unwrap();
    memory.write(base + BACK + 90, &[6, 0, 3]).unwrap();
    let destination = base + PAGE - 15;
    memory.write(destination, &[0x77; 15]).unwrap();
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::AgcGetFusedShaderSize,
            [destination, base + FRONT, base + BACK, 0, 0, 0]
        ),
        Err(Failure::Memory)
    );
    assert_eq!(read(&memory, destination, 15), [0x77; 15]);
}
