// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn native_packed_dword_minmax_handles_unsigned_order_and_vex128_zeroing() {
    let left = [
        u32::MAX,
        0x8000_0000,
        3,
        9,
        0x8000_0001,
        0x7fff_ffff,
        0,
        100,
    ];
    let right = [1_u32, 0x7fff_ffff, 4, 9, u32::MAX, 0x8000_0000, 1, 99];
    let mut p = Program::new();
    for (start, values) in [(0_u8, left), (32, right)] {
        for (index, pair) in values.chunks_exact(2).enumerate() {
            p.reg(0, u64::from(pair[0]) | (u64::from(pair[1]) << 32));
            p.save(start + index as u8 * 8);
        }
    }
    p.code.extend([0xc5, 0xfe, 0x6f, 0x0c, 0x24]); // VMOVDQU YMM1,[RSP].
    p.code.extend([0xc5, 0xfe, 0x6f, 0x54, 0x24, 32]);
    for (instruction, expected) in [
        (
            vec![0xc4, 0xe2, 0x75, 0x3b, 0xc2],
            [1, 0x7fff_ffff, 3, 9, 0x8000_0001, 0x7fff_ffff, 0, 99],
        ),
        (
            vec![0xc4, 0xe2, 0x75, 0x3f, 0xc2],
            [u32::MAX, 0x8000_0000, 4, 9, u32::MAX, 0x8000_0000, 1, 100],
        ),
        (
            vec![0xc4, 0xe2, 0x75, 0x39, 0xc2],
            [u32::MAX, 0x8000_0000, 3, 9, 0x8000_0001, 0x8000_0000, 0, 99],
        ),
        (
            vec![0xc4, 0xe2, 0x71, 0x3b, 0x44, 0x24, 32],
            [1, 0x7fff_ffff, 3, 9, 0, 0, 0, 0],
        ),
    ] {
        p.code.extend(instruction);
        p.code.extend([0xc5, 0xfe, 0x7f, 0x44, 0x24, 64]);
        for (index, pair) in expected.chunks_exact(2).enumerate() {
            p.word(
                64 + index as u8 * 8,
                u64::from(pair[0]) | (u64::from(pair[1]) << 32),
            );
        }
    }
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_modeled_vector_counters(&fields, [(10, 208), (4, 16), (4, 16)]);
    assert_eq!(fields["fp_status"], "0");
}
