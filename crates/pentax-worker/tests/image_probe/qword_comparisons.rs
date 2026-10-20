// SPDX-License-Identifier: GPL-2.0-only
//! Original signed qword masks consumed by following guest branches and loads.

use super::*;

fn set_word(p: &mut Program, offset: u8, value: u64) {
    p.reg(0, value);
    p.save(offset);
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
fn native_qword_comparisons_consume_signed_endpoints_aliases_and_all_upper_rules() {
    let upper = [0x1122_3344_5566_7788, 0x99aa_bbcc_ddee_ff00];
    let mut p = Program::new();
    p.stack(0x89, 4, 8); // Actual stack identity, separate from vector operands.
    set_word(&mut p, 0, 0x1f84);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]);
    set_word(&mut p, 120, 0x1234_5678_a5a5_5a5a);
    for (index, value) in [upper[0], upper[1], 0x8000_0000_0000_0000, u64::MAX]
        .into_iter()
        .enumerate()
    {
        set_word(&mut p, 16 + index as u8 * 8, value);
    }
    load_vector(&mut p, 15, 16); // Unrelated YMM state, including sign/NaN bit patterns.
    p.reg(8, 0x8877_6655_4433_2211);
    for form in 0..3 {
        for memory in [false, true] {
            // Literal results distinguish unsigned ordering, signed minima,
            // maxima, negative/zero boundaries and equal lanes.
            let (left, right, masks) = match (form, memory) {
                (0, false) => (
                    [
                        0x8000_0000_0000_0000,
                        0x7fff_ffff_ffff_ffff,
                        upper[0],
                        upper[1],
                    ],
                    [0x7fff_ffff_ffff_ffff, 0x8000_0000_0000_0000, 1, 2],
                    [0, u64::MAX, upper[0], upper[1]],
                ),
                (0, true) => (
                    [u64::MAX, 0, upper[0], upper[1]],
                    [u64::MAX, u64::MAX, 3, 4],
                    [0, u64::MAX, upper[0], upper[1]],
                ),
                (1, false) => (
                    [u64::MAX, 0, upper[0], upper[1]],
                    [u64::MAX, u64::MAX, 5, 6],
                    [0, u64::MAX, 0, 0],
                ),
                (1, true) => (
                    [
                        0x7fff_ffff_ffff_ffff,
                        0x8000_0000_0000_0000,
                        upper[0],
                        upper[1],
                    ],
                    [0x8000_0000_0000_0000, 0x8000_0000_0000_0000, 7, 8],
                    [u64::MAX, 0, 0, 0],
                ),
                (2, false) => (
                    [0x8000_0000_0000_0000, 0x7fff_ffff_ffff_ffff, u64::MAX, 0],
                    [0x7fff_ffff_ffff_ffff, 0x8000_0000_0000_0000, 0, u64::MAX],
                    [0, u64::MAX, 0, u64::MAX],
                ),
                (2, true) => (
                    [0x7fff_ffff_ffff_ffff, 0x8000_0000_0000_0000, u64::MAX, 0],
                    [
                        0x8000_0000_0000_0000,
                        0x8000_0000_0000_0000,
                        u64::MAX,
                        u64::MAX,
                    ],
                    [u64::MAX, 0, 0, u64::MAX],
                ),
                _ => unreachable!(),
            };
            let offset = if memory && form != 0 { 49 } else { 48 };
            for (index, value) in left.into_iter().enumerate() {
                set_word(&mut p, 16 + index as u8 * 8, value);
            }
            for (index, value) in right.into_iter().enumerate() {
                set_word(&mut p, offset + index as u8 * 8, value);
            }
            load_vector(&mut p, 13, 16);
            load_vector(&mut p, 14, offset);
            let destination = if form == 0 || (form == 1 && memory) {
                13 // Legacy destructive operand, or VEX left alias.
            } else if !memory {
                14 // VEX right alias must consume the original full operand.
            } else {
                9
            };
            p.reg(0, 1);
            p.reg(2, 2);
            p.code.extend([0x48, 0x39, 0xd0]); // CF/PF/SF set, ZF/OF clear.
            if form == 0 {
                p.code
                    .extend([0x66, if memory { 0x44 } else { 0x45 }, 0x0f, 0x38, 0x37]);
            } else {
                // Inverted vvvv=0010 denotes left source YMM13; W is ignored.
                p.code.extend([
                    0xc4,
                    if memory { 0x62 } else { 0x42 },
                    if form == 2 { 0x15 } else { 0x11 },
                    0x37,
                ]);
            }
            if memory {
                p.code
                    .extend([0x44 | ((destination & 7) << 3), 0x24, offset]);
            } else {
                p.code.push(0xc6 | ((destination & 7) << 3));
            }
            // These Jcc operations precede any flag-changing result checks.
            // Full RFLAGS equality is separately covered by platform fixtures.
            for failure in [0x83, 0x8b, 0x84, 0x89, 0x80] {
                p.code.extend([0x0f, failure, 0, 0, 0, 0]);
                p.branches.push(p.code.len() - 4);
            }
            store_vector(&mut p, destination);
            for (index, value) in masks.into_iter().enumerate() {
                p.word(88 + index as u8 * 8, value);
            }
            let (preserved, words) = if destination == 13 {
                (14, right)
            } else {
                (13, left)
            };
            store_vector(&mut p, preserved);
            for (index, value) in words.into_iter().enumerate() {
                p.word(88 + index as u8 * 8, value);
            }
            if memory {
                for (index, value) in right.into_iter().enumerate() {
                    p.word(offset + index as u8 * 8, value);
                }
            }
        }
    }
    store_vector(&mut p, 15);
    for (index, value) in [upper[0], upper[1], 0x8000_0000_0000_0000, u64::MAX]
        .into_iter()
        .enumerate()
    {
        p.word(88 + index as u8 * 8, value);
    }
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 0]);
    p.word(0, 0x1f84);
    p.word(120, 0x1234_5678_a5a5_5a5a);
    p.code.extend([0x4c, 0x89, 0xc0]); // Preserved unrelated R8.
    p.expect(0x8877_6655_4433_2211);
    p.code.extend([0x48, 0x89, 0xe0]);
    p.load(10, 8);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0xe00); // Raw original ELF starts at0x100; no imports/GOT.
    let original = input(&p.code, false);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    // Six comparisons, twelve32-byte operand loads and twelve32-byte result/
    // source stores, plus one unrelated vector load/store and MXCSR each way.
    // Memory comparison operands add exactly16+16+32 bytes.
    assert_eq!(fields["vector_ops"], "34", "{text}");
    assert_eq!(fields["vector_bytes"], "904", "{text}");
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
