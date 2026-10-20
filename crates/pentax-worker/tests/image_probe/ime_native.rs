// SPDX-License-Identifier: GPL-2.0-only
//! Original native consumers of the pinned local IME/keyboard profile.

use super::*;

const PARAM: i32 = 0x4100;
const KEYBOARD: i32 = 0x4200;
const RESOURCES: i32 = 0x4300;
const INFO: i32 = 0x4340;
const WIDTH: i32 = 0x4380;
const HEIGHT: i32 = 0x4390;
const CARET: i32 = 0x4400;
const GEOMETRY: i32 = 0x4440;
const TEXT: i32 = 0x5000;
const REPLACEMENT: i32 = 0x5100;
const INVALID: i32 = 0x5200;
const WORK: i32 = 0x8000;
const HANDLER: i32 = 0xf80;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;

// Reuse the original single-scope ELF encoder, adding only two finite scopes
// and a RW/NX data segment. Scope0=Ime,1=libSceIme,2=libkernel.
fn ime_image(code: &[u8], imports: &[(&str, u8)]) -> Input {
    assert!(imports.len() <= 8 && code.len() < 0x700);
    let nids = imports.iter().map(|(nid, _)| *nid).collect::<Vec<_>>();
    let mut input = scoped_system_image(code, &nids, "Ime", "Ime", 1);
    let bytes = &mut input.bytes;
    // The general filesystem helper also writes unused path strings atd00/d80.
    // These consumers use no paths; restore the complete original code window.
    put(bytes, 0x800, code);
    let old_names = u64::from_le_bytes(bytes[0x218..0x220].try_into().unwrap()) as usize;
    let mut added = Vec::new();
    let mut descriptors = Vec::new();
    for (scope, name) in [(1_u8, "libSceIme"), (2, "libkernel")] {
        let module = old_names + added.len();
        added.extend_from_slice(name.as_bytes());
        added.push(0);
        let library = old_names + added.len();
        added.extend_from_slice(name.as_bytes());
        added.push(0);
        let id = 2 + u64::from(scope) * 2;
        descriptors.push((
            0x6100_000f_i64,
            module as u64 | (1 << 32) | (1 << 40) | (id << 48),
        ));
        descriptors.push((
            0x6100_0015_i64,
            library as u64 | (1 << 32) | ((id + 1) << 48),
        ));
    }
    assert!(0x400 + old_names + added.len() < 0x600);
    put(bytes, 0x400 + old_names, &added);
    put(
        bytes,
        0x218,
        &((old_names + added.len()) as u64).to_le_bytes(),
    );
    for (index, (_, scope)) in imports.iter().enumerate() {
        let symbol = 0x600 + (index + 1) * 24;
        let name = u32::from_le_bytes(bytes[symbol..symbol + 4].try_into().unwrap()) as usize;
        if *scope != 0 {
            let module = char::from(b'C' + *scope * 2);
            let library = char::from(b'D' + *scope * 2);
            put(
                bytes,
                0x400 + name + 11,
                format!("#{library}#{module}").as_bytes(),
            );
        }
    }
    descriptors.push((0, 0));
    for (index, (tag, value)) in descriptors.into_iter().enumerate() {
        put(bytes, 0x200 + (10 + index) * 16, &tag.to_le_bytes());
        put(bytes, 0x208 + (10 + index) * 16, &value.to_le_bytes());
    }
    for at in [152, 160] {
        put(bytes, at, &(15_u64 * 16).to_le_bytes());
    }
    put(bytes, HANDLER as usize, &[0xc3]);
    bytes.resize(0x14000, 0xa5);
    put(bytes, 56, &3_u16.to_le_bytes());
    put(bytes, 176, &1_u32.to_le_bytes());
    put(bytes, 180, &6_u32.to_le_bytes());
    for (at, value) in [
        (184, 0x4000_u64),
        (192, 0x4000),
        (200, 0),
        (208, 0x10000),
        (216, 0x10000),
        (224, 0x4000),
    ] {
        put(bytes, at, &value.to_le_bytes());
    }
    bytes[KEYBOARD as usize..KEYBOARD as usize + 32].fill(0);
    bytes[PARAM as usize..PARAM as usize + 96].fill(0);
    put(bytes, PARAM as usize, &7_u32.to_le_bytes());
    put(bytes, PARAM as usize + 36, &8_u32.to_le_bytes());
    // Ordinary pointer fields are patched by the guest itself, never host pointers.
    put(
        bytes,
        TEXT as usize,
        &[0x41, 0, 0x3d, 0xd8, 0, 0xde, 0x42, 0, 0, 0],
    );
    put(bytes, REPLACEMENT as usize, &[0x5a, 0, 0x3d, 0xd8, 0, 0xde]);
    put(bytes, INVALID as usize, &[0, 0xd8, 0x41, 0]);
    bytes[CARET as usize..CARET as usize + 16].fill(0);
    bytes[GEOMETRY as usize..GEOMETRY as usize + 16].fill(0);
    put(bytes, GEOMETRY as usize, &24_f32.to_bits().to_le_bytes());
    put(
        bytes,
        GEOMETRY as usize + 4,
        &48_f32.to_bits().to_le_bytes(),
    );
    std::fs::write(&input.path, bytes).unwrap();
    input
}

fn finish(mut p: Program) -> (Vec<u8>, u64) {
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0x700);
    (p.code, p.calls)
}
fn check(p: Program, imports: &[(&str, u8)]) {
    let (code, calls) = finish(p);
    let input = ime_image(&code, imports);
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert_eq!(
        fields["kernel_handles_live"],
        fields["kernel_handles_accounted"]
    );
}
fn word(p: &mut Program, field: i32, expected: u64) {
    p.path(7, field);
    p.code.extend([0x48, 0x8b, 0x07]);
    p.expect(expected);
}
fn store(p: &mut Program, field: i32, value: u64) {
    p.reg(0, value);
    p.path(7, field);
    p.code.extend([0x48, 0x89, 0x07]);
}
fn store_pointer(p: &mut Program, field: i32, target: i32) {
    p.path(0, target);
    p.path(7, field);
    p.code.extend([0x48, 0x89, 0x07]);
}
fn errno_start(p: &mut Program, slot: i32) {
    p.call(slot);
    p.code
        .extend([0x49, 0x89, 0xc4, 0x41, 0xc7, 0x04, 0x24, 37, 0, 0, 0]);
}
fn errno(p: &mut Program) {
    p.code.extend([0x41, 0x8b, 0x04, 0x24]);
    p.expect(37);
}

#[test]
fn native_ime_keyboard_lifecycle_has_disconnected_outputs_and_exact_error_precedence() {
    let mut p = Program::new();
    errno_start(&mut p, 5);
    p.reg(7, u64::MAX);
    p.call(1);
    p.expect(0x80bc_0002);
    p.reg(7, u64::MAX);
    p.reg(6, 0);
    p.call(0);
    p.expect(0x80bc_0031);
    p.reg(7, u64::MAX);
    p.path(6, KEYBOARD);
    p.call(0);
    p.expect(0x80bc_0010);
    p.reg(7, 7);
    p.path(6, KEYBOARD);
    p.call(0);
    p.expect(0x80bc_0022);
    store_pointer(&mut p, KEYBOARD + 16, HANDLER);
    p.reg(7, 7);
    p.path(6, KEYBOARD);
    p.call(0);
    p.expect(0);
    p.reg(7, 9);
    p.path(6, KEYBOARD);
    p.call(0);
    p.expect(0x80bc_0001);
    p.reg(7, 99);
    p.path(6, RESOURCES);
    p.call(2);
    p.expect(0x80bc_0004);
    word(&mut p, RESOURCES, 99);
    word(&mut p, RESOURCES + 8, 0);
    word(&mut p, RESOURCES + 16, 0);
    word(&mut p, RESOURCES - 8, CANARY);
    word(&mut p, RESOURCES + 24, CANARY);
    p.reg(7, 0x1234);
    p.path(6, INFO);
    p.call(3);
    p.expect(0x80bc_0023);
    for offset in [0, 8, 16, 24, 32] {
        word(&mut p, INFO + offset, CANARY);
    }
    p.reg(7, u64::MAX);
    p.reg(6, 0);
    p.call(4);
    p.expect(0x80bc_0010);
    p.reg(7, 7);
    p.reg(6, 0x80);
    p.call(4);
    p.expect(0x80bc_0024);
    p.reg(7, 7);
    p.reg(6, 0x7f);
    p.call(4);
    p.expect(0);
    p.reg(7, u64::MAX);
    p.call(1);
    p.expect(0x80bc_0010);
    p.reg(7, 99);
    p.call(1);
    p.expect(0);
    p.reg(7, 7);
    p.path(6, RESOURCES);
    p.call(2);
    p.expect(0x80bc_0002);
    word(&mut p, RESOURCES, 7);
    word(&mut p, RESOURCES + 8, 0);
    word(&mut p, RESOURCES + 16, 0);
    p.reg(7, 0);
    p.path(6, INFO);
    p.call(3);
    p.expect(0x80bc_0002);
    word(&mut p, INFO, CANARY);
    word(&mut p, INFO + 32, CANARY);
    errno(&mut p);
    check(
        p,
        &[
            ("eaFXjfJv3xs", 1),
            ("PMVehSlfZ94", 0),
            ("dKadqZFgKKQ", 0),
            ("VkqLPArfFdc", 0),
            ("ua+13Hk9kKs", 0),
            ("9BcDykPmo1I", 2),
        ],
    );
}

#[test]
fn native_ime_text_open_utf16_setters_and_close_preserve_public_buffer_boundaries() {
    let mut p = Program::new();
    errno_start(&mut p, 7);
    store(&mut p, PARAM, CANARY);
    store(&mut p, PARAM + 8, CANARY);
    store(&mut p, PARAM + 88, CANARY);
    p.reg(7, 0);
    p.call(5); // Void null accepted; no return-value assumption.
    p.path(7, PARAM);
    p.call(5);
    word(&mut p, PARAM, 0x0000_0000_ffff_ffff);
    word(&mut p, PARAM + 8, 0);
    word(&mut p, PARAM + 88, 0);
    word(&mut p, PARAM - 8, CANARY);
    word(&mut p, PARAM + 96, CANARY);
    p.path(7, PARAM);
    p.path(6, WIDTH);
    p.path(2, HEIGHT);
    p.call(6);
    p.expect(0);
    word(&mut p, WIDTH, 0xa5a5_a5a5_0000_0319);
    word(&mut p, HEIGHT, 0xa5a5_a5a5_0000_0198);
    store(&mut p, PARAM, 7);
    store(&mut p, PARAM + 32, 8_u64 << 32);
    p.reg(7, 0);
    p.reg(6, 0);
    p.call(1);
    p.expect(0x80bc_0002);
    store_pointer(&mut p, PARAM + 40, TEXT);
    store_pointer(&mut p, PARAM + 64, WORK);
    store_pointer(&mut p, PARAM + 80, HANDLER);
    p.path(7, PARAM);
    p.reg(6, 0);
    p.call(0);
    p.expect(0);
    word(&mut p, TEXT, 0x0042_de00_d83d_0041);
    word(&mut p, WORK, 0x0042_de00_d83d_0041);
    word(&mut p, WORK + 8, 0);
    word(&mut p, WORK + 20472, 0);
    word(&mut p, WORK - 8, CANARY);
    word(&mut p, WORK + 20480, CANARY);
    p.reg(7, 0);
    p.reg(6, 0);
    p.call(0);
    p.expect(0x80bc_0001); // Busy before Param read.
    p.path(7, INVALID);
    p.reg(6, 2);
    p.call(1);
    p.expect(0x80bc_0006);
    word(&mut p, TEXT, 0x0042_de00_d83d_0041);
    word(&mut p, WORK, 0x0042_de00_d83d_0041);
    p.path(7, REPLACEMENT);
    p.reg(6, 3);
    p.call(1);
    p.expect(0);
    word(&mut p, TEXT, 0x0000_de00_d83d_005a);
    word(&mut p, WORK, 0x0000_de00_d83d_005a);
    word(&mut p, TEXT + 8, 0xa5a5_a5a5_a5a5_0000);
    store(&mut p, CARET + 8, 2_u64 << 32);
    p.path(7, CARET);
    p.call(2);
    p.expect(0); // Cursor inside surrogate pair is normalized.
    store(&mut p, CARET + 8, 4_u64 << 32);
    p.path(7, CARET);
    p.call(2);
    p.expect(0x80bc_0030);
    p.reg(7, 2);
    p.path(6, GEOMETRY);
    p.call(3);
    p.expect(0);
    p.reg(7, 1);
    p.path(6, GEOMETRY);
    p.call(3);
    p.expect(0x80bc_0030);
    p.call(4);
    p.expect(0);
    p.call(4);
    p.expect(0x80bc_0002);
    p.reg(7, 0);
    p.call(2);
    p.expect(0x80bc_0002);
    word(&mut p, TEXT, 0x0000_de00_d83d_005a);
    word(&mut p, WORK, 0x0000_de00_d83d_005a);
    word(&mut p, WORK - 8, CANARY);
    word(&mut p, WORK + 20480, CANARY);
    errno(&mut p);
    check(
        p,
        &[
            ("RPydv-Jr1bc", 0),
            ("ieCNrVrzKd4", 0),
            ("WLxUN2WMim8", 0),
            ("TXYHFRuL8UY", 0),
            ("TmVP8LzcFcY", 0),
            ("WmYDzdC4EHI", 0),
            ("ziPDcIjO0Vk", 0),
            ("9BcDykPmo1I", 2),
        ],
    );
}
