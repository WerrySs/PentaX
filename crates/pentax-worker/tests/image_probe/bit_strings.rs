// SPDX-License-Identifier: GPL-2.0-only
//! Original native bit strings branch over actual selected bits and preserved ZF.
use super::*;

fn capture_flags(p: &mut Program, carry: u64) {
    p.code
        .extend([0x41, 0x0f, 0x92, 0xc0, 0x41, 0x0f, 0x94, 0xc1]);
    p.code.extend([0x41, 0x0f, 0xb6, 0xc0]);
    p.expect(carry);
    p.code.extend([0x41, 0x0f, 0xb6, 0xc1]);
    p.expect(1);
}

#[test]
fn native_bit_strings_use_signed_indices_masked_immediates_and_live_dependency_splits() {
    for (size, original, set) in [
        (2u8, 0x1122_3344_5566_0000u64, 0x1122_3344_5566_8000u64),
        (4, 0x1122_3344_0000_0000, 0x1122_3344_8000_0000),
        (8, 0, 0x8000_0000_0000_0000),
    ] {
        let mut p = Program::new();
        p.reg(15, 0x7564_2398_baef_1023);
        p.ptr(0, 0);
        p.save(0);
        p.reg(0, 0x9b9b_9b9b_9b9b_9b9b);
        p.save(56);
        p.save(72);
        p.reg(0, original);
        p.save(64);
        for immediate in [false, true] {
            p.ptr(7, if immediate { 64 } else { 64 + size });
            for (opcode, extension, carry, expected) in [
                (0xabu8, 5u8, 0, set),
                (0xa3, 4, 1, set),
                (0xbb, 7, 1, original),
                (0xb3, 6, 0, original),
            ] {
                // The immediately preceding index write must split before the
                // memory operation; batching may not preflight an older RCX.
                p.reg(
                    1,
                    match size {
                        2 => 0x1234_5678_9abc_ffff,
                        4 => 0x1234_5678_ffff_ffff,
                        _ => u64::MAX,
                    },
                );
                p.code.extend([0x31, 0xc0]); // ZF=1; no assertion on undefined flags.
                if size == 2 {
                    p.code.push(0x66);
                } else if size == 8 {
                    p.code.push(0x48);
                }
                if immediate {
                    p.code.extend([0x0f, 0xba, (extension << 3) | 7, 0xff]);
                } else {
                    p.code.extend([0x0f, opcode, 0x0f]);
                }
                capture_flags(&mut p, carry);
                p.word(64, expected);
            }
        }
        if size == 8 {
            p.ptr(12, 64);
            p.reg(13, 2);
            p.reg(9, (-65i64) as u64);
            p.code.extend([0x31, 0xc0, 0x4f, 0x0f, 0xab, 0x4c, 0xac, 8]);
            capture_flags(&mut p, 0);
            p.word(64, set);
        }
        p.word(56, 0x9b9b_9b9b_9b9b_9b9b);
        p.word(72, 0x9b9b_9b9b_9b9b_9b9b);
        p.code.extend([0x4c, 0x89, 0xf8]);
        p.expect(0x7564_2398_baef_1023);
        p.ptr(0, 0);
        p.code.extend([0x48, 0x3b, 0x04, 0x24]);
        checked(&mut p.code, &mut p.branches);
        let (code, _) = p.finish(None);
        let (status, fields, text) = probe(&input(&code, false));
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["service_stops"], "0");
        assert_eq!(fields["integer_ops"], "0");
        assert_eq!(fields["vector_ops"], "0");
    }
}
