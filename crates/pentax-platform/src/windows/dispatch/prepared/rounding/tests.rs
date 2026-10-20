// SPDX-License-Identifier: GPL-2.0-only
//! Independent literal rounding oracles, not another implementation of ROUND.
use super::*;

#[test]
fn ieee_rounding_controls_ties_zero_nan_and_sticky_status_match_literal_oracles() {
    for (double, cases) in [
        (
            false,
            [
                (
                    0x3fc0_0000,
                    [0x4000_0000, 0x3f80_0000, 0x4000_0000, 0x3f80_0000],
                ),
                (
                    0xc020_0000,
                    [0xc000_0000, 0xc040_0000, 0xc000_0000, 0xc000_0000],
                ),
                (0x3f00_0000, [0, 0, 0x3f80_0000, 0]),
                (
                    0xbf00_0000,
                    [0x8000_0000, 0xbf80_0000, 0x8000_0000, 0x8000_0000],
                ),
                (
                    0x4aff_ffff,
                    [0x4b00_0000, 0x4aff_fffe, 0x4b00_0000, 0x4aff_fffe],
                ),
            ],
        ),
        (
            true,
            [
                (
                    0x3ff8_0000_0000_0000,
                    [
                        0x4000_0000_0000_0000,
                        0x3ff0_0000_0000_0000,
                        0x4000_0000_0000_0000,
                        0x3ff0_0000_0000_0000,
                    ],
                ),
                (
                    0xc004_0000_0000_0000,
                    [
                        0xc000_0000_0000_0000,
                        0xc008_0000_0000_0000,
                        0xc000_0000_0000_0000,
                        0xc000_0000_0000_0000,
                    ],
                ),
                (0x3fe0_0000_0000_0000, [0, 0, 0x3ff0_0000_0000_0000, 0]),
                (
                    0xbfe0_0000_0000_0000,
                    [
                        0x8000_0000_0000_0000,
                        0xbff0_0000_0000_0000,
                        0x8000_0000_0000_0000,
                        0x8000_0000_0000_0000,
                    ],
                ),
                (
                    0x432f_ffff_ffff_ffff,
                    [
                        0x4330_0000_0000_0000,
                        0x432f_ffff_ffff_fffe,
                        0x4330_0000_0000_0000,
                        0x432f_ffff_ffff_fffe,
                    ],
                ),
            ],
        ),
    ] {
        for (input, results) in cases {
            for (mode, expected) in results.into_iter().enumerate() {
                let control = 0x1f84 | (((mode as u32 + 1) & 3) << 13);
                assert_eq!(
                    calculate(double, input, 0xf0 | mode as u8, control),
                    Ok((expected, control | 0x20))
                );
                assert_eq!(
                    calculate(double, input, 0xf8 | mode as u8, control),
                    Ok((expected, control))
                );
                let selected = 0x1f84 | ((mode as u32) << 13);
                assert_eq!(
                    calculate(double, input, 0xf4 | ((mode as u8 + 1) & 3), selected),
                    Ok((expected, selected | 0x20))
                );
            }
        }
        let (sign, one, infinity, snan, quiet) = if double {
            (
                1 << 63,
                0x3ff0_0000_0000_0000,
                0x7ff0_0000_0000_0000,
                0x7ff0_0000_0000_0123,
                1 << 51,
            )
        } else {
            (1 << 31, 0x3f80_0000, 0x7f80_0000, 0x7f80_0123, 1 << 22)
        };
        for immediate in 0..=15 {
            for exact in [
                0,
                sign,
                one,
                one | sign,
                infinity,
                infinity | sign,
                snan | quiet,
            ] {
                assert_eq!(
                    calculate(double, exact, immediate, 0x9fa4),
                    Ok((exact, 0x9fa4))
                );
            }
            assert_eq!(
                calculate(double, snan | sign, immediate, 0x1fa4),
                Ok((snan | sign | quiet, 0x1fa5))
            );
        }
        for (input, immediate, expected) in [(1, 2, one), (sign | 1, 1, sign | one)] {
            assert_eq!(
                calculate(double, input, immediate, 0x1f80),
                Ok((expected, 0x1fa0))
            );
            assert_eq!(
                calculate(double, input, immediate | 8, 0x1f80),
                Ok((expected, 0x1f80))
            );
            if super::super::mxcsr_control::valid(0x1fc0) {
                assert_eq!(
                    calculate(double, input, immediate, 0x1fc0),
                    Ok((input & sign, 0x1fc0))
                );
            }
        }
    }
    for invalid in [0x1f00, 0x1f80 | 0x10000] {
        assert!(matches!(
            calculate(false, 0x3fc0_0000, 0, invalid),
            Err(NativeDispatchError::InvalidState)
        ));
    }
}
