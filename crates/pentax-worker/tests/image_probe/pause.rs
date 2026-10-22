// SPDX-License-Identifier: GPL-2.0-only
//! Original native PAUSE consumers, including a bounded pthread spin/handoff.
use super::*;

const CHILD: i32 = 0x1100;
const PARENT_VECTOR: u64 = 0x1111_2222_3333_4444;
const CHILD_VECTOR: u64 = 0x5555_6666_7777_8888;

fn vector(p: &mut Program, value: u64, check: bool) {
    if check {
        p.code.extend([0x66, 0x48, 0x0f, 0x7e, 0xc0]); // MOVQ RAX,XMM0
        p.expect(value);
    } else {
        p.reg(0, value);
        p.code.extend([0x66, 0x48, 0x0f, 0x6e, 0xc0]); // MOVQ XMM0,RAX
    }
}
fn check_register(p: &mut Program, register: u8, value: u64) {
    p.reg(10, value);
    p.code
        .extend([0x4c | u8::from(register >= 8), 0x39, 0xd0 | (register & 7)]);
    checked(&mut p.code, &mut p.branches);
}
fn tls(p: &mut Program, value: u64, check: bool) {
    if !check {
        p.reg(0, value);
    }
    p.code
        .extend([0x64, 0x48, if check { 0x8b } else { 0x89 }, 4, 0x25]);
    p.code.extend(0x28_u32.to_le_bytes());
    if check {
        p.expect(value);
    }
}

#[test]
fn native_pause_at_child_entry_polls_until_existing_quantum_handoff_without_yield_service() {
    let mut main = Program::new();
    main.set(64, 0); // Stage, followed by child's real polling count.
    main.set(72, 0);
    main.reg(13, 0x1212);
    vector(&mut main, PARENT_VECTOR, false);
    tls(&mut main, 0x1111, false);
    main.ptr(7, 0);
    main.reg(6, 0);
    main.path(2, CHILD);
    main.ptr(1, 64);
    main.reg(8, 0);
    main.call(0);
    main.expect(0);
    // Create starts the child. PAUSE at its very first instruction must not
    // synthesize a guest yield: the child publishes stage 1 and does actual
    // bounded polling before the unchanged scheduler quantum returns here.
    main.word(64, 1);
    main.load(0, 72);
    main.code
        .extend([0x48, 0x3d, 128, 0, 0, 0, 0x0f, 0x86, 0, 0, 0, 0]);
    main.branches.push(main.code.len() - 4); // Require count > 128.
    check_register(&mut main, 13, 0x1212);
    vector(&mut main, PARENT_VECTOR, true);
    tls(&mut main, 0x1111, true);
    main.set(64, 2);
    main.load(7, 0);
    main.ptr(6, 16);
    main.call(1); // Join parks until the actual child RET; no host timing wait.
    main.expect(0);
    main.word(16, 42);
    main.word(64, 3);
    check_register(&mut main, 13, 0x1212);
    vector(&mut main, PARENT_VECTOR, true);
    tls(&mut main, 0x1111, true);
    let (main, main_calls) = main.finish(None);

    let mut child = Program::new();
    child.code.insert(0, 0x90);
    child.code.insert(0, 0xf3); // The child's actual first instruction is PAUSE.
    child.code.extend([0x48, 0x89, 0xfb]); // RBX = owned parent's shared stage.
    child.reg(13, 0x3434);
    child.reg(12, 4096); // Hard bound even if no parent ever releases it.
    vector(&mut child, CHILD_VECTOR, false);
    tls(&mut child, 0x2222, false);
    child.code.extend([0x48, 0xc7, 3, 1, 0, 0, 0]);
    let poll = child.code.len();
    child.code.extend([0xf3, 0x90, 0x48, 0xff, 0x43, 8]); // PAUSE; INC count.
    child
        .code
        .extend([0x48, 0x83, 0x3b, 2, 0x0f, 0x84, 0, 0, 0, 0]);
    let released = child.code.len() - 4;
    child.code.extend([0x49, 0xff, 0xcc, 0x0f, 0x85]);
    child
        .code
        .extend((poll as i32 - (child.code.len() as i32 + 4)).to_le_bytes());
    child.code.extend([0x0f, 0x0b]); // Exhaustion never becomes success.
    let resume = child.code.len();
    child.code[released..released + 4]
        .copy_from_slice(&((resume - released - 4) as i32).to_le_bytes());
    check_register(&mut child, 13, 0x3434);
    vector(&mut child, CHILD_VECTOR, true);
    tls(&mut child, 0x2222, true);
    child.code.extend([0x48, 0xc7, 3, 3, 0, 0, 0]);
    let (child, child_calls) = child.finish(None);
    assert_eq!(child_calls, 0);
    let original = image(&[], &["6UgtwV+0zb4", "onNY9Byn-W8"]);
    let mut bytes = original.bytes.clone();
    bytes.resize(0x1800, 0);
    put(&mut bytes, 0x800, &main);
    put(&mut bytes, CHILD as usize, &child);
    put(&mut bytes, 96, &0x1800_u64.to_le_bytes());
    let input = create_input(bytes);
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], main_calls.to_string(), "{text}");
    assert_eq!(main_calls, 2);
    assert_eq!(fields["scheduling_threads"], "2", "{text}");
    assert_eq!(fields["scheduling_returned"], "1", "{text}");
    assert_eq!(fields["scheduling_pending_waits"], "0", "{text}");
    assert!(
        fields["native_instructions"].parse::<u64>().unwrap() > 256,
        "{text}"
    );
    println!(
        "pause_native_thread_entry_available=true pause_native_thread_entry_programs=1 pause_native_thread_entry_threads=2"
    );
}

#[test]
fn native_pause_finite_register_poll_retires_normally_with_unchanged_context_and_no_services() {
    let mut p = Program::new();
    p.reg(12, 513);
    p.reg(13, 0x5656);
    vector(&mut p, PARENT_VECTOR, false);
    let again = p.code.len();
    p.code.extend([0xf3, 0x90, 0x49, 0xff, 0xcc, 0x0f, 0x85]);
    p.code
        .extend((again as i32 - (p.code.len() as i32 + 4)).to_le_bytes());
    check_register(&mut p, 13, 0x5656);
    vector(&mut p, PARENT_VECTOR, true);
    let (code, calls) = p.finish(None);
    assert_eq!(calls, 0);
    let (status, fields, text) = probe(&image(&code, &[]));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], "0", "{text}");
    assert_eq!(fields["scheduling_threads"], "1", "{text}");
    assert_eq!(fields["scheduling_pending_waits"], "0", "{text}");
    assert!(
        fields["native_instructions"].parse::<u64>().unwrap() >= 1026,
        "{text}"
    );
    println!(
        "pause_native_finite_poll_available=true pause_native_finite_poll_programs=1 pause_native_finite_poll_iterations=513"
    );
}
