// SPDX-License-Identifier: GPL-2.0-only

//! Actual parked thread continuation, reserved counts and source wake results.
use super::*;

const IMPORTS: [&str; 7] = [
    "188x57JYp0g",
    "Zxa0VhQVTsk",
    "4czppHBiriw",
    "4DM06U2BNEY",
    "R1Jvn8bSCW8",
    "6UgtwV+0zb4",
    "12wOHk8ywb0",
];
fn create(p: &mut Program) {
    p.set(0, 0);
    p.ptr(7, 0);
    p.path(6, NAME);
    p.reg(2, 1);
    p.reg(1, 0);
    p.reg(8, 2);
    p.reg(9, 0);
    p.call(0);
    p.expect(0);
}

#[test]
fn native_kernel_semaphore_wait_resumes_with_signal_cancel_and_delete_results() {
    for (mode, expected) in [(0, 0), (1, 0x8002_0055), (2, 0x8002_000d)] {
        let mut p = Program::new();
        create(&mut p);
        p.set(24, 0);
        p.set(32, 0);
        p.set(48, 0);
        p.reg(0, 0xa5a5_a5a5_7fff_ffff);
        p.save(56);
        p.ptr(7, 16);
        p.reg(6, 0);
        p.path(2, 0xc00);
        p.ptr(1, 0);
        p.reg(8, 0);
        p.call(5);
        p.expect(0);
        p.word(24, 1); // Child reached the actual pending wait and yielded.
        p.load(7, 0);
        match mode {
            0 => {
                p.reg(6, 1);
                p.call(2);
                p.expect(0);
                // The grant reserves count BEFORE the parked child is selected.
                p.load(7, 0);
                p.reg(6, 1);
                p.call(6);
                p.expect(0x8002_0010);
            }
            1 => {
                p.reg(6, 0);
                p.ptr(2, 48);
                p.call(3);
                p.expect(0);
                p.word(48, 1);
            }
            _ => {
                p.call(4);
                p.expect(0);
            }
        }
        p.load(0, 24);
        p.code.extend([0x48, 0x83, 0xf8, 2, 0x75, 0xf5]);
        p.word(32, expected);
        if mode == 0 {
            // The long deadline is signaled, never slept unconditionally.
            p.load(0, 56);
            p.code.extend([0x48, 0xc1, 0xe8, 32]);
            p.expect(0xa5a5_a5a5);
            p.code.extend([
                0x8b, 0x44, 0x24, 56, 0x85, 0xc0, 0x0f, 0x95, 0xc0, 0x0f, 0xb6, 0xc0,
            ]);
            p.expect(1);
        }
        if mode != 2 {
            p.load(7, 0);
            p.call(4);
            p.expect(0);
        }
        let (mut code, calls) = p.finish(None);
        assert!(code.len() < 0x400);
        code.resize(0x400, 0xcc);
        let mut child = vec![0x53, 0x48, 0x89, 0xfb];
        child.extend([0x48, 0xc7, 0x43, 24, 1, 0, 0, 0]);
        child.extend([0x48, 0x8b, 0x3b, 0xbe, 1, 0, 0, 0]);
        if mode == 0 {
            child.extend([0x48, 0x8d, 0x53, 56]);
        } else {
            child.extend([0x31, 0xd2]);
        }
        rip(&mut child, 0xc00, &[0xff, 0x15], 0xf08);
        child.extend([
            0x48, 0x89, 0x43, 32, 0x48, 0xc7, 0x43, 24, 2, 0, 0, 0, 0x5b, 0xc3,
        ]);
        code.extend(child);
        let source = image(&code, &IMPORTS);
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["service_stops"], (calls + 1).to_string());
    }
}

#[test]
fn native_kernel_semaphore_finite_wait_expires_without_a_runnable_notifier() {
    let mut p = Program::new();
    create(&mut p);
    p.reg(0, 0xa5a5_a5a5_0000_1388);
    p.save(24); // uint32 5000microseconds.
    p.load(7, 0);
    p.reg(6, 1);
    p.ptr(2, 24);
    p.call(1);
    p.expect(0x8002_003c);
    p.word(24, 0xa5a5_a5a5_0000_0000); // Exact four-byte remaining-time output.
    p.load(7, 0);
    p.reg(6, 1);
    p.call(6);
    p.expect(0x8002_0010);
    p.load(7, 0);
    p.call(4);
    p.expect(0);
    let (code, calls) = p.finish(None);
    let source = image(&code, &IMPORTS);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    // Admit a full-width finite deadline, but bound actual all-parked host time
    // rather than sleeping for the caller's entire requested deadline.
    let mut long = code;
    let at = long
        .windows(8)
        .position(|bytes| bytes == 0xa5a5_a5a5_0000_1388_u64.to_le_bytes())
        .unwrap();
    long[at..at + 8].copy_from_slice(&0xa5a5_a5a5_7fff_ffff_u64.to_le_bytes());
    let source = image(&long, &IMPORTS);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "execution-budget");
    assert_eq!(fields["budget"], "idle-waits");
    assert!(!fields.contains_key("value"));
}

#[test]
fn native_kernel_semaphore_indefinite_wait_without_notifier_is_a_real_deadlock() {
    let mut p = Program::new();
    create(&mut p);
    p.load(7, 0);
    p.reg(6, 1);
    p.reg(2, 0);
    p.call(1);
    let (code, _) = p.finish(None);
    let source = image(&code, &IMPORTS);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "thread-deadlock");
    assert!(!fields.contains_key("value"));
}
