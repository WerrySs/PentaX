// SPDX-License-Identifier: GPL-2.0-or-later
//! A deliberately bounded x86-64 reference interpreter for bring-up fixtures.
//!
//! This is not the performance backend or a complete x86 decoder. Unsupported
//! instructions fault at their original RIP, rather than becoming silent no-ops.
//! MOV opcodes 8B/89 support 32/64-bit registers and base/displacement or
//! RIP-relative memory; FS applies only to those memory forms. SIB, other
//! segment prefixes and size overrides remain unsupported. Register PUSH/POP
//! use 64-bit operands with optional REX.W/B. FF/2 and FF/4 support register
//! or RIP-relative indirect CALL/JMP through guest memory only.
//! LEA supports 32/64-bit destinations and 64-bit ModRM/SIB address arithmetic
//! without reading or validating the resulting integer as a guest address.
//!
//! Layouts were checked against the [Intel instruction reference](https://cdrdv2-public.intel.com/835757/325383-sdm-vol-2abcd.pdf)
//! and the [AMD addressing description](https://docs.amd.com/api/khub/documents/sfvvekC9mDflu6vd3R0NXA/content).
//! Checked mathematical addresses and target-fetch preflight are explicit
//! reference-interpreter policies; no host segment or code pointer is used.
use crate::memory::{ADDRESS_LIMIT, GuestMemory, MemoryError};
use std::fmt;

mod addressing;
use addressing::{Operand, decode_lea_address, decode_mov_operand};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Breakpoint,
    Syscall,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CpuError {
    Memory(MemoryError),
    UnsupportedInstruction { rip: u64, opcode: u8 },
    InstructionLimit { rip: u64, limit: u64 },
    InstructionCountOverflow,
    InvalidAddress,
}
impl From<MemoryError> for CpuError {
    fn from(e: MemoryError) -> Self {
        Self::Memory(e)
    }
}
impl fmt::Display for CpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Memory(e) => write!(f, "{e}"),
            Self::UnsupportedInstruction { rip, opcode } => write!(
                f,
                "unsupported reference instruction {opcode:#04x} at {rip:#x}"
            ),
            Self::InstructionLimit { rip, limit } => {
                write!(f, "instruction budget {limit} exhausted at {rip:#x}")
            }
            Self::InvalidAddress => write!(f, "reference instruction address is out of range"),
            Self::InstructionCountOverflow => write!(f, "retired instruction counter overflow"),
        }
    }
}
impl std::error::Error for CpuError {}

/// Register order is the x86 encoding: RAX, RCX, RDX, RBX, RSP, RBP, RSI, RDI, R8..R15.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReferenceCpu {
    pub registers: [u64; 16],
    pub rip: u64,
    pub retired: u64,
    /// Modeled guest FS base only. Never installed in a host segment register.
    pub fs_base: u64,
    zero: bool,
}

impl ReferenceCpu {
    pub fn at(entry: u64) -> Self {
        Self {
            rip: entry,
            ..Self::default()
        }
    }

    pub fn run(&mut self, memory: &mut GuestMemory, budget: u64) -> Result<StopReason, CpuError> {
        for _ in 0..budget {
            if let Some(stop) = self.step(memory)? {
                return Ok(stop);
            }
        }
        Err(CpuError::InstructionLimit {
            rip: self.rip,
            limit: budget,
        })
    }

    /// A rejected instruction leaves CPU state and guest writes uncommitted.
    /// CALL, indirect JMP and RET preflight the first target byte's execute
    /// permission; decoding the full target remains the next step's operation.
    pub fn step(&mut self, memory: &mut GuestMemory) -> Result<Option<StopReason>, CpuError> {
        // CPU state commits only after a complete instruction has been validated.
        let mut next = self.clone();
        next.retired = self
            .retired
            .checked_add(1)
            .ok_or(CpuError::InstructionCountOverflow)?;
        let mut cursor = Cursor {
            memory,
            address: self.rip,
        };
        let first = cursor.byte()?;
        let fs = first == 0x64;
        let first = if fs { cursor.byte()? } else { first };
        let rex = if (0x40..=0x4f).contains(&first) {
            first
        } else {
            0
        };
        let opcode = if rex != 0 { cursor.byte()? } else { first };
        // One FS prefix, followed by at most one REX, is supported only for
        // memory MOV. These are deliberate subset restrictions, not claims
        // that other prefix combinations are invalid x86 instructions.
        if fs && !matches!(opcode, 0x8b | 0x89) {
            return self.unsupported(opcode);
        }
        let wide = rex & 8 != 0;
        let mut stop = None;
        let mut branch = None;
        match opcode {
            0x90 if rex == 0 => {} // NOP
            0xcc if rex == 0 => stop = Some(StopReason::Breakpoint),
            0x50..=0x5f => {
                // Long-mode register PUSH/POP defaults to eight bytes. One
                // REX.W/B is accepted; REX.R/X and other prefixes are outside
                // this reference subset even where hardware ignores them.
                if rex & 6 != 0 {
                    return self.unsupported(opcode);
                }
                let register = (opcode & 7) as usize + if rex & 1 != 0 { 8 } else { 0 };
                let original_rsp = self.registers[4];
                if opcode < 0x58 {
                    let rsp = original_rsp
                        .checked_sub(8)
                        .ok_or(CpuError::InvalidAddress)?;
                    // PUSH RSP stores the original value, before decrement.
                    let value = self.registers[register];
                    cursor.memory.write(rsp, &value.to_le_bytes())?;
                    next.registers[4] = rsp;
                } else {
                    let rsp = original_rsp
                        .checked_add(8)
                        .ok_or(CpuError::InvalidAddress)?;
                    let mut value = [0; 8];
                    cursor.memory.read(original_rsp, &mut value)?;
                    next.registers[4] = rsp;
                    // POP RSP's loaded value replaces the incremented RSP.
                    next.registers[register] = u64::from_le_bytes(value);
                }
            }
            0xb8..=0xbf => {
                let reg = (opcode - 0xb8) as usize + if rex & 1 != 0 { 8 } else { 0 };
                next.registers[reg] = if wide {
                    cursor.u64()?
                } else {
                    cursor.u32()? as u64
                };
            }
            0x8d => {
                let modrm = cursor.byte()?;
                let destination = usize::from((modrm >> 3) & 7) + if rex & 4 != 0 { 8 } else { 0 };
                let value = decode_lea_address(self, &mut cursor, rex, modrm)?;
                next.registers[destination] = if wide { value } else { u64::from(value as u32) };
            }
            0x8b | 0x89 => {
                if rex & 2 != 0 {
                    return self.unsupported(opcode); // No SIB/index semantics.
                }
                let modrm = cursor.byte()?;
                let reg = ((modrm >> 3) & 7) as usize + if rex & 4 != 0 { 8 } else { 0 };
                let rm = decode_mov_operand(self, &mut cursor, rex, modrm, fs, opcode)?;
                if opcode == 0x8b {
                    next.registers[reg] = match rm {
                        Operand::Register(source) => {
                            if wide {
                                self.registers[source]
                            } else {
                                self.registers[source] as u32 as u64
                            }
                        }
                        Operand::Memory(address) => {
                            if wide {
                                let mut data = [0; 8];
                                cursor.memory.read(address, &mut data)?;
                                u64::from_le_bytes(data)
                            } else {
                                let mut data = [0; 4];
                                cursor.memory.read(address, &mut data)?;
                                u64::from(u32::from_le_bytes(data))
                            }
                        }
                    };
                } else {
                    let value = self.registers[reg];
                    match rm {
                        Operand::Register(destination) => {
                            next.registers[destination] =
                                if wide { value } else { value as u32 as u64 }
                        }
                        Operand::Memory(address) => {
                            // GuestMemory preflights the complete write. There
                            // are no fallible operations after this point.
                            if wide {
                                cursor.memory.write(address, &value.to_le_bytes())?;
                            } else {
                                cursor
                                    .memory
                                    .write(address, &(value as u32).to_le_bytes())?;
                            }
                        }
                    }
                }
            }
            0x31 | 0x33 => {
                let modrm = cursor.byte()?;
                if modrm & 0xc0 != 0xc0 {
                    return self.unsupported(opcode);
                }
                let rm = (modrm & 7) as usize + if rex & 1 != 0 { 8 } else { 0 };
                let reg = ((modrm >> 3) & 7) as usize + if rex & 4 != 0 { 8 } else { 0 };
                let (dst, src) = if opcode == 0x31 { (rm, reg) } else { (reg, rm) };
                let value = next.registers[dst] ^ next.registers[src];
                next.registers[dst] = if wide { value } else { value as u32 as u64 };
                next.zero = next.registers[dst] == 0;
            }
            0x05 | 0x2d | 0x3d => {
                let immediate = cursor.u32()?;
                let a = next.registers[0];
                let b = if wide {
                    immediate as i32 as i64 as u64
                } else {
                    immediate as u64
                };
                let result = if opcode == 0x05 {
                    a.wrapping_add(b)
                } else {
                    a.wrapping_sub(b)
                };
                let result = if wide { result } else { result as u32 as u64 };
                if opcode != 0x3d {
                    next.registers[0] = result;
                }
                next.zero = result == 0;
            }
            0xeb | 0x74 | 0x75 => {
                let offset = cursor.byte()? as i8 as i64;
                let take = opcode == 0xeb
                    || (opcode == 0x74 && self.zero)
                    || (opcode == 0x75 && !self.zero);
                if take {
                    branch = Some(relative(cursor.address, offset)?);
                }
            }
            0xe9 | 0xe8 => {
                let offset = cursor.u32()? as i32 as i64;
                let target = relative(cursor.address, offset)?;
                if opcode == 0xe8 {
                    fetch_target(cursor.memory, target)?;
                    let rsp = next.registers[4]
                        .checked_sub(8)
                        .ok_or(CpuError::InvalidAddress)?;
                    let return_address = cursor.address;
                    cursor.memory.write(rsp, &return_address.to_le_bytes())?;
                    next.registers[4] = rsp;
                }
                branch = Some(target);
            }
            0xff => {
                // Near CALL/JMP r/m64 defaults to 64 bits. This subset accepts
                // optional REX.W/B; REX.R/X and other group operations remain
                // unsupported. Memory targets use only RIP-relative form.
                if rex & 6 != 0 {
                    return self.unsupported(opcode);
                }
                let modrm = cursor.byte()?;
                let operation = (modrm >> 3) & 7;
                if !matches!(operation, 2 | 4) {
                    return self.unsupported(opcode);
                }
                let target = if modrm & 0xc0 == 0xc0 {
                    let register = (modrm & 7) as usize + if rex & 1 != 0 { 8 } else { 0 };
                    self.registers[register]
                } else if modrm & 0xc7 == 5 && rex & 1 == 0 {
                    let displacement = cursor.u32()? as i32 as i64;
                    let address = relative(cursor.address, displacement)?;
                    let mut pointer = [0; 8];
                    cursor.memory.read(address, &mut pointer)?;
                    u64::from_le_bytes(pointer)
                } else {
                    return self.unsupported(opcode);
                };
                let target = fetch_target(cursor.memory, target)?;
                if operation == 2 {
                    let rsp = self.registers[4]
                        .checked_sub(8)
                        .ok_or(CpuError::InvalidAddress)?;
                    cursor.memory.write(rsp, &cursor.address.to_le_bytes())?;
                    next.registers[4] = rsp;
                }
                branch = Some(target);
            }
            0xc3 if rex == 0 => {
                let rsp = next.registers[4];
                let new_rsp = rsp.checked_add(8).ok_or(CpuError::InvalidAddress)?;
                let mut data = [0; 8];
                cursor.memory.read(rsp, &mut data)?;
                let target = fetch_target(cursor.memory, u64::from_le_bytes(data))?;
                next.registers[4] = new_rsp;
                branch = Some(target);
            }
            0x0f if rex == 0 => {
                if cursor.byte()? != 0x05 {
                    return self.unsupported(opcode);
                }
                next.registers[1] = cursor.address; // architectural SYSCALL return RIP
                // Flags are not fully modeled by the reference subset.
                // Kernel code must not treat this trap as a completed service.
                stop = Some(StopReason::Syscall);
            }
            _ => return self.unsupported(opcode),
        }
        next.rip = branch.unwrap_or(cursor.address);
        *self = next;
        Ok(stop)
    }

    fn unsupported<T>(&self, opcode: u8) -> Result<T, CpuError> {
        Err(CpuError::UnsupportedInstruction {
            rip: self.rip,
            opcode,
        })
    }
}

fn relative(address: u64, offset: i64) -> Result<u64, CpuError> {
    let target = address
        .checked_add_signed(offset)
        .ok_or(CpuError::InvalidAddress)?;
    checked_target(target)
}

fn checked_target(target: u64) -> Result<u64, CpuError> {
    if target >= ADDRESS_LIMIT {
        return Err(CpuError::InvalidAddress);
    }
    Ok(target)
}

/// Preflight one executable target byte before CALL writes a return address,
/// indirect JMP branches or RET commits CPU state. Full target instruction
/// decoding occurs on the next step; direct JMP keeps fetch-on-step behavior.
/// This is a transactional interpreter policy, not an exception model.
fn fetch_target(memory: &GuestMemory, target: u64) -> Result<u64, CpuError> {
    let target = checked_target(target)?;
    memory.fetch(target, &mut [0])?;
    Ok(target)
}

struct Cursor<'a> {
    memory: &'a mut GuestMemory,
    address: u64,
}
impl Cursor<'_> {
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N], CpuError> {
        let next = self
            .address
            .checked_add(N as u64)
            .ok_or(CpuError::InvalidAddress)?;
        let mut bytes = [0; N];
        self.memory.fetch(self.address, &mut bytes)?;
        self.address = next;
        Ok(bytes)
    }
    fn byte(&mut self) -> Result<u8, CpuError> {
        Ok(self.bytes::<1>()?[0])
    }
    fn u32(&mut self) -> Result<u32, CpuError> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }
    fn u64(&mut self) -> Result<u64, CpuError> {
        Ok(u64::from_le_bytes(self.bytes()?))
    }
}

#[cfg(test)]
#[path = "cpu/memory_tests.rs"]
mod memory_tests;

#[cfg(test)]
#[path = "cpu/stack_branch_tests.rs"]
mod stack_branch_tests;

#[cfg(test)]
mod lea_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{PAGE_SIZE, Permissions};

    fn fixture(code: &[u8]) -> (GuestMemory, ReferenceCpu) {
        let mut memory = GuestMemory::new(PAGE_SIZE * 2, PAGE_SIZE * 2);
        memory
            .map(0x4000, PAGE_SIZE, Permissions::READ_WRITE)
            .unwrap();
        memory.write(0x4000, code).unwrap();
        memory
            .protect(0x4000, PAGE_SIZE, Permissions::READ_EXECUTE)
            .unwrap();
        (memory, ReferenceCpu::at(0x4000))
    }

    #[test]
    fn arithmetic_loop_and_32bit_zero_extension() {
        // mov eax,3; sub eax,1; jnz -7; int3
        let (mut mem, mut cpu) = fixture(&[0xb8, 3, 0, 0, 0, 0x2d, 1, 0, 0, 0, 0x75, 0xf9, 0xcc]);
        cpu.registers[0] = u64::MAX;
        assert_eq!(cpu.run(&mut mem, 10).unwrap(), StopReason::Breakpoint);
        assert_eq!(cpu.registers[0], 0);
        assert_eq!(cpu.retired, 8);
    }

    #[test]
    fn unsupported_and_memory_fault_keep_original_rip() {
        let (mut mem, mut cpu) = fixture(&[0xf4]);
        let before = cpu.clone();
        assert!(matches!(
            cpu.step(&mut mem),
            Err(CpuError::UnsupportedInstruction { .. })
        ));
        assert_eq!(cpu, before);
        cpu.rip = 0x7fff;
        // Final byte is zero (unsupported); a MOV prefix with a truncated immediate tests fetch fault.
        mem.protect(0x4000, PAGE_SIZE, Permissions::READ_WRITE)
            .unwrap();
        mem.write(0x7fff, &[0xb8]).unwrap();
        mem.protect(0x4000, PAGE_SIZE, Permissions::READ_EXECUTE)
            .unwrap();
        let before = cpu.clone();
        assert!(matches!(cpu.step(&mut mem), Err(CpuError::Memory(_))));
        assert_eq!(cpu, before);
    }

    #[test]
    fn bounded_execution_and_execute_permissions() {
        let (mut mem, mut cpu) = fixture(&[0xeb, 0xfe]);
        assert!(matches!(
            cpu.run(&mut mem, 50),
            Err(CpuError::InstructionLimit { .. })
        ));
        assert_eq!(cpu.retired, 50);
        mem.protect(0x4000, PAGE_SIZE, Permissions::READ).unwrap();
        assert!(matches!(
            cpu.step(&mut mem),
            Err(CpuError::Memory(MemoryError::PermissionDenied(_)))
        ));
    }

    #[test]
    fn calls_use_guest_stack_and_ret_really_reads_it() {
        // call +1; int3; mov eax,42; ret
        let (mut mem, mut cpu) = fixture(&[0xe8, 1, 0, 0, 0, 0xcc, 0xb8, 42, 0, 0, 0, 0xc3]);
        mem.map(0x8000, PAGE_SIZE, Permissions::READ_WRITE).unwrap();
        cpu.registers[4] = 0xc000;
        assert_eq!(cpu.run(&mut mem, 10).unwrap(), StopReason::Breakpoint);
        assert_eq!(cpu.registers[0], 42);
        assert_eq!(cpu.registers[4], 0xc000);
        assert_eq!(cpu.retired, 4);
    }

    #[test]
    fn noncanonical_return_and_counter_overflow_preserve_cpu_state() {
        let (mut mem, mut cpu) = fixture(&[0xc3]);
        mem.map(0x8000, PAGE_SIZE, Permissions::READ_WRITE).unwrap();
        mem.write(0x8000, &ADDRESS_LIMIT.to_le_bytes()).unwrap();
        cpu.registers[4] = 0x8000;
        let before = cpu.clone();
        assert_eq!(cpu.step(&mut mem), Err(CpuError::InvalidAddress));
        assert_eq!(cpu, before);
        cpu.retired = u64::MAX;
        let before = cpu.clone();
        assert_eq!(cpu.step(&mut mem), Err(CpuError::InstructionCountOverflow));
        assert_eq!(cpu, before);
    }

    #[test]
    fn noncanonical_call_does_not_write_the_stack() {
        let mut mem = GuestMemory::new(PAGE_SIZE * 2, PAGE_SIZE * 2);
        mem.map(
            ADDRESS_LIMIT - PAGE_SIZE,
            PAGE_SIZE,
            Permissions::READ_WRITE,
        )
        .unwrap();
        mem.write(ADDRESS_LIMIT - 8, &[0xe8, 0xff, 0xff, 0, 0])
            .unwrap();
        mem.protect(
            ADDRESS_LIMIT - PAGE_SIZE,
            PAGE_SIZE,
            Permissions::READ_EXECUTE,
        )
        .unwrap();
        mem.map(0x4000, PAGE_SIZE, Permissions::READ_WRITE).unwrap();
        let mut cpu = ReferenceCpu::at(ADDRESS_LIMIT - 8);
        cpu.registers[4] = 0x8000;
        let before = cpu.clone();
        assert_eq!(cpu.step(&mut mem), Err(CpuError::InvalidAddress));
        assert_eq!(cpu, before);
        let mut stack = [1; 8];
        mem.read(0x7ff8, &mut stack).unwrap();
        assert_eq!(stack, [0; 8]);
    }
}
