// SPDX-License-Identifier: GPL-2.0-only
use super::*;

fn queries() -> [(Service, Vec<u8>); 2] {
    [
        (
            Service::SystemGetHdrToneMapLuminance,
            [0x42a0_0000_u32, 0x447a_0000, 0]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect(),
        ),
        (
            Service::SystemGetDisplaySafeAreaInfo,
            0x3f80_0000_u32.to_le_bytes().to_vec(),
        ),
    ]
}

#[test]
fn system_display_queries_write_exact_profile_fields_and_preserve_reserved_tails() {
    for (service, expected) in queries() {
        let mut owner = kernel(Default::default());
        let mut guest = TestGuest::new();
        guest.put(0, &[0xa5; 144]);
        call(&mut owner, &mut guest, service, &[ARG + 4]).unwrap();
        let mut whole = vec![0xa5; 144];
        whole[4..4 + expected.len()].copy_from_slice(&expected);
        assert_eq!(guest.get(ARG, 144), whole);
        assert_eq!(owner.usage().bytes_written, expected.len() as u64);
        assert_eq!(owner.usage().bytes_read, 0);

        // Safe-area's 128 reserved bytes are not part of its actual write.
        let end = ARG + PAGE - expected.len() as u64;
        call(&mut owner, &mut guest, service, &[end]).unwrap();
        assert_eq!(guest.get(end, expected.len()), expected);

        let before = guest.get(ARG, 144);
        let written = owner.usage().bytes_written;
        assert_eq!(call(&mut owner, &mut guest, service, &[0]), Ok(0x80a1_0003));
        assert_eq!(owner.usage().bytes_written, written);
        assert_eq!(guest.get(ARG, 144), before);
    }
}

#[test]
fn system_display_queries_refuse_faults_and_budgets_without_partial_publication() {
    for (service, expected) in queries() {
        for case in 0..7 {
            let mut owner = kernel(match case {
                4 => Limits {
                    max_bytes: expected.len() as u64 - 1,
                    ..Default::default()
                },
                5 => Limits {
                    max_work: 0,
                    ..Default::default()
                },
                6 => Limits {
                    max_attempts: 0,
                    ..Default::default()
                },
                _ => Default::default(),
            });
            let mut guest = TestGuest::new();
            guest.put(0, &[0xa5; 144]);
            guest.put(PAGE - 16, &[0x5a; 16]);
            let output = match case {
                0 => ARG + PAGE - expected.len() as u64 + 1,
                1 => u64::MAX - expected.len() as u64 + 1,
                _ => ARG,
            };
            if case == 2 {
                guest.regions[0].protection = 1;
            }
            if case == 3 {
                guest.refuse = true;
            }
            let effects = guest.effects;
            let result = call(&mut owner, &mut guest, service, &[output]);
            if case < 4 {
                assert!(matches!(result, Err(Failure::Guest(_))));
            } else {
                assert!(matches!(result, Err(Failure::Limit(_))));
            }
            assert_eq!(guest.get(ARG, 144), [0xa5; 144]);
            assert_eq!(guest.get(ARG + PAGE - 16, 16), [0x5a; 16]);
            assert_eq!(guest.effects, effects);
        }
    }
}

#[test]
fn system_display_queries_require_exact_reviewed_scopes_and_versions() {
    for (service, nid) in [
        (Service::SystemGetHdrToneMapLuminance, "mPpPxv5CZt4"),
        (Service::SystemGetDisplaySafeAreaInfo, "1n37q1Bvc5Y"),
    ] {
        for module in ["SystemService", "libSceSystemService"] {
            for library in ["SystemService", "libSceSystemService"] {
                assert_eq!(resolve(module, (1, 1), library, 1, nid), Some(service));
                assert_eq!(resolve(module, (1, 2), library, 1, nid), None);
                assert_eq!(resolve(module, (1, 1), library, 2, nid), None);
            }
        }
        assert_eq!(resolve("libkernel", (1, 1), "libkernel", 1, nid), None);
    }
}
