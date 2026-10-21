// SPDX-License-Identifier: GPL-2.0-only
//! Original real native continuations consume all ten modeled ROUND forms.
use super::*;

fn word(p: &mut Program, offset: u8, value: u64) {
    p.reg(0, value);
    p.save(offset);
}
fn control(p: &mut Program, value: u64) {
    word(p, 0, value);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]);
}
fn vector(p: &mut Program, register: u8, offset: u8, store: bool) {
    p.code.extend([
        0xc5,
        0x7e,
        if store { 0x7f } else { 0x6f },
        0x44 | ((register & 7) << 3),
        0x24,
        offset,
    ]);
}
fn instruction(p: &mut Program, form: usize, memory: bool, offset: u8) {
    let opcode = match form {
        0 | 4 | 5 => 8,
        1 | 6 | 7 => 9,
        2 | 8 => 10,
        _ => 11,
    };
    if form < 4 {
        p.code
            .extend([0x66, if memory { 0x44 } else { 0x45 }, 0x0f, 0x3a, opcode]);
    } else {
        p.code.extend([
            0xc4,
            if memory { 0x63 } else { 0x43 },
            if form >= 8 {
                0x09
            } else if matches!(form, 5 | 7) {
                0x7d
            } else {
                0x79
            },
            opcode,
        ]);
    }
    if memory {
        p.code.extend([0x7c, 0x24, offset]);
    } else {
        p.code.push(0xfd);
    }
    p.code.push(0x84); // MXCSR RC, immediate low2 deliberately disagree.
}

#[test]
fn native_rounding_forms_preserve_signed_results_sticky_status_scalar_merges_and_all_upper_rules() {
    let mut p = Program::new();
    p.reg(8, 0x9b67_d1e5_c38a_f024);
    let merge = [
        0x0123_4567_89ab_cdef_u64,
        0x7ff0_0000_0000_0001,
        0xa1b2_c3d4_e5f6_0718,
        0x2839_4a5b_6c7d_8e9f,
    ];
    let mut transferred = 0;
    for form in 0..10 {
        let double = matches!(form, 1 | 3 | 6 | 7 | 9);
        let scalar = matches!(form, 2 | 3 | 8 | 9);
        let wide = matches!(form, 5 | 7);
        let memory = form % 2 != 0;
        let offset = if form >= 4 && memory { 15 } else { 16 };
        let input = if double {
            [
                0x3ff8_0000_0000_0000_u64,
                0x7ff0_0000_0000_0123,
                0xc004_0000_0000_0000,
                0xbfe0_0000_0000_0000,
            ]
        } else {
            [
                0xc020_0000_3fc0_0000,
                0xbf00_0000_7f80_0123,
                0xbe80_0000_3f00_0000,
                0x7fc0_0456_7f80_0000,
            ]
        };
        let rounded = if double {
            [
                0x3ff0_0000_0000_0000_u64,
                0x7ff8_0000_0000_0123,
                0xc008_0000_0000_0000,
                0xbff0_0000_0000_0000,
            ]
        } else {
            [
                0xc040_0000_3f80_0000,
                0xbf80_0000_7fc0_0123,
                0xbf80_0000_0000_0000,
                0x7fc0_0456_7f80_0000,
            ]
        };
        for (index, value) in input.into_iter().enumerate() {
            word(&mut p, offset + index as u8 * 8, value);
        }
        for (index, value) in merge.into_iter().enumerate() {
            word(&mut p, 48 + index as u8 * 8, value);
        }
        vector(&mut p, 13, offset, false);
        // Seed merge and destination from one independent complete32-byte view.
        vector(&mut p, 14, 48, false);
        vector(&mut p, 15, 48, false);
        control(&mut p, 0x3f84);
        p.reg(0, 1);
        p.reg(2, 2);
        p.code.extend([0x48, 0x39, 0xd0]);
        instruction(&mut p, form, memory, offset);
        // Observe CF/PF/ZF before any expected-value comparison changes flags.
        p.code.extend([
            0x0f, 0x92, 0xc0, 0x0f, 0x9a, 0xc2, 0x0f, 0x94, 0xc1, 0x0f, 0xb6, 0xc0, 0x0f, 0xb6,
            0xd2, 0x0f, 0xb6, 0xc9, 0xc1, 0xe2, 2, 0xc1, 0xe1, 6, 0x09, 0xd0, 0x09, 0xc8,
        ]);
        p.expect(5);
        let mut expected = if scalar { merge } else { rounded };
        if scalar {
            expected[0] = if double {
                rounded[0]
            } else {
                (merge[0] & !0xffff_ffff) | (rounded[0] & 0xffff_ffff)
            };
        }
        if form < 4 {
            expected[2..].copy_from_slice(&merge[2..]);
        } else if !wide {
            expected[2..].fill(0);
        }
        vector(&mut p, 15, 80, true);
        for (index, value) in expected.into_iter().enumerate() {
            p.word(80 + index as u8 * 8, value);
        }
        vector(&mut p, 13, 80, true);
        for (index, value) in input.into_iter().enumerate() {
            p.word(80 + index as u8 * 8, value);
        }
        vector(&mut p, 14, 80, true);
        for (index, value) in merge.into_iter().enumerate() {
            p.word(80 + index as u8 * 8, value);
        }
        p.code.extend([0x0f, 0xae, 0x5c, 0x24, 0]);
        p.word(0, if scalar { 0x3fa4 } else { 0x3fa5 }); // Inactive scalar sNaN never raisesIE.
        transferred += 200
            + if memory {
                if scalar {
                    if double { 8 } else { 4 }
                } else if wide {
                    32
                } else {
                    16
                }
            } else {
                0
            };
    }
    p.code.extend([0x4c, 0x89, 0xc0]);
    p.expect(0x9b67_d1e5_c38a_f024);
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(0x100 + p.code.len() < 0x4000);
    let mut source = input(&[], false);
    source.bytes.resize(0x4000, 0);
    put(&mut source.bytes, 96, &0x4000_u64.to_le_bytes());
    put(&mut source.bytes, 0x100, &p.code);
    std::fs::write(&source.path, &source.bytes).unwrap();
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], "0", "{text}");
    // The literal total below still describes every original transfer. Only
    // the remaining mediated rounding/control bytes enter vector counters.
    assert_modeled_vector_counters(&fields, [(90, transferred), (30, 176), (30, 176)]);
    assert_eq!(transferred, 2096);
    assert_eq!(fields["fp_status"], "36", "{text}");
}
