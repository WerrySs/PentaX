// SPDX-License-Identifier: GPL-2.0-only
//! Original calls over silent content initialization, not a host media backend.
use super::*;

#[test]
fn native_content_init_exact_null_errors_opaque_success_and_search_minor_zero() {
    for (scope, nid, invalid, search) in [
        ("ContentExport", "0GnN4QCgIfs", 0x809d3016, false),
        ("libSceContentExport", "0GnN4QCgIfs", 0x809d3016, false),
        ("ContentSearch", "dPj4ZtRcIWk", 0x809d1003, true),
        ("ContentDelete", "zoxb0wEChEM", 0x809d5001, false),
    ] {
        let mut p = Program::new();
        for pointer in [0, u64::MAX, 1, 0] {
            p.reg(7, pointer);
            p.call(0);
            p.expect(if pointer == 0 { invalid } else { 0 });
        }
        let (code, calls) = p.finish(None);
        let mut input = scoped_system_image(&code, &[nid], scope, scope, 1);
        if search {
            put(
                &mut input.bytes,
                0x258,
                &(1_u64 | (1 << 40) | (2 << 48)).to_le_bytes(),
            );
        }
        put(&mut input.bytes, 0x800, &code);
        std::fs::write(&input.path, &input.bytes).unwrap();
        let (status, fields, text) = probe(&input);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["service_stops"], calls.to_string());
        assert_eq!(fields["unavailable_functions"], "0");
        assert_eq!(
            fields["kernel_handles_live"],
            fields["kernel_handles_accounted"]
        );
    }
}
