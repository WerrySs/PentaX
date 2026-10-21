// SPDX-License-Identifier: GPL-2.0-only
//! Original signed word products and wrapping pair sums consumed by guest code.

use super::*;

fn signed_words(values: [i16; 16]) -> [u8; 32] {
    let mut bytes = [0; 32];
    for (index, value) in values.into_iter().enumerate() {
        bytes[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn signed_dwords(values: [i32; 8]) -> [u8; 32] {
    let mut bytes = [0; 32];
    for (index, value) in values.into_iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
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
fn native_word_multiply_add_profiles_wrap_signed_pairs_aliases_and_preserve_state() {
    let left = signed_words([
        -32768, -32768, 32767, 32767, -32768, 32767, -1, 1, -32768, 32767, 123, -456, -123, 456, 0,
        -1,
    ]);
    let right = signed_words([
        -32768, -32768, 32767, 32767, 32767, -32768, -1, -1, -32768, -32768, -789, 321, -789, -321,
        32767, -32768,
    ]);
    // Literal independent pair-sum oracle. The first lane is the unique signed
    // overflow case: two (-32768 * -32768) products wrap to0x80000000.
    let sums = signed_dwords([
        i32::MIN,
        2_147_352_578,
        -2_147_418_112,
        0,
        32_768,
        -243_423,
        -49_329,
        32_768,
    ]);
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
            // Legacy m128 is aligned; both VEX memory forms deliberately use an
            // unaligned source. Every complete source remains unchanged.
            let offset = if memory && form != 0 { 49 } else { 48 };
            set_bytes(&mut p, 16, &left);
            set_bytes(&mut p, offset, &right);
            load_vector(&mut p, 13, 16);
            load_vector(&mut p, 14, offset);
            let destination = if form == 0 || (form == 1 && memory) {
                13 // Destructive legacy destination or VEX left alias.
            } else if !memory {
                14 // VEX right alias must consume the full original right input.
            } else {
                9 // Separate high destination.
            };
            p.reg(0, 1);
            p.reg(2, 2);
            p.code.extend([0x48, 0x39, 0xd0]); // CF/PF/SF set, ZF/OF clear.
            if form == 0 {
                p.code
                    .extend([0x66, if memory { 0x44 } else { 0x45 }, 0x0f, 0xf5]);
            } else {
                // Reserved encoding bits stay canonical; inverted vvvv=0010
                // selects the actual first source XMM13/YMM13.
                p.code.extend([
                    0xc4,
                    if memory { 0x61 } else { 0x41 },
                    if form == 2 { 0x15 } else { 0x11 },
                    0xf5,
                ]);
            }
            if memory {
                p.code
                    .extend([0x44 | ((destination & 7) << 3), 0x24, offset]);
            } else {
                p.code.push(0xc6 | ((destination & 7) << 3));
            }
            // Test status flags before comparisons can change their values.
            for failure in [0x83, 0x8b, 0x84, 0x89, 0x80] {
                p.code.extend([0x0f, failure, 0, 0, 0, 0]);
                p.branches.push(p.code.len() - 4);
            }
            let mut expected = sums;
            if form == 0 {
                expected[16..].copy_from_slice(&left[16..]);
            } else if form == 1 {
                expected[16..].fill(0);
            }
            store_vector(&mut p, destination);
            check_bytes(&mut p, 88, &expected);
            let (preserved, original) = if destination == 13 {
                (14, &right)
            } else {
                (13, &left)
            };
            store_vector(&mut p, preserved);
            check_bytes(&mut p, 88, original);
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
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    // Two legacy PMADDWDs remain existing native SSE2 operations. Four VEX
    // variants and both MXCSR accesses remain mediated on AVX; temporal input
    // and readback transfers promote. SSE2 still models all32 operations.
    // VEX m128+m256 reads add48 bytes to the8 modeled control bytes on AVX.
    assert_modeled_vector_counters(&fields, [(32, 888), (6, 56), (6, 56)]);
    assert_eq!(fields["integer_ops"], "0", "{text}");
    assert_eq!(fields["fp_status"], "4", "{text}");
    assert_eq!(fields["service_stops"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert!(
        fields["native_instructions"].parse::<u64>().unwrap() > 0,
        "{text}"
    );
    assert!(
        fields["block_attempts"].parse::<u64>().unwrap() < 1200,
        "{text}"
    );
}
