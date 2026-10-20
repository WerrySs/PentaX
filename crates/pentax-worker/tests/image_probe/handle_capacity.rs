// SPDX-License-Identifier: GPL-2.0-only

use super::*;

fn all_modules(p: &mut Program, slot: i32) {
    p.reg(12, 0);
    let begin = p.code.len();
    p.code.extend([0x4c, 0x89, 0xe7]); // RDI = callee-saved loop counter R12.
    p.call(slot);
    p.expect(0);
    p.code
        .extend([0x49, 0xff, 0xc4, 0x49, 0x81, 0xfc, 0x80, 1, 0, 0]);
    let next = p.code.len() + 6;
    p.code.extend([0x0f, 0x82]); // JB: all384 distinct original synthetic module IDs.
    p.code.extend(((begin as i32) - next as i32).to_le_bytes());
}

#[test]
fn native_long_startup_admits_more_than_compact_quota_and_balances_real_module_cleanup() {
    let mut p = Program::new();
    all_modules(&mut p, 0);
    p.reg(7, 0);
    p.call(0); // A repeated load retains a second reference, not another handle.
    p.expect(0);
    p.reg(7, 383);
    p.call(2);
    p.expect(0);
    all_modules(&mut p, 1);
    p.reg(7, 0);
    p.call(2);
    p.expect(0); // The extra reference must still be visible.
    p.reg(7, 0);
    p.call(1);
    p.expect(0);
    p.reg(7, 0);
    p.call(2);
    p.expect(0x805a_1001);
    let (code, _) = p.finish(None);
    let original = scoped_system_image(
        &code,
        &["g8cM39EUZ6o", "eR2bZFAAU0Q", "fMP5NHUOaMk"],
        "libSceSysmodule",
        "libSceSysmodule",
        1,
    );
    for profile in ["startup", "long-startup"] {
        let args = [
            OsString::from("--image"),
            original.path.as_os_str().to_owned(),
            OsString::from("--execution-profile"),
            OsString::from(profile),
        ];
        let (status, fields, text) = probe_arguments(&args);
        if profile == "startup" {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["budget"], "kernel-handles");
            assert_eq!(fields["public_function"], "SysmoduleLoadModule");
            assert_eq!(fields["kernel_handles_live"], "256");
            assert_eq!(fields["kernel_handles_accounted"], "256");
            assert_eq!(fields["kernel_handles_limit"], "256");
            assert!(fields["kernel_handles_by_type"].contains("modules:256"));
            assert!(!fields.contains_key("value"));
        } else {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["service_stops"], "773");
            assert_eq!(fields["kernel_handles_live"], "0");
            assert_eq!(fields["kernel_handles_accounted"], "0");
            assert_eq!(fields["kernel_handles_limit"], "4096");
            assert_eq!(fields["kernel_pthread_objects"], "0");
        }
    }
}
