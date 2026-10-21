// SPDX-License-Identifier: GPL-2.0-only
//! Original consumers of genuine command-buffer growth callbacks.
use super::*;

const CALLBACK: i32 = 0x1100;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const SAVED: u64 = 0x6b94_38c1_d752_a0ef;

fn stack(p: &mut Program, opcode: u8, reg: u8, offset: u8) {
    if offset <= 127 {
        p.stack(opcode, reg, offset);
    } else {
        p.code.extend([
            0x48 | if reg >= 8 { 4 } else { 0 },
            opcode,
            0x84 | ((reg & 7) << 3),
            0x24,
        ]);
        p.code.extend(u32::from(offset).to_le_bytes());
    }
}

fn image(code: &[u8], callback: &[u8], scope: &str) -> Input {
    let source = scoped_system_image(code, &["LtTouSCZjHM", "t7PlZ9nt5Lc"], scope, scope, 1);
    let mut bytes = source.bytes.clone();
    bytes.resize(0x1600, 0);
    put(&mut bytes, 96, &0x1600u64.to_le_bytes());
    assert!(CALLBACK as usize + callback.len() < bytes.len());
    put(&mut bytes, CALLBACK as usize, callback);
    create_input(bytes)
}

fn callback(grows: bool, accepts: bool) -> Vec<u8> {
    let mut code = vec![
        0x41, 0x54, // PUSH R12.
        0x48, 0x83, 0xec, 16, 0x48, 0x89, 0x3c,
        0x24, // Save descriptor across the real nested call.
        0x49, 0x89, 0xd4, // R12 = user pointer.
        0x48, 0x8b, 0x42, 8, 0x48, 0x39,
        0xc7, // Callback arg0 equals the supplied descriptor.
    ];
    let mut branches = Vec::new();
    checked(&mut code, &mut branches);
    code.extend([0x48, 0x83, 0xfe, 3]); // Two requested words + one reserved.
    checked(&mut code, &mut branches);
    code.extend([0x48, 0xff, 0x02, 0xbf, 3, 0, 0, 0]);
    rip(&mut code, CALLBACK, &[0xff, 0x15], 0xf08); // Genuine NopGetSize(3).
    code.extend([0x48, 0x83, 0xf8, 12]);
    checked(&mut code, &mut branches);
    code.extend([0x48, 0x8b, 0x3c, 0x24, 0x4c, 0x89, 0xe2]);
    if grows {
        // Descriptor is frame+8; the actual new down cursor is frame+112.
        code.extend([0x48, 0x8d, 0x47, 104, 0x48, 0x89, 0x47, 24]);
    }
    code.extend([0x48, 0xb8]);
    // AL is the bool return. Poisoned upper bits must not turn false into true.
    code.extend((0x5a5a_5a5a_5a5a_0000u64 | u64::from(accepts)).to_le_bytes());
    code.extend([0x48, 0x83, 0xc4, 16, 0x41, 0x5c, 0xc3]);
    finish_checks(&mut code, branches);
    code
}

fn consumer(grows: bool, accepts: bool) -> (Vec<u8>, u64) {
    let mut p = Program::new();
    stack(&mut p, 0x89, 4, 240);
    stack(&mut p, 0x89, 12, 248);
    p.reg(12, SAVED);
    p.reg(0, CANARY);
    for at in [0, 64, 72, 80, 88, 96, 104, 112] {
        p.save(at);
    }
    for (field, target) in [(8, 80), (16, 112), (24, 80), (32, 84), (48, 176)] {
        stack(&mut p, 0x8d, 0, target);
        p.save(field);
    }
    p.path(0, CALLBACK);
    p.save(40);
    p.set(56, 1);
    p.reg(0, 0);
    stack(&mut p, 0x89, 0, 176);
    p.ptr(0, 8);
    stack(&mut p, 0x89, 0, 184);
    p.ptr(7, 8);
    p.reg(6, 2);
    for reg in [2, 1, 8, 9, 11] {
        p.reg(reg, u64::MAX);
    }
    p.call(0); // Ignore the returned pointer; consume the actual cursor and packet.
    for (field, target) in [
        (24, if grows && accepts { 88 } else { 80 }),
        (32, if grows { 112 } else { 84 }),
    ] {
        p.load(0, field);
        p.ptr(10, target);
        p.code.extend([0x4c, 0x39, 0xd0]);
        checked(&mut p.code, &mut p.branches);
    }
    p.word(
        80,
        if grows && accepts {
            0xa5a5_a5a5_c000_1000
        } else {
            CANARY
        },
    );
    for at in [0, 64, 72, 88, 96, 104, 112] {
        p.word(at, CANARY);
    }
    stack(&mut p, 0x8b, 0, 176);
    p.expect(1); // Exactly one genuine callback, including on false/failed fit.
    p.code.extend([0x48, 0x89, 0xe0]);
    stack(&mut p, 0x8b, 10, 240);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x4c, 0x89, 0xe0]);
    p.expect(SAVED);
    stack(&mut p, 0x8b, 12, 248);
    p.finish(None)
}

fn check(scope: &str, grows: bool, accepts: bool) {
    let (code, calls) = consumer(grows, accepts);
    let source = image(&code, &callback(grows, accepts), scope);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], (calls + 1).to_string(), "{text}");
    assert_eq!(fields["graphics_calls"], (calls + 1).to_string(), "{text}");
    assert_eq!(fields["initializer_calls"], "0", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
}

#[test]
fn native_agc_command_growth_runs_a_real_reentrant_callback_in_both_exact_pairs() {
    for scope in ["Graphics5", "libSceAgc"] {
        check(scope, true, true);
    }
    println!(
        "agc_command_growth_native_available=true agc_command_growth_native_programs=2 agc_command_growth_native_calls=4"
    );
}

#[test]
fn native_agc_command_growth_uses_bool_low_byte_and_rechecks_actual_capacity() {
    check("libSceAgc", true, false);
    check("libSceAgc", false, true);
    println!(
        "agc_command_growth_native_refusal_programs=2 agc_command_growth_native_refusal_calls=4"
    );
}
