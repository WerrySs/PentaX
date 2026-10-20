// SPDX-License-Identifier: GPL-2.0-only
//! Original bit masks consumed by following integer branches, without host AVX.

use super::*;

fn encode(double: bool, vector_wide: bool, gpr_wide: bool, destination: u8) -> [u8; 5] {
    [
        0xc4,
        if destination >= 8 { 0x41 } else { 0xc1 },
        0x78 | u8::from(double) | if vector_wide { 4 } else { 0 } | if gpr_wide { 0x80 } else { 0 },
        0x50,
        0xc7 | ((destination & 7) << 3),
    ]
}
fn set_word(p: &mut Program, offset: u8, value: u64) {
    p.reg(0, value);
    p.save(offset);
}
fn finish(mut p: Program) -> Vec<u8> {
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0xc00); // Raw no-import image starts at0x100, no GOT.
    p.code
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

#[test]
fn native_all_float_mask_forms_zero_extend_high_gpr_and_keep_full_vector_flags_and_mxcsr() {
    let mut p = Program::new();
    set_word(&mut p, 120, 0x1f84);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 120]);
    for double in [false, true] {
        let words = if double {
            [
                0x8000_0000_0000_0000,
                0x7ff8_8080_8080_8080,
                0xfff0_0000_0000_0001,
                0xfff0_0000_0000_0000,
            ]
        } else {
            [
                0x7fc0_8080_8000_0000,
                0x3f80_0000_ff80_0001,
                0x0000_0000_bf80_0000,
                0xffc0_1234_7f80_0000,
            ]
        };
        for (index, value) in words.into_iter().enumerate() {
            set_word(&mut p, 16 + index as u8 * 8, value);
        }
        p.code.extend([0xc5, 0x7e, 0x6f, 0x7c, 0x24, 16]); // VMOVDQU YMM15, original bits.
        for vector_wide in [false, true] {
            for gpr_wide in [false, true] {
                p.reg(9, u64::MAX);
                p.reg(0, 1);
                p.reg(2, 2);
                p.code.extend([0x48, 0x39, 0xd0]); // Known CF/PF/ZF=1/1/0.
                p.code.extend(encode(double, vector_wide, gpr_wide, 9));
                p.code.extend([0x4c, 0x89, 0xc8]); // Capture full R9, not just R9D.
                p.save(112);
                check_flags(&mut p);
                p.word(
                    112,
                    if double {
                        if vector_wide { 13 } else { 1 }
                    } else if vector_wide {
                        0x95
                    } else {
                        5
                    },
                );
                // MOVMSK must preserve even the unused upper half of YMM15;
                // unlike a VEX vector destination it does not zero that state.
                p.code.extend([0xc5, 0x7e, 0x7f, 0x7c, 0x24, 80]);
                for (index, value) in words.into_iter().enumerate() {
                    p.word(80 + index as u8 * 8, value);
                }
                p.code.extend([0x0f, 0xae, 0x5c, 0x24, 120]);
                p.word(120, 0x1f84); // sNaN/denormal bit patterns have no FP effects.
            }
        }
    }
    let source = input(&finish(p), false);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["fp_status"], "4", "{text}");
    assert_eq!(fields["service_stops"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
}

#[test]
fn native_float_masks_refuse_esp_and_rsp_destinations_without_clean_return() {
    for (double, vector_wide, gpr_wide) in [(false, false, false), (true, true, true)] {
        let mut p = Program::new();
        for offset in [16, 24, 32, 40] {
            p.set(offset, 0);
        }
        p.code.extend([0xc5, 0x7e, 0x6f, 0x7c, 0x24, 16]);
        p.code.extend(encode(double, vector_wide, gpr_wide, 4));
        let source = input(&finish(p), false);
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "native-dispatch", "{text}");
        assert_eq!(fields["runtime_failure"], "invalid-stack", "{text}");
        assert_eq!(
            fields["planned_exit"],
            if double { "Vmovmskpd" } else { "Vmovmskps" },
            "{text}"
        );
        assert_eq!(fields["service_stops"], "0", "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert!(!fields.contains_key("value"), "{text}");
    }
}
