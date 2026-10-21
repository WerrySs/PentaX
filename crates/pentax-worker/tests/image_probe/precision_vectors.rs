// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn native_precision_scalar_conversions_merge_sources_and_clear_upper_vectors() {
    let mut p = Program::new();
    for (offset, value) in [
        (16, 0x3ff0_0000_1000_0000),
        (32, 0x1122_3344_5566_7788),
        (40, 0x99aa_bbcc_ddee_ff00),
        (64, u64::MAX),
        (72, u64::MAX),
        (80, u64::MAX),
        (88, u64::MAX),
    ] {
        p.reg(0, value);
        p.save(offset);
    }
    p.code.extend([0xc5, 0xfa, 0x6f, 0x4c, 0x24, 32]); // Merge XMM1.
    p.code.extend([0xc5, 0xfe, 0x6f, 0x44, 0x24, 64]); // Nonzero YMM0.
    p.code.extend([0xc4, 0xe1, 0x73, 0x5a, 0x44, 0x24, 16]);
    p.code.extend([0xc5, 0xfe, 0x7f, 0x44, 0x24, 64]);
    for (offset, value) in [
        (64, 0x1122_3344_3f80_0000),
        (72, 0x99aa_bbcc_ddee_ff00),
        (80, 0),
        (88, 0),
    ] {
        p.word(offset, value);
    }
    p.code.extend([0xc5, 0xf2, 0x5a, 0xc8]); // VCVTSS2SD XMM1,XMM1,XMM0.
    p.code.extend([0xc5, 0xfe, 0x7f, 0x4c, 0x24, 96]);
    for (offset, value) in [
        (96, 0x3ff0_0000_0000_0000),
        (104, 0x99aa_bbcc_ddee_ff00),
        (112, 0),
        (120, 0),
    ] {
        p.word(offset, value);
    }
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["fp_status"], "32");
    assert_modeled_vector_counters(&fields, [(6, 120), (2, 8), (2, 8)]);
}

#[test]
fn native_precision_packed_conversions_capture_aliased_sources_and_zero_unused_lanes() {
    let singles = [0x3fc0_0000_u32, 0xc010_0000, 0x40e0_0000, 0x4110_0000];
    let doubles = [
        0x3ff8_0000_0000_0000_u64,
        0xc002_0000_0000_0000,
        0x401c_0000_0000_0000,
        0x4022_0000_0000_0000,
    ];
    let mut p = Program::new();
    for widening in [false, true] {
        for wide in [false, true] {
            for index in 0..4 {
                let value = if widening {
                    if index < 2 {
                        u64::from(singles[index * 2]) | (u64::from(singles[index * 2 + 1]) << 32)
                    } else {
                        0
                    }
                } else {
                    doubles[index]
                };
                p.reg(0, value);
                p.save(index as u8 * 8);
            }
            p.code.extend([0xc5, 0xfe, 0x6f, 0x0c, 0x24]);
            p.code.extend([
                0xc5,
                0xf8 | u8::from(wide) << 2 | u8::from(!widening),
                0x5a,
                0xc9,
            ]);
            p.code.extend([0xc5, 0xfe, 0x7f, 0x4c, 0x24, 64]);
            let mut expected = [0; 4];
            if widening {
                expected[..if wide { 4 } else { 2 }]
                    .copy_from_slice(&doubles[..if wide { 4 } else { 2 }]);
            } else {
                expected[0] = u64::from(singles[0]) | (u64::from(singles[1]) << 32);
                if wide {
                    expected[1] = u64::from(singles[2]) | (u64::from(singles[3]) << 32);
                }
            }
            for (index, value) in expected.into_iter().enumerate() {
                p.word(64 + index as u8 * 8, value);
            }
        }
    }
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["fp_status"], "0");
    assert_modeled_vector_counters(&fields, [(12, 256), (4, 0), (4, 0)]);
}
