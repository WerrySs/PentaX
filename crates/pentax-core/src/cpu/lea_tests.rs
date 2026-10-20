// SPDX-License-Identifier: GPL-2.0-or-later

//! Original instruction encodings; no game instruction streams are retained.

use super::*;
use crate::memory::{PAGE_SIZE, Permissions};

const CODE: u64 = 0x4000;

fn fixture(code: &[u8]) -> (GuestMemory, ReferenceCpu) {
    let mut memory = GuestMemory::new(PAGE_SIZE, PAGE_SIZE);
    memory
        .map(CODE, PAGE_SIZE, Permissions::READ_WRITE)
        .unwrap();
    memory.write(CODE, code).unwrap();
    memory
        .protect(CODE, PAGE_SIZE, Permissions::READ_EXECUTE)
        .unwrap();
    (memory, ReferenceCpu::at(CODE))
}

#[test]
fn rip_relative_result_does_not_read_unmapped_or_noncanonical_memory() {
    let (mut memory, mut cpu) = fixture(&[0x48, 0x8d, 0x05, 0, 0x80, 0, 0]);
    let resident = memory.resident_bytes();
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[0], CODE + 7 + 0x8000);
    assert_eq!(memory.resident_bytes(), resident);

    let (mut memory, mut cpu) = fixture(&[0x48, 0x8d, 0x00]); // RAX = address [RAX].
    cpu.registers[0] = ADDRESS_LIMIT + 123;
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[0], ADDRESS_LIMIT + 123);
}

#[test]
fn address_arithmetic_wraps_and_32_bit_destination_clears_upper_bits() {
    let (mut memory, mut cpu) = fixture(&[0x45, 0x8d, 0x4d, 0x20]); // R9D <- [R13+32].
    cpu.registers[13] = u64::MAX - 15;
    cpu.registers[9] = u64::MAX;
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[9], 16);
    assert_eq!(cpu.registers[13], u64::MAX - 15);

    let (mut memory, mut cpu) = fixture(&[0x48, 0x8d, 0x40, 0xf8]); // RAX <- [RAX-8].
    cpu.registers[0] = 3;
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[0], u64::MAX - 4);
}

#[test]
fn sib_extends_base_index_destination_and_scales_before_displacement() {
    let (mut memory, mut cpu) = fixture(&[0x4f, 0x8d, 0x54, 0x8c, 0xf8]);
    cpu.registers[12] = 0x1000;
    cpu.registers[9] = 7;
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[10], 0x1014);

    let (mut memory, mut cpu) = fixture(&[0x4a, 0x8d, 0x04, 0x60]); // [RAX+R12*2].
    cpu.registers[0] = 10;
    cpu.registers[12] = 9;
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[0], 28);
}

#[test]
fn sib_no_index_and_displacement_only_forms_preserve_encoding_rules() {
    let (mut memory, mut cpu) = fixture(&[0x48, 0x8d, 0x4c, 0x24, 0x10]);
    cpu.registers[4] = 0x1234;
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[1], 0x1244);
    assert_eq!(cpu.registers[4], 0x1234);

    let (mut memory, mut cpu) = fixture(&[0x48, 0x8d, 0x04, 0x25, 0xf0, 0xff, 0xff, 0xff]);
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[0], u64::MAX - 15);
}

#[test]
fn lea_preserves_condition_flags() {
    let (mut memory, mut cpu) = fixture(&[0x31, 0xc0, 0x48, 0x8d, 0x40, 1]);
    cpu.step(&mut memory).unwrap();
    assert!(cpu.zero);
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[0], 1);
    assert!(cpu.zero);
    assert_eq!(cpu.retired, 2);
}

#[test]
fn unsupported_register_and_prefix_forms_preserve_state() {
    for code in [
        &[0x48, 0x8d, 0xc0][..],
        &[0x64, 0x48, 0x8d, 0x00],
        &[0x67, 0x48, 0x8d, 0x00],
        &[0x49, 0x8d, 0x05, 0, 0, 0, 0],
        &[0x49, 0x8d, 0x04, 0x25, 0, 0, 0, 0],
    ] {
        let (mut memory, mut cpu) = fixture(code);
        let before = cpu.clone();
        assert!(matches!(
            cpu.step(&mut memory),
            Err(CpuError::UnsupportedInstruction { .. })
        ));
        assert_eq!(cpu, before);
    }
}

#[test]
fn truncated_address_fields_preserve_state() {
    let mut memory = GuestMemory::new(PAGE_SIZE, PAGE_SIZE);
    memory
        .map(CODE, PAGE_SIZE, Permissions::READ_WRITE)
        .unwrap();
    let instruction = CODE + PAGE_SIZE - 4;
    memory
        .write(instruction, &[0x48, 0x8d, 0x84, 0x24])
        .unwrap();
    memory
        .protect(CODE, PAGE_SIZE, Permissions::READ_EXECUTE)
        .unwrap();
    let mut cpu = ReferenceCpu::at(instruction);
    cpu.registers[4] = 123;
    let before = cpu.clone();
    assert!(matches!(cpu.step(&mut memory), Err(CpuError::Memory(_))));
    assert_eq!(cpu, before);
}
