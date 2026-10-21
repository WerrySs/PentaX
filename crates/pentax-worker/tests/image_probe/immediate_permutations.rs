// SPDX-License-Identifier: GPL-2.0-only
//! Original immediate permutations consumed by subsequent guest loads/branches.

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
fn permutation(p: &mut Program, kind: usize, destination: u8, memory: bool, immediate: u8) {
    p.code.extend([
        0xc4,
        if memory { 0x63 } else { 0x43 },
        if kind < 2 { 0x0d } else { 0xfd },
        [0x06, 0x46, 0x01, 0x00][kind],
    ]);
    if memory {
        p.code.extend([0x44 | ((destination & 7) << 3), 0x24, 48]);
    } else {
        p.code.push(0xc5 | ((destination & 7) << 3));
    }
    p.code.push(immediate);
}

#[test]
fn native_immediate_permutations_continue_with_both_aliases_zero_overrides_and_full_results() {
    let a = [
        0x1122_3344_5566_7788,
        0x7ff0_0000_0000_0001,
        0x99aa_bbcc_ddee_ff00,
        0x8000_0000_7fc0_1234,
    ];
    let b = [
        0xfedc_ba98_7654_3210,
        0x7ff8_0000_0000_1234,
        0x0123_4567_89ab_cdef,
        0x8000_0000_0000_0000,
    ];
    let mut p = Program::new();
    for (index, value) in a.into_iter().chain(b).enumerate() {
        set_word(&mut p, 16 + index as u8 * 8, value);
    }
    set_word(&mut p, 112, 0x1f84);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 112]);
    for kind in 0..4 {
        let lane_cases = [
            // Bits2/6 ignored; destination aliases the first input.
            (false, 14, 0x65, [a[2], a[3], b[0], b[1]]),
            // Destination aliases second input, swapping its128-bit halves.
            (false, 13, 0x23, [b[2], b[3], b[0], b[1]]),
            // Zero overrides both selections but still reads the full m256.
            (true, 14, 0xec, [0; 4]),
        ];
        let qword_cases = [
            (false, 13, 0x1b, [b[3], b[2], b[1], b[0]]),
            (true, 15, 0x55, [b[1]; 4]),
        ];
        let cases: &[_] = if kind < 2 { &lane_cases } else { &qword_cases };
        for &(memory, destination, immediate, expected) in cases {
            p.code.extend([0xc5, 0x7e, 0x6f, 0x74, 0x24, 16]); // YMM14 = first input.
            p.code.extend([0xc5, 0x7e, 0x6f, 0x6c, 0x24, 48]); // YMM13 = second input.
            flags(&mut p);
            permutation(&mut p, kind, destination, memory, immediate);
            check_flags(&mut p);
            p.code
                .extend([0xc5, 0x7e, 0x7f, 0x44 | ((destination & 7) << 3), 0x24, 80]);
            for (index, value) in expected.into_iter().enumerate() {
                p.word(80 + index as u8 * 8, value);
            }
        }
    }
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 112]);
    p.word(112, 0x1f84);
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0xc00); // Raw image starts at0x100; no import/GOT tables.
    let original = input(&p.code, false);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_modeled_vector_counters(&fields, [(42, 1096), (12, 136), (12, 136)]);
    assert_eq!(fields["fp_status"], "4", "{text}");
    assert_eq!(fields["service_stops"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
}
