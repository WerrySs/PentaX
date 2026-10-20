// SPDX-License-Identifier: GPL-2.0-only
//! Original native calls; no host capture/menu/callback backend is inferred.
use super::*;
const INVALID: u64 = 0x8196_0002;
const UNSUPPORTED: u64 = 0x8196_0007;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;

fn media_image(code: &[u8], imports: &[&str], scope: &str) -> Input {
    assert_eq!(imports.last(), Some(&"9BcDykPmo1I"));
    let mut input = scoped_system_image(code, imports, scope, scope, 1);
    let bytes = &mut input.bytes;
    put(bytes, 0x800, code);
    let names = u64::from_le_bytes(bytes[0x218..0x220].try_into().unwrap()) as usize;
    let extra = b"libkernel\0libkernel\0";
    put(bytes, 0x400 + names, extra);
    put(bytes, 0x218, &((names + extra.len()) as u64).to_le_bytes());
    let symbol = 0x600 + imports.len() * 24;
    let name = u32::from_le_bytes(bytes[symbol..symbol + 4].try_into().unwrap()) as usize;
    put(bytes, 0x400 + name + 11, b"#F#E");
    let descriptors = [
        (
            0x6100_000f_i64,
            names as u64 | (1 << 32) | (1 << 40) | (4 << 48),
        ),
        (0x6100_0015, (names + 10) as u64 | (1 << 32) | (5 << 48)),
        (0, 0),
    ];
    for (index, (tag, value)) in descriptors.into_iter().enumerate() {
        put(bytes, 0x200 + (10 + index) * 16, &tag.to_le_bytes());
        put(bytes, 0x208 + (10 + index) * 16, &value.to_le_bytes());
    }
    for at in [152, 160] {
        put(bytes, at, &(13_u64 * 16).to_le_bytes());
    }
    std::fs::write(&input.path, bytes).unwrap();
    input
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
fn check(mut p: Program, imports: &[&str], scope: &str) {
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0x700);
    let input = media_image(&p.code, imports, scope);
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], p.calls.to_string());
    assert_eq!(fields["unavailable_functions"], "0");
    assert_eq!(
        fields["kernel_handles_live"],
        fields["kernel_handles_accounted"]
    );
}

#[test]
fn native_share_capture_exact_request_id_optional_output_opaque_param_and_errno() {
    for (scope, nid) in [
        ("Share", "4jt8pMDudgk"),
        ("Share", "AcDNpEpoT9U"),
        ("Share", "ErH6tKS7fzE"),
        ("Share", "GQTObcITIXI"),
        ("libSceShare", "ErH6tKS7fzE"),
        ("libSceShare", "GQTObcITIXI"),
    ] {
        let mut p = Program::new();
        errno_start(&mut p, 1);
        p.reg(7, u64::MAX);
        p.reg(6, 0);
        p.call(0);
        p.expect(UNSUPPORTED);
        p.reg(0, CANARY);
        p.save(8);
        p.ptr(7, 9);
        p.ptr(6, 9);
        p.call(0);
        p.expect(UNSUPPORTED);
        p.word(
            8,
            u64::from_le_bytes([0xa5, 0xff, 0xff, 0xff, 0xff, 0xa5, 0xa5, 0xa5]),
        );
        errno(&mut p);
        check(p, &[nid, "9BcDykPmo1I"], scope);
    }
}

#[test]
fn native_share_callbacks_do_not_invoke_poison_handlers_and_overlay_is_silent() {
    for scope in ["Share", "libSceShare"] {
        let mut imports = vec!["Sygnk9dr5WQ", "KnsfHKmZqFA"];
        if scope == "Share" {
            imports.push("T64o-315wbg");
        }
        imports.push("9BcDykPmo1I");
        let mut p = Program::new();
        errno_start(&mut p, imports.len() as i32 - 1);
        for word in [0, u64::MAX] {
            for slot in (0..imports.len() - 1).rev() {
                for reg in [7, 6, 2, 1] {
                    p.reg(reg, word);
                }
                p.call(slot as i32);
                p.expect(0);
            }
        }
        errno(&mut p);
        check(p, &imports, scope);
    }
}

#[test]
fn native_share_winning_permit_flags_params_and_exact_recording_status() {
    let mut p = Program::new();
    errno_start(&mut p, 7);
    for word in [0, 0x1111_1111_0000_0000, 0x2222_2222_8000_0000, u64::MAX] {
        for slot in 0..3 {
            p.reg(7, word);
            p.reg(6, u64::MAX);
            p.call(slot);
            p.expect(if word as u32 == 0 { INVALID } else { 0 });
        }
    }
    for word in [0, u64::MAX] {
        for slot in 3..5 {
            p.reg(7, word);
            p.call(slot);
            p.expect(if word == 0 { INVALID } else { 0 });
        }
    }
    for offset in [24, 32, 40, 48] {
        p.reg(0, CANARY);
        p.save(offset);
    }
    p.reg(7, 0);
    p.reg(6, u64::MAX);
    p.call(5);
    p.expect(INVALID);
    p.word(32, CANARY);
    p.reg(7, 0x1111_1111_8000_0000);
    p.ptr(6, 32);
    p.call(5);
    p.expect(0);
    p.word(24, CANARY);
    p.word(32, 0);
    p.word(40, 0);
    p.word(48, CANARY);
    p.reg(7, 0);
    p.call(6);
    p.expect(INVALID);
    p.ptr(7, 49);
    p.call(6);
    p.expect(0);
    p.word(48, u64::from_le_bytes([0xa5, 0, 0, 0, 0, 0xa5, 0xa5, 0xa5]));
    errno(&mut p);
    check(
        p,
        &[
            "YBiIdcDPrxs",
            "5wjxESwX68I",
            "kCurUZVFqcI",
            "7QZtURYnXG4",
            "ORspsWDXPps",
            "QNop2YAtIDE",
            "crFxyW3HdK0",
            "9BcDykPmo1I",
        ],
        "Share",
    );
}
