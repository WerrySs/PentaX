// SPDX-License-Identifier: GPL-2.0-only
//! Original two-way event-flag handoff with real parked native thread contexts.

use super::*;

const ENTRY: i32 = 0x1000;
const CHILD: i32 = 0x4000;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const IMPORTS: [&str; 6] = [
    "BpFoboUJoZU", // CreateEventFlag
    "JTvBflhYazQ", // WaitEventFlag
    "IOnSvHzqu6A", // SetEventFlag
    "8mql9OcQnd4", // DeleteEventFlag
    "6UgtwV+0zb4", // PthreadCreate
    "9lvj5DjHZiA", // PollEventFlag
];

fn call(p: &mut Program, base: i32, slot: i32) {
    rip(&mut p.code, base, &[0xff, 0x15], 0xf00 + slot * 8);
    p.calls += 1;
}

fn set_word(p: &mut Program, offset: u8, value: u64) {
    p.reg(0, value);
    p.save(offset);
}

fn vector_words(register: u8, child: bool) -> [u64; 4] {
    let tag = (u64::from(register) + if child { 33 } else { 1 }) * 0x0101_0101_0101_0101;
    [
        0x0123_4567_89ab_cdef ^ tag,
        0xfedc_ba98_7654_3210 ^ tag,
        0x8877_6655_4433_2211 ^ tag,
        0x89ab_cdef_0123_4567 ^ tag,
    ]
}

fn seed_vectors(p: &mut Program, child: bool) {
    for register in 0..16 {
        for (word, value) in vector_words(register, child).into_iter().enumerate() {
            set_word(p, 16 + word as u8 * 8, value);
        }
        p.code.extend([
            0xc5,
            if register >= 8 { 0x7e } else { 0xfe },
            0x6f,
            0x44 | ((register & 7) << 3),
            0x24,
            16,
        ]);
    }
}

fn check_vectors(p: &mut Program, child: bool) {
    for register in 0..16 {
        p.code.extend([
            0xc5,
            if register >= 8 { 0x7e } else { 0xfe },
            0x7f,
            0x44 | ((register & 7) << 3),
            0x24,
            80,
        ]);
        for (word, value) in vector_words(register, child).into_iter().enumerate() {
            p.word(80 + word as u8 * 8, value);
        }
    }
}

fn prepare_context(p: &mut Program, child: bool) {
    p.stack(0x89, 4, 8);
    // Different actual guest rounding controls prove per-thread MXCSR state;
    // the upper canary also detects any wider-than-four-byte control store.
    set_word(
        p,
        120,
        0xa5a5_a5a5_0000_0000 | if child { 0x3f84 } else { 0x5f84 },
    );
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 120]);
    seed_vectors(p, child);
}

fn check_context(p: &mut Program, child: bool) {
    check_vectors(p, child);
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 120]);
    p.word(
        120,
        0xa5a5_a5a5_0000_0000 | if child { 0x3f84 } else { 0x5f84 },
    );
    p.code.extend([0x48, 0x89, 0xe0]);
    p.load(10, 8);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
}

fn finish(mut p: Program, value: u64) -> Vec<u8> {
    p.reg(0, value);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    p.code
}

fn original_image(code: &[u8]) -> Input {
    let template = image(&[], &IMPORTS);
    let mut bytes = template.bytes.clone();
    bytes.resize(0x8000, 0);
    assert!(code.len() < 0x7000);
    put(&mut bytes, ENTRY as usize, code);
    put(&mut bytes, 24, &(ENTRY as u64).to_le_bytes());
    put(&mut bytes, 96, &0x8000_u64.to_le_bytes());
    put(&mut bytes, 104, &0x8000_u64.to_le_bytes());
    put(&mut bytes, NAME as usize, b"original-event-flag\0");
    create_input(bytes)
}

#[test]
fn native_event_flag_wait_parks_both_threads_and_restores_all_ymm_with_real_clear_modes() {
    // Shared parent stack: flag0, RSP8, seed16..47, childhandle48,
    // childstage56, childpattern64, childproof72, vectorcheck80..111,
    // parentpattern112, MXCSR+tailcanary120.
    let mut parent = Program::new();
    prepare_context(&mut parent, false);
    parent.set(0, 0);
    parent.set(48, 0);
    parent.set(56, 0);
    set_word(&mut parent, 64, CANARY);
    parent.set(72, 0);
    set_word(&mut parent, 112, CANARY);
    parent.ptr(7, 0);
    rip(&mut parent.code, ENTRY, &[0x48, 0x8d, 0x35], NAME);
    parent.reg(2, 0x21); // FIFO spelling + multiple waiters; no host priority claim.
    parent.reg(1, 0); // No bit can satisfy the child's initial wait.
    parent.reg(8, 0);
    call(&mut parent, ENTRY, 0);
    parent.expect(0);
    parent.ptr(7, 48);
    parent.reg(6, 0);
    rip(&mut parent.code, ENTRY, &[0x48, 0x8d, 0x15], CHILD);
    parent.ptr(1, 0);
    parent.reg(8, 0);
    call(&mut parent, ENTRY, 4); // The existing scheduler forces the real child first.
    parent.expect(0);
    parent.word(56, 1);
    parent.word(64, CANARY);
    parent.word(72, 0); // Child cannot have passed the unsatisfied wait.
    parent.load(7, 0);
    parent.reg(6, 1);
    call(&mut parent, ENTRY, 2);
    parent.expect(0);
    parent.load(7, 0);
    parent.reg(6, 2);
    parent.reg(2, 0x12); // OR + clear ALL, after observing the pre-clear pattern.
    parent.ptr(1, 112);
    parent.reg(8, 0); // An indefinite wait, not a timeout/poll busy loop.
    call(&mut parent, ENTRY, 1);
    parent.expect(0);
    parent.word(112, 2); // The child consumed bit1 before setting acknowledgement2.
    parent.word(56, 2);
    parent.word(64, 1);
    parent.word(72, 42); // Published only after the child's full context checks.
    check_context(&mut parent, false);
    parent.word(112, 2); // Full vector stores did not damage the adjacent output.
    for (word, value) in vector_words(15, false).into_iter().enumerate() {
        parent.word(16 + word as u8 * 8, value);
    }
    parent.load(7, 0);
    parent.reg(6, 3);
    parent.reg(2, 2); // OR, no clear; both bits must now be absent.
    parent.ptr(1, 112);
    call(&mut parent, ENTRY, 5);
    parent.expect(0x8002_0010); // Poll's source EBUSY, not an invented wake result.
    parent.word(112, 0);
    parent.load(7, 0);
    call(&mut parent, ENTRY, 3);
    parent.expect(0);
    assert_eq!(parent.calls, 6);
    let mut code = finish(parent, 42);
    assert!(code.len() < (CHILD - ENTRY) as usize);
    code.resize((CHILD - ENTRY) as usize, 0xcc);

    let mut child = Program::new();
    child.stack(0x89, 7, 0); // Save the actual shared argument on the child's own stack.
    prepare_context(&mut child, true);
    for offset in [56, 64, 72] {
        set_word(&mut child, offset, CANARY);
    }
    child.load(11, 0);
    child.code.extend([0x49, 0xc7, 0x43, 56, 1, 0, 0, 0]);
    child.code.extend([0x49, 0x8b, 0x3b]); // Same actual flag handle from the parent.
    child.reg(6, 1);
    child.reg(2, 0x21); // AND + clear only the requested bit.
    child.ptr(1, 64);
    child.reg(8, 0);
    call(&mut child, CHILD, 1);
    child.expect(0);
    child.word(56, CANARY);
    child.word(64, 1);
    child.word(72, CANARY);
    check_context(&mut child, true);
    child.load(11, 0);
    child.load(0, 64);
    child.code.extend([0x49, 0x89, 0x43, 64]);
    child.code.extend([0x49, 0xc7, 0x43, 56, 2, 0, 0, 0]);
    child.code.extend([0x49, 0xc7, 0x43, 72, 42, 0, 0, 0]);
    child.code.extend([0x49, 0x8b, 0x3b]);
    child.reg(6, 2);
    call(&mut child, CHILD, 2);
    child.expect(0);
    assert_eq!(child.calls, 2);
    code.extend(finish(child, 73));
    let source = original_image(&code);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], "8", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert_eq!(fields["scheduling_threads"], "2", "{text}");
    assert_eq!(fields["scheduling_pending_waits"], "0", "{text}");
    assert_eq!(fields["scheduling_main_state"], "unparked", "{text}");
    assert_eq!(fields["scheduling_main_wait"], "none", "{text}");
    assert_eq!(fields["scheduling_active_role"], "main", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    // Both contexts independently load/check all16 complete vectors and access
    // MXCSR each way. Temporal copies promote under the actual AVX profile;
    // all spans still receive live preflight and MXCSR remains mediated.
    assert_modeled_vector_counters(&fields, [(68, 2064), (4, 16), (4, 16)]);
    assert_eq!(fields["fp_status"], "4", "{text}");
    assert_eq!(
        fields["kernel_handles_live"], fields["kernel_handles_accounted"],
        "{text}"
    );
    assert!(
        fields["native_instructions"].parse::<u64>().unwrap() > 0,
        "{text}"
    );
}
