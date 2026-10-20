// SPDX-License-Identifier: GPL-2.0-only
// Direct adaptation of KytyPS5/KytyPS5 f186b35706ddbc3faf5513f326e2335d224802f7,
// src/libs/guestPrintf.cpp, blob afc61624f385f2e223625db1ff5777c2367c8197.
// Original algorithm: Marco Paland (info@paland.com), 2014-2019,
// PALANDesign Hannover, Germany; licensed under the MIT License.
// Exponential conversion contributed by Martijn Jasperse.
// Modified 2026 for PentaX: safe fixed-buffer Rust, fallible bounded emission,
// checked source-undefined conversions and recursion refusal. See NOTICE and
// docs/upstream-provenance.md for retained licenses and source attribution.

//! The pinned formatter's f/F/e/E/g/G profile, including its non-ISO quirks.
//! The parent admits width/precision/work before entry and charges every emitted
//! byte. This helper owns no dynamic storage, guest pointer or varargs cursor.

use super::*;

const BUFFER: usize = 32;
const DEFAULT_PRECISION: u32 = 6;
const MAX_FIXED: f64 = 1e9;
const MAX_DEPTH: u8 = 4;
const POW10: [f64; 10] = [
    1.0,
    10.0,
    100.0,
    1000.0,
    10000.0,
    100000.0,
    1000000.0,
    10000000.0,
    100000000.0,
    1000000000.0,
];

pub(super) fn format(
    value: f64,
    spec: u8,
    width: u32,
    precision: u32,
    mut flags: u32,
    emit: &mut impl FnMut(u8) -> Result<(), MemoryServiceError>,
) -> Result<(), MemoryServiceError> {
    match spec {
        b'f' | b'F' => {
            if spec == b'F' {
                flags |= FLAGS_UPPERCASE;
            }
            fixed(value, precision, width, flags, 0, emit)?;
        }
        b'e' | b'E' | b'g' | b'G' => {
            if matches!(spec, b'g' | b'G') {
                flags |= FLAGS_ADAPT_EXP;
            }
            if matches!(spec, b'E' | b'G') {
                flags |= FLAGS_UPPERCASE;
            }
            exponential(value, precision, width, flags, 0, emit)?;
        }
        _ => return Err(MemoryServiceError::UnsupportedFormat("floating conversion")),
    }
    Ok(())
}

fn reverse(
    bytes: &[u8],
    width: u32,
    flags: u32,
    emit: &mut impl FnMut(u8) -> Result<(), MemoryServiceError>,
) -> Result<u64, MemoryServiceError> {
    let mut written = 0_u64;
    if flags & (FLAGS_LEFT | FLAGS_ZEROPAD) == 0 {
        for _ in bytes.len() as u64..u64::from(width) {
            emit(b' ')?;
            written += 1;
        }
    }
    for &byte in bytes.iter().rev() {
        emit(byte)?;
        written += 1;
    }
    if flags & FLAGS_LEFT != 0 {
        while written < u64::from(width) {
            emit(b' ')?;
            written += 1;
        }
    }
    Ok(written)
}

fn integer(value: f64) -> Result<i32, MemoryServiceError> {
    if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
        return Err(MemoryServiceError::UnsupportedFormat(
            "floating integer conversion",
        ));
    }
    Ok(value as i32)
}

fn fixed(
    mut value: f64,
    mut precision: u32,
    mut width: u32,
    flags: u32,
    depth: u8,
    emit: &mut impl FnMut(u8) -> Result<(), MemoryServiceError>,
) -> Result<u64, MemoryServiceError> {
    if depth >= MAX_DEPTH {
        return Err(MemoryServiceError::UnsupportedFormat(
            "floating normalization",
        ));
    }
    let mut bytes = [0; BUFFER];
    let mut length = 0_usize;
    if value.is_nan() {
        return reverse(b"nan", width, flags, emit);
    }
    if value < -f64::MAX {
        return reverse(b"fni-", width, flags, emit);
    }
    if value > f64::MAX {
        return reverse(
            if flags & FLAGS_PLUS != 0 {
                b"fni+"
            } else {
                b"fni"
            },
            width,
            flags,
            emit,
        );
    }
    if !(-MAX_FIXED..=MAX_FIXED).contains(&value) {
        return exponential(value, precision, width, flags, depth + 1, emit);
    }
    // Deliberately uses <0 rather than signbit: source negative zero loses sign.
    let negative = value < 0.0;
    if negative {
        value = 0.0 - value;
    }
    if flags & FLAGS_PRECISION == 0 {
        precision = DEFAULT_PRECISION;
    }
    // In C++, precision>=42 exhausts the 32-byte scratch while still indexing
    // beyond pow10[9]. Refuse that undefined branch before indexing or output.
    if precision > 41 {
        return Err(MemoryServiceError::UnsupportedFormat("floating precision"));
    }
    while length < BUFFER && precision > 9 {
        bytes[length] = b'0';
        length += 1;
        precision -= 1;
    }
    let mut whole = integer(value)?;
    let temporary = (value - f64::from(whole)) * POW10[precision as usize];
    if !temporary.is_finite() || !(0.0..4294967296.0).contains(&temporary) {
        return Err(MemoryServiceError::UnsupportedFormat(
            "floating fraction conversion",
        ));
    }
    let mut fraction = temporary as u32;
    let mut difference = temporary - f64::from(fraction);
    if difference > 0.5 {
        fraction += 1;
        if f64::from(fraction) >= POW10[precision as usize] {
            fraction = 0;
            whole += 1;
        }
    } else if difference == 0.5 && (fraction == 0 || fraction & 1 != 0) {
        fraction += 1;
    }
    if precision == 0 {
        difference = value - f64::from(whole);
        if difference >= 0.5 && whole as u32 & 1 != 0 {
            whole += 1;
        }
    } else {
        let mut count = precision;
        while length < BUFFER {
            count = count.wrapping_sub(1);
            bytes[length] = b'0' + (fraction % 10) as u8;
            length += 1;
            fraction /= 10;
            if fraction == 0 {
                break;
            }
        }
        while length < BUFFER && count > 0 {
            count -= 1;
            bytes[length] = b'0';
            length += 1;
        }
        if length < BUFFER {
            bytes[length] = b'.';
            length += 1;
        }
    }
    while length < BUFFER {
        bytes[length] = b'0' + (whole % 10) as u8;
        length += 1;
        whole /= 10;
        if whole == 0 {
            break;
        }
    }
    if flags & FLAGS_LEFT == 0 && flags & FLAGS_ZEROPAD != 0 {
        if width != 0 && (negative || flags & (FLAGS_PLUS | FLAGS_SPACE) != 0) {
            width -= 1;
        }
        while length < width as usize && length < BUFFER {
            bytes[length] = b'0';
            length += 1;
        }
    }
    if length < BUFFER {
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
            bytes[length] = sign;
            length += 1;
        }
    }
    reverse(&bytes[..length], width, flags, emit)
}

/// The pinned _ntoa_long call used exclusively for a signed decimal exponent:
/// zero precision, minwidth-1 width, and exactly ZEROPAD|PLUS flags.
fn exponent(
    value: i32,
    mut width: u32,
    emit: &mut impl FnMut(u8) -> Result<(), MemoryServiceError>,
) -> Result<u64, MemoryServiceError> {
    let mut magnitude = value.unsigned_abs();
    let mut bytes = [0; BUFFER];
    let mut length = 0;
    loop {
        bytes[length] = b'0' + (magnitude % 10) as u8;
        length += 1;
        magnitude /= 10;
        if magnitude == 0 {
            break;
        }
    }
    width = width.saturating_sub(1);
    while length < width as usize && length < BUFFER {
        bytes[length] = b'0';
        length += 1;
    }
    if length < BUFFER {
        bytes[length] = if value < 0 { b'-' } else { b'+' };
        length += 1;
    }
    reverse(&bytes[..length], width, FLAGS_ZEROPAD | FLAGS_PLUS, emit)
}

// These rounded coefficients are part of the pinned algorithm, including its
// exact arithmetic. Replacing them with Rust constants changes source bits.
#[allow(clippy::approx_constant)]
fn exponential(
    mut value: f64,
    mut precision: u32,
    width: u32,
    mut flags: u32,
    depth: u8,
    emit: &mut impl FnMut(u8) -> Result<(), MemoryServiceError>,
) -> Result<u64, MemoryServiceError> {
    if depth >= MAX_DEPTH {
        return Err(MemoryServiceError::UnsupportedFormat(
            "floating normalization",
        ));
    }
    if !value.is_finite() {
        return fixed(value, precision, width, flags, depth + 1, emit);
    }
    let negative = value < 0.0;
    if negative {
        value = -value;
    }
    if flags & FLAGS_PRECISION == 0 {
        precision = DEFAULT_PRECISION;
    }
    let mut bits = value.to_bits();
    let mut binary_exponent = ((bits >> 52) & 0x07ff) as i32 - 1023;
    bits = (bits & ((1_u64 << 52) - 1)) | (1023_u64 << 52);
    let mut conversion = f64::from_bits(bits);
    let mut decimal_exponent = integer(
        0.1760912590558
            + f64::from(binary_exponent) * 0.301029995663981
            + (conversion - 1.5) * 0.289529654602168,
    )?;
    // C++ lround rounds halfway away from zero; this range is far inside i32.
    binary_exponent = integer((f64::from(decimal_exponent) * 3.321928094887362).round())?;
    let z = f64::from(decimal_exponent) * 2.302585092994046
        - f64::from(binary_exponent) * 0.6931471805599453;
    let z2 = z * z;
    bits = ((binary_exponent + 1023) as u64) << 52;
    conversion = f64::from_bits(bits);
    conversion *= 1.0 + 2.0 * z / (2.0 - z + z2 / (6.0 + z2 / (10.0 + z2 / 14.0)));
    if value < conversion {
        decimal_exponent -= 1;
        conversion /= 10.0;
    }
    let mut minimum_width = if decimal_exponent > -100 && decimal_exponent < 100 {
        4
    } else {
        5
    };
    if flags & FLAGS_ADAPT_EXP != 0 {
        if (1e-4..1e6).contains(&value) {
            // The public helper casts prec to signed int here. Refuse values
            // outside that representable range rather than guessing its cast.
            let signed_precision = i32::try_from(precision)
                .map_err(|_| MemoryServiceError::UnsupportedFormat("floating precision"))?;
            precision = if signed_precision > decimal_exponent {
                signed_precision
                    .checked_sub(decimal_exponent)
                    .and_then(|n| n.checked_sub(1))
                    .ok_or(MemoryServiceError::UnsupportedFormat("floating precision"))?
                    as u32
            } else {
                0
            };
            flags |= FLAGS_PRECISION;
            minimum_width = 0;
            decimal_exponent = 0;
        } else if precision > 0 && flags & FLAGS_PRECISION != 0 {
            precision -= 1;
        }
    }
    let mut float_width = width.saturating_sub(minimum_width);
    if flags & FLAGS_LEFT != 0 && minimum_width != 0 {
        float_width = 0;
    }
    if decimal_exponent != 0 {
        value /= conversion;
    }
    let mut written = fixed(
        if negative { -value } else { value },
        precision,
        float_width,
        flags & !FLAGS_ADAPT_EXP,
        depth + 1,
        emit,
    )?;
    if minimum_width != 0 {
        emit(if flags & FLAGS_UPPERCASE != 0 {
            b'E'
        } else {
            b'e'
        })?;
        written += 1;
        written += exponent(decimal_exponent, minimum_width - 1, emit)?;
        if flags & FLAGS_LEFT != 0 {
            while written < u64::from(width) {
                emit(b' ')?;
                written += 1;
            }
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests;
