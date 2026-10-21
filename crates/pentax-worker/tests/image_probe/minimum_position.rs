// SPDX-License-Identifier: GPL-2.0-only
//! Original unsigned minima consumed by subsequent guest branches and loads.

use super::*;

fn set_word(p: &mut Program, offset: u8, value: u64) {
    p.reg(0, value);
    p.save(offset);
}
fn flags(p: &mut Program) {
    p.reg(0, 1);
    p.reg(2, 2);
    p.code.extend([0x48, 0x39, 0xd0]);
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

#[test]
fn native_minimum_position_consumes_unsigned_ties_aliases_and_both_upper_rules() {
    let upper = [0x1122_3344_5566_7788, 0x99aa_bbcc_ddee_ff00];
    let mut p = Program::new();
    set_word(&mut p, 48, 0x1f84);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 48]);
    for vex in [false, true] {
        for (case, low, high, expected) in [
            (0, u64::MAX, u64::MAX, 0x0000_ffff),
            (1, 0xffff_7fff_8000_ffff, 0x8123_fffe_7ffe_8001, 0x0005_7ffe),
            (2, 0xffff_0000_ffff_ffff, 0xffff_ffff_0000_ffff, 0x0002_0000),
        ] {
            for (index, value) in [low, high, upper[0], upper[1]].into_iter().enumerate() {
                set_word(&mut p, 16 + index as u8 * 8, value);
            }
            p.code.extend([0xc5, 0x7e, 0x6f, 0x6c, 0x24, 16]); // YMM13 source/alias.
            p.code.extend([0xc5, 0x7e, 0x6f, 0x4c, 0x24, 16]); // Nonzero YMM9 upper half.
            let memory = case == 2;
            let destination = if case == 0 { 13 } else { 9 };
            let offset = if vex { 97 } else { 96 };
            if memory {
                set_word(&mut p, offset, low);
                set_word(&mut p, offset + 8, high);
            }
            flags(&mut p);
            if vex {
                p.code.extend([
                    0xc4,
                    if memory { 0x62 } else { 0x42 },
                    if case == 1 { 0xf9 } else { 0x79 },
                    0x41,
                ]); // WIG also exercises W1.
            } else {
                p.code
                    .extend([0x66, if memory { 0x44 } else { 0x45 }, 0x0f, 0x38, 0x41]);
            }
            if memory {
                p.code
                    .extend([0x44 | ((destination & 7) << 3), 0x24, offset]);
            } else {
                p.code.push(0xc5 | ((destination & 7) << 3));
            }
            check_flags(&mut p);
            p.code
                .extend([0xc5, 0x7e, 0x7f, 0x44 | ((destination & 7) << 3), 0x24, 64]);
            p.word(64, expected);
            p.word(72, 0);
            p.word(80, if vex { 0 } else { upper[0] });
            p.word(88, if vex { 0 } else { upper[1] });
        }
    }
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 48]);
    p.word(48, 0x1f84);
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0xc00); // Original raw image; no import/GOT tables.
    let original = input(&p.code, false);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_modeled_vector_counters(&fields, [(26, 616), (8, 40), (8, 40)]);
    assert_eq!(fields["fp_status"], "4", "{text}");
    assert_eq!(fields["service_stops"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
}
