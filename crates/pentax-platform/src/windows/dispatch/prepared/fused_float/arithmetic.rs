// SPDX-License-Identifier: GPL-2.0-only
//! Original integer-only binary32/binary64 FMA over the complete exponent range.
//! 66 limbs hold every exact finite double product plus addend (4196 bits).
use super::NativeDispatchError;
use std::cmp::Ordering;
const LIMBS: usize = 66;
type Wide = [u64; LIMBS];

#[derive(Clone, Copy)]
struct Format {
    fraction: u32,
    bias: i32,
    exponent_mask: u64,
    sign: u64,
    mask: u64,
}
impl Format {
    fn new(double: bool) -> Self {
        if double {
            Self {
                fraction: 52,
                bias: 1023,
                exponent_mask: 2047,
                sign: 1 << 63,
                mask: u64::MAX,
            }
        } else {
            Self {
                fraction: 23,
                bias: 127,
                exponent_mask: 255,
                sign: 1 << 31,
                mask: 0xffff_ffff,
            }
        }
    }
    fn infinity(self) -> u64 {
        self.exponent_mask << self.fraction
    }
    fn quantum(self) -> i32 {
        1 - self.bias - self.fraction as i32
    }
    fn decode(self, raw: u64, daz: bool) -> Number {
        let raw = raw & self.mask;
        let fraction = raw & ((1_u64 << self.fraction) - 1);
        let exponent = (raw >> self.fraction) & self.exponent_mask;
        let subnormal = exponent == 0 && fraction != 0;
        let raw = if daz && subnormal {
            raw & self.sign
        } else {
            raw
        };
        Number {
            raw,
            negative: raw & self.sign != 0,
            nan: exponent == self.exponent_mask && fraction != 0,
            infinity: exponent == self.exponent_mask && fraction == 0,
            denormal: subnormal && !daz,
            significand: if daz && subnormal {
                0
            } else {
                fraction | if exponent != 0 { 1 << self.fraction } else { 0 }
            },
            exponent: if exponent == 0 {
                self.quantum()
            } else {
                exponent as i32 - self.bias - self.fraction as i32
            },
        }
    }
}
#[derive(Clone, Copy)]
struct Number {
    raw: u64,
    negative: bool,
    nan: bool,
    infinity: bool,
    denormal: bool,
    significand: u64,
    exponent: i32,
}

fn shifted(value: u128, shift: u32) -> Wide {
    let mut result = [0; LIMBS];
    for (index, value) in [value as u64, (value >> 64) as u64].into_iter().enumerate() {
        if value == 0 {
            continue;
        }
        let word = shift as usize / 64 + index;
        let bits = shift % 64;
        result[word] |= value << bits;
        if bits != 0 {
            result[word + 1] |= value >> (64 - bits);
        }
    }
    result
}
fn add(left: &mut Wide, right: &Wide) {
    let mut carry = false;
    for (left, right) in left.iter_mut().zip(right) {
        let (sum, a) = left.overflowing_add(*right);
        let (sum, b) = sum.overflowing_add(u64::from(carry));
        *left = sum;
        carry = a || b;
    }
    debug_assert!(!carry);
}
fn subtract(left: &mut Wide, right: &Wide) {
    let mut borrow = false;
    for (left, right) in left.iter_mut().zip(right) {
        let (difference, a) = left.overflowing_sub(*right);
        let (difference, b) = difference.overflowing_sub(u64::from(borrow));
        *left = difference;
        borrow = a || b;
    }
    debug_assert!(!borrow);
}
fn low_nonzero(value: &Wide, bits: u32) -> bool {
    let word = bits as usize / 64;
    let tail = bits % 64;
    value[..word].iter().any(|v| *v != 0) || tail != 0 && value[word] & ((1_u64 << tail) - 1) != 0
}
fn bit(value: &Wide, index: u32) -> bool {
    value[index as usize / 64] & (1 << (index % 64)) != 0
}
fn high(value: &Wide, shift: u32) -> u64 {
    let word = shift as usize / 64;
    let bits = shift % 64;
    let mut result = value[word] >> bits;
    if bits != 0 && word + 1 < LIMBS {
        result |= value[word + 1] << (64 - bits);
    }
    result
}

/// Algebraic x*y+z, with independent product/addend negation. NaN selection
/// precedes sign changes. The existing backend admits masked MXCSR only.
pub(crate) fn calculate(
    double: bool,
    x: u64,
    y: u64,
    z: u64,
    negative_product: bool,
    negative_addend: bool,
    mxcsr: u32,
) -> Result<(u64, u32), NativeDispatchError> {
    if !super::super::mxcsr_control::valid(mxcsr) {
        return Err(NativeDispatchError::InvalidState);
    }
    let format = Format::new(double);
    let [x, y, z] = [x, y, z].map(|raw| format.decode(raw, mxcsr & 64 != 0));
    let quiet = 1_u64 << (format.fraction - 1);
    let mut flags = 0;
    if [x, y, z].iter().any(|n| n.nan && n.raw & quiet == 0) {
        flags |= 1;
    }
    if let Some(nan) = [x, y, z].iter().find(|n| n.nan) {
        return Ok((nan.raw | quiet, mxcsr | flags));
    }
    let product_sign = x.negative ^ y.negative ^ negative_product;
    let addend_sign = z.negative ^ negative_addend;
    let product_infinite = x.infinity || y.infinity;
    let invalid = x.infinity && !y.infinity && y.significand == 0
        || y.infinity && !x.infinity && x.significand == 0
        || product_infinite && z.infinity && product_sign != addend_sign;
    if invalid {
        return Ok((format.sign | format.infinity() | quiet, mxcsr | 1));
    }
    if [x, y, z].iter().any(|n| n.denormal) {
        flags |= 2;
    }
    if product_infinite || z.infinity {
        let negative = if product_infinite {
            product_sign
        } else {
            addend_sign
        };
        return Ok((
            format.infinity() | if negative { format.sign } else { 0 },
            mxcsr | flags,
        ));
    }
    let base = 2 * format.quantum();
    let mut product = shifted(
        u128::from(x.significand) * u128::from(y.significand),
        (x.exponent + y.exponent - base) as u32,
    );
    let mut addend = shifted(u128::from(z.significand), (z.exponent - base) as u32);
    let round = (mxcsr >> 13) & 3;
    let negative = if product_sign == addend_sign {
        add(&mut product, &addend);
        product_sign
    } else {
        match product.iter().rev().cmp(addend.iter().rev()) {
            Ordering::Greater => {
                subtract(&mut product, &addend);
                product_sign
            }
            Ordering::Less => {
                subtract(&mut addend, &product);
                product = addend;
                addend_sign
            }
            Ordering::Equal => {
                return Ok((if round == 1 { format.sign } else { 0 }, mxcsr | flags));
            }
        }
    };
    let sign = if negative { format.sign } else { 0 };
    let Some(last) = product.iter().rposition(|v| *v != 0) else {
        return Ok((sign, mxcsr | flags));
    };
    let highest = last as u32 * 64 + 63 - product[last].leading_zeros();
    // Intel tests tininess after rounding to the destination precision with an
    // unbounded exponent (SDM1 4.9.1.5). This is distinct from both the exact
    // product's exponent and the later rounding to the bounded subnormal grid.
    // In particular, a value may round normal here, or remain tiny here and
    // only round up to minimum-normal on the coarser subnormal grid below.
    let precision_shift = highest.saturating_sub(format.fraction);
    let precision = high(&product, precision_shift);
    let precision_inexact = low_nonzero(&product, precision_shift);
    let precision_increment = match round {
        0 => {
            precision_shift != 0
                && bit(&product, precision_shift - 1)
                && (low_nonzero(&product, precision_shift - 1) || precision & 1 != 0)
        }
        1 => negative && precision_inexact,
        2 => !negative && precision_inexact,
        _ => false,
    };
    let rounded_highest = highest
        + u32::from(precision + u64::from(precision_increment) == 1 << (format.fraction + 1));
    let tiny = rounded_highest as i32 + base < 1 - format.bias;
    let mut shift = (highest as i32 - format.fraction as i32).max(format.quantum() - base) as u32;
    let mut significand = high(&product, shift);
    let inexact = low_nonzero(&product, shift);
    let increment = match round {
        0 => bit(&product, shift - 1) && (low_nonzero(&product, shift - 1) || significand & 1 != 0),
        1 => negative && inexact,
        2 => !negative && inexact,
        _ => false,
    };
    significand += u64::from(increment);
    if significand == 1 << (format.fraction + 1) {
        significand >>= 1;
        shift += 1;
    }
    let exponent = if significand >= 1 << format.fraction {
        base + shift as i32 + format.fraction as i32 + format.bias
    } else {
        0
    };
    if exponent >= format.exponent_mask as i32 {
        let infinite = round == 0 || round == 1 && negative || round == 2 && !negative;
        return Ok((
            sign | if infinite {
                format.infinity()
            } else {
                format.infinity() - 1
            },
            mxcsr | flags | 40,
        ));
    }
    if inexact {
        flags |= 32;
        if tiny {
            flags |= 16;
        }
    }
    if tiny && mxcsr & 0x8000 != 0 {
        return Ok((sign, mxcsr | flags | 48));
    }
    let fraction = significand & ((1_u64 << format.fraction) - 1);
    Ok((
        sign | ((exponent as u64) << format.fraction) | fraction,
        mxcsr | flags,
    ))
}
