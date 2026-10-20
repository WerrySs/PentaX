// SPDX-License-Identifier: GPL-2.0-only

//! Original linked consumers exercise string mediation in the real worker.
use super::*;

fn ptr(p: &mut Program, reg: u8, offset: u32) {
    p.code.extend([0x48, 0x8d, 0x84 | (reg << 3), 0x24]);
    p.code.extend(offset.to_le_bytes());
}
fn finish(mut p: Program, allocation: u32) -> Vec<u8> {
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4]);
    p.code.extend(allocation.to_le_bytes());
    p.code.extend([0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    p.code
}
fn verify(code: &[u8], elements: u64, bytes: u64) {
    let (status, fields, text) = probe(&input(code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["string_elements"], elements.to_string());
    assert_eq!(fields["string_bytes"], bytes.to_string());
    assert_eq!(fields["service_stops"], "0");
}

#[test]
fn native_rep_strings_resume_multiple_quanta_and_check_actual_copied_loaded_scanned_and_compared_data()
 {
    let mut p = Program::new();
    p.code[4..8].copy_from_slice(&8192_u32.to_le_bytes());
    p.reg(0, 42);
    ptr(&mut p, 7, 0);
    p.reg(1, 257);
    p.code.extend([0xf3, 0x48, 0xab]); // REP STOSQ
    ptr(&mut p, 6, 0);
    ptr(&mut p, 7, 4096);
    p.reg(1, 257);
    p.code.extend([0xf3, 0x48, 0xa5]); // REP MOVSQ
    ptr(&mut p, 6, 4096);
    p.reg(1, 257);
    p.code.extend([0xf3, 0x48, 0xad]); // REP LODSQ
    p.expect(42);
    ptr(&mut p, 7, 4096);
    p.reg(1, 257);
    p.code.extend([0xf3, 0x48, 0xaf]); // REPE SCASQ
    p.code.extend([0x48, 0x89, 0xc8]);
    p.expect(0);
    ptr(&mut p, 6, 0);
    ptr(&mut p, 7, 4096);
    p.reg(1, 257);
    // ZF is deliberately clear before the first comparison: it must still run.
    p.reg(0, 1);
    p.code.extend([0x48, 0x85, 0xc0, 0xf3, 0x48, 0xa7]);
    p.code.extend([0x48, 0x89, 0xc8]);
    p.expect(0);
    verify(&finish(p, 8192), 1285, 14392);
}

#[test]
fn native_rep_move_preserves_destructive_overlap_and_direction_flag_round_trips() {
    let mut p = Program::new();
    p.reg(0, 0x0102_0304_0506_072a);
    p.save(0);
    p.ptr(6, 0);
    p.ptr(7, 1);
    p.reg(1, 7);
    p.code.extend([0xf3, 0xa4]);
    p.word(0, 0x2a2a_2a2a_2a2a_2a2a);
    p.reg(0, 0x0102_0304_0506_072a);
    p.save(0);
    p.ptr(6, 7);
    p.ptr(7, 6);
    p.reg(1, 7);
    p.code.extend([0xfd, 0xf3, 0xa4, 0xfc]);
    p.word(0, 0x0101_0101_0101_0101);
    verify(&finish(p, 256), 14, 28);
}

#[test]
fn native_rep_zero_count_avoids_invalid_addresses_and_nonzero_count_stops_without_a_result() {
    let mut p = Program::new();
    p.reg(6, u64::MAX);
    p.reg(7, u64::MAX);
    p.reg(1, 0);
    p.reg(0, 42);
    p.code.extend([0xfd, 0xf3, 0x48, 0xa5, 0xfc]);
    p.expect(42);
    verify(&finish(p, 256), 0, 0);
    let mut p = Program::new();
    p.reg(7, u64::MAX);
    p.reg(1, 1);
    p.code.extend([0xf3, 0xaa]);
    let (status, fields, text) = probe(&input(&finish(p, 256), false));
    assert_ne!(status, 0, "{text}");
    assert_eq!(fields["code"], "native-dispatch");
    assert_eq!(fields["string_elements"], "0");
    assert_eq!(fields["string_bytes"], "0");
    assert!(!fields.contains_key("value"));
}

#[test]
fn native_conditional_repeat_uses_the_first_comparison_before_inspecting_initial_flags_or_unused_count()
 {
    let mut p = Program::new();
    p.set(8, 42);
    p.set(16, 1);
    p.ptr(7, 8);
    p.reg(1, u64::MAX);
    p.reg(0, 42);
    // ZF starts clear, and the first equal byte ends REPNE immediately.
    p.code.extend([0x48, 0x85, 0xc0, 0xf2, 0xae]);
    p.code.extend([0x48, 0x89, 0xc8]);
    p.expect(u64::MAX - 1);
    p.ptr(6, 8);
    p.ptr(7, 16);
    p.reg(1, u64::MAX);
    // ZF starts set; the first unequal pair ends REPE immediately.
    p.code.extend([0x48, 0x31, 0xc0, 0xf3, 0xa6]);
    p.code.extend([0x48, 0x89, 0xc8]);
    p.expect(u64::MAX - 1);
    verify(&finish(p, 256), 2, 3);
}
