// SPDX-License-Identifier: GPL-2.0-only
//! Original failed public JSON body through genuine imported native calls.
use super::json_primitives::Program;
use super::*;

#[test]
fn native_json_body_failure_reports_only_the_attested_public_function() {
    for (module, library) in [
        (b"Json2".as_slice(), b"Json2".as_slice()),
        (b"libSceJson".as_slice(), b"libSceJson2".as_slice()),
    ] {
        let mut program = Program::new();
        // The original consumer's own RX entry is a real, readable destination
        // whose complete 32-byte constructor write must be denied. No callback
        // behavior, private input, fabricated syscall or unmapped host address.
        rip(&mut program.code, 0x100, &[0x48, 0x8d, 0x3d], 0x100);
        program.call_nid(b"qBMjqyBn3OM");
        let image = program.finish(module, library);
        let (status, fields, text) = probe(&image);
        assert_ne!(status, 0, "{text}");
        assert_eq!(fields["code"], "runtime-service", "{text}");
        assert_eq!(fields["runtime_failure"], "guest-write-denied", "{text}");
        assert_eq!(
            fields["public_function"], "LibJson2::JsonValueCtor",
            "{text}"
        );
        assert_eq!(fields["service_stops"], "1", "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert_eq!(fields["kernel_handles_live"], "0", "{text}");
        assert!(!fields.contains_key("value"), "{text}");
    }
}
