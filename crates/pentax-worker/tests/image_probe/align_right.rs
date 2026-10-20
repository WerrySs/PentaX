// SPDX-License-Identifier: GPL-2.0-only
//! Original native continuations consume lane-boundary and zero-fill results.
use super::*;

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

#[test]
fn native_align_right_consumes_concatenation_boundaries_aliases_zero_fill_and_independent_lanes() {
    let high = std::array::from_fn(|i| i as u8 + 0x81);
    let low = std::array::from_fn(|i| i as u8 + 1);
    let mut p = Program::new();
    p.stack(0x89, 4, 8);
    set_word(&mut p, 0, 0x3fa5);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]);
    set_word(&mut p, 120, 0x5abc_9876_4321_fed0);
    set_bytes(&mut p, 16, &high);
    vector(&mut p, 15, 16, false);
    p.reg(8, 0x8877_6655_4433_2211);
    for (case, (form, memory, immediate)) in [
        (0, false, 15),
        (0, true, 16),
        (1, false, 1),
        (1, true, 31),
        (2, false, 17),
        (2, true, 255),
    ]
    .into_iter()
    .enumerate()
    {
        let offset = if memory && form != 0 { 49 } else { 48 };
        set_bytes(&mut p, 16, &high);
        set_bytes(&mut p, offset, &low);
        vector(&mut p, 13, 16, false);
        vector(&mut p, 14, offset, false);
        let destination = if form == 0 || form == 1 && memory {
            13
        } else if !memory {
            14
        } else {
            9
        };
        p.reg(0, 1);
        p.reg(2, 2);
        p.code.extend([0x48, 0x39, 0xd0]);
        if form == 0 {
            p.code
                .extend([0x66, if memory { 0x44 } else { 0x45 }, 0x0f, 0x3a, 0x0f]);
        } else {
            p.code.extend([
                0xc4,
                if memory { 0x63 } else { 0x43 },
                if form == 2 { 0x15 } else { 0x11 },
                0x0f,
            ]);
        }
        if memory {
            p.code
                .extend([0x44 | ((destination & 7) << 3), 0x24, offset]);
        } else {
            p.code.push(0xc6 | ((destination & 7) << 3));
        }
        p.code.push(immediate);
        // Test CF/PF/ZF/SF/OF before comparisons can alter their values.
        for failure in [0x83, 0x8b, 0x84, 0x89, 0x80] {
            p.code.extend([0x0f, failure, 0, 0, 0, 0]);
            p.branches.push(p.code.len() - 4);
        }
        let mut expected = [0; 32];
        match case {
            0 => expected[..16].copy_from_slice(&[
                0x10, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d,
                0x8e, 0x8f,
            ]),
            1 => expected[..16].copy_from_slice(&[
                0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d, 0x8e,
                0x8f, 0x90,
            ]),
            2 => expected[..16]
                .copy_from_slice(&[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 0x81]),
            3 => expected[0] = 0x90,
            4 => expected.copy_from_slice(&[
                0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d, 0x8e, 0x8f,
                0x90, 0, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b, 0x9c, 0x9d,
                0x9e, 0x9f, 0xa0, 0,
            ]),
            _ => {}
        }
        if form == 0 {
            expected[16..].copy_from_slice(&high[16..]);
        }
        vector(&mut p, destination, 88, true);
        check_bytes(&mut p, 88, &expected);
        let (preserved, original) = if destination == 13 {
            (14, &low)
        } else {
            (13, &high)
        };
        vector(&mut p, preserved, 88, true);
        check_bytes(&mut p, 88, original);
        if memory {
            check_bytes(&mut p, offset, &low);
        }
    }
    vector(&mut p, 15, 88, true);
    check_bytes(&mut p, 88, &high);
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 0]);
    p.word(0, 0x3fa5);
    p.word(120, 0x5abc_9876_4321_fed0);
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
    assert_eq!(fields["vector_ops"], "34", "{text}");
    assert_eq!(fields["vector_bytes"], "904", "{text}");
    assert_eq!(fields["integer_ops"], "0", "{text}");
    assert_eq!(fields["fp_status"], "37", "{text}");
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
