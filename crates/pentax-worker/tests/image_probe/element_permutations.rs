// SPDX-License-Identifier: GPL-2.0-only
//! Original lane-local permutations consumed by following guest instructions.

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
fn load(p: &mut Program, register: u8, offset: u8) {
    p.code
        .extend([0xc5, 0x7e, 0x6f, 0x44 | ((register & 7) << 3), 0x24, offset]);
}

#[test]
fn native_element_permutations_consume_all_eight_forms_with_lane_isolation_and_aliases() {
    let data = [
        0x7f80_0001_1122_3344,
        0x8000_0000_99aa_bbcc,
        0x7fc0_1234_5566_7788,
        0xff80_0000_ddee_ff00,
    ];
    let ps_controls = [
        0xf135_79bc_f135_79bf,
        0xf135_79bd_f135_79be,
        0xf135_79bf_f135_79bd,
        0xf135_79be_f135_79bc,
    ];
    let pd_controls = [
        0xf135_79bd_2468_ace1,
        0xf135_79bd_2468_ace2,
        0xf135_79bd_2468_ace3,
        0xf135_79bd_2468_ace0,
    ];
    let mut p = Program::new();
    for (index, value) in data.into_iter().enumerate() {
        set_word(&mut p, 16 + index as u8 * 8, value);
    }
    set_word(&mut p, 112, 0x1f84);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 112]);
    for variable in [false, true] {
        for double in [false, true] {
            for wide in [false, true] {
                let control = if double { pd_controls } else { ps_controls };
                for (index, value) in control.into_iter().enumerate() {
                    set_word(&mut p, 48 + index as u8 * 8, value);
                }
                let memory = if variable { wide } else { double != wide };
                let destination = if variable {
                    if double || memory { 14 } else { 13 }
                } else if memory {
                    if double { 15 } else { 14 }
                } else {
                    13
                };
                load(&mut p, destination, 16); // Nonzero upper destination before XMM forms.
                if variable {
                    load(&mut p, 14, 16);
                    load(&mut p, 13, 48);
                } else {
                    load(&mut p, 13, 16);
                }
                flags(&mut p);
                p.code.extend([
                    0xc4,
                    (if memory { 0x60 } else { 0x40 }) | if variable { 2 } else { 3 },
                    (if variable { 0x09 } else { 0x79 }) | if wide { 4 } else { 0 },
                    if variable { 0x0c } else { 0x04 } + u8::from(double),
                ]);
                if memory {
                    p.code.extend([
                        0x44 | ((destination & 7) << 3),
                        0x24,
                        if variable { 48 } else { 16 },
                    ]);
                } else {
                    p.code.push(0xc5 | ((destination & 7) << 3));
                }
                if !variable {
                    p.code.push(if double { 0xf6 } else { 0x1b });
                }
                check_flags(&mut p);
                p.code
                    .extend([0xc5, 0x7e, 0x7f, 0x44 | ((destination & 7) << 3), 0x24, 80]);
                let mut expected = if double {
                    [data[0], data[1], data[3], data[2]]
                } else if variable {
                    [
                        0x1122_3344_8000_0000,
                        0x7f80_0001_99aa_bbcc,
                        0xff80_0000_7fc0_1234,
                        0xddee_ff00_5566_7788,
                    ]
                } else {
                    [
                        0x99aa_bbcc_8000_0000,
                        0x1122_3344_7f80_0001,
                        0xddee_ff00_ff80_0000,
                        0x5566_7788_7fc0_1234,
                    ]
                };
                if !wide {
                    expected[2..].fill(0);
                }
                for (index, value) in expected.into_iter().enumerate() {
                    p.word(80 + index as u8 * 8, value);
                }
            }
        }
    }
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 112]);
    p.word(112, 0x1f84);
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0xc00); // Original raw image; no import/GOT tables.
    let original = input(&p.code, false);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_modeled_vector_counters(&fields, [(38, 1016), (10, 120), (10, 120)]);
    assert_eq!(fields["fp_status"], "4", "{text}");
    assert_eq!(fields["service_stops"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
}
