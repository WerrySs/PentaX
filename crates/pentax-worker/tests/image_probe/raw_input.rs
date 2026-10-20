// SPDX-License-Identifier: GPL-2.0-only
//! Original native consumers of the pinned disconnected Mouse/Keyboard profile.

use super::*;

const DATA: i32 = 0x4200;
const STATE: i32 = 0x5000;
const CHARACTER: i32 = 0x5100;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;

fn raw_image(code: &[u8], imports: &[&str], scope: &str) -> Input {
    assert!(imports.len() <= 8 && code.len() < 0x700);
    let mut input = scoped_system_image(code, imports, scope, scope, 1);
    let bytes = &mut input.bytes;
    // The general helper's unused filesystem strings must not replace code.
    put(bytes, 0x800, code);
    let old_names = u64::from_le_bytes(bytes[0x218..0x220].try_into().unwrap()) as usize;
    let extra = b"libkernel\0libkernel\0";
    put(bytes, 0x400 + old_names, extra);
    put(
        bytes,
        0x218,
        &((old_names + extra.len()) as u64).to_le_bytes(),
    );
    let last_symbol = 0x600 + imports.len() * 24;
    let name = u32::from_le_bytes(bytes[last_symbol..last_symbol + 4].try_into().unwrap()) as usize;
    assert_eq!(imports.last(), Some(&"9BcDykPmo1I"));
    put(bytes, 0x400 + name + 11, b"#F#E");
    let mut descriptors = vec![
        (
            0x6100_000f_i64,
            old_names as u64 | (1 << 32) | (1 << 40) | (4 << 48),
        ),
        (
            0x6100_0015_i64,
            (old_names + 10) as u64 | (1 << 32) | (5 << 48),
        ),
    ];
    if scope == "Mouse" {
        let name_at = old_names + extra.len();
        let prefix = b"libSceMouse\0libSceMouse\0";
        put(bytes, 0x400 + name_at, prefix);
        put(
            bytes,
            0x218,
            &((name_at + prefix.len()) as u64).to_le_bytes(),
        );
        descriptors.push((
            0x6100_000f,
            name_at as u64 | (1 << 32) | (1 << 40) | (6 << 48),
        ));
        descriptors.push((0x6100_0015, (name_at + 12) as u64 | (1 << 32) | (7 << 48)));
        for (index, nid) in imports.iter().enumerate() {
            if matches!(*nid, "Qs0wWulgl7U" | "RaqxZIf6DvE" | "x8qnXqh-tiM") {
                let symbol = 0x600 + (index + 1) * 24;
                let name =
                    u32::from_le_bytes(bytes[symbol..symbol + 4].try_into().unwrap()) as usize;
                put(bytes, 0x400 + name + 11, b"#H#G");
            }
        }
    }
    descriptors.push((0, 0));
    let dynamic_count = 10 + descriptors.len();
    for (index, (tag, value)) in descriptors.into_iter().enumerate() {
        put(bytes, 0x200 + (10 + index) * 16, &tag.to_le_bytes());
        put(bytes, 0x208 + (10 + index) * 16, &value.to_le_bytes());
    }
    for at in [152, 160] {
        put(bytes, at, &((dynamic_count as u64) * 16).to_le_bytes());
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
fn check(p: Program, imports: &[&str], scope: &str) {
    let (code, calls) = finish(p);
    let input = raw_image(&code, imports, scope);
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert_eq!(
        fields["kernel_handles_live"],
        fields["kernel_handles_accounted"]
    );
}
fn word(p: &mut Program, address: i32, expected: u64) {
    p.path(7, address);
    p.code.extend([0x48, 0x8b, 0x07]);
    p.expect(expected);
}
fn zeros(p: &mut Program, address: i32, words: u64) {
    p.path(7, address);
    p.reg(8, words);
    let start = p.code.len();
    p.code.extend([0x48, 0x83, 0x3f, 0]); // CMP qword [RDI],0.
    checked(&mut p.code, &mut p.branches);
    p.code
        .extend([0x48, 0x83, 0xc7, 8, 0x49, 0xff, 0xc8, 0x0f, 0x85]);
    let displacement = start as i32 - (p.code.len() as i32 + 4);
    p.code.extend(displacement.to_le_bytes());
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
fn native_mouse_stateless_handles_opaque_open_and_uncapped_record_count_zero_exact_bytes() {
    let mut p = Program::new();
    errno_start(&mut p, 4);
    p.call(0);
    p.expect(0);
    p.reg(7, u64::MAX);
    p.reg(6, 0);
    p.reg(2, 1);
    p.reg(1, u64::MAX);
    p.call(1);
    p.expect(1); // Neither user nor fourth GP opaque param is dereferenced.
    p.reg(7, 7);
    p.reg(6, 1);
    p.reg(2, 0);
    p.reg(1, 0);
    p.call(1);
    p.expect(0x80df_0001);
    p.reg(7, 0);
    p.call(3);
    p.expect(0x80df_0003);
    p.reg(7, 0);
    p.reg(6, 0);
    p.reg(2, 0);
    p.call(2);
    p.expect(0x80df_0003);
    p.reg(7, 37);
    p.reg(6, 0);
    p.reg(2, 1);
    p.call(2);
    p.expect(0x80df_0001);
    p.reg(7, 37);
    p.path(6, DATA);
    p.reg(2, 0xffff_ffff);
    p.call(2);
    p.expect(0x80df_0001);
    word(&mut p, DATA, CANARY);
    p.reg(7, 37);
    p.call(3);
    p.expect(0);
    // More than Keyboard's sixteen-record limit is accepted by Mouse, even
    // after Close and with a positive handle never returned by Open.
    p.reg(7, 0x1111_0000_0025);
    p.path(6, DATA);
    p.reg(2, 0x2222_0000_0011);
    p.call(2);
    p.expect(0);
    zeros(&mut p, DATA, 85); // 17*40=680 bytes, all originally nonzero.
    word(&mut p, DATA - 8, CANARY);
    word(&mut p, DATA + 680, CANARY);
    errno(&mut p);
    check(
        p,
        &[
            "Qs0wWulgl7U",
            "RaqxZIf6DvE",
            "x8qnXqh-tiM",
            "cAnT0Rw-IwU",
            "9BcDykPmo1I",
        ],
        "Mouse",
    );
}

#[test]
fn native_keyboard_disconnected_records_and_six_gp_character_output_have_exact_tails() {
    let mut p = Program::new();
    errno_start(&mut p, 6);
    p.call(0);
    p.expect(0);
    p.reg(7, u64::MAX);
    p.reg(6, 0);
    p.reg(2, 0);
    p.reg(1, u64::MAX);
    p.call(1);
    p.expect(1);
    p.reg(7, 0);
    p.reg(6, 0);
    p.reg(2, 17);
    p.call(4);
    p.expect(0x80da_0003);
    p.reg(7, 23);
    p.path(6, DATA);
    p.reg(2, 17);
    p.call(4);
    p.expect(0x80da_0001);
    word(&mut p, DATA, CANARY);
    p.reg(7, 23);
    p.path(6, DATA);
    p.reg(2, 16);
    p.call(4);
    p.expect(0);
    zeros(&mut p, DATA, 192); // Includes every field and all96-byte record padding.
    word(&mut p, DATA - 8, CANARY);
    word(&mut p, DATA + 1536, CANARY);
    p.reg(7, 23);
    p.path(6, STATE);
    p.call(3);
    p.expect(0);
    zeros(&mut p, STATE, 12);
    word(&mut p, STATE - 8, CANARY);
    word(&mut p, STATE + 96, CANARY);
    p.reg(7, 23);
    p.reg(6, 2);
    p.reg(2, u64::MAX);
    p.reg(1, u64::MAX);
    p.reg(8, u64::MAX);
    p.path(9, CHARACTER);
    p.call(5);
    p.expect(0x80da_0001);
    word(&mut p, CHARACTER, CANARY);
    // RDX/RCX/R8 carry ignored LED/modifier/key bits. Only R9 is the output;
    // an incorrect third/fourth/sixth GP mapping cannot clear this canary.
    p.reg(7, 23);
    p.reg(6, 1);
    p.reg(2, u64::MAX);
    p.reg(1, u64::MAX);
    p.reg(8, u64::MAX);
    p.path(9, CHARACTER);
    p.call(5);
    p.expect(0);
    word(&mut p, CHARACTER - 8, CANARY);
    word(&mut p, CHARACTER, 0);
    word(&mut p, CHARACTER + 8, 0);
    word(&mut p, CHARACTER + 16, 0xa5a5_a5a5_0000_0000);
    p.reg(7, 23);
    p.call(2);
    p.expect(0);
    p.reg(7, 23);
    p.path(6, STATE);
    p.call(3);
    p.expect(0); // No retained handle lookup after Close.
    p.reg(7, u64::MAX);
    p.call(2);
    p.expect(0x80da_0003);
    errno(&mut p);
    check(
        p,
        &[
            "wadT3QBCGY0",
            "HJ+KnEHcaxI",
            "0LWei+c7RNc",
            "6HpE68bzX6M",
            "xybbGMCr738",
            "yO9JwdRhtSA",
            "9BcDykPmo1I",
        ],
        "Keyboard",
    );
}
