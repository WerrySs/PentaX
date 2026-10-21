// SPDX-License-Identifier: GPL-2.0-only
//! Original cooperative timing consumers. No host sleep or injected clock.

use super::*;

const CHILD: i32 = 0x1200;
const MICROS: u32 = 50_000;
const CANARY: u64 = 0x8877_6655_4433_2211;
const CHILD_CANARY: u64 = 0x1020_3040_5060_7080;
const VECTOR: [u64; 4] = [
    0x0123_4567_89ab_cdef,
    0xfedc_ba98_7654_3210,
    0x8877_6655_4433_2211,
    0x89ab_cdef_0123_4567,
];

fn load(p: &mut Program, register: u8, offset: i32) {
    p.code.extend([
        0x48 | if register >= 8 { 4 } else { 0 },
        0x8b,
        0x84 | ((register & 7) << 3),
        0x24,
    ]);
    p.code.extend(offset.to_le_bytes());
}
fn save(p: &mut Program, offset: i32) {
    p.code.extend([0x48, 0x89, 0x84, 0x24]);
    p.code.extend(offset.to_le_bytes());
}
fn store(p: &mut Program, offset: i32, value: u64) {
    p.reg(0, value);
    save(p, offset);
}
fn word(p: &mut Program, offset: i32, expected: u64) {
    load(p, 0, offset);
    p.expect(expected);
}
fn call(p: &mut Program, base: i32, slot: i32) {
    rip(&mut p.code, base, &[0xff, 0x15], 0xf00 + slot * 8);
    p.calls += 1;
}
fn refuse_if(p: &mut Program, condition: u8) {
    p.code.extend([0x0f, condition, 0, 0, 0, 0]);
    p.branches.push(p.code.len() - 4);
}
fn vector(p: &mut Program, opcode: u8, offset: i32) {
    p.code.extend([0xc5, 0x7e, opcode, 0xbc, 0x24]);
    p.code.extend(offset.to_le_bytes());
}
fn mxcsr(p: &mut Program, opcode: u8, offset: i32) {
    p.code.extend([0x0f, 0xae, opcode, 0x24]);
    p.code.extend(offset.to_le_bytes());
}
fn seed(p: &mut Program, child: bool) {
    let (rsp, control, input) = if child { (8, 80, 16) } else { (88, 96, 112) };
    p.code.extend([0x48, 0x89, 0xe0]);
    save(p, rsp);
    for (index, value) in VECTOR.into_iter().enumerate() {
        store(p, input + index as i32 * 8, value ^ u64::from(child));
    }
    store(
        p,
        control,
        (CANARY & !0xffff_ffff) | if child { 0x3fc0 } else { 0x5f84 },
    );
    mxcsr(p, 0x94, control);
    vector(p, 0x6f, input);
    p.reg(9, if child { CHILD_CANARY } else { CANARY });
}
fn preserved(p: &mut Program, child: bool) {
    let (rsp, control, output) = if child { (8, 80, 48) } else { (88, 96, 144) };
    vector(p, 0x7f, output);
    for (index, value) in VECTOR.into_iter().enumerate() {
        word(p, output + index as i32 * 8, value ^ u64::from(child));
    }
    mxcsr(p, 0x9c, control);
    word(
        p,
        control,
        (CANARY & !0xffff_ffff) | if child { 0x3fc0 } else { 0x5f84 },
    );
    p.code.extend([0x48, 0x89, 0xe0]);
    load(p, 10, rsp);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x4c, 0x89, 0xc8]);
    p.expect(if child { CHILD_CANARY } else { CANARY });
}

fn parent(wait_slot: i32) -> (Vec<u8>, u64) {
    let mut p = Program::new();
    p.call(5);
    p.save(8);
    p.call(4);
    p.expect(1);
    p.call(6);
    p.save(0);
    p.set_errno(37);
    p.code.extend([0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0]);
    p.save(16);
    seed(&mut p, false);
    // Both bindings observe low32 microseconds, including an immediate zero.
    for slot in [0, 1] {
        p.reg(7, 0xdead_beef_0000_0000);
        p.call(slot);
        p.expect(0);
        p.errno(37);
    }
    store(&mut p, 32, 0);
    store(&mut p, 184, 0); // Set only after the real timed syscall returns.
    store(&mut p, 200, CANARY);
    p.ptr(7, 24);
    p.reg(6, 0);
    p.path(2, CHILD);
    p.ptr(1, 32);
    p.reg(8, 0);
    p.call(3);
    p.expect(0);
    // Create forced the child to run; its release loop then yielded this CPU.
    // Fewer than one scheduler quantum separates release from the timed call.
    word(&mut p, 32, 1);
    p.call(2);
    p.save(72);
    store(&mut p, 32, 2);
    p.reg(7, 0xdead_beef_0000_0000 | u64::from(MICROS));
    p.call(wait_slot);
    p.expect(0);
    store(&mut p, 184, 1);
    p.call(2);
    p.save(80);
    // A host-blocking dispatcher cannot run the released child before return.
    // The child also checked the return flag before publishing this marker.
    word(&mut p, 32, 42);
    word(&mut p, 40, 2);
    load(&mut p, 0, 48);
    load(&mut p, 10, 24);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    load(&mut p, 10, 8);
    p.code.extend([0x4c, 0x39, 0xd0]);
    refuse_if(&mut p, 0x84);
    load(&mut p, 0, 56);
    load(&mut p, 10, 16);
    p.code.extend([0x4c, 0x39, 0xd0]);
    refuse_if(&mut p, 0x84);
    load(&mut p, 0, 64);
    p.code.extend([0x8b, 0x00]);
    p.expect(19);
    // Actual process-microsecond observations bracket the requested interval;
    // injected kernel-clock tests independently prove exact deadline behavior.
    load(&mut p, 0, 80);
    load(&mut p, 10, 72);
    p.code.extend([0x4c, 0x39, 0xd0]);
    refuse_if(&mut p, 0x82);
    p.code.extend([0x4c, 0x29, 0xd0, 0x48, 0x3d]);
    p.code.extend(MICROS.to_le_bytes());
    refuse_if(&mut p, 0x82);
    load(&mut p, 0, 192);
    load(&mut p, 10, 72);
    p.code.extend([0x4c, 0x39, 0xd0]);
    refuse_if(&mut p, 0x82);
    load(&mut p, 10, 80);
    p.code.extend([0x4c, 0x39, 0xd0]);
    refuse_if(&mut p, 0x87);
    p.call(5);
    load(&mut p, 10, 8);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.call(4);
    p.expect(1);
    p.call(6);
    load(&mut p, 10, 0);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.errno(37);
    p.code.extend([0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0]);
    load(&mut p, 10, 16);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    word(&mut p, 200, CANARY);
    preserved(&mut p, false);
    p.finish(None)
}

fn child() -> (Vec<u8>, u64) {
    let mut p = Program::new();
    p.code[0] = 0x53; // Preserve the callee-saved RBX used for this window.
    p.code.extend([0x48, 0x89, 0xfb]); // RBX is the shared guest window.
    seed(&mut p, true);
    p.code.extend([0x48, 0xc7, 0x03, 1, 0, 0, 0]);
    let wait = p.code.len();
    p.code.extend([0x48, 0x83, 0x3b, 2, 0x0f, 0x85]);
    p.code
        .extend((wait as i32 - (p.code.len() as i32 + 4)).to_le_bytes());
    for (slot, offset) in [(4, 8), (5, 16), (6, 32)] {
        call(&mut p, CHILD, slot);
        p.code.extend([0x48, 0x89, 0x43, offset]);
    }
    p.code.extend([0xc7, 0x00, 19, 0, 0, 0]);
    p.code.extend([0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0]);
    p.code.extend([0x48, 0x89, 0x43, 24]);
    // Parent's post-return flag is outside this child's private stack.
    p.code.extend([0x48, 0x8b, 0x83]);
    p.code.extend(152_i32.to_le_bytes());
    p.expect(0);
    call(&mut p, CHILD, 2);
    p.code.extend([0x48, 0x89, 0x83]);
    p.code.extend(160_i32.to_le_bytes());
    preserved(&mut p, true);
    p.code.extend([0x48, 0xc7, 0x03, 42, 0, 0, 0]);
    let calls = p.calls;
    p.reg(0, 73);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5b, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0x800);
    (p.code, calls)
}

#[test]
fn native_canonical_and_posix_usleep_park_while_another_guest_runs_and_preserve_context() {
    let imports = [
        ("1jfXLRVzisc", false),
        ("QcteRwbsnV0", true),
        ("4J2sUJmuHZQ", false),
        ("6UgtwV+0zb4", false),
        ("EI-5-jlq2dE", false),
        ("aI+OeCz8xrQ", false),
        ("9BcDykPmo1I", false),
    ];
    for wait_slot in [0, 1] {
        let (code, calls) = parent(wait_slot);
        let (child, child_calls) = child();
        assert_eq!((calls, child_calls), (12, 4));
        let original = posix::scoped_image_with_library(&code, &imports, 1, "libScePosix");
        let mut bytes = original.bytes.clone();
        bytes.resize(0x1a00, 0);
        put(&mut bytes, 96, &0x1a00_u64.to_le_bytes());
        put(&mut bytes, 0x800, &code);
        put(&mut bytes, CHILD as usize, &child);
        let input = create_input(bytes);
        let (status, fields, text) = probe(&input);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(
            fields["service_stops"],
            (calls + child_calls).to_string(),
            "{text}"
        );
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert_eq!(fields["initializer_calls"], "0", "{text}");
        assert_modeled_vector_counters(&fields, [(8, 144), (4, 16), (4, 16)]);
        assert_eq!(
            fields["kernel_handles_live"], fields["kernel_handles_accounted"],
            "{text}"
        );
        assert!(
            fields["primary_native_instructions"]
                .parse::<u64>()
                .unwrap()
                > 0,
            "{text}"
        );
    }
}
