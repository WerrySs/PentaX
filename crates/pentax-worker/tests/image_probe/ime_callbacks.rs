// SPDX-License-Identifier: GPL-2.0-only
//! Original native IME handlers, detached event snapshots, and actual returns.

use super::*;

const CALLBACK: i32 = 0x1100;
const PARAM: i32 = 0x4100;
const KEYBOARD: i32 = 0x4180;
const RECORD: i32 = 0x4200;
const TEXT: i32 = 0x4400;
const WORK: i32 = 0x5000;
const IMPORTS: [&str; 5] = [
    "RPydv-Jr1bc",
    "TmVP8LzcFcY",
    "eaFXjfJv3xs",
    "-4GCfYdNF1s",
    "PMVehSlfZ94",
];

fn image(code: &[u8], callback: &[u8]) -> Input {
    let original = scoped_system_image(code, &IMPORTS, "Ime", "Ime", 1);
    let mut bytes = original.bytes.clone();
    let old_length = u64::from_le_bytes(bytes[0x218..0x220].try_into().unwrap()) as usize;
    let module = old_length;
    let library = module + b"libSceIme\0".len();
    put(&mut bytes, 0x400 + old_length, b"libSceIme\0libSceIme\0");
    put(
        &mut bytes,
        0x218,
        &((library + b"libSceIme\0".len()) as u64).to_le_bytes(),
    );
    let symbol = 0x600 + 4 * 24;
    let name = u32::from_le_bytes(bytes[symbol..symbol + 4].try_into().unwrap()) as usize;
    put(&mut bytes, 0x400 + name + 11, b"#F#E"); // Exact observed prefixed Update only.
    for (index, (tag, value)) in [
        (
            0x6100_000f_i64,
            module as u64 | (1 << 32) | (1 << 40) | (4 << 48),
        ),
        (0x6100_0015, library as u64 | (1 << 32) | (5 << 48)),
        (0, 0),
    ]
    .into_iter()
    .enumerate()
    {
        put(&mut bytes, 0x200 + (10 + index) * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + (10 + index) * 16, &value.to_le_bytes());
    }
    for at in [152, 160] {
        put(&mut bytes, at, &(13u64 * 16).to_le_bytes());
    }
    bytes.resize(0xc000, 0);
    put(&mut bytes, 96, &0x1800u64.to_le_bytes());
    assert!(0x800 + code.len() < 0xf00);
    assert!(CALLBACK as usize + callback.len() < 0x1800);
    put(&mut bytes, CALLBACK as usize, callback);
    put(&mut bytes, 56, &3u16.to_le_bytes());
    put(&mut bytes, 176, &1u32.to_le_bytes());
    put(&mut bytes, 180, &6u32.to_le_bytes());
    for (at, value) in [
        (184, 0x4000u64),
        (192, 0x4000),
        (200, 0),
        (208, 0x8000),
        (216, 0x8000),
        (224, 0x4000),
    ] {
        put(&mut bytes, at, &value.to_le_bytes());
    }
    put(&mut bytes, PARAM as usize, &1i32.to_le_bytes());
    put(&mut bytes, PARAM as usize + 36, &1u32.to_le_bytes());
    create_input(bytes)
}
fn store_pointer(p: &mut Program, field: i32, target: i32) {
    p.path(0, target);
    p.path(10, field);
    p.code.extend([0x49, 0x89, 0x02]);
}
fn setup(p: &mut Program, valid_target: bool) {
    for (field, target) in [
        (PARAM + 40, TEXT),
        (PARAM + 64, WORK),
        (PARAM + 72, RECORD),
        (KEYBOARD + 8, RECORD),
    ] {
        store_pointer(p, field, target);
    }
    for field in [PARAM + 80, KEYBOARD + 16] {
        if valid_target {
            store_pointer(p, field, CALLBACK);
        } else {
            p.reg(0, u64::MAX);
            p.path(10, field);
            p.code.extend([0x49, 0x89, 0x02]);
        }
    }
}
fn expect_word(p: &mut Program, address: i32, value: u64) {
    p.path(0, address);
    p.code.extend([0x48, 0x8b, 0]);
    p.expect(value);
}
fn expect_pointer(p: &mut Program, address: i32, target: i32) {
    p.path(0, address);
    p.code.extend([0x48, 0x8b, 0]);
    p.path(10, target);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
}
fn update(p: &mut Program) {
    p.path(7, CALLBACK);
    p.call(3);
    p.expect(0);
}

fn callback() -> Vec<u8> {
    let mut code = vec![0x53, 0x48, 0x89, 0xfb, 0x48, 0x85, 0xd2]; // Save RBX, source arg, check benign third argument.
    let mut branches = Vec::new();
    checked(&mut code, &mut branches);
    for offset in [24, 32, 40, 48, 56, 64] {
        code.extend([0x48, 0x83, 0x7e, offset, 0]);
        checked(&mut code, &mut branches);
    }
    code.extend([
        0x48, 0x8b, 0x03, // Counter selects a 48-byte callback record.
        0x48, 0x6b, 0xc0, 48, 0x48, 0x8d, 0x44, 0x07, 32, 0x8b, 0x0e, 0x48, 0x89,
        0x08, // Public event ID, zero-extended.
        0x48, 0x89, 0x70, 8, // Ephemeral pointer, recorded but never dereferenced later.
        0x48, 0x89, 0x78, 16, 0x48, 0x89, 0x50, 24, 0x48, 0x8b, 0x4e, 8, 0x48, 0x89, 0x48, 32,
        0x48, 0x8b, 0x4e, 16, 0x48, 0x89, 0x48, 40, 0x48, 0xff, 0x03, 0x66, 0x0f, 0xef,
        0xc0, // Callback changes XMM0; suspended caller snapshot must survive.
        0x48, 0x83, 0x3b, 1, 0x0f, 0x85,
    ]);
    let skip = code.len();
    code.extend([0; 4]);
    rip(&mut code, CALLBACK, &[0xff, 0x15], 0xf08); // First event closes text IME.
    code.extend([0x48, 0x83, 0xf8, 0]);
    checked(&mut code, &mut branches);
    rip(&mut code, CALLBACK, &[0x48, 0x8d, 0x3d], PARAM);
    code.extend([0x31, 0xf6]);
    rip(&mut code, CALLBACK, &[0xff, 0x15], 0xf00); // Reopen queues a NEW event, outside this snapshot.
    code.extend([0x48, 0x83, 0xf8, 0]);
    checked(&mut code, &mut branches);
    let finish = code.len();
    code[skip..skip + 4].copy_from_slice(&((finish - skip - 4) as i32).to_le_bytes());
    code.extend([0x48, 0xb8]);
    code.extend(u64::MAX.to_le_bytes()); // Void numeric result must never become Update's result.
    code.extend([0x5b, 0xc3]);
    finish_checks(&mut code, branches);
    code
}

#[test]
fn native_ime_callbacks_keep_detached_keyboard_order_and_defer_reopened_text_events() {
    let mut p = Program::new();
    setup(&mut p, true);
    p.path(7, PARAM);
    p.reg(6, 0);
    p.call(0);
    p.expect(0);
    p.reg(7, 1);
    p.path(6, KEYBOARD);
    p.call(2);
    p.expect(0);
    const VECTOR: u64 = 0x1122_3344_5566_7788;
    p.reg(0, VECTOR);
    p.code.extend([0x66, 0x48, 0x0f, 0x6e, 0xc0]);
    update(&mut p);
    expect_word(&mut p, RECORD, 2);
    p.code.extend([0x66, 0x48, 0x0f, 0x7e, 0xc0]);
    p.expect(VECTOR);
    update(&mut p);
    expect_word(&mut p, RECORD, 3);
    update(&mut p);
    expect_word(&mut p, RECORD, 3); // Both queues drained; no replay.
    for (index, id) in [0u64, 256, 0].into_iter().enumerate() {
        let record = RECORD + 32 + index as i32 * 48;
        expect_word(&mut p, record, id);
        expect_pointer(&mut p, record + 16, RECORD);
        expect_word(&mut p, record + 24, 0);
        expect_word(&mut p, record + 32, u64::from(index == 1));
        expect_word(
            &mut p,
            record + 40,
            if index == 1 { 0 } else { (408u64 << 32) | 793 },
        );
    }
    p.path(0, RECORD + 40);
    p.code.extend([0x48, 0x8b, 0]);
    p.path(10, RECORD + 88);
    p.code.extend([0x4d, 0x8b, 0x12, 0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches); // One temporary event mapping reused within the snapshot.
    p.call(1);
    p.expect(0);
    p.reg(7, 1);
    p.call(4);
    p.expect(0);
    let (code, calls) = p.finish(None);
    let source = image(&code, &callback());
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], (calls + 2).to_string(), "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert_eq!(fields["initializer_calls"], "0", "{text}");
    assert_eq!(fields["kernel_handles_live"], "0", "{text}");
    assert_eq!(fields["kernel_handles_accounted"], "0", "{text}");
}

#[test]
fn native_ime_unbacked_or_unreturned_handlers_keep_pending_events_and_never_claim_delivery() {
    for unbacked in [true, false] {
        let mut p = Program::new();
        setup(&mut p, !unbacked);
        p.reg(7, 1);
        p.path(6, KEYBOARD);
        p.call(2);
        p.expect(0);
        if unbacked {
            p.reg(7, u64::MAX);
        } else {
            p.path(7, CALLBACK);
        }
        p.call(3);
        let (code, calls) = p.finish(None);
        let source = image(&code, &[0x0f, 0x0b]);
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 2, "{text}");
        assert_eq!(
            fields["code"],
            if unbacked {
                "runtime-service"
            } else {
                "unsupported-instruction"
            },
            "{text}"
        );
        assert!(!fields.contains_key("value"), "{text}");
        assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert_eq!(fields["initializer_calls"], "0", "{text}");
        assert_eq!(fields["kernel_handles_live"], "3", "{text}");
        assert_eq!(fields["kernel_handles_accounted"], "3", "{text}");
    }
}
