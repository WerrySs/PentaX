// SPDX-License-Identifier: GPL-2.0-only
//! Original native callers cross kernel/driver scopes and consume real releases.
use super::*;
const STREAM: i32 = 0x4100;
const LABEL: i32 = 0x4900;
const PACKET: i32 = 0x4a00;
const IMPORTS: [&str; 8] = [
    "D0OdFMjp46I",
    "fzyMKs9kim0",
    "w2rJhmD+dsE",
    "DL2RXaXOy88",
    "gSRnr79F8tQ",
    "5CdQTZIQPxM",
    "Zw7uUVPulbw",
    "IWIBBdTHit4",
];

fn pointer(p: &mut Program, reg: u8, address: i32) {
    p.code
        .extend([0x4c | u8::from(reg >= 8), 0x89, 0xe0 | (reg & 7)]);
    p.code
        .extend([0x48 | u8::from(reg >= 8), 0x81, 0xc0 | (reg & 7)]);
    p.code.extend((address - 0x4000).to_le_bytes());
}
fn store(p: &mut Program, address: i32) {
    pointer(p, 11, address);
    p.code.extend([0x49, 0x89, 0x03]);
}
fn poll(p: &mut Program, expected: u64) {
    p.load(7, 0);
    p.ptr(6, 64);
    p.reg(2, 1);
    p.ptr(1, 24);
    p.ptr(8, 16);
    p.call(1);
    p.expect(expected);
}
fn mixed_image(code: &[u8]) -> Input {
    let original = scoped_system_image(code, &IMPORTS, "Graphics5Driver", "Graphics5Driver", 1);
    let mut bytes = original.bytes.clone();
    bytes.resize(0x5000, 0);
    let mut names = b"\0Graphics5Driver\0Graphics5Driver\0".to_vec();
    let kernel = names.len() as u64;
    names.extend_from_slice(b"libkernel\0");
    let driver = names.len() as u64;
    names.extend_from_slice(b"libSceAgcDriver\0");
    for (slot, nid) in IMPORTS.iter().enumerate() {
        put(
            &mut bytes,
            0x600 + (slot + 1) * 24,
            &(names.len() as u32).to_le_bytes(),
        );
        let qualifier = match slot {
            0 | 1 | 7 => "F#E",
            4 => "D#C",
            _ => "H#G",
        };
        names.extend_from_slice(format!("{nid}#{qualifier}\0").as_bytes());
    }
    assert!(names.len() <= 0x200);
    bytes[0x400..0x600].fill(0);
    put(&mut bytes, 0x400, &names);
    put(&mut bytes, 0x218, &(names.len() as u64).to_le_bytes());
    for (at, tag, value) in [
        (
            0x2a0,
            0x6100_000f_i64,
            kernel | (1 << 32) | (1 << 40) | (4 << 48),
        ),
        (0x2b0, 0x6100_0015, kernel | (1 << 32) | (5 << 48)),
        (
            0x2c0,
            0x6100_000f,
            driver | (1 << 32) | (1 << 40) | (6 << 48),
        ),
        (0x2d0, 0x6100_0015, driver | (1 << 32) | (7 << 48)),
        (0x2e0, 0, 0),
    ] {
        put(&mut bytes, at, &tag.to_le_bytes());
        put(&mut bytes, at + 8, &value.to_le_bytes());
    }
    for at in [152, 160] {
        put(&mut bytes, at, &(15_u64 * 16).to_le_bytes());
    }
    put(&mut bytes, 56, &3_u16.to_le_bytes());
    put(&mut bytes, 176, &1_u32.to_le_bytes());
    put(&mut bytes, 180, &6_u32.to_le_bytes());
    for (at, value) in [
        (184, 0x4000_u64),
        (192, 0x4000),
        (208, 0x1000),
        (216, 0x4000),
        (224, 0x4000),
    ] {
        put(&mut bytes, at, &value.to_le_bytes());
    }
    // Original data is copied into an actual explicit CPU/GPU RW mapping.
    let words = [
        0xc006_1060_u32,
        0x62f,
        0x4200_0000,
        0,
        0,
        0x89ab_cdef,
        0x1234_5678,
        0xf000_0011,
        0xc006_4900,
        0x628,
        4 << 24,
        0,
        0,
        0,
        0,
        19,
    ];
    for (i, word) in words.into_iter().enumerate() {
        put(&mut bytes, STREAM as usize + i * 4, &word.to_le_bytes());
    }
    put(&mut bytes, PACKET as usize + 8, &16_u32.to_le_bytes());
    bytes[LABEL as usize..LABEL as usize + 16].fill(0xa5);
    put(&mut bytes, 0x4f00, b"original-graphics-queue\0");
    put(&mut bytes, 0x800, code);
    create_input(bytes)
}

#[test]
fn native_graphics_event_registration_and_ordered_releases_reach_real_kernel_records() {
    let mut p = Program::new();
    p.set(96, 0);
    p.ptr(7, 96);
    p.reg(6, 0x4000);
    p.reg(2, 0x33);
    p.reg(1, 0);
    p.call(7);
    p.expect(0);
    p.load(12, 96);
    p.path(6, 0x4000);
    pointer(&mut p, 7, 0x4000);
    p.reg(1, 0x1000);
    p.code.extend([0xf3, 0xa4]);
    p.ptr(7, 0);
    pointer(&mut p, 6, 0x4f00);
    p.call(0);
    p.expect(0);
    p.load(7, 0);
    p.reg(6, 0x20);
    p.reg(2, 0x1234);
    p.call(2);
    p.expect(0);
    p.set(16, 0);
    p.reg(0, 0xa5a5_a5a5_7fff_ffff);
    p.save(24);
    poll(&mut p, 0x8002_003c); // Registration never invents an interrupt.
    pointer(&mut p, 0, LABEL);
    store(&mut p, STREAM + 12);
    pointer(&mut p, 0, STREAM);
    store(&mut p, PACKET);
    p.reg(7, 0x20);
    pointer(&mut p, 6, PACKET);
    p.call(4);
    p.expect(0);
    pointer(&mut p, 11, LABEL);
    p.code.extend([0x49, 0x8b, 0x03]);
    p.expect(0x1234_5678_89ab_cdef);
    pointer(&mut p, 11, LABEL + 8);
    p.code.extend([0x49, 0x8b, 0x03]);
    p.expect(0xa5a5_a5a5_a5a5_a5a5);
    poll(&mut p, 0);
    for (offset, value) in [
        (24, 0xa5a5_a5a5_0000_0001),
        (64, 0x20),
        (72, 0x0000_0002_0000_fff2),
        (80, 19),
        (88, 0x1234),
    ] {
        p.word(offset, value);
    }
    p.ptr(7, 64);
    p.call(5);
    p.expect(0x20);
    p.ptr(7, 64);
    p.call(6);
    p.expect(19);
    poll(&mut p, 0x8002_003c); // Flags0 graphics events still reset on delivery.
    p.load(7, 0);
    p.reg(6, 0x20);
    p.call(3);
    p.expect(0);
    p.load(7, 0);
    p.reg(6, 0x20);
    p.call(3);
    p.expect(0x8002_0002);
    let (code, calls) = p.finish(None);
    let original = mixed_image(&code);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["unavailable_functions"], "0");
    assert_eq!(fields["service_stops"], calls.to_string());
}

#[test]
fn native_graphics_event_apis_keep_source_null_widths_and_exact_scope_version_rules() {
    for (module, library, version) in [
        ("Graphics5Driver", "Graphics5Driver", 1),
        ("AgcDriver", "AgcDriver", 1),
        ("libSceAgcDriver", "libSceAgcDriver", 1),
        ("libSceAgcDriver", "libSceAgcDriver", 2),
        ("Graphics5", "Graphics5", 1),
    ] {
        let mut p = Program::new();
        for slot in 0..4 {
            p.reg(7, 0);
            p.call(slot);
            p.expect(if slot < 2 { 0x8002_0009 } else { 0 });
        }
        let (code, _) = p.finish(None);
        let original = scoped_system_image(
            &code,
            &["w2rJhmD+dsE", "DL2RXaXOy88", "5CdQTZIQPxM", "Zw7uUVPulbw"],
            module,
            library,
            version,
        );
        let (status, fields, text) = probe(&original);
        if version == 2 {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "import-version-mismatch");
        } else if module == "Graphics5" {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "unavailable-function");
        } else {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
        }
    }
}
