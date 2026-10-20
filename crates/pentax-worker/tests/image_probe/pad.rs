// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn native_controller_entries_use_exact_scopes_return_widths_and_one_byte_output() {
    for (module, library, version) in [
        ("Pad", "Pad", 1),
        ("libScePad", "Pad", 1),
        ("Pad", "libScePad", 1),
        ("libScePad", "libScePad", 1),
        ("Pad", "Pad", 2),
    ] {
        let mut p = Program::new();
        p.call(0);
        p.expect(0);
        for (user, port, index, open, handle) in [
            (1, 0, 0, 1, 1),
            (1, 2, 0, 1, 1),
            (255, 16, 0, 1, 1),
            (1, 0, 1, 0x8092_0001, 0x8092_0008),
        ] {
            for (slot, expected) in [(1, open), (2, handle)] {
                p.reg(7, user);
                p.reg(6, port);
                p.reg(2, index);
                p.reg(1, u64::MAX); // The optional source parameter is ignored.
                p.call(slot);
                p.expect(expected);
            }
        }
        p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
        p.save(8);
        p.save(16);
        p.reg(7, 1);
        p.ptr(6, 15);
        p.call(4);
        p.expect(0);
        p.word(8, 0x00a5_a5a5_a5a5_a5a5);
        p.word(16, 0xa5a5_a5a5_a5a5_a5a5);
        p.reg(7, 2);
        p.reg(6, 0);
        p.call(4);
        p.expect(0x8092_0003);
        p.reg(7, u64::MAX);
        p.call(3);
        p.expect(0);
        let (code, calls) = p.finish(None);
        let original = scoped_system_image(
            &code,
            &[
                "hv1luiJrqQM",
                "xk0AcarP3V4",
                "u1GRHp+oWoY",
                "6ncge5+l5Qs",
                "fCWdlnmB1Ks",
            ],
            module,
            library,
            version,
        );
        let (status, fields, text) = probe(&original);
        if version == 1 {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["service_stops"], calls.to_string());
            assert_eq!(fields["unavailable_functions"], "0");
        } else {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "import-version-mismatch");
            assert!(!fields.contains_key("value"));
        }
    }
}
