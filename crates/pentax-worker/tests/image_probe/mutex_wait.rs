// SPDX-License-Identifier: GPL-2.0-only

//! Original native contention, thread-local state and finite wait continuations.

use super::*;

const CHILD: i32 = 0xc00;

fn quantum(p: &mut Program) {
    p.reg(12, 3000);
    let start = p.code.len();
    p.code.extend([0x49, 0xff, 0xcc, 0x0f, 0x85]); // DEC R12; JNZ.
    let displacement = start as i32 - (p.code.len() as i32 + 4);
    p.code.extend(displacement.to_le_bytes());
}

fn same_slots(p: &mut Program, left: u8, right: u8, equal: bool) {
    p.load(0, left);
    p.load(10, right);
    p.code.extend([0x4c, 0x39, 0xd0]);
    p.code
        .extend([0x0f, if equal { 0x85 } else { 0x84 }, 0, 0, 0, 0]);
    p.branches.push(p.code.len() - 4);
}

fn child_zero(code: &mut Vec<u8>, branches: &mut Vec<usize>) {
    code.extend([0x48, 0x85, 0xc0]); // TEST RAX,RAX; full return register.
    checked(code, branches);
}

fn child_call(code: &mut Vec<u8>, slot: i32) {
    rip(code, CHILD, &[0xff, 0x15], 0xf00 + slot * 8);
}

fn child_set(code: &mut Vec<u8>, offset: u8, value: u8) {
    code.extend([0x48, 0xc7, 0x43, offset, value, 0, 0, 0]);
}

fn append_child(code: &mut Vec<u8>, child: Vec<u8>) {
    assert!(code.len() <= (CHILD - 0x800) as usize);
    code.resize((CHILD - 0x800) as usize, 0xcc);
    code.extend(child);
    assert!(code.len() <= (NAME - 0x800) as usize);
}

fn success(source: &Input, calls: u64) {
    let (status, fields, text) = probe(source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["phase"], "main-entry");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert_eq!(fields["kernel_pthread_objects"], "0");
    assert_eq!(
        fields["kernel_handles_live"],
        fields["kernel_handles_accounted"]
    );
}

#[test]
fn native_mutex_wait_hands_off_same_object_and_preserves_distinct_thread_tls() {
    // Stack: create handle0, mutex8, parent self16/errno24, stage32,
    // protected40, release48, child self56/errno64/id72, original token80.
    let imports = [
        "aI+OeCz8xrQ",
        "9BcDykPmo1I",
        "EI-5-jlq2dE",
        "9UK1vLZQft4",
        "tn3VlD0hG60",
        "2Of0f+3mhhE",
        "6UgtwV+0zb4",
    ];
    let mut p = Program::new();
    for offset in [0, 8, 32, 40, 48, 56, 64, 72] {
        p.set(offset, 0);
    }
    p.call(0);
    p.save(16);
    p.call(1);
    p.save(24);
    p.code.extend([0xc7, 0x00, 37, 0, 0, 0]);
    p.call(2);
    p.expect(1);
    p.ptr(7, 8);
    p.call(3);
    p.expect(0);
    p.load(0, 8);
    p.save(80);
    p.ptr(7, 0);
    p.reg(6, 0);
    p.path(2, CHILD);
    p.ptr(1, 0);
    p.reg(8, 0);
    p.call(6);
    p.expect(0);
    // Create forces the child to run. It reaches Lock and really parks;
    // neither a busy return nor invented acquisition may pass its checks.
    p.word(32, 1);
    p.word(40, 0);
    quantum(&mut p);
    p.word(32, 1);
    p.word(40, 0);
    same_slots(&mut p, 8, 80, true);
    p.ptr(7, 8);
    p.call(5);
    p.expect(I32_BUSY);
    p.set(48, 1);
    p.ptr(7, 8);
    p.call(4);
    p.expect(0);
    p.load(0, 32);
    p.code.extend([0x48, 0x83, 0xf8, 2, 0x75, 0xf5]);
    p.word(40, 42);
    p.word(72, 2);
    same_slots(&mut p, 0, 56, true);
    same_slots(&mut p, 16, 56, false);
    same_slots(&mut p, 24, 64, false);
    p.call(0);
    p.load(10, 16);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.call(2);
    p.expect(1);
    p.call(1);
    p.load(10, 24);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x8b, 0x00]);
    p.expect(37);
    p.load(0, 64);
    p.code.extend([0x8b, 0x00]);
    p.expect(19);
    p.ptr(7, 8);
    p.call(5);
    p.expect(0);
    p.word(8, 0);
    let (mut code, calls) = p.finish(None);

    let mut child = vec![0x53, 0x48, 0x89, 0xfb]; // RBX=shared parent stack.
    let mut branches = Vec::new();
    for (slot, offset) in [(0, 56), (1, 64), (2, 72)] {
        child_call(&mut child, slot);
        child.extend([0x48, 0x89, 0x43, offset]);
        if slot == 1 {
            child.extend([0xc7, 0x00, 19, 0, 0, 0]);
        }
    }
    child_set(&mut child, 32, 1);
    child.extend([0x48, 0x8d, 0x7b, 8]);
    child_call(&mut child, 3);
    child_zero(&mut child, &mut branches);
    child.extend([0x48, 0x83, 0x7b, 48, 1]);
    checked(&mut child, &mut branches);
    child.extend([0x48, 0x8b, 0x43, 8, 0x48, 0x3b, 0x43, 80]);
    checked(&mut child, &mut branches);
    child_set(&mut child, 40, 42);
    child.extend([0x48, 0x8d, 0x7b, 8]);
    child_call(&mut child, 4);
    child_zero(&mut child, &mut branches);
    child_set(&mut child, 32, 2);
    child.extend([0xb8, 73, 0, 0, 0, 0x5b, 0xc3]);
    finish_checks(&mut child, branches);
    append_child(&mut code, child);
    let source = image(&code, &imports);
    success(&source, calls + 5);
}

#[test]
fn native_mutex_stored_protocols_one_and_two_preserve_real_exclusion() {
    // These stored source protocols do not claim priority inheritance/protection.
    let imports = [
        "F8bUHwAG284",
        "1FGvU0i9saQ",
        "cmo1RIYva9o",
        "smWEktiyyG0",
        "9UK1vLZQft4",
        "tn3VlD0hG60",
        "2Of0f+3mhhE",
        "6UgtwV+0zb4",
    ];
    for protocol in [1, 2] {
        let mut p = Program::new();
        for offset in [0, 8, 16, 24, 32, 40] {
            p.set(offset, 0);
        }
        p.ptr(7, 0);
        p.call(0);
        p.expect(0);
        p.ptr(7, 0);
        p.reg(6, protocol);
        p.call(1);
        p.expect(0);
        p.ptr(7, 8);
        p.ptr(6, 0);
        p.reg(2, 0);
        p.call(2);
        p.expect(0);
        p.ptr(7, 0);
        p.call(3);
        p.expect(0);
        p.word(0, 0);
        p.ptr(7, 8);
        p.call(4);
        p.expect(0);
        p.ptr(7, 16);
        p.reg(6, 0);
        p.path(2, CHILD);
        p.ptr(1, 0);
        p.reg(8, 0);
        p.call(7);
        p.expect(0);
        p.word(24, 1);
        p.word(32, 0);
        quantum(&mut p);
        p.word(24, 1);
        p.word(32, 0);
        p.set(40, 1);
        p.ptr(7, 8);
        p.call(5);
        p.expect(0);
        p.load(0, 24);
        p.code.extend([0x48, 0x83, 0xf8, 2, 0x75, 0xf5]);
        p.word(32, 42);
        p.ptr(7, 8);
        p.call(6);
        p.expect(0);
        p.word(8, 0);
        let (mut code, calls) = p.finish(None);
        let mut child = vec![0x53, 0x48, 0x89, 0xfb];
        let mut branches = Vec::new();
        child_set(&mut child, 24, 1);
        child.extend([0x48, 0x8d, 0x7b, 8]);
        child_call(&mut child, 4);
        child_zero(&mut child, &mut branches);
        child.extend([0x48, 0x83, 0x7b, 40, 1]);
        checked(&mut child, &mut branches);
        child_set(&mut child, 32, 42);
        child.extend([0x48, 0x8d, 0x7b, 8]);
        child_call(&mut child, 5);
        child_zero(&mut child, &mut branches);
        child_set(&mut child, 24, 2);
        child.extend([0xb8, 73, 0, 0, 0, 0x5b, 0xc3]);
        finish_checks(&mut child, branches);
        append_child(&mut code, child);
        success(&image(&code, &imports), calls + 2);
    }
}

#[test]
fn native_mutex_finite_deadlines_preserve_owner_and_indefinite_cycle_stops() {
    // Both finite self waits must expire without releasing the held mutex.
    // POSIX returns positive60 without changing the selected thread's errno.
    let mut p = Program::new();
    p.call(7);
    p.save(0);
    p.set_errno(37);
    p.set(8, 0);
    p.ptr(7, 8);
    p.call(0);
    p.expect(0);
    p.ptr(7, 8);
    p.reg(6, 0xa5a5_a5a5_0000_1388); // Relative low32: 5000microseconds.
    p.call(1);
    p.expect(0x8002_003c);
    p.errno(37);
    p.reg(7, 0); // CLOCK_REALTIME.
    p.ptr(6, 32);
    p.call(3);
    p.expect(0);
    p.load(0, 40);
    p.code.extend([0x48, 0x05]); // ADD RAX,5000000nanoseconds.
    p.code.extend(5_000_000_u32.to_le_bytes());
    p.code.extend([0x48, 0x3d]); // CMP RAX,1000000000.
    p.code.extend(1_000_000_000_u32.to_le_bytes());
    p.code.extend([0x72, 11, 0x48, 0x2d]); // JB normalized; SUB RAX,1e9.
    p.code.extend(1_000_000_000_u32.to_le_bytes());
    p.code.extend([0x48, 0xff, 0x44, 0x24, 32]); // INC seconds on carry.
    p.save(40);
    p.ptr(7, 8);
    p.ptr(6, 32);
    p.call(2);
    p.expect(60);
    p.errno(37);
    p.ptr(7, 8);
    p.call(4);
    p.expect(I32_BUSY);
    p.ptr(7, 8);
    p.call(5);
    p.expect(0);
    p.ptr(7, 8);
    p.call(6);
    p.expect(0);
    p.word(8, 0);
    let (code, calls) = p.finish(None);
    let source = image(
        &code,
        &[
            "9UK1vLZQft4",
            "IafI2PxcPnQ",
            "Io9+nTKXZtA",
            "lLMT9vJAck0",
            "upoVrzMHFeE",
            "tn3VlD0hG60",
            "2Of0f+3mhhE",
            "9BcDykPmo1I",
        ],
    );
    success(&source, calls);

    // Parent holds A; child holds B and parks on A. Parent then parks on B.
    // Any premature completion reaches an explicit guest failure instruction.
    let mut p = Program::new();
    for offset in [0, 8, 16, 24] {
        p.set(offset, 0);
    }
    p.ptr(7, 0);
    p.call(1);
    p.expect(0);
    p.ptr(7, 16);
    p.reg(6, 0);
    p.path(2, CHILD);
    p.ptr(1, 0);
    p.reg(8, 0);
    p.call(0);
    p.expect(0);
    p.word(24, 1);
    p.ptr(7, 8);
    p.call(1);
    p.code.extend([0x0f, 0x0b]);
    let (mut code, calls) = p.finish(None);
    let mut child = vec![0x53, 0x48, 0x89, 0xfb, 0x48, 0x8d, 0x7b, 8];
    let mut branches = Vec::new();
    child_call(&mut child, 1);
    child_zero(&mut child, &mut branches);
    child_set(&mut child, 24, 1);
    child.extend([0x48, 0x89, 0xdf]); // RDI=&A.
    child_call(&mut child, 1);
    child.extend([0x0f, 0x0b]);
    finish_checks(&mut child, branches);
    append_child(&mut code, child);
    let source = image(&code, &["6UgtwV+0zb4", "9UK1vLZQft4"]);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "thread-deadlock");
    assert_eq!(fields["service_stops"], (calls + 2).to_string());
    assert!(!fields.contains_key("value"));
}
