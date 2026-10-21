// SPDX-License-Identifier: GPL-2.0-only
//! Original native keyboard-type consumers and real guest-backing refusals.
use super::*;

const SCOPE: &str = "libSceConvertKeycode";
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const SAVED: u64 = 0x6a95_713c_d842_b0ef;

fn image(code: &[u8]) -> Input {
    let original = scoped_system_image(code, &["mUuUOWI-C+0"], SCOPE, SCOPE, 1);
    let mut bytes = original.bytes.clone();
    // Original module1.0 descriptor; only its minor version differs from the
    // generic fixture's module1.1. Library remains version1.
    put(
        &mut bytes,
        0x258,
        &(1u64 | (1 << 40) | (2 << 48)).to_le_bytes(),
    );
    create_input(bytes)
}

fn call(p: &mut Program, user: u64, output: Option<u8>, expected: u64) {
    p.reg(0, CANARY);
    for at in [0, 8, 16] {
        p.save(at);
    }
    p.reg(7, user);
    if let Some(offset) = output {
        p.ptr(6, offset);
    } else {
        p.reg(6, 0);
    }
    for register in [2, 1] {
        p.reg(register, u64::MAX);
    }
    p.call(0);
    p.expect(expected);
    p.word(0, CANARY);
    p.word(
        8,
        if expected == 0 {
            0xa5a5_0000_0000_a5a5
        } else {
            CANARY
        },
    );
    p.word(16, CANARY);
    p.code.extend([0x48, 0x89, 0xe0]);
    p.load(10, 24);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x4c, 0x89, 0xe0]);
    p.expect(SAVED);
}

#[test]
fn native_keyboard_type_uses_low_signed_user_null_precedence_and_exact_unaligned_output() {
    let mut p = Program::new();
    p.stack(0x89, 4, 24);
    p.stack(0x89, 12, 32);
    p.reg(12, SAVED);
    call(&mut p, 0, Some(10), 0);
    call(&mut p, 0xffff_ffff_0000_0001, Some(10), 0);
    call(&mut p, 0x7fff_ffff, Some(10), 0);
    call(&mut p, u64::MAX, Some(10), 0x80bc_0010);
    call(&mut p, 0x8000_0000, None, 0x80bc_0031);
    call(&mut p, 1, None, 0x80bc_0031);
    p.load(12, 32);
    let (code, calls) = p.finish(None);
    assert_eq!(calls, 6);
    let (status, fields, text) = probe(&image(&code));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], "6", "{text}");
    assert_eq!(fields["initializer_calls"], "0", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    println!(
        "convert_keycode_native_available=true convert_keycode_native_programs=1 convert_keycode_native_calls=6"
    );
}

#[test]
fn native_keyboard_type_retains_unmapped_and_immutable_backing_failures() {
    for readonly in [false, true] {
        let mut p = Program::new();
        p.reg(7, 1);
        if readonly {
            p.path(6, 0x800);
        } else {
            p.reg(6, 1);
        }
        p.call(0);
        p.expect(0);
        let (code, calls) = p.finish(None);
        assert_eq!(calls, 1);
        let (status, fields, text) = probe(&image(&code));
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "runtime-service", "{text}");
        assert_eq!(
            fields["runtime_failure"],
            if readonly {
                "guest-write-denied"
            } else {
                "guest-unmapped"
            },
            "{text}"
        );
        assert_eq!(
            fields["public_function"], "ConvertKeycodeGetImeKeyboardType",
            "{text}"
        );
        assert_eq!(fields["service_stops"], "1", "{text}");
        assert!(!fields.contains_key("value"), "{text}");
    }
    println!("convert_keycode_native_refusal_programs=2 convert_keycode_native_refusal_calls=2");
}
