// SPDX-License-Identifier: GPL-2.0-only
//! Original VEX mask consumers continue to checked integer branches and stores.

use super::*;

fn set_word(p: &mut Program, offset: u8, value: u64) {
    p.reg(0, value);
    p.save(offset);
}
fn comparison_flags(p: &mut Program) {
    p.reg(0, 1);
    p.reg(2, 2);
    p.code.extend([0x48, 0x39, 0xd0]); // CF/PF set, ZF clear before VCMP.
}
fn check_flags(p: &mut Program) {
    p.code
        .extend([0x0f, 0x92, 0xc0, 0x0f, 0x9a, 0xc2, 0x0f, 0x94, 0xc1]);
    p.code
        .extend([0x0f, 0xb6, 0xc0, 0x0f, 0xb6, 0xd2, 0x0f, 0xb6, 0xc9]);
    p.code
        .extend([0xc1, 0xe2, 2, 0xc1, 0xe1, 6, 0x09, 0xd0, 0x09, 0xc8]);
    p.expect(5);
}
fn control(p: &mut Program, value: u64) {
    set_word(p, 112, value);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 112]);
}
fn check_control(p: &mut Program, value: u64) {
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 112]);
    p.word(112, value);
}
fn check(mut p: Program, fp_status: u32) {
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    // This no-import original image begins at0x100 and has no GOT. Keeping
    // code below0xc00 leaves its complete source inside the initialized page.
    assert!(p.code.len() < 0xc00);
    let source = input(&p.code, false);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["fp_status"], fp_status.to_string(), "{text}");
    assert_eq!(fields["service_stops"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
}

#[test]
fn native_packed_mask_comparisons_capture_aliases_keep_flags_and_combine_cross_lane_status() {
    let mut p = Program::new();
    for double in [false, true] {
        for wide in [false, true] {
            let (left, right) = if double {
                (
                    [
                        0x7ff8_0000_0000_0123,
                        1,
                        0x3ff0_0000_0000_0000,
                        0x4000_0000_0000_0000,
                    ],
                    [1, 0, 0x4000_0000_0000_0000, 0x3ff0_0000_0000_0000],
                )
            } else {
                (
                    [
                        0x0000_0001_7fc0_0123,
                        0x4000_0000_3f80_0000,
                        0x0000_0001_7fc0_0123,
                        0x4000_0000_3f80_0000,
                    ],
                    [1, 0x3f80_0000_4000_0000, 1, 0x3f80_0000_4000_0000],
                )
            };
            for index in 0..4 {
                set_word(&mut p, 16 + index * 8, left[index as usize]);
                set_word(&mut p, 48 + index * 8, right[index as usize]);
            }
            p.code.extend([0xc5, 0xfe, 0x6f, 0x4c, 0x24, 16]);
            p.code.extend([0xc5, 0xfe, 0x6f, 0x54, 0x24, 48]);
            control(&mut p, 0x1f84);
            comparison_flags(&mut p);
            let destination = if wide { 2 } else { 1 };
            p.code.extend([
                0xc5,
                0xf0 | u8::from(double) | if wide { 4 } else { 0 },
                0xc2,
            ]);
            if wide {
                p.code.push(0xc2 | (destination << 3));
            } else {
                p.code.extend([0x44 | (destination << 3), 0x24, 48]);
            }
            p.code.push(0xee); // GT_OS, with all reserved immediate bits set.
            check_flags(&mut p);
            p.code
                .extend([0xc5, 0xfe, 0x7f, 0x44 | (destination << 3), 0x24, 80]);
            let expected = if double {
                [0, u64::MAX, 0, u64::MAX]
            } else {
                [0xffff_ffff_0000_0000; 4]
            };
            for index in 0..4 {
                p.word(
                    80 + index * 8,
                    if index >= 2 && !wide {
                        0
                    } else {
                        expected[index as usize]
                    },
                );
            }
            check_control(&mut p, 0x1f87); // Existing ZE plus IE and separate-lane DE.
        }
    }
    check(p, 7);
}

#[test]
fn native_scalar_mask_comparisons_merge_source_one_zero_upper_and_ignore_inactive_nan_lanes() {
    let mut p = Program::new();
    for double in [false, true] {
        for daz in [false, true] {
            let first = if double { 1 } else { 0x7f80_0001_0000_0001 };
            let second = if double {
                0x7ff0_0000_0000_0001
            } else {
                0x1122_3344_5566_7788
            };
            for (offset, value) in [
                (16, first),
                (24, second),
                (32, 0x99aa_bbcc_ddee_ff00),
                (40, 0x1234_5678_9abc_def0),
                (48, 0),
                (56, 0xa5a5_a5a5_a5a5_a5a5),
                (64, 0x5a5a_5a5a_5a5a_5a5a),
                (72, u64::MAX),
            ] {
                set_word(&mut p, offset, value);
            }
            p.code.extend([0xc5, 0xfe, 0x6f, 0x4c, 0x24, 16]);
            p.code.extend([0xc5, 0xfe, 0x6f, 0x44, 0x24, 48]);
            let mxcsr = 0x1f84 | if daz { 0x40 } else { 0 };
            control(&mut p, mxcsr);
            comparison_flags(&mut p);
            p.code
                .extend([0xc5, 0xf2 | u8::from(double), 0xc2, 0x44, 0x24, 48, 0xfe]); // GT_OQ scalar memory form.
            check_flags(&mut p);
            p.code.extend([0xc5, 0xfe, 0x7f, 0x44, 0x24, 80]);
            let mask = if daz { 0 } else { u64::MAX };
            p.word(
                80,
                if double {
                    mask
                } else {
                    (first & 0xffff_ffff_0000_0000) | (mask & 0xffff_ffff)
                },
            );
            p.word(88, second);
            p.word(96, 0);
            p.word(104, 0);
            check_control(&mut p, mxcsr | if daz { 0 } else { 2 });
        }
    }
    check(p, 4);
}
