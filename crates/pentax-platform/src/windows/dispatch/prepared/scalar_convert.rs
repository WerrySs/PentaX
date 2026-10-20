// SPDX-License-Identifier: GPL-2.0-only

//! Scalar IEEE binary32/binary64 and signed integer conversions using integers.
//! The admitted CPU profile masks SIMD exceptions; guest sticky flags are data.

// Intel SDM CVT(T)SS/SD2SI: conversion has invalid/precision, never a denormal
// exception. DAZ makes denormal inputs signed zero; FTZ has no integer effect.
pub(super) fn to_signed_bits(
    bits: u64,
    double: bool,
    wide: bool,
    rounding: u32,
    daz: bool,
) -> (u64, u32) {
    let (fraction, bias, exponent_mask, sign_shift) = if double {
        (52, 1023, 0x7ff, 63)
    } else {
        (23, 127, 0xff, 31)
    };
    let negative = bits >> sign_shift & 1 != 0;
    let raw_exponent = bits >> fraction & exponent_mask;
    let fraction_bits = bits & ((1_u64 << fraction) - 1);
    let indefinite = 1_u64 << if wide { 63 } else { 31 };
    let invalid = (indefinite, 1);
    if raw_exponent == exponent_mask {
        return invalid;
    }
    if raw_exponent == 0 && (fraction_bits == 0 || daz) {
        return (0, 0);
    }
    let exponent = if raw_exponent == 0 {
        1 - bias
    } else {
        raw_exponent as i32 - bias
    };
    if exponent > if wide { 63 } else { 31 } {
        return invalid;
    }
    let significand = fraction_bits | if raw_exponent == 0 { 0 } else { 1 << fraction };
    let (magnitude, inexact, above_half, tie) = if exponent >= fraction {
        (significand << (exponent - fraction), false, false, false)
    } else {
        let shift = (fraction - exponent) as u32;
        if shift >= 64 {
            // The significand has at most53bits: it is strictly below half.
            (0, true, false, false)
        } else {
            let remainder = significand & ((1_u64 << shift) - 1);
            let half = 1_u64 << (shift - 1);
            (
                significand >> shift,
                remainder != 0,
                remainder > half,
                remainder == half,
            )
        }
    };
    let increment = match rounding & 3 {
        0 => above_half || tie && magnitude & 1 != 0,
        1 => negative && inexact,
        2 => !negative && inexact,
        _ => false,
    };
    let magnitude = magnitude + u64::from(increment);
    let maximum = indefinite - u64::from(!negative);
    if magnitude > maximum {
        return invalid;
    } // Invalid suppresses precision.
    let signed = if negative {
        0_u64.wrapping_sub(magnitude)
    } else {
        magnitude
    };
    (
        if wide {
            signed
        } else {
            u64::from(signed as u32)
        },
        u32::from(inexact) << 5,
    )
}

pub(super) fn signed_bits(value: i64, double: bool, rounding: u32) -> (u64, bool) {
    let magnitude = value.unsigned_abs();
    if magnitude == 0 {
        return (0, false);
    }
    let (fraction, bias, sign_shift) = if double {
        (52, 1023, 63)
    } else {
        (23, 127, 31)
    };
    let mut exponent = 63 - magnitude.leading_zeros();
    let mut inexact = false;
    let mut significand = if exponent <= fraction {
        magnitude << (fraction - exponent)
    } else {
        let shift = exponent - fraction;
        let mut truncated = magnitude >> shift;
        let remainder = magnitude & ((1_u64 << shift) - 1);
        inexact = remainder != 0;
        let increment = match rounding & 3 {
            0 => {
                let halfway = 1_u64 << (shift - 1);
                remainder > halfway || remainder == halfway && truncated & 1 != 0
            }
            1 => value < 0 && inexact,
            2 => value > 0 && inexact,
            _ => false,
        };
        truncated += u64::from(increment);
        truncated
    };
    if significand == 1_u64 << (fraction + 1) {
        significand >>= 1;
        exponent += 1;
    }
    let bits = (u64::from(value < 0) << sign_shift)
        | (u64::from(exponent + bias) << fraction)
        | (significand & ((1_u64 << fraction) - 1));
    (bits, inexact)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_to_signed_matches_an_independent_numeric_oracle_at_ieee_and_integer_boundaries() {
        for double in [false, true] {
            let values: [f64; 20] = [
                0.0,
                -0.0,
                1.0,
                -1.0,
                0.5,
                -0.5,
                1.5,
                -1.5,
                2.5,
                -2.5,
                2_147_483_520.0,
                2_147_483_647.0,
                2_147_483_647.5,
                2_147_483_648.0,
                -2_147_483_648.0,
                -2_147_483_648.75,
                9_223_372_036_854_775_808.0,
                -9_223_372_036_854_775_808.0,
                f64::MAX,
                -f64::MAX,
            ];
            let mut inputs: Vec<_> = values
                .into_iter()
                .map(|value| {
                    if double {
                        value.to_bits()
                    } else {
                        u64::from((value as f32).to_bits())
                    }
                })
                .collect();
            if double {
                inputs.extend([
                    1,
                    0x8000_0000_0000_0001,
                    0x000f_ffff_ffff_ffff,
                    0x0010_0000_0000_0000,
                    0x43df_ffff_ffff_ffff,
                    0xc3e0_0000_0000_0001,
                    0x7ff0_0000_0000_0000,
                    0xfff0_0000_0000_0000,
                    0x7ff8_0000_0000_0001,
                    0x7ff0_0000_0000_0001,
                ]);
            } else {
                inputs.extend([
                    1,
                    0x8000_0001,
                    0x007f_ffff,
                    0x0080_0000,
                    0x5eff_ffff,
                    0xdf00_0001,
                    0x7f80_0000,
                    0xff80_0000,
                    0x7fc0_0001,
                    0x7f80_0001,
                ]);
            }
            for bits in inputs {
                for wide in [false, true] {
                    for rounding in 0..4 {
                        for daz in [false, true] {
                            let value = if double {
                                f64::from_bits(bits)
                            } else {
                                f64::from(f32::from_bits(bits as u32))
                            };
                            let subnormal = if double {
                                f64::from_bits(bits).is_subnormal()
                            } else {
                                f32::from_bits(bits as u32).is_subnormal()
                            };
                            let value = if daz && subnormal { 0.0 } else { value };
                            let rounded = match rounding {
                                0 => value.round_ties_even(),
                                1 => value.floor(),
                                2 => value.ceil(),
                                _ => value.trunc(),
                            };
                            let boundary = if wide {
                                9_223_372_036_854_775_808.0
                            } else {
                                2_147_483_648.0
                            };
                            let expected = if !rounded.is_finite()
                                || rounded < -boundary
                                || rounded >= boundary
                            {
                                (1_u64 << if wide { 63 } else { 31 }, 1)
                            } else {
                                let signed = rounded as i64;
                                (
                                    if wide {
                                        signed as u64
                                    } else {
                                        u64::from(signed as u32)
                                    },
                                    u32::from(rounded != value) << 5,
                                )
                            };
                            assert_eq!(
                                to_signed_bits(bits, double, wide, rounding, daz),
                                expected,
                                "bits={bits:x}, double={double}, wide={wide}, rounding={rounding}, daz={daz}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn signed_conversion_rounds_halfway_signs_and_exponent_carry() {
        // Expected IEEE encodings, independent of host floating-point state.
        for (value, double, inexact, expected) in [
            (0, false, false, [0; 4]),
            (-1, false, false, [0xbf80_0000; 4]),
            (
                16_777_217,
                false,
                true,
                [0x4b80_0000, 0x4b80_0000, 0x4b80_0001, 0x4b80_0000],
            ),
            (
                16_777_219,
                false,
                true,
                [0x4b80_0002, 0x4b80_0001, 0x4b80_0002, 0x4b80_0001],
            ),
            (
                -16_777_217,
                false,
                true,
                [0xcb80_0000, 0xcb80_0001, 0xcb80_0000, 0xcb80_0000],
            ),
            (
                i64::MAX,
                false,
                true,
                [0x5f00_0000, 0x5eff_ffff, 0x5f00_0000, 0x5eff_ffff],
            ),
            (i64::MIN, false, false, [0xdf00_0000; 4]),
            (i32::MAX as i64, true, false, [0x41df_ffff_ffc0_0000; 4]),
            (
                9_007_199_254_740_993,
                true,
                true,
                [
                    0x4340_0000_0000_0000,
                    0x4340_0000_0000_0000,
                    0x4340_0000_0000_0001,
                    0x4340_0000_0000_0000,
                ],
            ),
            (
                -9_007_199_254_740_995,
                true,
                true,
                [
                    0xc340_0000_0000_0002,
                    0xc340_0000_0000_0002,
                    0xc340_0000_0000_0001,
                    0xc340_0000_0000_0001,
                ],
            ),
            (
                i64::MAX,
                true,
                true,
                [
                    0x43e0_0000_0000_0000,
                    0x43df_ffff_ffff_ffff,
                    0x43e0_0000_0000_0000,
                    0x43df_ffff_ffff_ffff,
                ],
            ),
            (i64::MIN, true, false, [0xc3e0_0000_0000_0000; 4]),
        ] {
            for (rounding, bits) in expected.into_iter().enumerate() {
                assert_eq!(signed_bits(value, double, rounding as u32), (bits, inexact));
            }
        }
    }
}
