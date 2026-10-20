// SPDX-License-Identifier: GPL-2.0-only

use super::*;

const CASES: [(Service, &str, &str, u8); 4] = [
    (
        Service::GameIntentInitialize,
        "m87BHxt-H60",
        "LibNpGameIntent::NpGameIntentInitialize",
        1,
    ),
    (
        Service::GameIntentTerminate,
        "0HBYxYAjmf0",
        "LibNpGameIntent::NpGameIntentTerminate",
        0,
    ),
    (
        Service::GameIntentReceiveIntent,
        "jEIXUAr9XE8",
        "LibNpGameIntent::NpGameIntentReceiveIntent",
        1,
    ),
    (
        Service::GameIntentGetPropertyValueString,
        "rPl0INNc-M8",
        "LibNpGameIntent::NpGameIntentGetPropertyValueString",
        4,
    ),
];

#[test]
fn game_intent_catalog_appends_four_public_roles_in_two_exact_scopes() {
    assert_eq!(
        REGISTRATIONS[960].service,
        Service::PafMspaceMallocStatsFast
    );
    assert_eq!(
        Service::GameIntentInitialize as usize,
        Service::PafMspaceMallocStatsFast as usize + 1
    );
    for (index, (service, nid, name, arguments)) in CASES.into_iter().enumerate() {
        assert_eq!(
            REGISTRATIONS
                .iter()
                .filter(|entry| entry.service == service)
                .count(),
            2
        );
        for (scope, namespace) in ["NpGameIntent", "libSceNpGameIntent"]
            .into_iter()
            .enumerate()
        {
            let row = &REGISTRATIONS[961 + index * 2 + scope];
            assert_eq!(
                (row.service, row.nid, row.source_name, row.arguments),
                (service, nid, name, arguments)
            );
            assert_eq!((row.module, row.library), (namespace, namespace));
            assert_eq!((row.module_version, row.library_version), ((1, 1), 1));
            assert_eq!(resolve(namespace, (1, 1), namespace, 1, nid), Some(service));
        }
    }
}

#[test]
fn game_intent_catalog_refuses_mixed_names_and_other_versions() {
    for (_, nid, _, _) in CASES {
        for (module, library) in [
            ("NpGameIntent", "libSceNpGameIntent"),
            ("libSceNpGameIntent", "NpGameIntent"),
            ("GameIntent", "GameIntent"),
        ] {
            assert_eq!(resolve(module, (1, 1), library, 1, nid), None);
        }
        for namespace in ["NpGameIntent", "libSceNpGameIntent"] {
            for (module_version, library_version) in [((2, 1), 1), ((1, 2), 1), ((1, 1), 2)] {
                assert_eq!(
                    resolve(namespace, module_version, namespace, library_version, nid),
                    None
                );
            }
        }
    }
}
