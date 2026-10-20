// SPDX-License-Identifier: GPL-2.0-or-later

//! Bounded ModRM addressing. SIB is admitted only for LEA; address-size
//! overrides remain unsupported.

use super::{CpuError, Cursor, ReferenceCpu, checked_target};

pub(super) enum Operand {
    Register(usize),
    Memory(u64),
}

/// LEA computes an integer without checking or reading the resulting address.
/// Default address size is 64 bits, independently of the destination width.
pub(super) fn decode_lea_address(
    cpu: &ReferenceCpu,
    cursor: &mut Cursor<'_>,
    rex: u8,
    modrm: u8,
) -> Result<u64, CpuError> {
    let mode = modrm >> 6;
    let rm = modrm & 7;
    if mode == 3 {
        return cpu.unsupported(0x8d);
    }
    if rm == 5 && mode == 0 {
        if rex & 1 != 0 {
            return cpu.unsupported(0x8d); // Explicit subset policy, as for MOV.
        }
        let displacement = i64::from(cursor.u32()? as i32);
        return Ok(cursor.address.wrapping_add_signed(displacement));
    }
    let mut base;
    if rm == 4 {
        let sib = cursor.byte()?;
        let base_register = sib & 7;
        let index_register = (sib >> 3) & 7;
        let index = if index_register == 4 && rex & 2 == 0 {
            0
        } else {
            let register = usize::from(index_register) + if rex & 2 != 0 { 8 } else { 0 };
            cpu.registers[register].wrapping_shl(u32::from(sib >> 6))
        };
        if base_register == 5 && mode == 0 {
            if rex & 1 != 0 {
                return cpu.unsupported(0x8d);
            }
            let displacement = i64::from(cursor.u32()? as i32);
            return Ok(index.wrapping_add_signed(displacement));
        }
        let register = usize::from(base_register) + if rex & 1 != 0 { 8 } else { 0 };
        base = cpu.registers[register].wrapping_add(index);
    } else {
        let register = usize::from(rm) + if rex & 1 != 0 { 8 } else { 0 };
        base = cpu.registers[register];
    }
    let displacement = match mode {
        0 => 0,
        1 => i64::from(cursor.byte()? as i8),
        2 => i64::from(cursor.u32()? as i32),
        _ => unreachable!("register mode rejected above"),
    };
    base = base.wrapping_add_signed(displacement);
    Ok(base)
}

pub(super) fn decode_mov_operand(
    cpu: &ReferenceCpu,
    cursor: &mut Cursor<'_>,
    rex: u8,
    modrm: u8,
    fs: bool,
    opcode: u8,
) -> Result<Operand, CpuError> {
    let mode = modrm >> 6;
    let rm = modrm & 7;
    if mode == 3 {
        if fs {
            return cpu.unsupported(opcode);
        }
        return Ok(Operand::Register(
            usize::from(rm) + if rex & 1 != 0 { 8 } else { 0 },
        ));
    }
    if rm == 4 {
        return cpu.unsupported(opcode); // Includes R12 and absolute-SIB forms.
    }
    let (base, displacement) = if mode == 0 && rm == 5 {
        // In this subset REX.B is refused for the RIP-relative encoding,
        // rather than silently ignored or misread as an R13 base register.
        if rex & 1 != 0 {
            return cpu.unsupported(opcode);
        }
        let displacement = cursor.u32()? as i32 as i64;
        (cursor.address, displacement)
    } else {
        let base = cpu.registers[usize::from(rm) + if rex & 1 != 0 { 8 } else { 0 }];
        let displacement = match mode {
            0 => 0,
            1 => cursor.byte()? as i8 as i64,
            2 => cursor.u32()? as i32 as i64,
            _ => unreachable!("register mode returned above"),
        };
        (base, displacement)
    };
    // Checked mathematical guest address arithmetic, including a modeled
    // FS base, deliberately refuses wraparound. A negative displacement may
    // address bytes below FS/base when the final unsigned sum is representable.
    let address = base
        .checked_add(if fs { cpu.fs_base } else { 0 })
        .and_then(|base| base.checked_add_signed(displacement))
        .ok_or(CpuError::InvalidAddress)?;
    Ok(Operand::Memory(checked_target(address)?))
}
