// SPDX-License-Identifier: GPL-2.0-only
//! Original SystemService consumers of explicit configuration and PentaX identity.

use super::*;

const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const PARAMETER: u64 = 0x80a1_0003;
const IMPORTS: [&str; 7] = [
    "Vo5V8KAwCmk",
    "SsC-m-S9JTA",
    "3RQ5aQfnstU",
    "8Lo6Zv94aho",
    "Q3utJvma4Mo",
    "XbbJC3E+L5M",
    "3s8cHiCBKBE",
];

fn image(code: &[u8], prefixed: &[usize]) -> Input {
    assert!(code.len() < 0x700);
    let original = scoped_system_image(code, &IMPORTS, "SystemService", "SystemService", 1);
    let mut bytes = original.bytes.clone();
    put(&mut bytes, 0x800, code); // Restore the helper's unused path-name window.
    if !prefixed.is_empty() {
        let start = u64::from_le_bytes(bytes[0x218..0x220].try_into().unwrap()) as usize;
        let name = b"libSceSystemService\0";
        put(&mut bytes, 0x400 + start, name);
        put(&mut bytes, 0x400 + start + name.len(), name);
        put(
            &mut bytes,
            0x218,
            &((start + 2 * name.len()) as u64).to_le_bytes(),
        );
        for &index in prefixed {
            assert!(index < IMPORTS.len());
            let symbol = 0x600 + (index + 1) * 24;
            let offset = u32::from_le_bytes(bytes[symbol..symbol + 4].try_into().unwrap()) as usize;
            put(&mut bytes, 0x400 + offset + 11, b"#F#E");
        }
        for (index, (tag, value)) in [
            (
                0x6100_000f_i64,
                start as u64 | (1 << 32) | (1 << 40) | (4 << 48),
            ),
            (
                0x6100_0015,
                (start + name.len()) as u64 | (1 << 32) | (5 << 48),
            ),
            (0, 0),
        ]
        .into_iter()
        .enumerate()
        {
            put(&mut bytes, 0x200 + (10 + index) * 16, &tag.to_le_bytes());
            put(&mut bytes, 0x208 + (10 + index) * 16, &value.to_le_bytes());
        }
        for at in [152, 160] {
            put(&mut bytes, at, &(13_u64 * 16).to_le_bytes());
        }
    }
    create_input(bytes)
}
fn set_word(p: &mut Program, offset: u8, value: u64) {
    p.reg(0, value);
    p.save(offset);
}
fn finish(mut p: Program) -> (Vec<u8>, u64) {
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0x700);
    (p.code, p.calls)
}
fn profile(skip: bool) -> Program {
    let mut p = Program::new();
    for offset in [8, 16, 24, 40, 48, 56] {
        set_word(&mut p, offset, CANARY);
    }
    for operation in [None, Some(4), Some(3)] {
        if let Some(slot) = operation {
            p.reg(7, u64::MAX); // No-argument source calls ignore stale GPR values.
            p.call(slot);
            p.expect(0);
        }
        p.ptr(7, 16);
        p.call(2);
        p.expect(0);
        p.word(8, CANARY);
        p.word(16, (CANARY & !0xff) | u64::from(skip));
        p.word(24, CANARY);
    }
    p.reg(7, 0);
    p.call(2);
    p.expect(PARAMETER);
    for slot in [0, 5, 6] {
        p.reg(7, u64::MAX); // Report's info is opaque, never dereferenced.
        p.call(slot);
        p.expect(0);
    }
    p.reg(7, 0xffff_ffff_0000_0006);
    p.ptr(6, 48);
    p.reg(2, 7);
    p.call(1);
    p.expect(0);
    p.word(40, CANARY);
    p.word(48, 0xa500_5861_746e_6550); // Exactly "PentaX\0", with tail0xa5.
    p.word(56, CANARY);
    p
}
fn check(status: i32, fields: &HashMap<String, String>, text: &str, calls: u64) {
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert_eq!(fields["initializer_calls"], "0", "{text}");
    assert_eq!(fields["kernel_handles_live"], "0", "{text}");
    assert_eq!(fields["kernel_handles_accounted"], "0", "{text}");
}

#[test]
fn native_system_notice_default_is_one_byte_and_brand_has_exact_bounds_and_error_precedence() {
    let mut p = profile(false);
    set_word(&mut p, 48, CANARY);
    for (id, null, size) in [(u64::MAX, true, 7), (u64::MAX, false, 0), (6, false, 6)] {
        p.reg(7, id);
        if null {
            p.reg(6, 0);
        } else {
            p.ptr(6, 48);
        }
        p.reg(2, size);
        p.call(1);
        p.expect(PARAMETER);
        p.word(48, CANARY);
    }
    p.reg(7, 0x1234_5678_0000_0006);
    p.ptr(6, 48);
    p.reg(2, 0x1_0000_0000); // Full64 capacity; low32 zero must not reject.
    p.call(1);
    p.expect(0);
    p.word(48, 0xa500_5861_746e_6550);
    p.word(56, CANARY);
    let (code, calls) = finish(p);
    let source = image(&code, &[]);
    let (status, fields, text) = probe(&source);
    check(status, &fields, &text, calls);
}

#[test]
fn native_system_notice_real_config_true_and_false_survive_both_source_noop_setters() {
    for skip in [false, true] {
        let (code, calls) = finish(profile(skip));
        // Only HideSplashScreen and GetNoticeScreenSkipFlag have observed
        // full-prefix bindings. Every other import keeps its canonical scope.
        let source = image(&code, &[2, 0]);
        let config = create_input(
            format!("schema_version = 1\n[system]\nskip_notice_screen = {skip}\n").into_bytes(),
        );
        let (status, fields, text) = probe_arguments(&[
            "--image".into(),
            source.path.as_os_str().to_owned(),
            "--config".into(),
            config.path.as_os_str().to_owned(),
        ]);
        check(status, &fields, &text, calls);
    }
}

#[test]
fn native_system_notice_unbacked_outputs_and_unknown_string_id_never_fabricate_success() {
    for kind in 0..4 {
        let mut p = Program::new();
        if kind == 0 {
            p.reg(7, u64::MAX);
            p.call(2);
        } else {
            p.reg(7, if kind == 3 { u64::MAX } else { 6 });
            match kind {
                1 => p.reg(6, u64::MAX),
                2 => p.path(6, 0x800), // Real initialized RX source cannot be an output.
                _ => p.ptr(6, 48),
            }
            p.reg(2, if kind == 3 { 1 } else { 7 });
            p.call(1);
        }
        let (code, calls) = finish(p);
        let source = image(&code, &[]);
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "runtime-service", "{text}");
        assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert_eq!(fields["initializer_calls"], "0", "{text}");
        assert!(!fields.contains_key("value"), "{text}");
    }
}
