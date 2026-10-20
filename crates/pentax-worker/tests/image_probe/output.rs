// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn native_standard_output_preserves_results_and_keeps_payload_out_of_protocol() {
    let mut p = Program::new();
    p.call(0); // errno location
    p.save(0);
    p.set_errno(37);
    p.reg(0, u64::from_le_bytes(*b"PX\0bytes"));
    p.save(16);
    for (slot, descriptor, length) in [(1, 1, 8), (2, 2, 8), (3, 0, 3)] {
        p.reg(7, descriptor);
        p.ptr(6, 16);
        p.reg(2, length);
        p.call(slot);
        p.expect(length);
        p.errno(37);
    }
    p.reg(7, 1);
    p.reg(6, 0);
    p.reg(2, 1);
    p.call(1); // KernelWrite: int64 negative kernel error, no errno change.
    p.expect(0xffff_ffff_8002_000e);
    p.errno(37);
    p.reg(7, 1);
    p.reg(6, 0);
    p.reg(2, 1);
    p.call(3); // POSIX write: int64 -1 and per-thread errno.
    p.expect(u64::MAX);
    p.errno(14);
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&image(
        &code,
        &["9BcDykPmo1I", "4wSze92BhLI", "FN4gaPmuFV8", "FxVZqBAA7ks"],
    ));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["output_bytes"], "19");
    assert!(!text.contains("PX\0bytes"));
    assert!(!fields.contains_key("public_function"));
}
