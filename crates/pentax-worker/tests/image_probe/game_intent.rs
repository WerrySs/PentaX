// SPDX-License-Identifier: GPL-2.0-only
//! Original imported four-function no-event consumer in both admitted profiles.
use super::json_primitives::Program;
use super::*;

fn pointer(p: &mut Program, register: usize, offset: u32) {
    p.code
        .extend([0x48, 0x8d, [0xbc, 0xb4, 0x94][register], 0x24]);
    p.code.extend(offset.to_le_bytes());
}
fn word(p: &mut Program, offset: u32, expected: u64) {
    p.code.extend([0x48, 0x8b, 0x84, 0x24]);
    p.code.extend(offset.to_le_bytes());
    p.result(expected);
}
fn passed(scope: &[u8]) {
    let mut p = Program::new();
    p.code[4..8].copy_from_slice(&32768_u32.to_le_bytes());
    pointer(&mut p, 0, 512);
    p.code.extend([0x48, 0xb8]);
    p.code.extend(0xa7a7_a7a7_a7a7_a7a7_u64.to_le_bytes());
    p.code.extend([0xb9]);
    p.code.extend(2089_u32.to_le_bytes());
    p.code.extend([0xf3, 0x48, 0xab]); // Fill 16704-byte info and 8-byte canary.
    p.code.extend([0x48, 0xbf]);
    p.code.extend(u64::MAX.to_le_bytes()); // Source initializer ignores pointer.
    p.call_nid(b"m87BHxt-H60");
    p.result(0);
    p.call_nid(b"0HBYxYAjmf0");
    p.result(0);
    pointer(&mut p, 0, 512);
    p.call_nid(b"jEIXUAr9XE8");
    p.result(0x8055_3806);
    for offset in [0, 45, 52, 300, 16704] {
        word(&mut p, 512 + offset, 0xa7a7_a7a7_a7a7_a7a7);
    }
    word(&mut p, 520, u64::from(u32::MAX)); // User and first four type bytes.
    for offset in [12, 20, 28, 37, 308, 8192, 16692] {
        word(&mut p, 512 + offset, 0);
    }
    p.code.extend([0x8b, 0x84, 0x24]);
    p.code.extend((512 + 16700_u32).to_le_bytes());
    p.result(0xa7a7_a7a7); // Struct's four alignment bytes are untouched.
    pointer(&mut p, 0, 512 + 308);
    p.argument(u64::MAX); // Key is nonnull opaque input; no host logging walk.
    pointer(&mut p, 2, 512 + 72);
    p.code.extend([0x48, 0xb9]);
    p.code.extend(u64::MAX.to_le_bytes());
    p.call_nid(b"rPl0INNc-M8");
    p.result(0x8055_3807);
    word(&mut p, 512 + 72, 0xa7a7_a7a7_a7a7_a700);
    p.code.extend([0x48, 0x81, 0xc4]);
    p.code.extend((32768 - 256_u32).to_le_bytes());
    let input = p.finish(scope, scope);
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert_eq!(fields["kernel_handles_live"], "0", "{text}");
    assert_eq!(fields["kernel_handles_accounted"], "0", "{text}");
}

#[test]
fn native_game_intent_preserves_layout_and_returns_real_no_event_outputs() {
    for scope in [b"NpGameIntent".as_slice(), b"libSceNpGameIntent".as_slice()] {
        passed(scope);
    }
}
