// SPDX-License-Identifier: GPL-2.0-only
//! Original native consumers of public Font records, real glyphs and text views.

use super::*;

const MEMORY: i32 = 0x4100;
const LIBRARY: i32 = 0x4180;
const FONT: i32 = 0x4190;
const SURFACE: i32 = 0x4200;
const METRICS: i32 = 0x4300;
const RESULT: i32 = 0x4340;
const SOURCE: i32 = 0x4400;
const STRING: i32 = 0x4500;
const WRITING: i32 = 0x4600;
const TEXT: i32 = 0x4800;
const PIXELS: i32 = 0x5000;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;

// Start with the existing original ELF helper; add only two closed namespaces
// and one RW/NX data segment. Scope0=Font,1=FontFt,2=libkernel.
fn font_image(code: &[u8], imports: &[(&str, u8)]) -> Input {
    assert!(imports.len() <= 8 && 0x800 + code.len() < 0xf00);
    let nids = imports.iter().map(|(nid, _)| *nid).collect::<Vec<_>>();
    let mut input = scoped_system_image(code, &nids, "Font", "Font", 1);
    let bytes = &mut input.bytes;
    let old_names = u64::from_le_bytes(bytes[0x218..0x220].try_into().unwrap()) as usize;
    let mut added = Vec::new();
    let mut descriptors = Vec::new();
    for (scope, name) in [(1_u8, "FontFt"), (2, "libkernel")] {
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
    bytes.resize(0x8000, 0xa5);
    put(bytes, 56, &3_u16.to_le_bytes());
    put(bytes, 176, &1_u32.to_le_bytes());
    put(bytes, 180, &6_u32.to_le_bytes());
    for (at, value) in [
        (184, 0x4000_u64),
        (192, 0x4000),
        (200, 0),
        (208, 0x4000),
        (216, 0x4000),
        (224, 0x4000),
    ] {
        put(bytes, at, &value.to_le_bytes());
    }
    bytes[PIXELS as usize..PIXELS as usize + 256].fill(0);
    put(bytes, TEXT as usize, b"A\0");
    std::fs::write(&input.path, bytes).unwrap();
    input
}

fn finish(mut p: Program) -> (Vec<u8>, u64) {
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0x700); // Entry0x800 must remain below GOT0xf00.
    (p.code, p.calls)
}

fn check(code: &[u8], calls: u64, imports: &[(&str, u8)]) {
    let input = font_image(code, imports);
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert_eq!(
        fields["kernel_handles_live"],
        fields["kernel_handles_accounted"]
    );
}

fn load(p: &mut Program, register: u8, field: i32) {
    assert!(!matches!(register & 7, 4));
    p.path(register, field);
    let low = register & 7;
    p.code.extend([
        0x48 | if register >= 8 { 5 } else { 0 },
        0x8b,
        (low << 3) | low | if low == 5 { 0x40 } else { 0 },
    ]);
    if low == 5 {
        p.code.push(0);
    }
}
fn word(p: &mut Program, field: i32, expected: u64) {
    load(p, 0, field);
    p.expect(expected);
}
fn dword(p: &mut Program, field: i32, expected: u64) {
    p.path(7, field);
    p.code.extend([0x8b, 0x07]);
    p.expect(expected);
}
fn pointer(p: &mut Program, field: i32, target: i32) {
    p.path(6, target);
    load(p, 0, field);
    p.code.extend([0x48, 0x39, 0xf0]);
    checked(&mut p.code, &mut p.branches);
}
fn store(p: &mut Program, field: i32, value: u64) {
    p.reg(0, value);
    p.path(7, field);
    p.code.extend([0x48, 0x89, 0x07]);
}
fn xmm(p: &mut Program, register: u8, value: f32) {
    p.reg(0, u64::from(value.to_bits()));
    p.code.extend([0x66, 0x0f, 0x6e, 0xc0 | (register << 3)]);
}
fn branch(p: &mut Program, condition: u8) {
    p.code.extend([0x0f, condition, 0, 0, 0, 0]);
    p.branches.push(p.code.len() - 4);
}
fn back(p: &mut Program, condition: u8, target: usize) {
    p.code.extend([0x0f, condition]);
    p.code
        .extend(((target as isize - (p.code.len() + 4) as isize) as i32).to_le_bytes());
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
fn native_font_seventh_gp_exact_memory_lifecycle_and_ft_owned_selection() {
    let mut p = Program::new();
    errno_start(&mut p, 7);
    p.reg(0, 0x1122_3344_5566_7788);
    p.save(0); // Seventh GP at caller RSP.
    p.path(7, MEMORY);
    p.reg(6, u64::MAX);
    p.reg(2, 0xdead_beef_1234_5678);
    p.reg(1, 0x1111);
    p.reg(8, 0x2222);
    p.reg(9, 0x3333);
    p.reg(10, 0x9999);
    p.call(0);
    p.expect(0);
    for (offset, value) in [
        (0, 0x1234_5678_0000_0001),
        (8, u64::MAX),
        (16, 0x2222),
        (24, 0x1111),
        (32, 0x3333),
        (40, 0x1122_3344_5566_7788),
        (48, 0),
        (56, 0),
        (64, CANARY),
    ] {
        word(&mut p, MEMORY + offset, value);
    }
    store(&mut p, MEMORY, 0x1234_5678_cafe_0001); // Nonzero attr proves a two-byte Term store.
    p.path(7, MEMORY);
    p.call(1);
    p.expect(0);
    word(&mut p, MEMORY, 0x1234_5678_cafe_0000);
    word(&mut p, MEMORY + 40, 0x1122_3344_5566_7788);
    p.reg(7, 0);
    p.call(1);
    p.expect(0);
    p.reg(7, 0);
    p.call(0);
    p.expect(u64::from(u32::MAX));
    errno(&mut p);

    p.reg(7, 17);
    p.call(2);
    p.save(16);
    p.code.extend([0x48, 0x8b, 0x00]);
    p.expect(17); // Public SelectionState value/kind0.
    p.reg(7, 23);
    p.call(2);
    p.code.extend([0x48, 0x3b, 0x44, 0x24, 16]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x48, 0x8b, 0x00]);
    p.expect(23);
    p.path(7, MEMORY);
    p.load(6, 16);
    p.reg(2, 0x1020_3040_5060_7080);
    p.path(1, LIBRARY);
    p.reg(10, u64::MAX);
    p.call(3);
    p.expect(0);
    load(&mut p, 7, LIBRARY);
    p.reg(6, 0);
    p.reg(2, 0);
    p.reg(1, u64::MAX);
    p.path(8, FONT);
    p.call(5);
    p.expect(0); // Detail is opaque with source size0.
    load(&mut p, 7, FONT);
    p.call(6);
    p.expect(0);
    p.path(7, LIBRARY);
    p.call(4);
    p.expect(0);
    word(&mut p, LIBRARY, 0);
    errno(&mut p);
    let (code, calls) = finish(p);
    check(
        &code,
        calls,
        &[
            ("whrS4oksXc4", 0),
            ("h6hIgxXEiEc", 0),
            ("oM+XCzVG3oM", 1),
            ("n590hj5Oe-k", 0),
            ("FXP359ygujs", 0),
            ("cKYtVmeSTcw", 0),
            ("vzHs3C8lWJk", 0),
            ("9BcDykPmo1I", 2),
        ],
    );
}

// Scan precisely the public descriptor's pitch*height, bounded by height128.
// OR-ing64-bit words keeps the original native loop below the normal block cap.
fn actual_image(p: &mut Program) {
    load(p, 13, RESULT);
    p.code.extend([0x41, 0x8b, 0x45, 8]);
    p.expect(128);
    p.code.extend([0x41, 0x8b, 0x45, 12, 0x85, 0xc0]);
    branch(p, 0x84);
    p.code.extend([0x3d, 128, 0, 0, 0]);
    branch(p, 0x87);
    p.code.extend([0x41, 0x8b, 0x4d, 16, 0x85, 0xc9]);
    branch(p, 0x84);
    p.code.extend([0x81, 0xf9, 128, 0, 0, 0]);
    branch(p, 0x87);
    p.code
        .extend([0x49, 0x8b, 0x7d, 0, 0xc1, 0xe1, 4, 0x31, 0xc0]);
    let loop_at = p.code.len();
    p.code
        .extend([0x48, 0x0b, 0x07, 0x48, 0x83, 0xc7, 8, 0xff, 0xc9]);
    back(p, 0x85, loop_at);
    p.code.extend([0x48, 0x85, 0xc0]);
    branch(p, 0x84);
}

fn clipped_pixels(p: &mut Program) {
    p.path(7, PIXELS);
    p.code.extend([0x31, 0xc9, 0x45, 0x31, 0xff]);
    let loop_at = p.code.len();
    p.code.extend([0x0f, 0xb6, 0x04, 0x0f, 0x41, 0x09, 0xc7]); // EAX=byte, R15D OR coverage.
    p.code.extend([
        0x89, 0xca, 0x83, 0xe2, 15, 0x41, 0x89, 0xc8, 0x41, 0xc1, 0xe8, 4,
    ]);
    let mut outside = Vec::new();
    for (comparison, condition) in [
        (&[0x83, 0xfa, 4][..], 0x82),
        (&[0x83, 0xfa, 10][..], 0x83),
        (&[0x41, 0x83, 0xf8, 4][..], 0x82),
        (&[0x41, 0x83, 0xf8, 12][..], 0x83),
    ] {
        p.code.extend(comparison);
        p.code.extend([0x0f, condition, 0, 0, 0, 0]);
        outside.push(p.code.len() - 4);
    }
    p.code.extend([0xe9, 0, 0, 0, 0]);
    let inside = p.code.len() - 4;
    let outside_at = p.code.len();
    p.code.extend([0x85, 0xc0]);
    branch(p, 0x85);
    let next = p.code.len();
    for at in outside {
        p.code[at..at + 4].copy_from_slice(&((outside_at - at - 4) as i32).to_le_bytes());
    }
    p.code[inside..inside + 4].copy_from_slice(&((next - inside - 4) as i32).to_le_bytes());
    p.code.extend([0xff, 0xc1, 0x81, 0xf9, 0, 1, 0, 0]);
    back(p, 0x82, loop_at);
    p.code.extend([0x45, 0x85, 0xff]);
    branch(p, 0x84);
}

#[test]
fn native_font_xmm_arguments_real_trans_image_and_clipped_surface_keep_canaries() {
    let mut p = Program::new();
    errno_start(&mut p, 7);
    p.reg(7, 0);
    p.reg(6, u64::MAX);
    p.reg(2, 7);
    p.path(1, LIBRARY);
    p.call(0);
    p.expect(0);
    load(&mut p, 7, LIBRARY);
    p.reg(6, 0);
    p.reg(2, 0);
    p.reg(1, 0);
    p.path(8, FONT);
    p.call(1);
    p.expect(0);
    xmm(&mut p, 0, 29.0);
    xmm(&mut p, 1, 24.0);
    load(&mut p, 7, FONT);
    p.reg(6, u64::MAX);
    p.reg(2, u64::MAX);
    p.call(2);
    p.expect(0);
    xmm(&mut p, 0, 0.0);
    xmm(&mut p, 1, 0.0);
    load(&mut p, 7, FONT);
    p.reg(6, 0xfeed_beef_1234_5678);
    p.reg(1, u64::MAX);
    p.call(3);
    p.expect(0);
    p.path(7, SURFACE);
    p.path(6, PIXELS);
    p.reg(2, 16);
    p.reg(1, 1);
    p.reg(8, 16);
    p.reg(9, 16);
    p.call(4); // Void result is intentionally not asserted.
    pointer(&mut p, SURFACE, PIXELS);
    word(&mut p, SURFACE + 8, 0x0000_0001_0000_0010);
    word(&mut p, SURFACE + 16, 0x0000_0010_0000_0010);
    word(&mut p, SURFACE + 24, 0);
    word(&mut p, SURFACE + 40, 0);
    word(&mut p, SURFACE + 120, 0);
    word(&mut p, SURFACE + 128, CANARY);
    store(&mut p, SURFACE + 24, 0x0000_0004_0000_0004);
    store(&mut p, SURFACE + 32, 0x0000_000c_0000_000a);
    xmm(&mut p, 0, 2.0);
    xmm(&mut p, 1, 24.0);
    load(&mut p, 7, FONT);
    p.reg(6, u64::from(b'A'));
    p.path(2, SURFACE);
    p.path(1, METRICS);
    p.path(8, RESULT);
    p.reg(9, u64::MAX);
    p.reg(10, u64::MAX);
    p.call(5);
    p.expect(0);
    dword(&mut p, METRICS + 28, u64::from(24.0_f32.to_bits()));
    word(&mut p, METRICS + 32, CANARY);
    pointer(&mut p, RESULT + 8, PIXELS);
    word(&mut p, RESULT + 16, 0xa5a5_0001_0000_0010);
    word(&mut p, RESULT + 64, CANARY);
    actual_image(&mut p);
    clipped_pixels(&mut p);
    word(&mut p, PIXELS - 8, CANARY);
    word(&mut p, PIXELS + 256, CANARY);
    errno(&mut p);
    load(&mut p, 7, FONT);
    p.call(6);
    p.expect(0);
    errno(&mut p);
    let (code, calls) = finish(p);
    check(
        &code,
        calls,
        &[
            ("n590hj5Oe-k", 0),
            ("cKYtVmeSTcw", 0),
            ("N1EBMeGhf7E", 0),
            ("v0phZwa4R5o", 0),
            ("gdUCnU0gHdI", 0),
            ("kAenWy1Zw5o", 0),
            ("vzHs3C8lWJk", 0),
            ("9BcDykPmo1I", 2),
        ],
    );
}

#[test]
fn native_font_text_clear_before_scan_raw_character_views_and_writing_replacement() {
    let mut p = Program::new();
    p.path(7, SOURCE);
    p.path(6, SOURCE + 80);
    p.reg(2, 0);
    p.reg(1, 0);
    p.reg(8, u64::MAX);
    p.call(0);
    p.expect(0);
    pointer(&mut p, SOURCE + 8, SOURCE + 80);
    pointer(&mut p, SOURCE + 16, SOURCE + 80);
    word(&mut p, SOURCE + 56, 0x10);
    word(&mut p, SOURCE + 64, 0);
    word(&mut p, SOURCE + 96, CANARY);
    p.path(7, SOURCE);
    p.path(6, TEXT);
    p.reg(2, 0);
    p.reg(1, 0);
    p.reg(8, 0);
    p.call(0);
    p.expect(0);
    p.reg(7, u64::MAX);
    p.path(6, SOURCE);
    p.reg(2, 0);
    p.path(1, STRING);
    p.reg(10, u64::MAX);
    p.call(1);
    p.expect(0);
    pointer(&mut p, SOURCE + 24, TEXT + 1);
    load(&mut p, 7, STRING);
    p.path(6, STRING + 8);
    p.call(2);
    p.save(16);
    word(&mut p, STRING + 8, 0xa5a5_a5a5_0000_0001);
    p.load(7, 16);
    p.path(6, STRING + 16);
    p.path(2, STRING + 24);
    p.call(3);
    p.expect(0);
    word(&mut p, STRING + 16, 0);
    word(&mut p, STRING + 24, 0xa5a5_a5a5_0000_0041);
    p.path(7, WRITING);
    load(&mut p, 6, STRING);
    p.reg(2, 0);
    p.call(5);
    p.expect(0);
    p.path(7, WRITING);
    p.call(6);
    p.save(24);
    load(&mut p, 0, WRITING + 8);
    p.code.extend([0x48, 0x3b, 0x44, 0x24, 24]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x8b, 0x40, 28]);
    p.expect(u64::from(b'A')); // Public WritingStep.glyph_code.
    p.path(7, WRITING);
    p.path(6, METRICS);
    p.call(7);
    p.expect(0);
    word(&mut p, METRICS, u64::from(8.0_f32.to_bits()));
    word(
        &mut p,
        METRICS + 8,
        u64::from((-12.0_f32).to_bits()) | (u64::from(4.0_f32.to_bits()) << 32),
    );
    word(&mut p, METRICS + 16, u64::from(8.0_f32.to_bits()) << 32);
    word(&mut p, METRICS + 24, CANARY);
    word(&mut p, WRITING + 256, CANARY);
    p.path(7, WRITING);
    p.reg(6, 0);
    p.reg(2, 0);
    p.call(5);
    p.expect(0);
    p.path(7, STRING);
    p.call(4);
    p.expect(0);
    word(&mut p, STRING, 0);
    let (code, calls) = finish(p);
    check(
        &code,
        calls,
        &[
            ("oaJ1BpN2FQk", 0),
            ("MO24vDhmS4E", 0),
            ("Avv7OApgCJk", 0),
            ("zN3+nuA0SFQ", 0),
            ("SSCaczu2aMQ", 0),
            ("fD5rqhEXKYQ", 0),
            ("W-2WOXEHGck", 0),
            ("fljdejMcG1c", 0),
        ],
    );
}
