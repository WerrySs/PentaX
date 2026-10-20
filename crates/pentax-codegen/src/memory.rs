// SPDX-License-Identifier: GPL-2.0-only

//! Numeric ordinary-instruction footprints, not ownership or execution proof.

use crate::{Code, InstructionRecord, Mnemonic, Register};
use iced_x86::{
    CodeSize, Instruction, InstructionInfo, InstructionInfoFactory, OpAccess, OpKind, UsedMemory,
};

/// Conservative GPR dependencies, including partial/conditional/implicit writes.
/// A previous write to any address register requires a fresh CPU observation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryRegisterDependencies {
    /// Address bases/indices, including memory bit-string register offsets, in
    /// RAX,RCX,RDX,RBX,RSP,RBP,RSI,RDI,R8..R15 order.
    pub address: u16,
    /// Any written alias in the same order, including conditional writes.
    pub written: u16,
}

/// Immutable decoder output owned by one instruction record. These are address
/// expressions, never evaluated addresses, register values or permission proofs.
/// The complete count preserves refusal order when Iced reports more than the
/// supported two accesses: earlier dynamic errors still precede the third slot.
#[derive(Debug)]
pub(crate) struct InstructionMemoryTemplate {
    dependencies: MemoryRegisterDependencies,
    memory: [UsedMemory; 2],
    memory_count: usize,
}
impl InstructionMemoryTemplate {
    pub(crate) fn register_dependencies(&self) -> MemoryRegisterDependencies {
        self.dependencies
    }
}

fn dependencies(info: &InstructionInfo, instruction: &Instruction) -> MemoryRegisterDependencies {
    let mut output = MemoryRegisterDependencies::default();
    for used in info.used_memory() {
        for register in [used.base(), used.index()] {
            if let Some(index) = slot(register.full_register()) {
                output.address |= 1 << index;
            }
        }
    }
    for used in info.used_registers() {
        if matches!(
            used.access(),
            OpAccess::Write | OpAccess::CondWrite | OpAccess::ReadWrite | OpAccess::ReadCondWrite
        ) && let Some(index) = slot(used.register().full_register())
        {
            output.written |= 1 << index;
        }
    }
    // ModRM.reg is not part of the ordinary address expression, but a signed
    // bit-string register offset selects another complete memory element.
    // Planning and execution MUST use this same dependency rule.
    if instruction.op0_kind() == OpKind::Memory
        && bit_string_width(instruction.code()).is_some()
        && instruction.op1_kind() == OpKind::Register
        && let Some(index) = slot(instruction.op1_register().full_register())
    {
        output.address |= 1 << index;
    }
    output
}

fn bit_string_width(code: Code) -> Option<u64> {
    use Code::*;
    Some(match code {
        Bt_rm16_r16 | Bt_rm16_imm8 | Btc_rm16_r16 | Btc_rm16_imm8 | Btr_rm16_r16
        | Btr_rm16_imm8 | Bts_rm16_r16 | Bts_rm16_imm8 => 2,
        Bt_rm32_r32 | Bt_rm32_imm8 | Btc_rm32_r32 | Btc_rm32_imm8 | Btr_rm32_r32
        | Btr_rm32_imm8 | Bts_rm32_r32 | Bts_rm32_imm8 => 4,
        Bt_rm64_r64 | Bt_rm64_imm8 | Btc_rm64_r64 | Btc_rm64_imm8 | Btr_rm64_r64
        | Btr_rm64_imm8 | Bts_rm64_r64 | Bts_rm64_imm8 => 8,
        _ => return None,
    })
}

/// Intel SDM2A Bit(), Table3-2 and BT/BTC/BTR/BTS: register offsets are signed
/// at operand width and DIV rounds down; immediates select within one element.
/// An assembler may already fold higher immediate bits into the displacement.
fn bit_string_displacement(
    instruction: &Instruction,
    width: u64,
    gpr: &[u64; 16],
) -> Result<i64, MemoryAccessError> {
    if instruction.op_count() != 2 || instruction.op0_kind() != OpKind::Memory {
        return Err(MemoryAccessError::UnsupportedBitString);
    }
    match instruction.op1_kind() {
        OpKind::Immediate8 => Ok(0), // High immediate bits never advance memory.
        OpKind::Register => {
            let register = instruction.op1_register();
            if register.size() as u64 != width {
                return Err(MemoryAccessError::UnsupportedBitString);
            }
            let raw = gpr
                [slot(register.full_register()).ok_or(MemoryAccessError::UnsupportedBitString)?];
            let signed = match width {
                2 => i64::from(raw as i16),
                4 => i64::from(raw as i32),
                8 => raw as i64,
                _ => return Err(MemoryAccessError::UnsupportedBitString),
            };
            signed
                .div_euclid(width as i64 * 8)
                .checked_mul(width as i64)
                .ok_or(MemoryAccessError::InvalidRange)
        }
        _ => Err(MemoryAccessError::UnsupportedBitString),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MemoryAccessKind {
    #[default]
    Read,
    Write,
    ReadWrite,
}

/// Derived numeric address/width. No host pointer or live permission is implied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeclaredMemoryAccess {
    pub address: u64,
    pub size: u64,
    pub kind: MemoryAccessKind,
}

/// The positive integer/legacy-SSE profile has at most two ordinary accesses
/// (for example PUSH/POP with a memory operand). No output allocation is used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InstructionMemoryAccesses {
    entries: [DeclaredMemoryAccess; 2],
    length: usize,
    dependencies: MemoryRegisterDependencies,
}
impl InstructionMemoryAccesses {
    pub fn accesses(&self) -> &[DeclaredMemoryAccess] {
        &self.entries[..self.length]
    }
    pub fn register_dependencies(&self) -> MemoryRegisterDependencies {
        self.dependencies
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryAccessError {
    NotEmitted,
    UnsupportedAddress,
    UnsupportedSize,
    UnsupportedBitString,
    TooManyAccesses,
    InvalidRange,
    /// PentaX policy avoids split locks; this is not a guest alignment fault.
    UnalignedAtomic,
}
impl std::fmt::Display for MemoryAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ordinary memory footprint: {self:?}")
    }
}
impl std::error::Error for MemoryAccessError {}

/// Planning reuses Iced's instruction-info scratch to retain immutable decoded
/// templates. `analyze` reuses the exact record's template and evaluates current
/// PRE-instruction GPR values without rebuilding Iced information. As with Iced's
/// decode tables, planning's dependency-internal allocations use its ordinary
/// allocator; recoverable process-wide OOM is not promised. Template and caller
/// output storage are fixed two-entry values, charged with the owning records.
#[derive(Debug)]
pub struct MemoryAccessAnalyzer {
    factory: InstructionInfoFactory,
}
impl Default for MemoryAccessAnalyzer {
    fn default() -> Self {
        Self {
            factory: InstructionInfoFactory::new(),
        }
    }
}
impl MemoryAccessAnalyzer {
    pub(crate) fn prepare(&mut self, instruction: &Instruction) -> InstructionMemoryTemplate {
        let info = self.factory.info(instruction);
        let mut memory = [UsedMemory::default(); 2];
        for (destination, source) in memory.iter_mut().zip(info.used_memory()) {
            *destination = *source;
        }
        InstructionMemoryTemplate {
            dependencies: dependencies(info, instruction),
            memory,
            memory_count: info.used_memory().len(),
        }
    }
    /// Derive explicit and implicit accesses using PRE-instruction GPR values
    /// in RAX,RCX,RDX,RBX,RSP,RBP,RSI,RDI,R8..R15 order. Conditional accesses are
    /// conservatively required regardless of flags. PUSH uses decremented RSP;
    /// POP's memory destination uses its post-increment address; LEAVE reads RBP.
    /// LEA and memory-form NOP do not read memory. Original RIP targets are
    /// retained by Iced's decoded used-memory metadata, not cache addresses.
    ///
    /// Exact ordinary BT/BTC/BTR/BTS memory forms admit the full selected2/4/8B
    /// element. Signed register offsets also participate in address dependencies;
    /// immediate offsets never extend the element beyond decoded displacement.
    pub fn analyze(
        &mut self,
        record: &InstructionRecord,
        gpr: &[u64; 16],
    ) -> Result<InstructionMemoryAccesses, MemoryAccessError> {
        if record.emitted_len() == 0 {
            return Err(MemoryAccessError::NotEmitted);
        }
        let instruction = record.instruction();
        let info = record
            .memory_template
            .as_ref()
            .ok_or(MemoryAccessError::NotEmitted)?;
        let mut output = InstructionMemoryAccesses {
            dependencies: info.dependencies,
            ..Default::default()
        };
        let atomic = record.has_atomic_memory();
        if atomic && info.memory_count != 1 {
            return Err(MemoryAccessError::UnsupportedAddress);
        }
        if info.memory_count == 0
            && crate::policy::potential_memory(instruction)
            && !matches!(instruction.mnemonic(), Mnemonic::Lea | Mnemonic::Nop)
        {
            return Err(MemoryAccessError::UnsupportedAddress);
        }
        let bit_string = info.memory_count != 0
            && matches!(
                instruction.mnemonic(),
                Mnemonic::Bt | Mnemonic::Btc | Mnemonic::Btr | Mnemonic::Bts
            );
        let bit_width = if bit_string {
            if info.memory_count != 1 {
                return Err(MemoryAccessError::UnsupportedBitString);
            }
            Some(
                bit_string_width(instruction.code())
                    .ok_or(MemoryAccessError::UnsupportedBitString)?,
            )
        } else {
            None
        };
        for index in 0..info.memory_count {
            if output.length == output.entries.len() {
                return Err(MemoryAccessError::TooManyAccesses);
            }
            // More than two descriptors were deliberately not retained. The
            // preceding guard refuses the third visit before indexing, after
            // all earlier descriptor/address failures have kept their order.
            let used = &info.memory[index];
            if used.address_size() != CodeSize::Code64 || used.vsib_size() != 0 {
                return Err(MemoryAccessError::UnsupportedAddress);
            }
            let size = used.memory_size().size() as u64;
            let packed_read = size == 32
                && info.memory_count == 1
                && used.access() == OpAccess::Read
                && crate::policy::native_packed_float_memory_width(instruction) == Some(32);
            if size == 0 || (size > 16 && !packed_read) {
                return Err(MemoryAccessError::UnsupportedSize);
            }
            let kind = match used.access() {
                OpAccess::Read | OpAccess::CondRead => MemoryAccessKind::Read,
                OpAccess::Write | OpAccess::CondWrite => MemoryAccessKind::Write,
                OpAccess::ReadWrite | OpAccess::ReadCondWrite => MemoryAccessKind::ReadWrite,
                _ => return Err(MemoryAccessError::UnsupportedAddress),
            };
            let mut address = used
                .virtual_address(0, |register, _, _| match register {
                    Register::None | Register::CS | Register::DS | Register::ES | Register::SS => {
                        Some(0)
                    }
                    _ => slot(register).map(|index| gpr[index]),
                })
                .ok_or(MemoryAccessError::UnsupportedAddress)?;
            if let Some(width) = bit_width {
                if size != width {
                    return Err(MemoryAccessError::UnsupportedBitString);
                }
                address =
                    address.wrapping_add_signed(bit_string_displacement(instruction, width, gpr)?);
            }
            if atomic {
                if kind != MemoryAccessKind::ReadWrite {
                    return Err(MemoryAccessError::UnsupportedAddress);
                }
                if !address.is_multiple_of(size) {
                    return Err(MemoryAccessError::UnalignedAtomic);
                }
            }
            if address
                .checked_add(size)
                .is_none_or(|end| end > crate::source::ADDRESS_LIMIT)
            {
                return Err(MemoryAccessError::InvalidRange);
            }
            output.entries[output.length] = DeclaredMemoryAccess {
                address,
                size,
                kind,
            };
            output.length += 1;
        }
        Ok(output)
    }
}

fn slot(register: Register) -> Option<usize> {
    use Register::*;
    Some(match register {
        RAX => 0,
        RCX => 1,
        RDX => 2,
        RBX => 3,
        RSP => 4,
        RBP => 5,
        RSI => 6,
        RDI => 7,
        R8 => 8,
        R9 => 9,
        R10 => 10,
        R11 => 11,
        R12 => 12,
        R13 => 13,
        R14 => 14,
        R15 => 15,
        _ => return Option::None,
    })
}

#[cfg(test)]
#[path = "memory/template_tests.rs"]
mod template_tests;
