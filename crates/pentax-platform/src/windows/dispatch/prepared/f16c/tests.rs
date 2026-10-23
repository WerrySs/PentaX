// SPDX-License-Identifier: GPL-2.0-only
//! Fixed public-format literals, never a host F16C or floating-point oracle.
use super::*;

#[test]
fn half_conversion_rounding_ties_directed_overflow_and_tininess_use_original_bits_once() {
    for (bits, outputs, flags) in [
        (0x3f80_1000, [0x3c00, 0x3c00, 0x3c01, 0x3c00], [0x20; 4]),
        (0xbf80_1000, [0xbc00, 0xbc01, 0xbc00, 0xbc00], [0x20; 4]),
        (0x3f80_3000, [0x3c02, 0x3c01, 0x3c02, 0x3c01], [0x20; 4]),
        (
            0x477f_f000,
            [0x7c00, 0x7bff, 0x7c00, 0x7bff],
            [0x28, 0x20, 0x28, 0x20],
        ),
        (
            0xc77f_f000,
            [0xfc00, 0xfc00, 0xfbff, 0xfbff],
            [0x28, 0x28, 0x20, 0x20],
        ),
        (0x7f7f_ffff, [0x7c00, 0x7bff, 0x7c00, 0x7bff], [0x28; 4]),
        (0x3380_0000, [1, 1, 1, 1], [0; 4]), // exact minimum half subnormal
        (0x3300_0000, [0, 0, 1, 0], [0x30; 4]), // zero/subnormal midpoint
        (0xb300_0000, [0x8000, 0x8001, 0x8000, 0x8000], [0x30; 4]),
        (0x387f_c000, [0x03ff; 4], [0; 4]), // exact maximum half subnormal
        (0x3880_0000, [0x0400; 4], [0; 4]),
    ] {
        for rounding in 0..4 {
            for ftz in [0, 0x8000] {
                assert_eq!(
                    narrow(bits, rounding, 0x1f80 | ftz),
                    (outputs[rounding as usize], flags[rounding as usize]),
                    "bits={bits:x} RC={rounding}"
                );
            }
        }
    }
    // Both final half results round to minimum normal. Their precision-only
    // and underflow+precision outcomes distinguish the unbounded p11 probe.
    assert_eq!(narrow(0x387f_f800, 0, 0x1f80), (0x0400, 0x20));
    assert_eq!(narrow(0x387f_e800, 0, 0x1f80), (0x0400, 0x30));
    assert_eq!(narrow(0xb87f_f800, 0, 0x1f80), (0x8400, 0x20));
    assert_eq!(narrow(0xb87f_e800, 0, 0x1f80), (0x8400, 0x30));
}

#[test]
fn half_special_values_keep_sign_payload_quiet_snan_and_apply_direction_specific_denormal_rules() {
    for (bits, half, flags) in [
        (0, 0, 0),
        (0x8000_0000, 0x8000, 0),
        (0x7f80_0000, 0x7c00, 0),
        (0xff80_0000, 0xfc00, 0),
        (0x7fc0_2000, 0x7e01, 0),
        (0xffc0_2000, 0xfe01, 0),
        (0x7f80_0001, 0x7e00, 1),
        (0xffa5_4321, 0xff2a, 1),
    ] {
        assert_eq!(narrow(bits, 0, 0x1f80), (half, flags));
    }
    for (bits, single, flags) in [
        (0, 0, 0),
        (0x8000, 0x8000_0000, 0),
        (1, 0x3380_0000, 0),
        (0x8001, 0xb380_0000, 0),
        (0x03ff, 0x387f_c000, 0),
        (0x0400, 0x3880_0000, 0),
        (0x7bff, 0x477f_e000, 0),
        (0x7c00, 0x7f80_0000, 0),
        (0x7c01, 0x7fc0_2000, 1),
        (0xfc01, 0xffc0_2000, 1),
        (0x7fff, 0x7fff_e000, 0),
    ] {
        assert_eq!(widen(bits), (single, flags));
    }
    for bits in [1, 0x007f_ffff, 0x8000_0001, 0x807f_ffff] {
        let sign = (bits >> 16) as u16 & 0x8000;
        assert_eq!(narrow(bits, 0, 0x1f80), (sign, 0x32));
        assert_eq!(narrow(bits, 0, 0x1fc0), (sign, 0));
        assert_eq!(
            narrow(bits, if sign != 0 { 1 } else { 2 }, 0x1f80),
            (sign | 1, 0x32)
        );
    }
}

#[test]
fn half_packed_flags_aggregate_only_active_lanes_and_ph_expansion_ignores_daz_ftz() {
    let singles = [
        0x7f80_0001_u32,
        1,
        0x7f7f_ffff,
        0x3f80_1000,
        0,
        0x8000_0000,
        0x3880_0000,
        0x3380_0000,
    ];
    let mut input = [0; 32];
    for (lane, bits) in singles.iter().enumerate() {
        input[lane * 4..lane * 4 + 4].copy_from_slice(&bits.to_le_bytes());
    }
    let (output, flags) = convert(
        Form {
            narrow: true,
            lanes: 8,
        },
        &input,
        0,
        0x1f80,
    );
    assert_eq!(flags, 0x3b); // IE|DE|OE|UE|PE from distinct lanes
    let expected: [u16; 8] = [0x7e00, 0, 0x7c00, 0x3c00, 0, 0x8000, 0x0400, 1];
    for (lane, half) in expected.into_iter().enumerate() {
        assert_eq!(&output[lane * 2..lane * 2 + 2], &half.to_le_bytes());
    }
    let (output, flags) = convert(
        Form {
            narrow: true,
            lanes: 4,
        },
        &input,
        0,
        0x1fc0,
    );
    assert_eq!(flags, 0x29); // DAZ suppressed input DE/tiny lane, no inactive lane flags
    assert_eq!(output[8..], [0; 24]);
    let half = [1_u16, 0x8001, 0x7c01, 0x7e01];
    let mut input = [0xff; 32];
    for (lane, half) in half.into_iter().enumerate() {
        input[lane * 2..lane * 2 + 2].copy_from_slice(&half.to_le_bytes());
    }
    let baseline = convert(
        Form {
            narrow: false,
            lanes: 4,
        },
        &input,
        0,
        0x1f80,
    );
    assert_eq!(baseline.1, 1);
    assert_eq!(baseline.0[16..], [0; 16]);
    assert_eq!(
        convert(
            Form {
                narrow: false,
                lanes: 4
            },
            &input,
            3,
            0xffc0
        ),
        baseline
    );
}
