// SPDX-License-Identifier: GPL-2.0-only
//! Original packed-conversion results consumed by subsequent guest branches.

use super::*;

fn set_word(p: &mut Program, offset: u8, value: u64) {
    p.reg(0, value);
    p.save(offset);
}
fn instruction(p: &mut Program, kind: u8, wide: bool, memory: bool) {
    let (opcode, pp) = match kind {
        0 => (0x5b, 1),
        1 => (0x5b, 2),
        2 => (0xe6, 3),
        3 => (0xe6, 1),
        4 => (0x5b, 0),
        _ => (0xe6, 2),
    };
    p.code.extend([
        0xc4,
        if memory { 0x61 } else { 0x41 },
        0x78 | pp | if wide { 4 } else { 0 },
        opcode,
    ]);
    if memory {
        p.code.extend([0x7c, 0x24, 16]);
    } else {
        p.code.push(0xff);
    }
}
fn control(p: &mut Program, value: u64) {
    set_word(p, 120, value);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 120]);
}
fn check_control(p: &mut Program, value: u64) {
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 120]);
    p.word(120, value);
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
fn check(mut p: Program, fp_status: u32, bytes: u64, operations: u64) {
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0xc00); // Raw source begins at0x100, no import/GOT tables.
    let source = input(&p.code, false);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["fp_status"], fp_status.to_string(), "{text}");
    assert_eq!(fields["vector_bytes"], bytes.to_string(), "{text}");
    assert_eq!(fields["vector_ops"], operations.to_string(), "{text}");
    assert_eq!(fields["service_stops"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
}

#[test]
fn native_float_to_packed_dword_rounding_truncation_and_mixed_lane_flags_continue_exactly() {
    let mut p = Program::new();
    for kind in 0..4 {
        for wide in [false, true] {
            let source = if kind < 2 {
                [
                    0x3fc0_0000_7fc0_0123,
                    0xc020_0000_0000_0001,
                    0x4f00_0000_cf00_0000,
                    0x8000_0000_bfc0_0000,
                ]
            } else {
                [
                    0x7ff8_0000_0000_0123,
                    0x3ff8_0000_0000_0000,
                    1,
                    0xc004_0000_0000_0000,
                ]
            };
            for (index, value) in source.into_iter().enumerate() {
                set_word(&mut p, 16 + index as u8 * 8, value);
            }
            p.code.extend([0xc5, 0x7e, 0x6f, 0x7c, 0x24, 16]); // Nonzero full YMM15 input/destination.
            control(&mut p, 0x5f84); // Upward RC; truncating forms must ignore it.
            flags(&mut p);
            instruction(&mut p, kind, wide, kind & 1 != 0);
            check_flags(&mut p);
            p.code.extend([0xc5, 0x7e, 0x7f, 0x7c, 0x24, 80]);
            let first = if kind & 1 != 0 {
                0x0000_0001_8000_0000
            } else {
                0x0000_0002_8000_0000
            };
            let second = if kind & 1 != 0 {
                0xffff_fffe_0000_0000
            } else {
                0xffff_fffe_0000_0001
            };
            let expected = if kind < 2 {
                [
                    first,
                    second,
                    if wide { 0x8000_0000_8000_0000 } else { 0 },
                    if wide { 0x0000_0000_ffff_ffff } else { 0 },
                ]
            } else {
                [first, if wide { second } else { 0 }, 0, 0]
            };
            for (index, value) in expected.into_iter().enumerate() {
                p.word(80 + index as u8 * 8, value);
            }
            check_control(&mut p, 0x5fa5); // Existing ZE + packed IE|PE, never DE.
        }
    }
    // Eight32-byte source loads/stores, eight4-byte controls each way, and
    // four truncating memory sources16/32/16/32. No hidden wider read is counted.
    check(p, 37, 672, 40);
}

#[test]
fn native_packed_dword_to_float_uses_rounding_exact_doubles_and_narrow_memory_sources() {
    let mut p = Program::new();
    let source = [
        0xfeff_ffff_0100_0001,
        0x8000_0000_7fff_ffff,
        0xffff_ffff_0000_0001,
        0x0000_0003_0000_0000,
    ];
    for kind in [4, 5] {
        for wide in [false, true] {
            for (index, value) in source.into_iter().enumerate() {
                set_word(&mut p, 16 + index as u8 * 8, value);
            }
            p.code.extend([0xc5, 0x7e, 0x6f, 0x7c, 0x24, 16]);
            control(&mut p, 0x3f84); // Downward RC, including negative inexact integers.
            flags(&mut p);
            instruction(&mut p, kind, wide, kind == 5);
            check_flags(&mut p);
            p.code.extend([0xc5, 0x7e, 0x7f, 0x7c, 0x24, 80]);
            let expected = if kind == 4 {
                [
                    0xcb80_0001_4b80_0000,
                    0xcf00_0000_4eff_ffff,
                    if wide { 0xbf80_0000_3f80_0000 } else { 0 },
                    if wide { 0x4040_0000_0000_0000 } else { 0 },
                ]
            } else {
                [
                    0x4170_0000_1000_0000,
                    0xc170_0000_1000_0000,
                    if wide { 0x41df_ffff_ffc0_0000 } else { 0 },
                    if wide { 0xc1e0_0000_0000_0000 } else { 0 },
                ]
            };
            for (index, value) in expected.into_iter().enumerate() {
                p.word(80 + index as u8 * 8, value);
            }
            check_control(&mut p, if kind == 4 { 0x3fa4 } else { 0x3f84 });
        }
    }
    // Four32-byte loads/stores and control transfers plus exact8/16-byte
    // DQ->PD source reads. L0 reads m64 even though its register source is XMM.
    check(p, 4, 312, 20);
}
