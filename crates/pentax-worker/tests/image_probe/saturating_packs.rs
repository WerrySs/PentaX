// SPDX-License-Identifier: GPL-2.0-only
//! Original saturation boundaries and lane ordering consumed by guest branches.

use super::*;

type PackProfile = ([u8; 32], [u8; 32], [u8; 32]);

fn words(values: [u16; 16]) -> [u8; 32] {
    let mut bytes = [0; 32];
    for (index, value) in values.into_iter().enumerate() {
        bytes[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}
fn signed_words(values: [i16; 16]) -> [u8; 32] {
    words(values.map(|value| value as u16))
}
fn dwords(values: [i32; 8]) -> [u8; 32] {
    let mut bytes = [0; 32];
    for (index, value) in values.into_iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}
fn profile(kind: usize) -> PackProfile {
    // Outputs are literal source-oracle results, not a second saturation loop.
    // Each half orders SRC1's narrowed elements before SRC2's narrowed elements.
    match kind {
        0 => (
            signed_words([
                i16::MIN,
                -129,
                -128,
                -127,
                -1,
                0,
                126,
                127,
                200,
                -200,
                2,
                3,
                4,
                5,
                6,
                7,
            ]),
            signed_words([
                128,
                129,
                i16::MAX,
                42,
                -42,
                1,
                -1,
                0,
                -8,
                -7,
                -6,
                -5,
                -4,
                -3,
                -2,
                300,
            ]),
            [
                0x80, 0x80, 0x80, 0x81, 0xff, 0, 126, 127, 127, 127, 127, 42, 0xd6, 1, 0xff, 0,
                127, 0x80, 2, 3, 4, 5, 6, 7, 0xf8, 0xf9, 0xfa, 0xfb, 0xfc, 0xfd, 0xfe, 127,
            ],
        ),
        1 => (
            dwords([i32::MIN, -32769, -32768, -1, 32766, 32767, 32768, i32::MAX]),
            dwords([0, 1, -32767, 32768, 42, -42, -32769, -7]),
            signed_words([
                -32768, -32768, -32768, -1, 0, 1, -32767, 32767, 32766, 32767, 32767, 32767, 42,
                -42, -32768, -7,
            ]),
        ),
        2 => (
            signed_words([
                i16::MIN,
                -1,
                0,
                1,
                254,
                255,
                256,
                i16::MAX,
                2,
                3,
                4,
                5,
                300,
                -300,
                100,
                200,
            ]),
            signed_words([
                256,
                255,
                254,
                0,
                -1,
                i16::MIN,
                42,
                1,
                7,
                8,
                9,
                10,
                11,
                12,
                13,
                14,
            ]),
            [
                0, 0, 0, 1, 254, 255, 255, 255, 255, 255, 254, 0, 0, 0, 42, 1, 2, 3, 4, 5, 255, 0,
                100, 200, 7, 8, 9, 10, 11, 12, 13, 14,
            ],
        ),
        3 => (
            dwords([i32::MIN, -1, 0, 1, 65534, 65535, 65536, i32::MAX]),
            dwords([-65536, 65536, 65535, 65534, 42, -42, -1, 7]),
            words([
                0, 0, 0, 1, 0, 65535, 65535, 65534, 65534, 65535, 65535, 65535, 42, 0, 0, 7,
            ]),
        ),
        _ => unreachable!(),
    }
}
fn set_word(p: &mut Program, offset: u8, value: u64) {
    p.reg(0, value);
    p.save(offset);
}
fn set_bytes(p: &mut Program, offset: u8, bytes: &[u8; 32]) {
    for (index, word) in bytes.chunks_exact(8).enumerate() {
        set_word(
            p,
            offset + index as u8 * 8,
            u64::from_le_bytes(word.try_into().unwrap()),
        );
    }
}
fn check_bytes(p: &mut Program, offset: u8, bytes: &[u8; 32]) {
    for (index, word) in bytes.chunks_exact(8).enumerate() {
        p.word(
            offset + index as u8 * 8,
            u64::from_le_bytes(word.try_into().unwrap()),
        );
    }
}
fn load_vector(p: &mut Program, register: u8, offset: u8) {
    p.code
        .extend([0xc5, 0x7e, 0x6f, 0x44 | ((register & 7) << 3), 0x24, offset]);
}
fn store_vector(p: &mut Program, register: u8) {
    p.code
        .extend([0xc5, 0x7e, 0x7f, 0x44 | ((register & 7) << 3), 0x24, 88]);
}

#[test]
fn native_all_pack_profiles_saturate_signed_sources_and_preserve_lane_order_aliases_and_state() {
    // Four bounded original no-import programs keep every profile within the
    // existing immutable code window; all twelve forms execute in this test.
    for kind in 0..4 {
        let (left, right, packed) = profile(kind);
        let mut p = Program::new();
        p.stack(0x89, 4, 8);
        set_word(&mut p, 0, 0x1f84);
        p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]);
        set_word(&mut p, 120, 0x1234_5678_a5a5_5a5a);
        set_bytes(&mut p, 16, &left);
        load_vector(&mut p, 15, 16);
        p.reg(8, 0x8877_6655_4433_2211);
        for form in 0..3 {
            for memory in [false, true] {
                let offset = if memory && form != 0 { 49 } else { 48 };
                set_bytes(&mut p, 16, &left);
                set_bytes(&mut p, offset, &right);
                load_vector(&mut p, 13, 16);
                load_vector(&mut p, 14, offset);
                let destination = if form == 0 || (form == 1 && memory) {
                    13
                } else if !memory {
                    14
                } else {
                    9
                };
                p.reg(0, 1);
                p.reg(2, 2);
                p.code.extend([0x48, 0x39, 0xd0]); // CF/PF/SF set, ZF/OF clear.
                let opcode = [0x63, 0x6b, 0x67, 0x2b][kind];
                if form == 0 {
                    p.code
                        .extend([0x66, if memory { 0x44 } else { 0x45 }, 0x0f]);
                    if kind == 3 {
                        p.code.push(0x38);
                    }
                    p.code.push(opcode);
                } else {
                    p.code.extend([
                        0xc4,
                        (if memory { 0x61 } else { 0x41 }) + u8::from(kind == 3),
                        if form == 2 { 0x15 } else { 0x11 },
                        opcode,
                    ]);
                }
                if memory {
                    p.code
                        .extend([0x44 | ((destination & 7) << 3), 0x24, offset]);
                } else {
                    p.code.push(0xc6 | ((destination & 7) << 3));
                }
                // Check flags before any result comparison changes them.
                for failure in [0x83, 0x8b, 0x84, 0x89, 0x80] {
                    p.code.extend([0x0f, failure, 0, 0, 0, 0]);
                    p.branches.push(p.code.len() - 4);
                }
                let mut expected = packed;
                if form == 0 {
                    expected[16..].copy_from_slice(&left[16..]);
                } else if form == 1 {
                    expected[16..].fill(0);
                }
                store_vector(&mut p, destination);
                check_bytes(&mut p, 88, &expected);
                let (preserved, bytes) = if destination == 13 {
                    (14, &right)
                } else {
                    (13, &left)
                };
                store_vector(&mut p, preserved);
                check_bytes(&mut p, 88, bytes);
                if memory {
                    check_bytes(&mut p, offset, &right);
                }
            }
        }
        store_vector(&mut p, 15);
        check_bytes(&mut p, 88, &left);
        p.code.extend([0x0f, 0xae, 0x5c, 0x24, 0]);
        p.word(0, 0x1f84);
        p.word(120, 0x1234_5678_a5a5_5a5a);
        p.code.extend([0x4c, 0x89, 0xc0]);
        p.expect(0x8877_6655_4433_2211);
        p.code.extend([0x48, 0x89, 0xe0]);
        p.load(10, 8);
        p.code.extend([0x4c, 0x39, 0xd0]);
        checked(&mut p.code, &mut p.branches);
        p.reg(0, 42);
        p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
        finish_checks(&mut p.code, p.branches);
        assert!(p.code.len() < 0xe00);
        let original = input(&p.code, false);
        let (status, fields, text) = probe(&original);
        assert_eq!(status, 0, "profile={kind}: {text}");
        assert_eq!(fields["value"], "42", "profile={kind}: {text}");
        // Three legacy SSE2 families remain native: their two pack operations
        // and16-byte source read do not enter mediated vector counters. USDW
        // remains mediated. The two32-byte loads/stores in each of six cases,
        // unrelated vector each way and two MXCSR operations are unchanged.
        // The optional YMM bridge promotes only the register VEX128 form on
        // AVX, plus register VEX256 on AVX2. Exact memory counts do not change.
        assert_modeled_vector_operations(
            &fields,
            if kind == 3 {
                ["34", "33", "32"]
            } else {
                ["32", "31", "30"]
            },
        );
        assert_eq!(
            fields["vector_bytes"],
            if kind == 3 { "904" } else { "888" },
            "profile={kind}: {text}"
        );
        assert_eq!(fields["integer_ops"], "0", "{text}");
        assert_eq!(fields["fp_status"], "4", "{text}");
        assert_eq!(fields["service_stops"], "0", "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert!(
            fields["native_instructions"].parse::<u64>().unwrap() < 1200,
            "{text}"
        );
        assert!(
            fields["block_attempts"].parse::<u64>().unwrap() < 1200,
            "{text}"
        );
    }
}
