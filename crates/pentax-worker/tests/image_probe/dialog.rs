// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn native_common_dialog_base_observes_exact_scopes_and_has_no_implicit_active_panel() {
    let mut p = Program::new();
    for reg in [7, 6, 2, 1, 8, 9] {
        p.reg(reg, u64::MAX);
    }
    for slot in [1, 0, 0, 1] {
        p.call(slot);
        p.expect(0);
    }
    let (code, calls) = p.finish(None);
    for (module, library, version, success) in [
        ("CommonDialog", "CommonDialog", 1, true),
        ("libSceCommonDialog", "CommonDialog", 1, true),
        ("CommonDialog", "libSceCommonDialog", 1, true),
        ("libSceCommonDialog", "libSceCommonDialog", 1, true),
        ("libSceCommonDialog", "libSceCommonDialog", 2, false),
    ] {
        let source = scoped_system_image(
            &code,
            &["uoUpLGNkygk", "BQ3tey0JmQM"],
            module,
            library,
            version,
        );
        let (status, fields, text) = probe(&source);
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
