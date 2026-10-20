// SPDX-License-Identifier: GPL-2.0-or-later

//! Original interpreter byte sequences and guest-memory fixtures only.

use super::*;
use crate::memory::{PAGE_SIZE, Permissions};

const CODE: u64 = 0x4000;
const DATA: u64 = 0x8000;
const STACK: u64 = 0xc000;

fn fixture(code: &[u8]) -> (GuestMemory, ReferenceCpu) {
    let mut memory = GuestMemory::new(4 * PAGE_SIZE, 4 * PAGE_SIZE);
    for address in [CODE, DATA, STACK] {
        memory
            .map(address, PAGE_SIZE, Permissions::READ_WRITE)
            .unwrap();
    }
    memory.write(CODE, code).unwrap();
    memory
        .protect(CODE, PAGE_SIZE, Permissions::READ_EXECUTE)
        .unwrap();
    let mut cpu = ReferenceCpu::at(CODE);
    cpu.registers[4] = STACK + PAGE_SIZE;
    (memory, cpu)
}

fn word(memory: &GuestMemory, address: u64) -> u64 {
    let mut value = [0; 8];
    memory.read(address, &mut value).unwrap();
    u64::from_le_bytes(value)
}

fn install(memory: &mut GuestMemory, address: u64, bytes: &[u8]) {
    let page = crate::memory::page_base(address);
    memory
        .protect(page, PAGE_SIZE, Permissions::READ_WRITE)
        .unwrap();
    memory.write(address, bytes).unwrap();
    memory
        .protect(page, PAGE_SIZE, Permissions::READ_EXECUTE)
        .unwrap();
}

#[test]
fn memory_mov_load_store_widths_zero_extension_and_flags() {
    for wide in [false, true] {
        let mut load = vec![];
        if wide {
            load.push(0x48);
        }
        load.extend([0x8b, 0x03]);
        let (mut memory, mut cpu) = fixture(&load);
        cpu.registers[3] = DATA + 32;
        cpu.registers[0] = u64::MAX;
        cpu.zero = true;
        let value = 0x8877_6655_4433_2211_u64;
        memory.write(DATA + 32, &value.to_le_bytes()).unwrap();
        cpu.step(&mut memory).unwrap();
        assert_eq!(cpu.registers[0], if wide { value } else { 0x4433_2211 });
        assert!(cpu.zero);
        assert_eq!(cpu.retired, 1);
        assert_eq!(cpu.rip, CODE + load.len() as u64);

        let mut store = vec![];
        if wide {
            store.push(0x48);
        }
        store.extend([0x89, 0x03]);
        install(&mut memory, CODE, &store);
        cpu.rip = CODE;
        cpu.registers[0] = value;
        memory.write(DATA + 32, &[0xaa; 8]).unwrap();
        cpu.step(&mut memory).unwrap();
        assert_eq!(
            word(&memory, DATA + 32),
            if wide { value } else { 0xaaaa_aaaa_4433_2211 }
        );
        assert_eq!(cpu.registers[0], value);
        assert!(cpu.zero);
    }
}

#[test]
fn extended_register_base_destination_and_both_register_mov_directions() {
    // MOV R9,[R13+8]; MOV [R9+disp32],R10.
    let (mut memory, mut cpu) = fixture(&[0x4d, 0x8b, 0x4d, 8]);
    cpu.registers[13] = DATA;
    memory
        .write(DATA + 8, &(DATA + 0x100).to_le_bytes())
        .unwrap();
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[9], DATA + 0x100);
    install(
        &mut memory,
        CODE,
        &[0x4d, 0x89, 0x91, 0x00, 0xff, 0xff, 0xff],
    );
    cpu.rip = CODE;
    cpu.registers[10] = 0x0123_4567_89ab_cdef;
    cpu.step(&mut memory).unwrap();
    assert_eq!(word(&memory, DATA), cpu.registers[10]);
    for (code, wide) in [
        (&[0x45, 0x8b, 0xc1][..], false),
        (&[0x4d, 0x89, 0xc8][..], true),
    ] {
        install(&mut memory, CODE, code);
        cpu.rip = CODE;
        cpu.registers[8] = u64::MAX;
        cpu.registers[9] = 0x1234_5678_90ab_cdef;
        cpu.step(&mut memory).unwrap();
        assert_eq!(
            cpu.registers[8],
            if wide { cpu.registers[9] } else { 0x90ab_cdef }
        );
    }
}

#[test]
fn base_displacements_are_signed_at_both_widths() {
    for (mode, displacement) in [(1_u8, -128_i32), (1, 127), (2, -0x180), (2, 0x180)] {
        let mut code = vec![0x48, 0x8b, mode << 6 | 3];
        if mode == 1 {
            code.push(displacement as i8 as u8);
        } else {
            code.extend(displacement.to_le_bytes());
        }
        let (mut memory, mut cpu) = fixture(&code);
        cpu.registers[3] = DATA + 0x200;
        let address = (DATA + 0x200)
            .checked_add_signed(i64::from(displacement))
            .unwrap();
        memory
            .write(address, &0xfedc_ba98_7654_3210_u64.to_le_bytes())
            .unwrap();
        cpu.step(&mut memory).unwrap();
        assert_eq!(cpu.registers[0], 0xfedc_ba98_7654_3210);
    }
    // RBP requires a displacement, including an explicit zero.
    let (mut memory, mut cpu) = fixture(&[0x8b, 0x45, 0]);
    cpu.registers[5] = DATA;
    memory.write(DATA, &17_u32.to_le_bytes()).unwrap();
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[0], 17);
}

#[test]
fn rip_relative_uses_full_instruction_end_including_fs_and_rex() {
    for fs in [false, true] {
        for negative in [false, true] {
            for opcode in [0x8b, 0x89] {
                let (mut memory, mut cpu) = fixture(&[0xcc]);
                let rip = if negative { STACK + 0x100 } else { CODE };
                let mut code = vec![];
                if fs {
                    code.push(0x64);
                }
                code.extend([0x48, opcode, 0x05]);
                let end = rip + code.len() as u64 + 4;
                cpu.fs_base = if fs { 0x100 } else { 0 };
                let displacement =
                    i32::try_from(DATA as i64 - cpu.fs_base as i64 - end as i64).unwrap();
                code.extend(displacement.to_le_bytes());
                install(&mut memory, rip, &code);
                cpu.rip = rip;
                memory
                    .write(DATA, &0x1234_5678_90ab_cdef_u64.to_le_bytes())
                    .unwrap();
                cpu.registers[0] = 0x8877_6655_4433_2211;
                cpu.step(&mut memory).unwrap();
                assert_eq!(cpu.rip, end);
                if opcode == 0x8b {
                    assert_eq!(cpu.registers[0], 0x1234_5678_90ab_cdef);
                } else {
                    assert_eq!(word(&memory, DATA), 0x8877_6655_4433_2211);
                }
            }
        }
    }
}

#[test]
fn fs_is_guest_state_and_supports_fixture_reads_counter_store_and_negative_displacement() {
    let (mut memory, mut cpu) = fixture(&[0x64, 0x8b, 0x03]);
    assert_eq!(cpu.fs_base, 0);
    cpu.fs_base = DATA;
    cpu.registers[3] = 0;
    memory.write(DATA, &37_u64.to_le_bytes()).unwrap();
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[0], 37);
    install(&mut memory, CODE, &[0x64, 0x48, 0x8b, 0x43, 8]);
    cpu.rip = CODE;
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[0], 0);
    install(&mut memory, CODE, &[0x64, 0x48, 0x89, 0x43, 8]);
    cpu.rip = CODE;
    cpu.registers[0] = 1;
    cpu.step(&mut memory).unwrap();
    assert_eq!(word(&memory, DATA + 8), 1);
    install(&mut memory, CODE, &[0x64, 0x48, 0x8b, 0x43, 0xf8]);
    cpu.rip = CODE;
    cpu.fs_base = DATA + 8;
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[0], 37);
}

#[test]
fn effective_address_overflow_underflow_fs_and_guest_bounds_are_transactional() {
    for (fs, base, displacement) in [
        (0, u64::MAX, 8),
        (0, 0, -8),
        (u64::MAX, 1, 0),
        (ADDRESS_LIMIT, 0, 0),
    ] {
        let code = [0x64, 0x48, 0x89, 0x43, displacement as i8 as u8];
        let (mut memory, mut cpu) = fixture(&code);
        cpu.fs_base = fs;
        cpu.registers[3] = base;
        cpu.registers[0] = u64::MAX;
        let before = cpu.clone();
        let resident = memory.resident_bytes();
        assert_eq!(cpu.step(&mut memory), Err(CpuError::InvalidAddress));
        assert_eq!(cpu, before);
        assert_eq!(memory.resident_bytes(), resident);
        assert_eq!(word(&memory, DATA), 0);
    }
    let (mut memory, mut cpu) = fixture(&[0x64, 0x48, 0x89, 0x03]);
    cpu.fs_base = ADDRESS_LIMIT - 4;
    let before = cpu.clone();
    assert_eq!(
        cpu.step(&mut memory),
        Err(CpuError::Memory(MemoryError::InvalidRange))
    );
    assert_eq!(cpu, before);
}

#[test]
fn memory_permissions_fail_without_changing_registers_or_bytes() {
    for (opcode, permissions) in [(0x8b, Permissions::WRITE), (0x89, Permissions::READ)] {
        let (mut memory, mut cpu) = fixture(&[0x48, opcode, 0x03]);
        cpu.registers[3] = DATA;
        cpu.registers[0] = 0x0123_4567_89ab_cdef;
        cpu.zero = true;
        memory.write(DATA, &42_u64.to_le_bytes()).unwrap();
        memory.protect(DATA, PAGE_SIZE, permissions).unwrap();
        let before = cpu.clone();
        assert_eq!(
            cpu.step(&mut memory),
            Err(CpuError::Memory(MemoryError::PermissionDenied(DATA)))
        );
        assert_eq!(cpu, before);
        memory.protect(DATA, PAGE_SIZE, Permissions::READ).unwrap();
        assert_eq!(word(&memory, DATA), 42);
    }
}

#[test]
fn cross_page_store_permission_and_resident_budget_faults_leave_all_bytes_unchanged() {
    let (mut memory, mut cpu) = fixture(&[0x48, 0x89, 0x03]);
    cpu.registers[3] = STACK - 4;
    cpu.registers[0] = u64::MAX;
    memory.write(STACK - 4, &[0x55; 8]).unwrap();
    memory.protect(STACK, PAGE_SIZE, Permissions::READ).unwrap();
    let before = cpu.clone();
    assert_eq!(
        cpu.step(&mut memory),
        Err(CpuError::Memory(MemoryError::PermissionDenied(STACK)))
    );
    assert_eq!(cpu, before);
    assert_eq!(word(&memory, STACK - 4), 0x5555_5555_5555_5555);

    let mut memory = GuestMemory::new(2 * PAGE_SIZE, 3 * PAGE_SIZE);
    for address in [CODE, DATA, STACK] {
        memory
            .map(address, PAGE_SIZE, Permissions::READ_WRITE)
            .unwrap();
    }
    memory.write(CODE, &[0x48, 0x89, 0x03]).unwrap();
    memory
        .protect(CODE, PAGE_SIZE, Permissions::READ_EXECUTE)
        .unwrap();
    memory.write(STACK - 4, &[0x55; 4]).unwrap();
    let mut cpu = ReferenceCpu::at(CODE);
    cpu.registers[3] = STACK - 4;
    cpu.registers[0] = u64::MAX;
    let before = cpu.clone();
    let resident = memory.resident_bytes();
    assert_eq!(
        cpu.step(&mut memory),
        Err(CpuError::Memory(MemoryError::BudgetExceeded))
    );
    assert_eq!(cpu, before);
    assert_eq!(memory.resident_bytes(), resident);
    assert_eq!(word(&memory, STACK - 4), 0x5555_5555);
}

#[test]
fn exact_32bit_access_at_guest_end_succeeds_but_64bit_access_does_not() {
    let (mut memory, mut cpu) = fixture(&[0x89, 0x03]);
    memory
        .map(
            ADDRESS_LIMIT - PAGE_SIZE,
            PAGE_SIZE,
            Permissions::READ_WRITE,
        )
        .unwrap();
    cpu.registers[3] = ADDRESS_LIMIT - 4;
    cpu.registers[0] = 0x8877_6655_4433_2211;
    cpu.step(&mut memory).unwrap();
    let mut bytes = [0; 4];
    memory.read(ADDRESS_LIMIT - 4, &mut bytes).unwrap();
    assert_eq!(bytes, 0x4433_2211_u32.to_le_bytes());
    install(&mut memory, CODE, &[0x48, 0x89, 0x03]);
    cpu.rip = CODE;
    let before = cpu.clone();
    assert_eq!(
        cpu.step(&mut memory),
        Err(CpuError::Memory(MemoryError::InvalidRange))
    );
    assert_eq!(cpu, before);
}

#[test]
fn every_truncated_new_instruction_keeps_complete_cpu_state() {
    for complete in [
        &[0x64, 0x48, 0x8b, 0x83, 1, 2, 3, 4][..],
        &[0x48, 0x89, 0x43, 8][..],
        &[0xff, 0x15, 1, 2, 3, 4][..],
        &[0x41, 0xff, 0xd0][..],
    ] {
        for length in 1..complete.len() {
            let mut memory = GuestMemory::new(PAGE_SIZE, PAGE_SIZE);
            memory
                .map(CODE, PAGE_SIZE, Permissions::READ_WRITE)
                .unwrap();
            let rip = CODE + PAGE_SIZE - length as u64;
            memory.write(rip, &complete[..length]).unwrap();
            memory
                .protect(CODE, PAGE_SIZE, Permissions::READ_EXECUTE)
                .unwrap();
            let mut cpu = ReferenceCpu::at(rip);
            cpu.fs_base = DATA;
            cpu.registers.fill(0x1234);
            cpu.zero = true;
            let before = cpu.clone();
            assert!(
                matches!(cpu.step(&mut memory), Err(CpuError::Memory(_))),
                "{complete:x?} length {length}"
            );
            assert_eq!(cpu, before);
        }
    }
}

#[test]
fn unsupported_prefixes_sib_addressing_and_ff_groups_remain_explicit() {
    let cases: &[&[u8]] = &[
        &[0x64, 0x8b, 0xc3],
        &[0x64, 0x90],
        &[0x64, 0xff, 0xd0],
        &[0x64, 0x64, 0x8b, 0x03],
        &[0x48, 0x64, 0x8b, 0x03],
        &[0x48, 0x48, 0x8b, 0x03],
        &[0x65, 0x8b, 0x03],
        &[0x66, 0x8b, 0x03],
        &[0x67, 0x8b, 0x03],
        &[0xf0, 0x89, 0x03],
        &[0xf3, 0x8b, 0x03],
        &[0x42, 0x8b, 0x03],
        &[0x48, 0x8b, 0x04, 0x23],
        &[0x48, 0x89, 0x44, 0x23, 8],
        &[0x49, 0x8b, 0x05, 0, 0, 0, 0],
        &[0xff, 0xc0],
        &[0xff, 0xe8],
        &[0xff, 0x13],
        &[0xff, 0x14, 0x25, 0, 0, 0, 0],
        &[0x44, 0xff, 0xd0],
        &[0x42, 0xff, 0xd0],
        &[0x41, 0xff, 0x15, 0, 0, 0, 0],
    ];
    for &code in cases {
        let (mut memory, mut cpu) = fixture(code);
        cpu.registers[3] = DATA;
        cpu.fs_base = DATA;
        cpu.registers[0] = u64::MAX;
        let before = cpu.clone();
        let resident = memory.resident_bytes();
        assert!(
            matches!(
                cpu.step(&mut memory),
                Err(CpuError::UnsupportedInstruction { rip: CODE, .. })
            ),
            "{code:x?}"
        );
        assert_eq!(cpu, before);
        assert_eq!(memory.resident_bytes(), resident);
        assert_eq!(word(&memory, DATA), 0);
        assert_eq!(word(&memory, STACK + PAGE_SIZE - 8), 0);
    }
}

#[test]
fn register_indirect_calls_push_exact_return_addresses_and_ret_reads_them() {
    for (code, register) in [
        (&[0xff, 0xd0][..], 0),
        (&[0x41, 0xff, 0xd0][..], 8),
        (&[0x48, 0xff, 0xd6][..], 6),
        (&[0x41, 0xff, 0xd7][..], 15),
    ] {
        let mut instructions = code.to_vec();
        instructions.push(0xcc);
        let (mut memory, mut cpu) = fixture(&instructions);
        install(&mut memory, CODE + 0x100, &[0xc3]);
        cpu.registers[register] = CODE + 0x100;
        let rsp = cpu.registers[4];
        cpu.step(&mut memory).unwrap();
        assert_eq!(cpu.rip, CODE + 0x100);
        assert_eq!(cpu.registers[4], rsp - 8);
        assert_eq!(word(&memory, rsp - 8), CODE + code.len() as u64);
        cpu.step(&mut memory).unwrap();
        assert_eq!(cpu.rip, CODE + code.len() as u64);
        assert_eq!(cpu.registers[4], rsp);
        assert_eq!(cpu.retired, 2);
    }
}

#[test]
fn rip_relative_indirect_call_reads_pointer_before_overlapping_stack_push() {
    for rex in [false, true] {
        let mut code = vec![];
        if rex {
            code.push(0x48);
        }
        code.extend([0xff, 0x15]);
        let return_address = CODE + code.len() as u64 + 4;
        code.extend(
            i32::try_from(DATA as i64 - return_address as i64)
                .unwrap()
                .to_le_bytes(),
        );
        code.push(0xcc);
        let (mut memory, mut cpu) = fixture(&code);
        install(&mut memory, CODE + 0x100, &[0xc3]);
        memory.write(DATA, &(CODE + 0x100).to_le_bytes()).unwrap();
        cpu.registers[4] = DATA + 8;
        cpu.step(&mut memory).unwrap();
        assert_eq!(cpu.rip, CODE + 0x100);
        assert_eq!(word(&memory, DATA), return_address);
        cpu.step(&mut memory).unwrap();
        assert_eq!(cpu.rip, return_address);
        assert_eq!(cpu.registers[4], DATA + 8);
    }
}

#[test]
fn call_rsp_reads_original_register_before_decrementing_it() {
    let (mut memory, mut cpu) = fixture(&[0xff, 0xd4, 0xcc]);
    memory
        .protect(CODE, PAGE_SIZE, Permissions::new(true, true, true))
        .unwrap();
    memory.write(CODE + 0x100, &[0xcc]).unwrap();
    cpu.registers[4] = CODE + 0x100;
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.rip, CODE + 0x100);
    assert_eq!(cpu.registers[4], CODE + 0xf8);
    assert_eq!(word(&memory, CODE + 0xf8), CODE + 2);
}

#[test]
fn direct_indirect_call_and_ret_target_faults_preserve_cpu_and_stack() {
    for target in [ADDRESS_LIMIT, 0x18000, DATA] {
        for form in 0..3 {
            let code = if form == 0 {
                // The canonical/unmapped and nonexecutable targets fit rel32.
                // For the noncanonical case use an indirect form below instead.
                if target == ADDRESS_LIMIT {
                    continue;
                }
                let mut bytes = vec![0xe8];
                bytes.extend(
                    i32::try_from(target as i64 - (CODE + 5) as i64)
                        .unwrap()
                        .to_le_bytes(),
                );
                bytes
            } else if form == 1 {
                vec![0xff, 0xd0]
            } else {
                vec![0xc3]
            };
            let (mut memory, mut cpu) = fixture(&code);
            cpu.registers[0] = target;
            if form == 2 {
                cpu.registers[4] = STACK;
                memory.write(STACK, &target.to_le_bytes()).unwrap();
            } else {
                memory
                    .write(STACK + PAGE_SIZE - 8, &0x1234_5678_u64.to_le_bytes())
                    .unwrap();
            }
            let stack_at = if form == 2 {
                STACK
            } else {
                STACK + PAGE_SIZE - 8
            };
            let original = word(&memory, stack_at);
            let before = cpu.clone();
            assert!(cpu.step(&mut memory).is_err());
            assert_eq!(cpu, before);
            assert_eq!(word(&memory, stack_at), original);
        }
    }
}

#[test]
fn indirect_pointer_reads_and_stack_push_faults_are_transactional() {
    // RIP-relative pointer read requires READ even if the slot is executable.
    let mut code = vec![0xff, 0x15];
    code.extend(((DATA - CODE - 6) as i32).to_le_bytes());
    let (mut memory, mut cpu) = fixture(&code);
    memory.write(DATA, &CODE.to_le_bytes()).unwrap();
    memory
        .protect(DATA, PAGE_SIZE, Permissions::EXECUTE)
        .unwrap();
    let before = cpu.clone();
    assert_eq!(
        cpu.step(&mut memory),
        Err(CpuError::Memory(MemoryError::PermissionDenied(DATA)))
    );
    assert_eq!(cpu, before);
    assert_eq!(word(&memory, STACK + PAGE_SIZE - 8), 0);
    for rsp in [4, 0x18008, DATA + 8] {
        let (mut memory, mut cpu) = fixture(&[0xff, 0xd0]);
        cpu.registers[0] = CODE;
        cpu.registers[4] = rsp;
        memory.write(DATA, &99_u64.to_le_bytes()).unwrap();
        memory.protect(DATA, PAGE_SIZE, Permissions::READ).unwrap();
        let before = cpu.clone();
        assert!(cpu.step(&mut memory).is_err());
        assert_eq!(cpu, before);
        assert_eq!(word(&memory, DATA), 99);
    }
}

#[test]
fn retired_counter_overflow_cannot_commit_a_memory_store() {
    let (mut memory, mut cpu) = fixture(&[0x48, 0x89, 0x03]);
    cpu.registers[3] = DATA;
    cpu.registers[0] = u64::MAX;
    cpu.retired = u64::MAX;
    let before = cpu.clone();
    assert_eq!(
        cpu.step(&mut memory),
        Err(CpuError::InstructionCountOverflow)
    );
    assert_eq!(cpu, before);
    assert_eq!(word(&memory, DATA), 0);
}

#[test]
fn indirect_call_targets_keep_all_64_pointer_bits_before_stack_writes() {
    let mut rip_call = vec![0xff, 0x15];
    rip_call.extend(((DATA - CODE - 6) as i32).to_le_bytes());
    for code in [&[0xff, 0xd0][..], rip_call.as_slice()] {
        let (mut memory, mut cpu) = fixture(code);
        install(&mut memory, CODE + 0x100, &[0xcc]);
        // Its low 32 bits name a valid executable address; truncating the
        // register or pointer slot would incorrectly make this call succeed.
        let target = (1_u64 << 48) | (CODE + 0x100);
        memory.write(DATA, &target.to_le_bytes()).unwrap();
        cpu.registers[0] = target;
        let before = cpu.clone();
        assert_eq!(cpu.step(&mut memory), Err(CpuError::InvalidAddress));
        assert_eq!(cpu, before);
        assert_eq!(word(&memory, STACK + PAGE_SIZE - 8), 0);
    }
}
