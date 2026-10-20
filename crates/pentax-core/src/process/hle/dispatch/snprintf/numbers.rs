// SPDX-License-Identifier: GPL-2.0-only
// Integer algorithm adapted from Marco Paland's MIT printf via pinned KytyPS5.
// Copyright (c) 2014-2019 Marco Paland, PALANDesign; see upstream-provenance.md.
use super::*;

pub(super) fn format(
    engine: &mut Engine<'_, impl ServiceMemory>,
    raw: u64,
    spec: u8,
    mut width: u32,
    precision: u32,
    mut flags: u32,
) -> Result<(), MemoryServiceError> {
    let base = match spec {
        b'x' | b'X' | b'p' => 16,
        b'o' => 8,
        b'b' => 2,
        _ => 10,
    };
    if base == 10 {
        flags &= !FLAGS_HASH;
    }
    if spec == b'X' {
        flags |= FLAGS_UPPERCASE;
    }
    if spec == b'p' {
        width = 16;
        flags |= FLAGS_ZEROPAD | FLAGS_UPPERCASE;
    }
    let signed = spec == b'd' || spec == b'i';
    if spec != b'p' {
        if !signed {
            flags &= !(FLAGS_PLUS | FLAGS_SPACE);
        }
        if flags & FLAGS_PRECISION != 0 {
            flags &= !FLAGS_ZEROPAD;
        }
    }
    // The source routes either l or ll to the 64-bit branch. The guest's
    // t/j/z are likewise 64-bit, independent of the host Rust/C long type.
    let wide = spec == b'p' || flags & (FLAGS_LONG | FLAGS_LONG_LONG) != 0;
    let (mut value, negative) = if signed {
        let value = if wide {
            raw as i64
        } else if flags & FLAGS_CHAR != 0 {
            raw as i8 as i64
        } else if flags & FLAGS_SHORT != 0 {
            raw as i16 as i64
        } else {
            raw as i32 as i64
        };
        (value.unsigned_abs(), value < 0)
    } else {
        let value = if wide {
            raw
        } else if flags & FLAGS_CHAR != 0 {
            u64::from(raw as u8)
        } else if flags & FLAGS_SHORT != 0 {
            u64::from(raw as u16)
        } else {
            u64::from(raw as u32)
        };
        (value, false)
    };
    if value == 0 {
        flags &= !FLAGS_HASH;
    }
    let mut buffer = [0; 32];
    let mut length = 0;
    if flags & FLAGS_PRECISION == 0 || value != 0 {
        loop {
            let digit = (value % base) as u8;
            buffer[length] = if digit < 10 {
                b'0' + digit
            } else if flags & FLAGS_UPPERCASE != 0 {
                b'A' + digit - 10
            } else {
                b'a' + digit - 10
            };
            length += 1;
            value /= base;
            if value == 0 || length == buffer.len() {
                break;
            }
        }
    }
    let output_width = width;
    if flags & FLAGS_LEFT == 0 {
        if width != 0
            && flags & FLAGS_ZEROPAD != 0
            && (negative || flags & (FLAGS_PLUS | FLAGS_SPACE) != 0)
        {
            width -= 1;
        }
        while length < precision as usize && length < buffer.len() {
            buffer[length] = b'0';
            length += 1;
        }
        while flags & FLAGS_ZEROPAD != 0 && length < width as usize && length < buffer.len() {
            buffer[length] = b'0';
            length += 1;
        }
    }
    if flags & FLAGS_HASH != 0 {
        if flags & FLAGS_PRECISION == 0
            && length != 0
            && (length == precision as usize || length == width as usize)
        {
            length -= 1;
            if length != 0 && base == 16 {
                length -= 1;
            }
        }
        if length < buffer.len() {
            let prefix = match base {
                16 if flags & FLAGS_UPPERCASE != 0 => Some(b'X'),
                16 => Some(b'x'),
                2 => Some(b'b'),
                _ => None,
            };
            if let Some(prefix) = prefix {
                buffer[length] = prefix;
                length += 1;
            }
        }
        if length < buffer.len() {
            buffer[length] = b'0';
            length += 1;
        }
    }
    if length < buffer.len() {
        let sign = if negative {
            Some(b'-')
        } else if flags & FLAGS_PLUS != 0 {
            Some(b'+')
        } else if flags & FLAGS_SPACE != 0 {
            Some(b' ')
        } else {
            None
        };
        if let Some(sign) = sign {
            buffer[length] = sign;
            length += 1;
        }
    }
    // Width is passed by value to the source's formatting helper: its local
    // sign adjustment also applies to the subsequent reverse-output helper.
    let padding = width.saturating_sub(length as u32);
    if flags & (FLAGS_LEFT | FLAGS_ZEROPAD) == 0 {
        engine.padding(padding, b' ')?;
    }
    for byte in buffer[..length].iter().rev().copied() {
        engine.emit(byte)?;
    }
    if flags & FLAGS_LEFT != 0 {
        engine.padding(output_width.saturating_sub(length as u32), b' ')?;
    }
    Ok(())
}
