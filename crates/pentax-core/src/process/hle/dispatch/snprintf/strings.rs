// SPDX-License-Identifier: GPL-2.0-only
// String profile adapted from pinned KytyPS5 guestPrintf.cpp and stringUtils.h.
use super::*;

fn units(
    engine: &mut Engine<'_, impl ServiceMemory>,
    address: u64,
    index: u64,
) -> Result<u16, MemoryServiceError> {
    let mut bytes = [0; 2];
    let offset = index
        .checked_mul(2)
        .ok_or(MemoryServiceError::AddressOverflow {
            argument: PointerArgument::Second,
        })?;
    engine.read(at(address, offset)?, &mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

fn wide(
    engine: &mut Engine<'_, impl ServiceMemory>,
    address: u64,
    precision: u32,
    flags: u32,
) -> Result<Vec<u8>, MemoryServiceError> {
    // Reuse the bounded output sink as a temporary UTF-8 builder. Its original
    // vector stays charged while both old/new conversion capacities are live.
    let original = std::mem::take(&mut engine.output);
    engine.held_extra = original.capacity() as u64;
    let maximum = engine.budget.limits.max_span_bytes / 2;
    let precise = flags & FLAGS_PRECISION != 0;
    let limit = if precise {
        u64::from(precision).min(maximum)
    } else {
        maximum
    };
    let mut index = 0;
    let mut ended = false;
    while index < limit {
        let first = units(engine, address, index)?;
        if first == 0 {
            ended = true;
            break;
        }
        index += 1;
        let scalar = if (0xd800..=0xdbff).contains(&first) {
            if precise && index == u64::from(precision) {
                ended = true;
                break;
            }
            if index == limit {
                break;
            }
            let second = units(engine, address, index)?;
            if !(0xdc00..=0xdfff).contains(&second) {
                return Err(MemoryServiceError::UnsupportedFormat(
                    "invalid UTF-16 sequence",
                ));
            }
            index += 1;
            0x10000 + ((u32::from(first) - 0xd800) << 10) + u32::from(second) - 0xdc00
        } else {
            if (0xdc00..=0xdfff).contains(&first) {
                return Err(MemoryServiceError::UnsupportedFormat(
                    "invalid UTF-16 sequence",
                ));
            }
            u32::from(first)
        };
        engine.work(4)?;
        let character = char::from_u32(scalar).ok_or(MemoryServiceError::UnsupportedFormat(
            "invalid UTF-16 scalar",
        ))?;
        let mut utf8 = [0; 4];
        for byte in character.encode_utf8(&mut utf8).bytes() {
            engine.emit(byte)?;
        }
    }
    if !ended && !(precise && index == u64::from(precision)) {
        return Err(MemoryServiceError::UnterminatedString {
            argument: PointerArgument::Second,
            maximum: engine.budget.limits.max_span_bytes,
        });
    }
    let converted = std::mem::replace(&mut engine.output, original);
    engine.held_extra = converted.capacity() as u64;
    Ok(converted)
}

pub(super) fn format(
    engine: &mut Engine<'_, impl ServiceMemory>,
    address: u64,
    width: u32,
    precision: u32,
    flags: u32,
) -> Result<(), MemoryServiceError> {
    if flags & FLAGS_LONG != 0 {
        let converted = wide(engine, address, precision, flags)?;
        let mut length = converted.len();
        if flags & FLAGS_PRECISION != 0 {
            length = length.min(precision as usize);
        }
        if length < converted.len() {
            while length != 0 && converted[length] & 0xc0 == 0x80 {
                length -= 1;
            }
        }
        let padding = width.saturating_sub(length as u32);
        if flags & FLAGS_LEFT == 0 {
            engine.padding(padding, b' ')?;
        }
        for byte in converted[..length].iter().copied() {
            engine.emit(byte)?;
        }
        if flags & FLAGS_LEFT != 0 {
            engine.padding(padding, b' ')?;
        }
        engine.held_extra = 0;
        return Ok(());
    }
    let precise = flags & FLAGS_PRECISION != 0;
    let maximum = engine.budget.limits.max_span_bytes;
    let limit = if precise {
        u64::from(precision).min(maximum)
    } else {
        maximum
    };
    let mut length = 0;
    let mut ended = false;
    while length < limit {
        if engine.byte(at(address, length)?)? == 0 {
            ended = true;
            break;
        }
        length += 1;
    }
    if !ended && !(precise && length == u64::from(precision)) {
        return Err(MemoryServiceError::UnterminatedString {
            argument: PointerArgument::Second,
            maximum,
        });
    }
    let padding = width.saturating_sub(length as u32);
    if flags & FLAGS_LEFT == 0 {
        engine.padding(padding, b' ')?;
    }
    for index in 0..length {
        let byte = engine.byte(at(address, index)?)?;
        engine.emit(byte)?;
    }
    if flags & FLAGS_LEFT != 0 {
        engine.padding(padding, b' ')?;
    }
    Ok(())
}
