// SPDX-License-Identifier: GPL-2.0-only
//! Original native consumers of the finite, disconnected controller profile.

use super::*;

// Seven runtime entries plus initialization, exactly the existing eight-import
// fixture bound. Each is one canonical module1.1/library1 GLOBAL FUNC import.
const NIDS: [&str; 8] = [
    "vDLMoJLde8I", // PadSetTiltCorrectionState.
    "znaWI0gpuo8", // PadGetTriggerEffectState.
    "yFVnOdGxvZY", // PadSetVibration.
    "q1cHNfGycLI", // PadRead.
    "gjP9-KQzoUk", // PadGetControllerInformation.
    "2JgFB2n9oUM", // PadSetTriggerEffect.
    "RR4novUEENY", // PadSetLightBar.
    "hv1luiJrqQM", // PadInit.
];
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const SAVED: u64 = 0x6b94_32c7_d851_a0ef;
const INVALID_ARGUMENT: u64 = 0x8092_0001;
const INVALID_HANDLE: u64 = 0x8092_0003;

fn image(code: &[u8]) -> Input {
    scoped_system_image(code, &NIDS, "libScePad", "libScePad", 1)
}

fn stack(p: &mut Program, opcode: u8, reg: u8, offset: u32) {
    if offset <= 127 {
        p.stack(opcode, reg, offset as u8);
    } else {
        p.code.extend([
            0x48 | if reg >= 8 { 4 } else { 0 },
            opcode,
            0x84 | ((reg & 7) << 3),
            0x24,
        ]);
        p.code.extend_from_slice(&offset.to_le_bytes());
    }
}

fn word(p: &mut Program, offset: u32, expected: u64) {
    stack(p, 0x8b, 0, offset);
    p.expect(expected);
}

fn fill(p: &mut Program, last: u32) {
    p.reg(0, CANARY);
    for offset in (0..=last).step_by(8) {
        stack(p, 0x89, 0, offset);
    }
}

fn start() -> Program {
    let mut p = Program::new();
    stack(&mut p, 0x89, 4, 240); // Actual caller RSP within its 256-byte frame.
    stack(&mut p, 0x89, 12, 248);
    p.reg(12, SAVED);
    p
}

fn caller_context(p: &mut Program) {
    // RSP and R12 are ABI obligations. Volatile scratch is reloaded after every
    // service; the caller never presumes preservation of RCX, R11 or arguments.
    p.code.extend([0x48, 0x89, 0xe0]);
    stack(p, 0x8b, 10, 240);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x4c, 0x89, 0xe0]);
    p.expect(SAVED);
}

fn poison_unused(p: &mut Program) {
    for register in [1, 8, 9, 11] {
        p.reg(register, u64::MAX);
    }
}

fn initialize(p: &mut Program) {
    p.call(7);
    p.expect(0);
    caller_context(p);
}

fn finish(mut p: Program) -> (Vec<u8>, u64) {
    stack(&mut p, 0x8b, 12, 248);
    p.finish(None)
}

fn success(p: Program, expected_calls: u64) {
    let (code, calls) = finish(p);
    assert_eq!(calls, expected_calls);
    let input = image(&code);
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["code"], "outer-return", "{text}");
    assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert_eq!(fields["initializer_calls"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert!(fields["native_instructions"].parse::<u64>().unwrap() > 0);
}

#[test]
fn native_pad_read_consumes_one_current_disconnected_record_and_stable_timestamp() {
    let mut p = start();
    initialize(&mut p);
    fill(&mut p, 128);
    p.reg(7, 1);
    p.ptr(6, 8);
    p.reg(2, 1);
    poison_unused(&mut p);
    p.call(3);
    p.expect(1);
    // The public 120-byte layout has neutral axes, identity orientation and
    // one-g Y acceleration. Every other fixed field/padding is zero, including
    // disconnected, connection count, buttons, touches and extension lengths.
    for (index, expected) in [
        0x8080_8080_0000_0000,
        0,
        0,
        0x0000_0000_3f80_0000,
        0x0000_0000_3f80_0000,
        0,
        0,
        0,
        0,
        0,
        0, // Timestamp: separately compared against the next unchanged read.
        0,
        0,
        0,
        0,
    ]
    .into_iter()
    .enumerate()
    {
        if index != 10 {
            word(&mut p, 8 + index as u32 * 8, expected);
        }
    }
    word(&mut p, 0, CANARY);
    word(&mut p, 128, CANARY);
    stack(&mut p, 0x8b, 0, 88);
    stack(&mut p, 0x89, 0, 168);
    caller_context(&mut p);
    // Request capacity two: an empty queue returns ONE actual current record,
    // not two fabricated copies. The second-record canary remains intact.
    p.reg(7, 1);
    p.ptr(6, 8);
    p.reg(2, 2);
    poison_unused(&mut p);
    p.call(3);
    p.expect(1);
    stack(&mut p, 0x8b, 0, 88);
    stack(&mut p, 0x8b, 10, 168);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    word(&mut p, 128, CANARY);
    caller_context(&mut p);
    success(p, 3);
    println!(
        "pad_runtime_native_available=true pad_runtime_native_read_programs=1 pad_runtime_native_read_calls=3"
    );
}

#[test]
fn native_pad_information_writes_exact_unaligned_28_bytes_for_disconnected_device() {
    let mut p = start();
    initialize(&mut p);
    fill(&mut p, 48);
    p.reg(7, 1);
    p.ptr(6, 10);
    poison_unused(&mut p);
    p.call(4);
    p.expect(0);
    // Unaligned public layout: density44.86, touch resolution1920x1080,
    // deadzones2/2 and honest zero connection status/count/class/reserved bytes.
    word(&mut p, 0, CANARY);
    word(
        &mut p,
        8,
        (1920_u64 << 48) | (u64::from(44.86_f32.to_bits()) << 16) | 0xa5a5,
    );
    word(&mut p, 16, 0x0000_0000_0202_0438);
    word(&mut p, 24, 0);
    word(&mut p, 32, 0xa5a5_0000_0000_0000);
    word(&mut p, 38, CANARY);
    caller_context(&mut p);
    success(p, 2);
    println!("pad_runtime_native_information_programs=1 pad_runtime_native_information_calls=2");
}

#[test]
fn native_pad_requests_do_not_fabricate_trigger_effects_or_modify_input_bytes() {
    let mut p = start();
    initialize(&mut p);
    p.reg(7, 0xfedc_ba98_0000_0001); // Actual signed32 handle argument.
    p.reg(6, 0x1234_5678_0000_0001); // The source bool uses its low byte.
    p.call(0);
    p.expect(0);
    caller_context(&mut p);

    fill(&mut p, 24);
    p.reg(0, 0xa5a5_a5a5_a5a5_3412);
    p.save(8);
    p.reg(7, 1);
    p.ptr(6, 8);
    poison_unused(&mut p);
    p.call(2);
    p.expect(0);
    word(&mut p, 0, CANARY);
    word(&mut p, 8, 0xa5a5_a5a5_a5a5_3412);
    word(&mut p, 16, CANARY);
    caller_context(&mut p);

    p.reg(0, 0xa5a5_a533_2211_a5a5);
    p.save(8);
    p.reg(7, 1);
    p.ptr(6, 10); // Three RGB bytes, deliberately unaligned.
    p.call(6);
    p.expect(0);
    word(&mut p, 8, 0xa5a5_a533_2211_a5a5);
    caller_context(&mut p);

    // One complete120-byte trigger request, maskleft/mode6. Its request state
    // is not evidence that a disconnected physical effect has become active.
    p.reg(0, 0);
    for offset in (8..128).step_by(8) {
        stack(&mut p, 0x89, 0, offset);
    }
    p.set(8, 1);
    p.set(16, 6);
    p.reg(0, CANARY);
    for offset in [0, 128, 136, 144] {
        stack(&mut p, 0x89, 0, offset);
    }
    p.reg(7, 1);
    p.ptr(6, 8);
    p.call(5);
    p.expect(0);
    caller_context(&mut p);
    p.reg(7, 1);
    stack(&mut p, 0x8d, 6, 136);
    p.call(1);
    p.expect(0);
    word(&mut p, 136, 0); // Actual effectstate is still 0/0.
    word(&mut p, 128, CANARY);
    word(&mut p, 144, CANARY);
    caller_context(&mut p);

    // Even an unselected right command is validated before the left request.
    p.set(72, 7);
    p.reg(7, 1);
    p.ptr(6, 8);
    p.call(5);
    p.expect(INVALID_ARGUMENT);
    word(&mut p, 8, 1);
    word(&mut p, 16, 6);
    word(&mut p, 72, 7);
    caller_context(&mut p);
    p.reg(7, 1);
    stack(&mut p, 0x8d, 6, 136);
    p.call(1);
    p.expect(0);
    word(&mut p, 136, 0);
    caller_context(&mut p);
    success(p, 8);
    println!("pad_runtime_native_request_programs=1 pad_runtime_native_request_calls=8");
}

#[test]
fn native_pad_wrong_handles_nulls_and_read_counts_keep_normal_sce32_errors() {
    // Invalidhandle precedes NULL/count validation, including uninitializedRead.
    let mut p = start();
    for slot in 0..7 {
        p.reg(7, 2);
        p.reg(6, 0);
        if slot == 3 {
            p.reg(2, 0);
        }
        p.call(slot);
        p.expect(INVALID_HANDLE);
        caller_context(&mut p);
    }
    success(p, 7);

    let mut p = start();
    initialize(&mut p);
    fill(&mut p, 24);
    for slot in 1..7 {
        p.reg(7, 1);
        p.reg(6, 0);
        if slot == 3 {
            p.reg(2, 1);
        }
        p.call(slot);
        p.expect(INVALID_ARGUMENT);
        caller_context(&mut p);
    }
    for count in [0, u64::MAX] {
        p.reg(7, 1);
        p.ptr(6, 8);
        p.reg(2, count);
        p.call(3);
        p.expect(INVALID_ARGUMENT);
        caller_context(&mut p);
    }
    p.reg(7, 1);
    p.reg(6, 0); // A false bool is valid, not a NULL pointer.
    p.call(0);
    p.expect(0);
    caller_context(&mut p);
    for offset in [0, 8, 16, 24] {
        word(&mut p, offset, CANARY);
    }
    success(p, 10);
    println!("pad_runtime_native_validation_programs=2 pad_runtime_native_validation_calls=17");
}

#[test]
fn native_pad_guest_input_and_output_backing_faults_remain_typed() {
    let cases = [
        (1, "PadGetTriggerEffectState", false, "guest-unmapped"),
        (1, "PadGetTriggerEffectState", true, "guest-write-denied"),
        (2, "PadSetVibration", false, "guest-unmapped"),
        (3, "PadRead", false, "guest-unmapped"),
        (3, "PadRead", true, "guest-write-denied"),
        (4, "PadGetControllerInformation", false, "guest-unmapped"),
        (4, "PadGetControllerInformation", true, "guest-write-denied"),
        (5, "PadSetTriggerEffect", false, "guest-unmapped"),
        (6, "PadSetLightBar", false, "guest-unmapped"),
    ];
    for (slot, function, readonly, expected) in cases {
        let mut p = start();
        initialize(&mut p);
        p.reg(7, 1);
        if readonly {
            p.path(6, 0x800); // Actual same-owner initialized RX code.
        } else {
            p.reg(6, 1); // Nonnull but genuinely unmapped backing.
        }
        if slot == 3 {
            p.reg(2, 1);
        }
        p.call(slot);
        p.expect(0); // No fabricated normal result may reach this consumer.
        let (code, calls) = finish(p);
        assert_eq!(calls, 2);
        let input = image(&code);
        let (status, fields, text) = probe(&input);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "runtime-service", "{text}");
        assert_eq!(fields["runtime_failure"], expected, "{text}");
        assert_eq!(fields["public_function"], function, "{text}");
        assert_eq!(fields["service_stops"], "2", "{text}");
        assert_eq!(fields["initializer_calls"], "0", "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert!(!fields.contains_key("value"), "{text}");
    }
    println!("pad_runtime_native_refusal_programs=9 pad_runtime_native_refusal_calls=18");
}
