// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn native_system_display_queries_publish_exact_fields_and_source_null_errors() {
    let mut p = Program::new();
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    for offset in [16, 24, 32, 48, 56] {
        p.save(offset);
    }
    p.ptr(7, 16);
    p.call(0);
    p.expect(0);
    p.word(16, 0x447a_0000_42a0_0000);
    p.word(24, 0xa5a5_a5a5_0000_0000);
    p.word(32, 0xa5a5_a5a5_a5a5_a5a5);
    p.ptr(7, 48);
    p.call(1);
    p.expect(0);
    p.word(48, 0xa5a5_a5a5_3f80_0000);
    p.word(56, 0xa5a5_a5a5_a5a5_a5a5);
    for slot in [0, 1] {
        p.reg(7, 0);
        p.call(slot);
        p.expect(0x80a1_0003);
    }
    let (code, _) = p.finish(None);
    let source = scoped_system_image(
        &code,
        &["mPpPxv5CZt4", "1n37q1Bvc5Y"],
        "libSceSystemService",
        "libSceSystemService",
        1,
    );
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "4");
}

#[test]
fn native_system_parameters_use_selected_config_and_preserve_exact_scopes() {
    let mut p = Program::new();
    for id in [1, 400] {
        p.set(16, 0);
        p.reg(7, id);
        p.ptr(6, 16);
        p.call(0);
        p.expect(0);
        p.word(16, 4);
    }
    p.reg(7, 1);
    p.reg(6, 0);
    p.call(0);
    p.expect(0x80a1_0003);
    let (code, _) = p.finish(None);
    let config = create_input(b"schema_version = 1\n[system]\nlanguage_id = 4\n".to_vec());
    for (module, library, version, expected) in [
        ("SystemService", "SystemService", 1, "exit"),
        ("libSceSystemService", "SystemService", 1, "exit"),
        ("SystemService", "libSceSystemService", 1, "exit"),
        ("libSceSystemService", "libSceSystemService", 1, "exit"),
        ("libkernel", "libkernel", 1, "unavailable-function"),
        (
            "libSceSystemService",
            "libSceSystemService",
            2,
            "import-version-mismatch",
        ),
    ] {
        let source = scoped_system_image(&code, &["fZo48un7LK4"], module, library, version);
        let (status, fields, text) = probe_arguments(&[
            "--image".into(),
            source.path.as_os_str().to_owned(),
            "--config".into(),
            config.path.as_os_str().to_owned(),
        ]);
        if expected == "exit" {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
        } else {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], expected, "{text}");
        }
    }
}
