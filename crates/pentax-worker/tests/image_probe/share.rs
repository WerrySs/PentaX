// SPDX-License-Identifier: GPL-2.0-only
//! Original native consumers; these do not establish host recording/sharing.

use super::*;

#[test]
fn native_share_exact_lifecycle_and_winning_permit_mask_preserve_source_order() {
    for (scope, imports) in [
        ("Share", vec!["nBDD66kiFW8", "YBiIdcDPrxs", "0IL1keINExQ"]),
        ("libSceShare", vec!["nBDD66kiFW8", "0IL1keINExQ"]),
    ] {
        let mut p = Program::new();
        for word in [u64::MAX, 0, 0x1111_1111_8000_0000] {
            // Reverse order also exercises Terminate/Permit before Init.
            for slot in (0..imports.len()).rev() {
                p.reg(7, word);
                p.reg(6, word);
                p.reg(2, word);
                p.call(slot as i32);
                p.expect(if scope == "Share" && slot == 1 && word as u32 == 0 {
                    0x8196_0002
                } else {
                    0
                });
            }
        }
        let (code, calls) = p.finish(None);
        assert!(code.len() < 0x700);
        let mut input = scoped_system_image(&code, &imports, scope, scope, 1);
        // The common file fixture's unused path strings may overlap a longer
        // original body. Keep actual instructions intact before execution.
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
