// SPDX-License-Identifier: GPL-2.0-only

use super::*;

const CASES: [(Service, &str, u8); 5] = [
    (Service::EntitlementAccessInitialize, "jO8DM8oyego", 2),
    (Service::EntitlementAccessGetSkuFlag, "lPDO62PpJIA", 1),
    (
        Service::EntitlementAccessGetAddcontEntitlementInfoList,
        "TFyU+KFBv54",
        4,
    ),
    (
        Service::EntitlementAccessGetAddcontEntitlementInfo,
        "xddD23+8TfQ",
        3,
    ),
    (
        Service::EntitlementAccessGetEntitlementKey,
        "5LiMEPuW0DQ",
        3,
    ),
];

#[test]
fn entitlement_access_catalog_appends_five_exact_roles_and_two_observed_aliases() {
    assert_eq!(REGISTRATIONS.len(), 1283);
    assert_eq!(REGISTRATIONS[1049].service, Service::UdsTerminate);
    assert_eq!(
        Service::EntitlementAccessInitialize as usize,
        Service::UdsTerminate as usize + 1
    );
    for (index, (service, nid, arguments)) in CASES.into_iter().enumerate() {
        let row = &REGISTRATIONS[1050 + index];
        assert_eq!(
            (row.service, row.nid, row.arguments),
            (service, nid, arguments)
        );
        assert_eq!(
            (row.module, row.library),
            ("NpEntitlementAccess", "NpEntitlementAccess")
        );
        assert_eq!((row.module_version, row.library_version), ((1, 1), 1));
        let role = format!("{service:?}");
        assert_eq!(row.source_name, format!("LibNpEntitlementAccess::Np{role}"));
        assert_eq!(
            resolve(row.module, (1, 1), row.library, 1, nid),
            Some(service)
        );
        assert_eq!(
            REGISTRATIONS
                .iter()
                .filter(|row| row.service == service)
                .count(),
            if matches!(index, 0 | 2) { 2 } else { 1 }
        );
    }
    for (alias, canonical) in [(1055, 1050), (1056, 1052)] {
        let row = &REGISTRATIONS[alias];
        assert_eq!(
            (row.service, row.nid, row.arguments),
            (
                REGISTRATIONS[canonical].service,
                REGISTRATIONS[canonical].nid,
                REGISTRATIONS[canonical].arguments
            )
        );
        assert_eq!(
            (row.module, row.library),
            ("libSceNpEntitlementAccess", "libSceNpEntitlementAccess")
        );
        assert_eq!(
            resolve(row.module, (1, 1), row.library, 1, row.nid),
            Some(row.service)
        );
    }
}

#[test]
fn entitlement_access_catalog_refuses_unobserved_mixed_names_and_versions() {
    for (index, (_, nid, _)) in CASES.into_iter().enumerate() {
        if !matches!(index, 0 | 2) {
            assert_eq!(
                resolve(
                    "libSceNpEntitlementAccess",
                    (1, 1),
                    "libSceNpEntitlementAccess",
                    1,
                    nid
                ),
                None
            );
        }
        for (module, library) in [
            ("NpEntitlementAccess", "libSceNpEntitlementAccess"),
            ("libSceNpEntitlementAccess", "NpEntitlementAccess"),
            ("EntitlementAccess", "EntitlementAccess"),
        ] {
            assert_eq!(resolve(module, (1, 1), library, 1, nid), None);
        }
        for scope in ["NpEntitlementAccess", "libSceNpEntitlementAccess"] {
            for (module_version, library_version) in [((2, 1), 1), ((1, 2), 1), ((1, 1), 2)] {
                assert_eq!(
                    resolve(scope, module_version, scope, library_version, nid),
                    None
                );
            }
        }
    }
}
