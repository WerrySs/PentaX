// SPDX-License-Identifier: GPL-2.0-or-later
//! Original register-stack and indirect-branch fixtures only.

use super::*;
use crate::memory::{PAGE_SIZE, Permissions};

const CODE: u64 = 0x4000;
const DATA: u64 = 0x8000;
const STACK: u64 = 0xc000;
const TARGET: u64 = CODE + 0x100;
const VALUE: u64 = 0xfedc_ba98_7654_3210;

fn fixture(code: &[u8]) -> (GuestMemory, ReferenceCpu) {
    let mut memory = GuestMemory::new(4 * PAGE_SIZE, 4 * PAGE_SIZE);
    for address in [CODE, DATA, STACK] {
        memory
            .map(address, PAGE_SIZE, Permissions::READ_WRITE)
            .unwrap();
    }
    memory.write(CODE, code).unwrap();
    memory.write(TARGET, &[0xcc]).unwrap();
    memory
        .protect(CODE, PAGE_SIZE, Permissions::READ_EXECUTE)
        .unwrap();
    let mut cpu = ReferenceCpu::at(CODE);
    cpu.registers[4] = STACK + PAGE_SIZE;
    cpu.fs_base = 0x1234;
    cpu.zero = true;
    (memory, cpu)
}

fn word(memory: &GuestMemory, address: u64) -> u64 {
    let mut bytes = [0; 8];
    memory.read(address, &mut bytes).unwrap();
    u64::from_le_bytes(bytes)
}

fn register_instruction(opcode: u8, register: usize, wide: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    let rex = if wide { 8 } else { 0 } | if register >= 8 { 1 } else { 0 };
    if rex != 0 {
        bytes.push(0x40 | rex);
    }
    bytes.push(opcode + (register as u8 & 7));
    bytes
}

#[test]
fn push_pop_every_register_uses_eight_bytes_and_preserves_flags() {
    for register in 0..16 {
        for wide in [false, true] {
            let mut code = register_instruction(0x50, register, wide);
            let pop_at = CODE + code.len() as u64;
            code.extend(register_instruction(0x58, register, wide));
            let (mut memory, mut cpu) = fixture(&code);
            let rsp = cpu.registers[4];
            if register != 4 {
                cpu.registers[register] = VALUE;
            }
            let original = cpu.clone();
            let pushed = cpu.registers[register];
            cpu.step(&mut memory).unwrap();
            assert_eq!(cpu.rip, pop_at);
            assert_eq!(cpu.registers[4], rsp - 8);
            assert_eq!(word(&memory, rsp - 8), pushed);
            assert!(cpu.zero);
            if register != 4 {
                cpu.registers[register] = 0;
            }
            cpu.step(&mut memory).unwrap();
            assert_eq!(cpu.registers, original.registers);
            assert_eq!(cpu.fs_base, original.fs_base);
            assert!(cpu.zero);
            assert_eq!(cpu.retired, 2);
            assert_eq!(cpu.rip, CODE + code.len() as u64);
        }
    }
}

#[test]
fn push_rsp_stores_original_and_pop_rsp_loaded_value_wins() {
    let (mut memory, mut cpu) = fixture(&[0x54, 0x5c]);
    let original_rsp = cpu.registers[4];
    cpu.step(&mut memory).unwrap();
    assert_eq!(word(&memory, original_rsp - 8), original_rsp);
    // POP assigns its loaded data after incrementing, even for a value that
    // would fail a later guest stack access. Loading data is not a fetch.
    memory
        .write(original_rsp - 8, &u64::MAX.to_le_bytes())
        .unwrap();
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[4], u64::MAX);
    assert_eq!(cpu.rip, CODE + 2);
}

#[test]
fn push_and_pop_can_cross_a_guest_page_boundary() {
    let (mut memory, mut cpu) = fixture(&[0x50, 0x5a]);
    cpu.registers[0] = VALUE;
    cpu.registers[4] = STACK + 4;
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[4], STACK - 4);
    assert_eq!(word(&memory, STACK - 4), VALUE);
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[2], VALUE);
    assert_eq!(cpu.registers[4], STACK + 4);
}

#[test]
fn push_permission_and_resident_budget_failures_do_not_change_bytes_or_cpu() {
    for budget_failure in [false, true] {
        let mut memory = GuestMemory::new(
            if budget_failure {
                2 * PAGE_SIZE
            } else {
                3 * PAGE_SIZE
            },
            3 * PAGE_SIZE,
        );
        for address in [CODE, DATA, STACK] {
            memory
                .map(address, PAGE_SIZE, Permissions::READ_WRITE)
                .unwrap();
        }
        memory.write(CODE, &[0x50]).unwrap();
        memory
            .protect(CODE, PAGE_SIZE, Permissions::READ_EXECUTE)
            .unwrap();
        memory.write(STACK - 4, &[0x55; 4]).unwrap();
        if !budget_failure {
            memory.protect(STACK, PAGE_SIZE, Permissions::READ).unwrap();
        }
        let mut cpu = ReferenceCpu::at(CODE);
        cpu.registers[4] = STACK + 4;
        cpu.registers[0] = VALUE;
        let before = cpu.clone();
        let resident = memory.resident_bytes();
        let expected = if budget_failure {
            MemoryError::BudgetExceeded
        } else {
            MemoryError::PermissionDenied(STACK)
        };
        assert_eq!(cpu.step(&mut memory), Err(CpuError::Memory(expected)));
        assert_eq!(cpu, before);
        assert_eq!(memory.resident_bytes(), resident);
        assert_eq!(word(&memory, STACK - 4), 0x5555_5555);
    }
}

#[test]
fn pop_requires_the_entire_readable_stack_word_before_register_commit() {
    let (mut memory, mut cpu) = fixture(&[0x5c]);
    cpu.registers[4] = STACK - 4;
    memory.write(STACK - 4, &VALUE.to_le_bytes()).unwrap();
    memory
        .protect(STACK, PAGE_SIZE, Permissions::WRITE)
        .unwrap();
    let before = cpu.clone();
    let resident = memory.resident_bytes();
    assert_eq!(
        cpu.step(&mut memory),
        Err(CpuError::Memory(MemoryError::PermissionDenied(STACK)))
    );
    assert_eq!(cpu, before);
    assert_eq!(memory.resident_bytes(), resident);
    memory.protect(STACK, PAGE_SIZE, Permissions::READ).unwrap();
    assert_eq!(word(&memory, STACK - 4), VALUE);
}

#[test]
fn invalid_and_unmapped_stack_ranges_leave_cpu_and_guest_data_unchanged() {
    for opcode in [0x50, 0x58] {
        let bad_stacks: &[u64] = if opcode == 0x50 {
            &[0, 7, 0x18008, ADDRESS_LIMIT + 4, u64::MAX]
        } else {
            &[0x18000, ADDRESS_LIMIT - 4, ADDRESS_LIMIT, u64::MAX - 3]
        };
        for &rsp in bad_stacks {
            let (mut memory, mut cpu) = fixture(&[opcode]);
            memory.write(DATA, &VALUE.to_le_bytes()).unwrap();
            cpu.registers[0] = VALUE;
            cpu.registers[4] = rsp;
            let before = cpu.clone();
            let resident = memory.resident_bytes();
            assert!(cpu.step(&mut memory).is_err(), "{opcode:x} {rsp:x}");
            assert_eq!(cpu, before);
            assert_eq!(word(&memory, DATA), VALUE);
            assert_eq!(memory.resident_bytes(), resident);
        }
    }
}

#[test]
fn stack_words_at_the_last_valid_guest_bytes_work() {
    let (mut memory, mut cpu) = fixture(&[0x50, 0x5a]);
    memory
        .map(
            ADDRESS_LIMIT - PAGE_SIZE,
            PAGE_SIZE,
            Permissions::READ_WRITE,
        )
        .unwrap();
    cpu.registers[0] = VALUE;
    cpu.registers[4] = ADDRESS_LIMIT;
    cpu.step(&mut memory).unwrap();
    assert_eq!(word(&memory, ADDRESS_LIMIT - 8), VALUE);
    cpu.step(&mut memory).unwrap();
    assert_eq!(cpu.registers[2], VALUE);
    assert_eq!(cpu.registers[4], ADDRESS_LIMIT);
}

#[test]
fn register_indirect_jmp_preserves_all_registers_and_never_uses_the_stack() {
    for register in 0..16 {
        for wide in [false, true] {
            let mut code = register_instruction(0xe0, register, wide);
            code.insert(code.len() - 1, 0xff);
            let (mut memory, mut cpu) = fixture(&code);
            cpu.registers[4] = 4; // A CALL would underflow its stack pointer.
            cpu.registers[register] = TARGET;
            let registers = cpu.registers;
            memory.protect(STACK, PAGE_SIZE, Permissions::READ).unwrap();
            cpu.step(&mut memory).unwrap();
            assert_eq!(cpu.rip, TARGET);
            assert_eq!(cpu.registers, registers);
            assert_eq!(cpu.retired, 1);
            assert!(cpu.zero);
            assert_eq!(word(&memory, STACK + PAGE_SIZE - 8), 0);
        }
    }
}

#[test]
fn rip_relative_jmp_uses_full_instruction_end_and_signed_displacement() {
    for wide in [false, true] {
        for negative in [false, true] {
            let rip = if negative { CODE + 0x200 } else { CODE };
            let pointer = if negative { CODE + 0x80 } else { DATA };
            let mut code = if wide {
                vec![0x48, 0xff, 0x25]
            } else {
                vec![0xff, 0x25]
            };
            let end = rip + code.len() as u64 + 4;
            code.extend(
                i32::try_from(pointer as i64 - end as i64)
                    .unwrap()
                    .to_le_bytes(),
            );
            let (mut memory, mut cpu) = fixture(&[0xcc]);
            memory
                .protect(CODE, PAGE_SIZE, Permissions::READ_WRITE)
                .unwrap();
            memory.write(rip, &code).unwrap();
            memory.write(pointer, &TARGET.to_le_bytes()).unwrap();
            memory
                .protect(CODE, PAGE_SIZE, Permissions::READ_EXECUTE)
                .unwrap();
            cpu.rip = rip;
            cpu.registers[4] = u64::MAX;
            let before = cpu.clone();
            cpu.step(&mut memory).unwrap();
            assert_eq!(cpu.rip, TARGET);
            assert_eq!(cpu.registers, before.registers);
            assert_eq!(word(&memory, pointer), TARGET);
        }
    }
}

#[test]
fn indirect_jmp_target_faults_keep_full_pointer_bits_and_cpu_state() {
    for target in [DATA, 0x18000, ADDRESS_LIMIT, (1_u64 << 48) | TARGET] {
        for pointer_form in [false, true] {
            let mut code = if pointer_form {
                vec![0xff, 0x25]
            } else {
                vec![0xff, 0xe0]
            };
            if pointer_form {
                code.extend(
                    i32::try_from(DATA as i64 - (CODE + 6) as i64)
                        .unwrap()
                        .to_le_bytes(),
                );
            }
            let (mut memory, mut cpu) = fixture(&code);
            cpu.registers[0] = target;
            memory.write(DATA, &target.to_le_bytes()).unwrap();
            memory
                .write(STACK + PAGE_SIZE - 8, &VALUE.to_le_bytes())
                .unwrap();
            let before = cpu.clone();
            let resident = memory.resident_bytes();
            assert!(cpu.step(&mut memory).is_err());
            assert_eq!(cpu, before);
            assert_eq!(memory.resident_bytes(), resident);
            assert_eq!(word(&memory, STACK + PAGE_SIZE - 8), VALUE);
            assert_eq!(word(&memory, DATA), target);
        }
    }
}

#[test]
fn jmp_pointer_requires_eight_readable_bytes_without_any_stack_write() {
    for permission_failure in [false, true] {
        let pointer = if permission_failure {
            DATA
        } else {
            STACK + PAGE_SIZE - 4
        };
        let mut code = vec![0xff, 0x25];
        code.extend(
            i32::try_from(pointer as i64 - (CODE + 6) as i64)
                .unwrap()
                .to_le_bytes(),
        );
        let (mut memory, mut cpu) = fixture(&code);
        if permission_failure {
            memory.write(pointer, &TARGET.to_le_bytes()).unwrap();
            memory
                .protect(DATA, PAGE_SIZE, Permissions::EXECUTE)
                .unwrap();
        } else {
            memory
                .write(pointer, &(TARGET as u32).to_le_bytes())
                .unwrap();
        }
        cpu.registers[4] = 4;
        let before = cpu.clone();
        let resident = memory.resident_bytes();
        let error = if permission_failure {
            MemoryError::PermissionDenied(DATA)
        } else {
            MemoryError::Unmapped(STACK + PAGE_SIZE)
        };
        assert_eq!(cpu.step(&mut memory), Err(CpuError::Memory(error)));
        assert_eq!(cpu, before);
        assert_eq!(memory.resident_bytes(), resident);
    }
}

#[test]
fn truncated_stack_prefixes_and_indirect_jmp_leave_state_unchanged() {
    for complete in [
        &[0x41, 0x50][..],
        &[0x48, 0x5c][..],
        &[0x41, 0xff, 0xe7][..],
        &[0xff, 0x25, 1, 2, 3, 4][..],
    ] {
        for length in 1..complete.len() {
            let (mut memory, mut cpu) = fixture(&[0xcc]);
            let rip = STACK + PAGE_SIZE - length as u64;
            memory.write(rip, &complete[..length]).unwrap();
            memory
                .protect(STACK, PAGE_SIZE, Permissions::READ_EXECUTE)
                .unwrap();
            cpu.rip = rip;
            let before = cpu.clone();
            let resident = memory.resident_bytes();
            assert!(matches!(cpu.step(&mut memory), Err(CpuError::Memory(_))));
            assert_eq!(cpu, before);
            assert_eq!(memory.resident_bytes(), resident);
        }
    }
}

#[test]
fn unsupported_stack_and_jmp_prefixes_or_address_forms_are_explicit() {
    for code in [
        &[0x66, 0x50][..],
        &[0x67, 0x58],
        &[0x64, 0x50],
        &[0xf0, 0x58],
        &[0x42, 0x50],
        &[0x44, 0x58],
        &[0x48, 0x41, 0x50],
        &[0xff, 0x23],
        &[0xff, 0x24, 0x25, 0, 0, 0, 0],
        &[0xff, 0x63, 8],
        &[0xff, 0xa3, 0, 0, 0, 0],
        &[0x41, 0xff, 0x25, 0, 0, 0, 0],
        &[0x42, 0xff, 0xe0],
        &[0x44, 0xff, 0xe0],
        &[0x64, 0xff, 0xe0],
        &[0xff, 0x28],
    ] {
        let (mut memory, mut cpu) = fixture(code);
        cpu.registers[0] = TARGET;
        cpu.registers[3] = DATA;
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
