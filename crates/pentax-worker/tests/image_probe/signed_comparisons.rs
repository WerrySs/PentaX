// SPDX-License-Identifier: GPL-2.0-only
//! Original signed masks consumed by guest loads/branches in every vector profile.
use super::*;

fn word(p: &mut Program, offset: u8, value: u64) {
    p.reg(0, value);
    p.save(offset);
}
fn load(p: &mut Program, register: u8, offset: u8) {
    p.code
        .extend([0xc5, 0x7e, 0x6f, 0x44 | ((register & 7) << 3), 0x24, offset]);
}
fn store(p: &mut Program, register: u8) {
    p.code
        .extend([0xc5, 0x7e, 0x7f, 0x44 | ((register & 7) << 3), 0x24, 88]);
}
fn operands(width: usize) -> ([u8; 32], [u8; 32], [u8; 32]) {
    let (min, max) = match width {
        1 => (i64::from(i8::MIN), i64::from(i8::MAX)),
        2 => (i64::from(i16::MIN), i64::from(i16::MAX)),
        4 => (i64::from(i32::MIN), i64::from(i32::MAX)),
        8 => (i64::MIN, i64::MAX),
        _ => unreachable!(),
    };
    // Low and high qword lanes differ; byte/word/dword lanes include equality.
    let lanes = [
        (min, max, false),
        (max, min, true),
        (-1, -1, false),
        (0, -1, true),
        (min, min, false),
        (-1, 0, false),
        (1, 0, true),
        (max, max, false),
    ];
    let mut left = [0; 32];
    let mut right = [0; 32];
    let mut masks = [0; 32];
    for lane in 0..32 / width {
        let (a, b, greater) = lanes[lane % lanes.len()];
        let at = lane * width;
        left[at..at + width].copy_from_slice(&a.to_le_bytes()[..width]);
        right[at..at + width].copy_from_slice(&b.to_le_bytes()[..width]);
        masks[at..at + width].fill(if greater { 0xff } else { 0 });
    }
    (left, right, masks)
}
fn words(p: &mut Program, offset: u8, bytes: &[u8; 32]) {
    for (index, bytes) in bytes.chunks_exact(8).enumerate() {
        word(
            p,
            offset + index as u8 * 8,
            u64::from_le_bytes(bytes.try_into().unwrap()),
        );
    }
}
fn check(p: &mut Program, offset: u8, bytes: &[u8; 32]) {
    for (index, bytes) in bytes.chunks_exact(8).enumerate() {
        p.word(
            offset + index as u8 * 8,
            u64::from_le_bytes(bytes.try_into().unwrap()),
        );
    }
}

#[test]
fn native_signed_compare_masks_consume_extrema_equality_aliases_memory_and_upper_zeroing() {
    for width in [1, 2, 4, 8] {
        for size in [16, 32] {
            let (left, right, masks) = operands(width);
            let mut p = Program::new();
            p.stack(0x89, 4, 8);
            word(&mut p, 0, 0x1f84);
            p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]);
            word(&mut p, 120, 0x1234_5678_a5a5_5a5a);
            words(&mut p, 16, &left);
            load(&mut p, 15, 16);
            p.reg(8, 0x8877_6655_4433_2211);
            for form in 0..4 {
                let memory = form == 3;
                let offset = if memory { 49 } else { 48 };
                words(&mut p, 16, &left);
                words(&mut p, offset, &right);
                load(&mut p, 13, 16);
                load(&mut p, 14, offset);
                let destination = match form {
                    1 => 13,
                    2 => 14,
                    _ => 9,
                };
                if destination == 9 {
                    load(&mut p, 9, 16);
                } // Nonzero old upper half must be replaced.
                p.reg(0, 1);
                p.reg(2, 2);
                p.code.extend([0x48, 0x39, 0xd0]);
                let map = if width == 8 { 2 } else { 1 };
                let opcode = match width {
                    1 => 0x64,
                    2 => 0x65,
                    4 => 0x66,
                    8 => 0x37,
                    _ => unreachable!(),
                };
                p.code.extend([
                    0xc4,
                    (if memory { 0x60 } else { 0x40 }) | map,
                    0x11 | if size == 32 { 4 } else { 0 } | if form % 2 != 0 { 0x80 } else { 0 },
                    opcode,
                ]);
                if memory {
                    p.code
                        .extend([0x44 | ((destination & 7) << 3), 0x24, offset]);
                } else {
                    p.code.push(0xc6 | ((destination & 7) << 3));
                }
                // Actual guest branches see exactly the flags from CMP1,2.
                for failure in [0x83, 0x8b, 0x84, 0x89, 0x80] {
                    p.code.extend([0x0f, failure, 0, 0, 0, 0]);
                    p.branches.push(p.code.len() - 4);
                }
                store(&mut p, destination);
                let mut expected = masks;
                expected[size..].fill(0);
                check(&mut p, 88, &expected);
                let preserved = if destination == 13 { 14 } else { 13 };
                store(&mut p, preserved);
                check(&mut p, 88, if preserved == 13 { &left } else { &right });
                if memory {
                    check(&mut p, offset, &right);
                }
            }
            store(&mut p, 15);
            check(&mut p, 88, &left);
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
            assert_eq!(status, 0, "width={width} size={size} {text}");
            assert_eq!(fields["value"], "42", "{text}");
            // Twenty complete vector transfers, four comparisons, MXCSR each way.
            // Only existing qword register compares may promote under AVX/AVX2.
            let avx = if width == 8 && size == 16 { 3 } else { 6 };
            let avx2 = if width == 8 { 3 } else { 6 };
            assert_modeled_vector_counters(
                &fields,
                [
                    (26, 648 + size as u64),
                    (avx, 8 + size as u64),
                    (avx2, 8 + size as u64),
                ],
            );
            assert_eq!(fields["integer_ops"], "0", "{text}");
            assert_eq!(fields["fp_status"], "4", "{text}");
            assert_eq!(fields["service_stops"], "0", "{text}");
            assert_eq!(fields["phase"], "main-entry", "{text}");
            assert!(
                fields["native_instructions"].parse::<u64>().unwrap() < 1600,
                "{text}"
            );
            assert!(
                fields["block_attempts"].parse::<u64>().unwrap() < 1600,
                "{text}"
            );
        }
    }
    println!(
        "signed_compare_native_available=true signed_compare_native_programs=8 signed_compare_native_returns=8"
    );
}
