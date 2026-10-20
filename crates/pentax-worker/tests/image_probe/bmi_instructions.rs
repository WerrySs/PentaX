// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn native_popcnt_preserves_operand_width_memory_access_and_result_flags() {
    if !std::is_x86_feature_detected!("popcnt") {
        return;
    }
    let mut p = Program::new();
    for (encoding, value, expected, zero) in [
        (
            &[0x66, 0xf3, 0x0f, 0xb8, 0xc0][..],
            0x1234_5678_9abc_def0,
            0x1234_5678_9abc_000a,
            0,
        ),
        (&[0xf3, 0x0f, 0xb8, 0xc0][..], 0xffff_ffff_0000_0000, 0, 1),
        (
            &[0xf3, 0x48, 0x0f, 0xb8, 0x44, 0x24, 0x10][..],
            0xfedc_ba98_7654_3210,
            32,
            0,
        ),
    ] {
        // Seed SF, OF, PF and CF, then leave flags intact when preparing data.
        p.code
            .extend([0xb8, 0xff, 0xff, 0xff, 0x7f, 0x83, 0xc0, 1, 0xf9]);
        p.reg(0, value);
        p.save(16);
        p.code.extend(encoding);
        p.save(24);
        // Snapshot ZF/SF/CF/OF/PF before comparisons replace them.
        p.code.extend([
            0x0f, 0x94, 0xc1, 0x0f, 0x98, 0xc2, 0x0f, 0x92, 0xc3, 0x41, 0x0f, 0x90, 0xc0, 0x41,
            0x0f, 0x9a, 0xc1,
        ]);
        for (register, flag) in [(1, zero), (2, 0), (3, 0), (8, 0), (9, 0)] {
            if register >= 8 {
                p.code.push(0x41);
            }
            p.code.extend([0x0f, 0xb6, 0xc0 | (register & 7)]);
            p.expect(flag);
        }
        p.word(24, expected);
    }
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
}

#[test]
fn native_prefixed_alignment_nops_do_not_dereference_or_require_segment_bases() {
    let mut p = Program::new();
    p.reg(0, u64::MAX);
    for segment in [0x2e, 0x36, 0x3e, 0x26, 0x64, 0x65] {
        p.code.extend([
            segment, 0x66, 0x0f, 0x1f, 0x84, 0x00, 0xff, 0xff, 0xff, 0x7f,
        ]);
    }
    p.expect(u64::MAX);
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["segment_ops"], "0");
}

#[test]
fn native_bmi_gpr_families_execute_with_both_widths_and_zero_extended_destinations() {
    if !std::is_x86_feature_detected!("bmi1") || !std::is_x86_feature_detected!("bmi2") {
        return;
    }
    for wide in [false, true] {
        let sign = if wide { 1_u64 << 63 } else { 1 << 31 };
        let mut p = Program::new();
        for (third, opcode, modrm, immediate, control, source, expected) in [
            (0xf0, 0xf2, 0xc2, false, 0x105, 0xd6, 0xd2), // ANDN
            (0xf0, 0xf7, 0xc2, false, 0x804, 0xd6, 0xd),  // BEXTR
            (0xf8, 0xf3, 0xda, false, 0, 0xd6, 2),        // BLSI
            (0xf8, 0xf3, 0xd2, false, 0, 0xd6, 3),        // BLSMSK
            (0xf8, 0xf3, 0xca, false, 0, 0xd6, 0xd4),     // BLSR
            (0xf0, 0xf5, 0xc2, false, 5, 0xd6, 0x16),     // BZHI
            (0xf3, 0xf6, 0xc2, false, 0, 0xd6, 0),        // MULX high
            (0xf3, 0xf5, 0xc2, false, 0xb, 0xd4, 0x94),   // PDEP
            (0xf2, 0xf5, 0xc2, false, 0xb4, 0xd4, 0xb),   // PEXT
            (
                0xfb,
                0xf0,
                0xc2,
                true,
                0,
                0xd6,
                if wide {
                    0xac00_0000_0000_0001
                } else {
                    0xac00_0001
                },
            ), // RORX
            (
                0xf2,
                0xf7,
                0xc2,
                false,
                5,
                sign | 0xd6,
                if wide {
                    0xfc00_0000_0000_0006
                } else {
                    0xfc00_0006
                },
            ), // SARX
            (0xf1, 0xf7, 0xc2, false, 5, 0xd6, 0x1ac0),   // SHLX
            (0xf3, 0xf7, 0xc2, false, 5, sign | 0xd6, (sign >> 5) | 6), // SHRX
        ] {
            p.reg(0, u64::MAX);
            p.reg(1, control);
            p.reg(2, source);
            p.code.extend([
                0xc4,
                if immediate { 0xe3 } else { 0xe2 },
                if wide { third } else { third & 0x7f },
                opcode,
                modrm,
            ]);
            if immediate {
                p.code.push(7);
            }
            p.expect(expected);
            if opcode == 0xf6 {
                p.code.extend([0x48, 0x89, 0xc8]); // MOV RAX,RCX: low product.
                p.expect(0xb2e4);
            }
        }
        let (code, _) = p.finish(None);
        let (status, fields, text) = probe(&input(&code, false));
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
    }
}

#[test]
fn native_bmi_andn_memory_aliases_and_flags_survive_checked_block_boundaries() {
    if !std::is_x86_feature_detected!("bmi1") {
        return;
    }
    for wide in [false, true] {
        let mut p = Program::new();
        for (left, right, expected, flags) in [
            (u64::MAX, u64::MAX, 0, [1, 0, 0, 0]),
            (
                0,
                u64::MAX,
                if wide { u64::MAX } else { u32::MAX as u64 },
                [0, 1, 0, 0],
            ),
        ] {
            p.reg(0, right);
            p.save(16);
            p.reg(0, left); // Destination aliases the inverted source.
            p.code.extend([0xf9]); // STC, must be cleared.
            p.code.extend([
                0xc4,
                0xe2,
                if wide { 0xf8 } else { 0x78 },
                0xf2,
                0x44,
                0x24,
                16,
            ]);
            p.save(24);
            // Capture ZF/SF/CF/OF before any comparison changes flags.
            p.code.extend([
                0x0f, 0x94, 0xc1, 0x0f, 0x98, 0xc2, 0x0f, 0x92, 0xc3, 0x41, 0x0f, 0x90, 0xc0,
            ]);
            for (register, expected) in [1, 2, 3, 8].into_iter().zip(flags) {
                if register == 8 {
                    p.code.push(0x41);
                }
                p.code.extend([0x0f, 0xb6, 0xc0 | (register & 7)]);
                p.expect(expected);
            }
            p.word(24, expected);
        }
        let (code, _) = p.finish(None);
        let (status, fields, text) = probe(&input(&code, false));
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
    }
    // The new encoding must still fail before dereferencing an unowned span.
    let code = [0x31, 0xd2, 0xc4, 0xe2, 0xf0, 0xf2, 0x02, 0xc3];
    let (status, fields, text) = probe(&input(&code, false));
    assert_ne!(status, 0, "{text}");
    assert!(!fields.contains_key("value"));
    assert_eq!(fields["native_instructions"], "1");
}
