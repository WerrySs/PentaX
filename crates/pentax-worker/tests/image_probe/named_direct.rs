// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn runtime_named_direct_maps_an_external_hint_with_actual_shared_aliases() {
    let mut p = Program::new();
    p.reg(7, 0x4000);
    p.reg(6, 0);
    p.reg(2, 3);
    p.ptr(1, 8);
    p.call(0);
    p.expect(0);
    p.set(16, 0);
    for output in [16, 24] {
        if output == 24 {
            p.load(0, 16);
            p.reg(10, 64 * 1024 * 1024 * 1024);
            p.code.extend([0x4c, 0x01, 0xd0]); // ADD RAX, R10, beyond initial arena.
            p.save(24);
        }
        p.ptr(7, output);
        p.reg(6, 0x4000);
        p.reg(2, 3);
        p.reg(1, 0);
        p.load(8, 8);
        p.reg(9, 0x4000);
        p.path(0, NAME);
        p.save(0); // Real seventh SysV argument.
        p.call(1);
        p.expect(0);
    }
    p.load(11, 16);
    p.code.extend([0x49, 0xc7, 0x03, 42, 0, 0, 0]);
    p.load(11, 24);
    p.code.extend([0x49, 0x8b, 0x03]);
    p.expect(42);
    let (code, calls) = p.finish(None);
    let original = image(&code, &["B+vc2AO2Zrc", "NcaWUxfMNIQ"]);
    let (status, fields, text) = probe_arguments(&[
        "--image".into(),
        original.path.as_os_str().to_owned(),
        "--memory-profile".into(),
        "runtime".into(),
    ]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}

#[test]
fn native_named_direct_reads_the_real_seventh_argument_and_preserves_mapping_semantics() {
    for case in 0..5 {
        let protection = if case == 4 { 0x8000_0003 } else { 3 };
        let mut p = Program::new();
        p.set(16, 0);
        p.set(24, 0);
        p.set(32, 0);
        p.call(4);
        p.expect(0); // Guest address-sanitizer instrumentation is disabled.
        p.reg(7, 0x4000);
        p.reg(6, 0);
        p.reg(2, 3);
        p.ptr(1, 8);
        p.call(0); // Allocate a genuine physical block.
        p.expect(0);
        p.ptr(7, 16);
        p.reg(6, 0x4000);
        p.reg(2, protection);
        p.reg(1, 0);
        p.load(8, 8);
        p.reg(9, 0x4000);
        match case {
            1 => p.reg(0, 0),
            3 => p.reg(0, u64::MAX),
            _ => p.path(0, 0xc00),
        }
        p.save(0); // Caller RSP; CALL pushes return below this seventh argument.
        p.call(1);
        if matches!(case, 0 | 4) {
            p.expect(0);
            p.load(7, 16);
            p.reg(6, 0);
            p.reg(2, 0);
            p.ptr(1, 32);
            p.call(3);
            p.expect(0);
            p.word(32, protection);
            p.ptr(7, 24);
            p.reg(6, 0x4000);
            p.reg(2, 3);
            p.reg(1, 0);
            p.load(8, 8);
            p.reg(9, 0x4000);
            p.call(2); // Refill caller-saved arguments for the unnamed alias.
            p.expect(0);
            p.load(11, 16);
            p.code.extend([0x49, 0xc7, 0x03, 42, 0, 0, 0]);
            p.load(11, 24);
            p.code.extend([0x49, 0x8b, 0x03]);
            p.expect(42);
        } else if case < 3 {
            p.expect(if case == 1 { 0x8002_000e } else { 0x8002_003f });
            p.word(16, 0);
        }
        let (code, calls) = p.finish(None);
        let original = image(
            &code,
            &[
                "B+vc2AO2Zrc",
                "NcaWUxfMNIQ",
                "L-Q3LEjIbgA",
                "WFcfL2lzido",
                "jh+8XiK4LeE",
            ],
        );
        let mut bytes = original.bytes.clone();
        put(&mut bytes, 0xc00, &[0xfe; 32]);
        bytes[0xc00 + if case == 2 { 32 } else { 31 }] = 0;
        let (status, fields, text) = probe(&create_input(bytes));
        assert_eq!(fields["unavailable_functions"], "0");
        assert_eq!(fields["service_stops"], calls.to_string());
        if case == 3 {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "runtime-service");
            assert!(!fields.contains_key("value"));
        } else {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
        }
    }
}
