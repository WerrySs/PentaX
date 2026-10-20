// SPDX-License-Identifier: GPL-2.0-only
//! Original priority ABI and cooperative scheduling consumers, without host controls.

use super::*;

const CHILD: i32 = 0x1100;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;

fn set_word(p: &mut Program, offset: u8, value: u64) {
    p.reg(0, value);
    p.save(offset);
}
fn finish(mut p: Program, result: Option<u8>) -> (Vec<u8>, u64) {
    if let Some(offset) = result {
        p.load(0, offset);
    } else {
        p.reg(0, 42);
    }
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0x700);
    (p.code, p.calls)
}
fn input(code: &[u8], imports: &[(&str, bool)], child: Option<&[u8]>) -> Input {
    assert!(imports.len() <= 8 && code.len() < 0x700);
    let original = posix::scoped_image(code, imports, 1);
    let mut bytes = original.bytes.clone();
    put(&mut bytes, 0x800, code); // Shared helper's unused names overlap this window.
    if let Some(child) = child {
        assert!(CHILD as usize + child.len() < 0x1800);
        bytes.resize(0x1800, 0);
        put(&mut bytes, 96, &0x1800_u64.to_le_bytes());
        put(&mut bytes, CHILD as usize, child);
    }
    create_input(bytes)
}
fn success(source: &Input, calls: u64) {
    let (status, fields, text) = probe(source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert_eq!(fields["initializer_calls"], "0", "{text}");
    assert_eq!(
        fields["kernel_handles_live"], fields["kernel_handles_accounted"],
        "{text}"
    );
}
fn wait(p: &mut Program, offset: u8) {
    let begin = p.code.len();
    p.load(0, offset);
    p.code.extend([0x48, 0x83, 0xf8, 1, 0x0f, 0x85]);
    p.code
        .extend((begin as i32 - (p.code.len() as i32 + 4)).to_le_bytes());
    quantum(p, 3000);
}
fn quantum(p: &mut Program, iterations: u64) {
    p.reg(12, iterations);
    let begin = p.code.len();
    p.code.extend([0x49, 0xff, 0xcc, 0x0f, 0x85]);
    p.code
        .extend((begin as i32 - (p.code.len() as i32 + 4)).to_le_bytes());
}
fn child_call(code: &mut Vec<u8>, slot: i32) {
    rip(code, CHILD, &[0xff, 0x15], 0xf00 + slot * 8);
}
fn child_expect(code: &mut Vec<u8>, branches: &mut Vec<usize>, value: u64) {
    code.extend([0x49, 0xba]);
    code.extend(value.to_le_bytes());
    code.extend([0x4c, 0x39, 0xd0]);
    checked(code, branches);
}
fn child_release(code: &mut Vec<u8>) {
    let begin = code.len();
    code.extend([0x48, 0x83, 0x7b, 8, 1, 0x0f, 0x85]);
    code.extend((begin as i32 - (code.len() as i32 + 4)).to_le_bytes());
}

#[test]
fn native_libkernel_priority_signed_words_limits_errno_and_invalid_output_boundaries() {
    let imports = [
        ("aI+OeCz8xrQ", false),
        ("9BcDykPmo1I", false),
        ("1tKyG7RlMJo", false),
        ("W0Hpm2X0uPE", false),
        ("CBNtXOoef-E", false),
        ("m0iS6jNsXds", false),
    ];
    let mut p = Program::new();
    p.call(1);
    p.save(0);
    p.set_errno(37);
    p.call(0);
    p.save(8);
    p.reg(7, 0);
    p.reg(6, 0);
    p.call(2); // Null thread wins over the source-null output trap.
    p.expect(0x8002_0003);
    p.errno(37);
    p.reg(7, 0);
    p.reg(6, u64::MAX);
    p.call(3);
    p.expect(0x8002_0003);
    p.errno(37);
    p.reg(7, u64::MAX);
    p.call(4);
    p.expect(256);
    p.errno(37);
    p.reg(7, 0xaaaa_0000_8000_0000);
    p.call(5);
    p.expect(767);
    p.errno(37);
    for value in [
        0x1122_3344_ffff_ffff,
        0x5566_7788_8000_0000,
        0x9988_7766_7fff_ffff,
        478,
        479,
        733,
    ] {
        p.load(7, 8);
        p.reg(6, value);
        p.call(3);
        p.expect(0);
        set_word(&mut p, 16, CANARY);
        p.load(7, 8);
        p.ptr(6, 16);
        p.call(2);
        p.expect(0);
        p.word(16, (CANARY & 0xffff_ffff_0000_0000) | (value & 0xffff_ffff));
        p.errno(37);
    }
    let (code, calls) = finish(p, None);
    success(&input(&code, &imports, None), calls);
    for output in [0, u64::MAX] {
        let mut p = Program::new();
        p.call(0);
        p.code.extend([0x48, 0x89, 0xc7]);
        p.reg(6, output);
        p.call(1);
        let (code, calls) = finish(p, None);
        let source = input(
            &code,
            &[("aI+OeCz8xrQ", false), ("1tKyG7RlMJo", false)],
            None,
        );
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "runtime-service", "{text}");
        assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
        assert!(!fields.contains_key("value"), "{text}");
    }
}

#[test]
fn native_posix_priority_aliases_preserve_policy_priority_store_order_and_errno() {
    let imports = [
        ("aI+OeCz8xrQ", false),
        ("9BcDykPmo1I", false),
        ("a2P9wYGeZvc", true),
        ("FIs3-UQT9sg", true),
        ("P41kTWUS3EI", true),
        ("Xs9hdiD7sAA", true),
        ("oIRFTjoILbg", true),
    ];
    let mut p = Program::new();
    p.call(1);
    p.save(0);
    p.set_errno(37);
    p.call(0);
    p.save(8);
    p.reg(7, 0);
    p.reg(6, u64::MAX);
    p.call(2);
    p.expect(3);
    p.errno(37);
    for slot in [3, 4] {
        p.reg(7, 0);
        p.reg(6, u64::MAX);
        p.reg(2, u64::MAX);
        p.call(slot);
        p.expect(22);
        p.load(7, 8);
        p.reg(6, 0);
        p.reg(2, u64::MAX);
        p.call(slot);
        p.expect(22);
        p.load(7, 8);
        p.ptr(6, 56);
        p.reg(2, 0);
        p.call(slot);
        p.expect(22);
        p.errno(37);
    }
    for slot in [5, 6] {
        p.reg(7, 0);
        p.reg(6, u64::MAX);
        p.reg(2, u64::MAX);
        p.call(slot);
        p.expect(22);
        p.load(7, 8);
        p.reg(6, u64::MAX);
        p.reg(2, 0);
        p.call(slot);
        p.expect(22);
    }
    set_word(&mut p, 32, 0xa5a5_a5a5_8000_0000);
    p.load(7, 8);
    p.reg(6, 0x1122_3344_ffff_ffff);
    p.ptr(2, 32);
    p.call(5);
    p.expect(0);
    for slot in [3, 4] {
        set_word(&mut p, 56, CANARY);
        set_word(&mut p, 64, CANARY);
        p.load(7, 8);
        p.ptr(6, 56);
        p.ptr(2, 64);
        p.call(slot);
        p.expect(0);
        p.word(56, 0xa5a5_a5a5_ffff_ffff);
        p.word(64, 0xa5a5_a5a5_8000_0000);
    }
    set_word(&mut p, 80, CANARY);
    p.load(7, 8);
    p.ptr(6, 80);
    p.ptr(2, 80);
    p.call(3);
    p.expect(0);
    p.word(80, 0xa5a5_a5a5_8000_0000); // Priority replaces policy at the exact alias.
    set_word(&mut p, 80, CANARY);
    p.load(7, 8);
    p.ptr(6, 80);
    p.ptr(2, 82);
    p.call(4);
    p.expect(0);
    p.word(80, 0xa5a5_8000_0000_ffff); // Partial alias, four-byte stores in source order.
    set_word(&mut p, 32, 0xa5a5_a5a5_7fff_ffff);
    p.load(7, 8);
    p.reg(6, 0x5566_7788_1234_5678);
    p.ptr(2, 32);
    p.call(6);
    p.expect(0);
    p.load(7, 8);
    p.reg(6, 700);
    p.call(2);
    p.expect(0);
    p.load(7, 8);
    p.ptr(6, 56);
    p.ptr(2, 64);
    p.call(4);
    p.expect(0);
    p.word(56, 0xa5a5_a5a5_1234_5678);
    p.word(64, 0xa5a5_a5a5_0000_02bc);
    p.errno(37);
    let (code, calls) = finish(p, None);
    success(&input(&code, &imports, None), calls);
}

#[test]
fn native_priority_changes_target_created_thread_while_parent_context_stays_independent() {
    let imports = [
        ("aI+OeCz8xrQ", false),
        ("9BcDykPmo1I", false),
        ("6UgtwV+0zb4", false),
        ("1tKyG7RlMJo", false),
        ("W0Hpm2X0uPE", false),
        ("a2P9wYGeZvc", true),
        ("FIs3-UQT9sg", true),
        ("Xs9hdiD7sAA", true),
    ];
    let mut p = Program::new();
    p.call(1);
    p.save(0);
    p.set_errno(37);
    p.call(0);
    p.save(8);
    for offset in [16, 24, 32, 64, 72] {
        p.set(offset, 0);
    }
    for offset in [40, 48, 56] {
        set_word(&mut p, offset, CANARY);
    }
    p.ptr(7, 16);
    p.reg(6, 0);
    p.path(2, CHILD);
    p.ptr(1, 16);
    p.reg(8, 0);
    p.call(2);
    p.expect(0);
    p.load(7, 16);
    p.reg(6, 0xaaaa_aaaa_ffff_fd25); // Arbitrary signed priority -731.
    p.call(4);
    p.expect(0);
    set_word(&mut p, 56, 0xa5a5_a5a5_7fff_ffff);
    p.load(7, 16);
    p.reg(6, 0x1122_3344_8000_0000);
    p.ptr(2, 56);
    p.call(7); // Changes attributes; runtime class is separately tested below.
    p.expect(0);
    p.load(7, 16);
    p.ptr(6, 40);
    p.call(3);
    p.expect(0);
    p.word(40, 0xa5a5_a5a5_7fff_ffff);
    p.load(7, 16);
    p.reg(6, 478);
    p.call(5);
    p.expect(0);
    p.set(24, 1);
    wait(&mut p, 32);
    p.word(40, 0xa5a5_a5a5_0000_01de);
    p.word(48, 0xa5a5_a5a5_8000_0000);
    p.word(56, 0xa5a5_a5a5_0000_01de);
    p.load(0, 16);
    p.load(10, 64);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.call(0);
    p.load(10, 8);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.call(1);
    p.load(10, 0);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.errno(37);
    p.load(0, 72);
    p.code.extend([0x8b, 0]);
    p.expect(19);
    p.load(7, 8);
    p.ptr(6, 40);
    p.call(3);
    p.expect(0);
    p.word(40, 0xa5a5_a5a5_0000_02bc);
    let (code, calls) = finish(p, None);
    let mut child = vec![0x53, 0x48, 0x89, 0xfb]; // Shared parent stack, not a host pointer.
    let mut branches = Vec::new();
    child_call(&mut child, 0);
    child.extend([0x48, 0x89, 0x43, 48]);
    child_call(&mut child, 1);
    child.extend([0x48, 0x89, 0x43, 56, 0xc7, 0, 19, 0, 0, 0]);
    child_release(&mut child);
    child.extend([0x48, 0x8b, 0x7b, 48, 0x48, 0x8d, 0x73, 24]);
    child_call(&mut child, 3);
    child_expect(&mut child, &mut branches, 0);
    child.extend([0x48, 0x8b, 0x43, 24]);
    child_expect(&mut child, &mut branches, 0xa5a5_a5a5_0000_01de);
    child.extend([
        0x48, 0x8b, 0x7b, 48, 0x48, 0x8d, 0x73, 32, 0x48, 0x8d, 0x53, 40,
    ]);
    child_call(&mut child, 6);
    child_expect(&mut child, &mut branches, 0);
    child.extend([
        0x48, 0xc7, 0x43, 16, 1, 0, 0, 0, 0xb8, 73, 0, 0, 0, 0x5b, 0xc3,
    ]);
    finish_checks(&mut child, branches);
    success(&input(&code, &imports, Some(&child)), calls + 4);
}

#[test]
fn native_cooperative_priority_changes_real_child_progress_without_host_timing() {
    let mut progress = Vec::new();
    for priority in [0, 700, 767] {
        let mut p = Program::new();
        for offset in [0, 32, 40, 48, 56] {
            p.set(offset, 0);
        }
        p.ptr(7, 0);
        p.reg(6, 0); // Default child affinity is0x7f, so no single-core demotion.
        p.path(2, CHILD);
        p.ptr(1, 32);
        p.reg(8, 0);
        p.call(0);
        p.expect(0);
        p.load(7, 0);
        p.reg(6, priority);
        p.call(1);
        p.expect(0);
        p.set(32, 1);
        quantum(&mut p, 8192); // Same original instruction loop in every run.
        p.load(0, 40);
        p.save(64);
        p.set(48, 1);
        wait(&mut p, 56);
        let (code, calls) = finish(p, Some(64));
        let mut child = vec![0x53, 0x48, 0x89, 0xfb];
        // This counter's release flag is offset0 rather than the ABI fixture's8.
        let release = child.len();
        child.extend([0x48, 0x83, 0x3b, 1, 0x0f, 0x85]);
        child.extend((release as i32 - (child.len() as i32 + 4)).to_le_bytes());
        let count = child.len();
        child.extend([0x48, 0xff, 0x43, 8, 0x48, 0x83, 0x7b, 16, 0, 0x0f, 0x84]);
        child.extend((count as i32 - (child.len() as i32 + 4)).to_le_bytes());
        child.extend([
            0x48, 0xc7, 0x43, 24, 1, 0, 0, 0, 0xb8, 73, 0, 0, 0, 0x5b, 0xc3,
        ]);
        let source = input(
            &code,
            &[("6UgtwV+0zb4", false), ("W0Hpm2X0uPE", false)],
            Some(&child),
        );
        // Use the existing finite100000-block startup profile for this sample;
        // its deliberate loops exceed the compact10000-block probe allowance.
        let (status, fields, text) = probe_arguments(&[
            "--image".into(),
            source.path.as_os_str().to_owned(),
            "--execution-profile".into(),
            "startup".into(),
        ]);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert_eq!(fields["initializer_calls"], "0", "{text}");
        assert_eq!(
            fields["kernel_handles_live"], fields["kernel_handles_accounted"],
            "{text}"
        );
        progress.push(fields["value"].parse::<u64>().unwrap());
    }
    assert!(
        progress[0] > progress[1] && progress[1] > progress[2] && progress[2] > 0,
        "Actual child progress by High/Normal/Low: {progress:?}"
    );
}
