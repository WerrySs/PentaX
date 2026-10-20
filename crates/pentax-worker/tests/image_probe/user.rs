// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn native_user_session_uses_configured_identity_and_delivers_login_only_once_across_initializers() {
    let mut p = Program::new();
    for reg in [7, 6, 2, 1, 8, 9] {
        p.reg(reg, u64::MAX);
    }
    p.call(0);
    p.expect(0);
    p.call(1);
    p.expect(0);
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(0);
    p.ptr(7, 0);
    p.call(2);
    p.expect(0);
    p.word(0, 0xa5a5_a5a5_0000_0025);
    p.ptr(7, 16);
    p.call(4);
    p.expect(0);
    p.word(16, 0xffff_ffff_0000_0025);
    p.word(24, u64::MAX);
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(48);
    p.save(56);
    p.reg(7, 0xffff_ffff_0000_0025);
    p.ptr(6, 48);
    p.reg(2, u64::MAX);
    p.call(5);
    p.expect(0);
    let mut expected = [0xa5; 16];
    expected[..12].copy_from_slice("PentaX 🦀\0".as_bytes());
    p.word(48, u64::from_le_bytes(expected[..8].try_into().unwrap()));
    p.word(56, u64::from_le_bytes(expected[8..].try_into().unwrap()));
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(112);
    p.reg(7, 37);
    p.ptr(6, 112);
    p.call(6);
    p.expect(0);
    p.word(112, 0xa5a5_a5a5_0000_0001);
    p.ptr(7, 32);
    p.call(3);
    p.expect(0);
    p.word(32, 37_u64 << 32);
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(32);
    p.call(0);
    p.expect(0);
    p.ptr(7, 32);
    p.call(3);
    p.expect(0x8096_0007);
    p.word(32, 0xa5a5_a5a5_a5a5_a5a5);
    p.reg(0, 9);
    p.save(80);
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(88);
    p.save(96);
    p.reg(7, 37);
    p.ptr(6, 80);
    p.call(7);
    p.expect(0);
    p.word(80, 40);
    p.word(88, 0xa5a5_a5a5_a5a5_a500);
    p.word(96, 0xa5a5_a5a5_a5a5_a5a5);
    p.reg(7, 9);
    p.reg(6, 0);
    p.reg(2, 0);
    p.call(5);
    p.expect(0x8096_0009);
    let (code, calls) = p.finish(None);
    let config = create_input(
        "schema_version=1\n[user]\nid=37\nname='PentaX 🦀'\n"
            .as_bytes()
            .to_vec(),
    );
    let nids = [
        "j3YMu1MVNNo",
        "az-0R6eviZ0",
        "CdWp0oHWGr0",
        "yH17Q6NWtVg",
        "fPhymKNvK-A",
        "1xxcMiGu2fo",
        "bwFjS+bX9mA",
        "-sD02mFDBh4",
    ];
    for (module, library, version, success) in [
        ("UserService", "UserService", 1, true),
        ("libSceUserService", "UserService", 1, true),
        ("UserService", "libSceUserService", 1, true),
        ("libSceUserService", "libSceUserService", 1, true),
        ("libSceUserService", "libSceUserService", 2, false),
    ] {
        let input = scoped_system_image(&code, &nids, module, library, version);
        let (status, fields, text) = probe_arguments(&[
            "--image".into(),
            input.path.as_os_str().to_owned(),
            "--config".into(),
            config.path.as_os_str().to_owned(),
        ]);
        if success {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["service_stops"], calls.to_string());
        } else {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "import-version-mismatch");
        }
    }
}

#[test]
fn native_user_scalar_preferences_and_number_alias_keep_int32_spans_and_source_errors() {
    let mut p = Program::new();
    for (slot, value) in [1_u64, 9, 0, 0, 1, 1, 0, 0].into_iter().enumerate() {
        p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
        p.save(0);
        p.reg(7, 1);
        p.ptr(6, 0);
        p.call(slot as i32);
        p.expect(0);
        p.word(0, 0xa5a5_a5a5_0000_0000 | value);
    }
    p.reg(7, 9);
    p.reg(6, 0);
    p.call(0);
    p.expect(0x8096_0005);
    p.reg(7, 9);
    p.ptr(6, 0);
    p.call(0);
    p.expect(0x8096_0009);
    let (code, calls) = p.finish(None);
    let input = scoped_system_image(
        &code,
        &[
            "qbwy0Ub8b3M",
            "woNpu+45RLk",
            "rnEhHqG-4xo",
            "ZKJtxdgvzwg",
            "qWYHOFwqCxY",
            "-3Y5GO+-i78",
            "hD-H81EN9Vg",
            "O6IW1-Dwm-w",
        ],
        "libSceUserService",
        "libSceUserService",
        1,
    );
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}
