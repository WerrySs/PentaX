// SPDX-License-Identifier: GPL-2.0-only
//! Original integer SSE4a bitfield implementation; no host SSE4a requirement.
//! AMD APM 26568 rev3.26, EXTRQ pp172-173 and INSERTQ pp195-196:
//! https://docs.amd.com/v/u/en-US/26568_3.26_APM_Vol4
//! Undefined upper XMM quadwords are preserved as an emulator policy, not an
//! architectural guarantee. Undefined field ranges are refused before writes.

use super::{NativeCpuState, NativeDispatchError, vectors};
use pentax_codegen::{Code, Instruction, OpKind};

pub(super) fn supported(code: Code) -> bool {
    matches!(
        code,
        Code::Extrq_xmm_imm8_imm8
            | Code::Extrq_xmm_xmm
            | Code::Insertq_xmm_xmm_imm8_imm8
            | Code::Insertq_xmm_xmm
    )
}

pub(super) fn execute(
    instruction: &Instruction,
    state: &mut NativeCpuState,
) -> Result<(), NativeDispatchError> {
    let (insert, immediate) = match instruction.code() {
        Code::Extrq_xmm_imm8_imm8 => (false, true),
        Code::Extrq_xmm_xmm => (false, false),
        Code::Insertq_xmm_xmm_imm8_imm8 => (true, true),
        Code::Insertq_xmm_xmm => (true, false),
        _ => return Err(NativeDispatchError::UnsupportedVectorInstruction),
    };
    let registers = if insert || !immediate { 2 } else { 1 };
    if instruction.op_count() != registers + if immediate { 2 } else { 0 }
        || immediate
            && (instruction.op_kind(registers) != OpKind::Immediate8
                || instruction.op_kind(registers + 1) != OpKind::Immediate8_2nd)
    {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    // Validate and snapshot all operands before publishing any result. This is
    // essential when INSERTQ's source data and controls alias its destination.
    let slot = vectors::register_slot(instruction, 0, 16)?;
    let destination = u64::from_le_bytes(state.xmm[slot..slot + 8].try_into().unwrap());
    let source = if registers == 2 {
        vectors::vector_register(instruction, 1, 16, state)?
    } else {
        [0; 32]
    };
    let (length, index) = if immediate {
        (instruction.immediate8(), instruction.immediate8_2nd())
    } else {
        let offset = if insert { 8 } else { 0 };
        (source[offset], source[offset + 1])
    };
    let length = u32::from(length & 63);
    let length = if length == 0 { 64 } else { length };
    let index = u32::from(index & 63);
    if index + length > 64 {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    // A 64-bit field is legal only at index zero; never shift a u64 by 64.
    let mask = if length == 64 {
        u64::MAX
    } else {
        (1_u64 << length) - 1
    };
    let result = if insert {
        let input = u64::from_le_bytes(source[..8].try_into().unwrap());
        (destination & !(mask << index)) | ((input & mask) << index)
    } else {
        (destination >> index) & mask
    };
    state.xmm[slot..slot + 8].copy_from_slice(&result.to_le_bytes());
    // Legacy YMM[255:128], GPRs, flags, MXCSR and TLS are untouched.
    Ok(())
}
