// SPDX-License-Identifier: GPL-2.0-only
//! Original lane transfers consumed by subsequent guest branches and loads.

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
fn native_lane_extract_both_forms_continue_with_alias_zeroing_and_exact_unaligned_stores() {
    let source = [
        0x1122_3344_5566_7788,
        0x7ff0_0000_0000_0001,
        0x99aa_bbcc_ddee_ff00,
        0x8000_0000_7fc0_1234,
    ];
    const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
    let mut p = Program::new();
    for (index, value) in source.into_iter().enumerate() {
        set_word(&mut p, 16 + index as u8 * 8, value);
    }
    set_word(&mut p, 48, 0x1f84);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 48]); // Existing ZE remains sticky.
    for opcode in [0x19, 0x39] {
        p.code.extend([0xc5, 0x7e, 0x6f, 0x7c, 0x24, 16]); // YMM15.
        p.code.extend([0xc5, 0x7e, 0x6f, 0x4c, 0x24, 16]); // Nonzero YMM9 upper half.
        flags(&mut p);
        p.code.extend([0xc4, 0x43, 0x7d, opcode, 0xf9, 0xfe]); // Low half -> XMM9.
        check_flags(&mut p);
        p.code.extend([0xc5, 0x7e, 0x7f, 0x4c, 0x24, 80]);
        for (index, value) in [source[0], source[1], 0, 0].into_iter().enumerate() {
            p.word(80 + index as u8 * 8, value);
        }

        p.code.extend([0xc5, 0x7e, 0x6f, 0x7c, 0x24, 16]);
        flags(&mut p);
        p.code.extend([0xc4, 0x43, 0x7d, opcode, 0xff, 0xff]); // High half -> same XMM15.
        check_flags(&mut p);
        p.code.extend([0xc5, 0x7e, 0x7f, 0x7c, 0x24, 80]);
        for (index, value) in [source[2], source[3], 0, 0].into_iter().enumerate() {
            p.word(80 + index as u8 * 8, value);
        }

        for offset in [64, 72, 80, 88] {
            set_word(&mut p, offset, CANARY);
        }
        p.code.extend([0xc5, 0x7e, 0x6f, 0x7c, 0x24, 16]);
        for (immediate, a, b) in [(0xfe, source[0], source[1]), (0xff, source[2], source[3])] {
            flags(&mut p);
            // Exactly16 bytes at an unaligned address; both neighboring bytes survive.
            p.code
                .extend([0xc4, 0x63, 0x7d, opcode, 0x7c, 0x24, 65, immediate]);
            check_flags(&mut p);
            p.word(64, (a << 8) | 0xa5);
            p.word(72, (a >> 56) | (b << 8));
            p.word(80, (CANARY & !0xff) | (b >> 56));
            p.word(88, CANARY);
        }
        p.code.extend([0xc5, 0x7e, 0x7f, 0x7c, 0x24, 96]);
        for (index, value) in source.into_iter().enumerate() {
            p.word(96 + index as u8 * 8, value);
        }
        p.code.extend([0x0f, 0xae, 0x5c, 0x24, 48]);
        p.word(48, 0x1f84); // Even signaling-NaN bit patterns cause no FP exception.
    }
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0xc00); // Original raw image starts at0x100; no import/GOT tables.
    let original = input(&p.code, false);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["vector_ops"], "25", "{text}");
    assert_eq!(fields["vector_bytes"], "524", "{text}");
    assert_eq!(fields["fp_status"], "4", "{text}");
    assert_eq!(fields["service_stops"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
}
