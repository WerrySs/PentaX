// SPDX-License-Identifier: GPL-2.0-only
//! Checked integer-only extended80 to IEEE32/64 stores. Destination precision
//! and guest RC govern conversion; guest PC does not pre-round the source.
//! AMD APM Volume 5 Rev. 3.13 pp. 303-304; Intel SDM Volume 2A FST/FSTP.
//! Existing data/arithmetic validators and physical source bytes stay unchanged.
#![forbid(unsafe_code)]
use super::x87_data::{pending, pop, raise, tag, top};
use super::{NativeCpuState, NativeDispatchError};
use crate::native_memory::PausedNativeMemory;
use pentax_codegen::{Code, Instruction, OpKind};

#[derive(Clone, Copy)]
struct Form {
    size: usize,
    popping: bool,
}
fn form(code: Code) -> Option<Form> {
    let (size, popping) = match code {
        Code::Fst_m32fp => (4, false),
        Code::Fst_m64fp => (8, false),
        Code::Fstp_m32fp => (4, true),
        Code::Fstp_m64fp => (8, true),
        _ => return None,
    };
    Some(Form { size, popping })
}
pub(super) fn supported(code: Code) -> bool {
    form(code).is_some()
}
struct Rounded {
    units: u64,
    inexact: bool,
    increment: bool,
}
fn round(significand: u64, shift: u32, negative: bool, rc: u16) -> Rounded {
    let (units, remainder, half) = if shift == 0 {
        (significand, 0, 0)
    } else if shift < 64 {
        (
            significand >> shift,
            significand & ((1_u64 << shift) - 1),
            1_u64 << (shift - 1),
        )
    } else if shift == 64 {
        (0, significand, 1_u64 << 63)
    } else {
        (0, u64::from(significand != 0), 2)
    };
    let inexact = remainder != 0;
    let increment = inexact
        && match rc {
            0 => remainder > half || remainder == half && units & 1 != 0,
            1 => negative,
            2 => !negative,
            _ => false,
        };
    Rounded {
        units: units + u64::from(increment),
        inexact,
        increment,
    }
}
struct Conversion {
    bits: u64,
    flags: u16,
    increment: bool,
    tiny: bool,
}
fn narrow(value: &[u8; 10], size: usize, rc: u16) -> Result<Conversion, NativeDispatchError> {
    let sig = u64::from_le_bytes(value[..8].try_into().unwrap());
    let field = u16::from_le_bytes(value[8..].try_into().unwrap());
    let exponent = field & 0x7fff;
    let negative = field & 0x8000 != 0;
    let integer_bit = sig >> 63 != 0;
    if exponent == 0 && integer_bit || exponent != 0 && !integer_bit {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    let (precision, bias, exponent_bits) = if size == 4 {
        (24, 127, 8)
    } else {
        (53, 1023, 11)
    };
    let fraction_bits = precision - 1;
    let sign = u64::from(negative) << (size * 8 - 1);
    let fraction_mask = (1_u64 << fraction_bits) - 1;
    let infinity = ((1_u64 << exponent_bits) - 1) << fraction_bits;
    if exponent == 0x7fff {
        let fraction = sig & 0x7fff_ffff_ffff_ffff;
        let (bits, flags) = if fraction == 0 {
            (infinity, 0)
        } else {
            let quiet = 1_u64 << (fraction_bits - 1);
            let payload = (sig >> (64 - precision)) & fraction_mask;
            (
                infinity | payload | quiet,
                if sig & (1 << 62) == 0 { 1 } else { 0 },
            )
        };
        return Ok(Conversion {
            bits: sign | bits,
            flags,
            increment: false,
            tiny: false,
        });
    }
    if sig == 0 {
        return Ok(Conversion {
            bits: sign,
            flags: 0,
            increment: false,
            tiny: false,
        });
    }
    let mut power = i32::from(exponent.max(1)) - 16383;
    let emin = 1 - bias;
    let normal = round(sig, 64 - precision, negative, rc);
    let normal_carry = normal.units == 1_u64 << precision;
    let precision_power = power + i32::from(normal_carry);
    // This independent unbounded-exponent rounding is only a tininess probe.
    // Its rounded significand NEVER feeds destination rounding or stored bits.
    let tiny = precision_power < emin;
    if power < emin {
        let shift = (63 + emin - fraction_bits as i32 - power) as u32;
        let rounded = round(sig, shift, negative, rc);
        return Ok(Conversion {
            bits: sign | rounded.units,
            flags: if rounded.inexact { 0x20 } else { 0 },
            increment: rounded.increment,
            tiny,
        });
    }
    let mut units = normal.units;
    if normal_carry {
        units >>= 1;
        power += 1;
    }
    if power > bias {
        // Derived C1 policy: destination infinity is magnitude-roundup; maxfinite
        // is truncation toward zero. This follows the specified default results
        // and precision-rounding definition, not a claimed host measurement.
        let to_infinity = rc == 0 || rc == 1 && negative || rc == 2 && !negative;
        return Ok(Conversion {
            bits: sign | if to_infinity { infinity } else { infinity - 1 },
            flags: 0x28,
            increment: to_infinity,
            tiny: false,
        });
    }
    Ok(Conversion {
        bits: sign | (((power + bias) as u64) << fraction_bits) | (units & fraction_mask),
        flags: if normal.inexact { 0x20 } else { 0 },
        increment: normal.increment,
        tiny: false,
    })
}
pub(super) fn execute(
    instruction: &Instruction,
    next: &mut NativeCpuState,
    memory: &mut PausedNativeMemory<'_>,
    used_bytes: u64,
) -> Result<u64, NativeDispatchError> {
    let form = form(instruction.code()).ok_or(NativeDispatchError::UnsupportedVectorInstruction)?;
    if pending(&next.x87) {
        return Err(NativeDispatchError::UnsupportedX87Exception);
    }
    if instruction.op_count() != 1
        || instruction.op0_kind() != OpKind::Memory
        || instruction.memory_size().size() != form.size
    {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    let transferred = form.size as u64;
    used_bytes
        .checked_add(transferred)
        .ok_or(NativeDispatchError::Budget("vector bytes"))?;
    let address = super::vectors::address(instruction, 0, next)?;
    memory.preflight_write(address, transferred)?;
    let mut encoded = [0; 15];
    memory.fetch_source(instruction.ip(), &mut encoded[..instruction.len()])?;
    let first = encoded[..instruction.len()]
        .iter()
        .position(|b| (0xd8..=0xdf).contains(b))
        .ok_or(NativeDispatchError::UnsupportedVectorInstruction)?;
    let second = *encoded[..instruction.len()]
        .get(first + 1)
        .ok_or(NativeDispatchError::UnsupportedVectorInstruction)?;
    let opcode = (u16::from(encoded[first] & 7) << 8) | u16::from(second);
    let mut state = next.x87;
    state.status_word &= !0x200;
    let slot = top(&state);
    let bits = if tag(&state, slot) == 3 {
        raise(&mut state, 1, Some(false))?;
        if form.size == 4 {
            0xffc0_0000
        } else {
            0xfff8_0000_0000_0000
        }
    } else {
        let rc = (state.control_word >> 10) & 3;
        let conversion = narrow(&state.registers[slot], form.size, rc)?;
        // A new unmasked exact-tiny exception is refused even when PE is clear.
        // Masked underflow requires both tininess and destination inexactness.
        if conversion.tiny && state.control_word & 0x10 == 0 {
            raise(&mut state, 0x10, None)?;
        }
        let flags = conversion.flags
            | if conversion.tiny && conversion.flags & 0x20 != 0 {
                0x10
            } else {
                0
            };
        raise(&mut state, flags, None)?;
        if conversion.increment {
            state.status_word |= 0x200;
        }
        conversion.bits
    };
    if form.popping {
        pop(&mut state);
    }
    // All checks, conversion and exception decisions precede the sole write.
    // A failed output publishes no pop, status, pointer or partial CPU state.
    memory.write(address, &bits.to_le_bytes()[..form.size])?;
    state.instruction_pointer = instruction.ip();
    state.instruction_opcode = opcode;
    state.data_pointer = address;
    next.x87 = state;
    Ok(transferred)
}
