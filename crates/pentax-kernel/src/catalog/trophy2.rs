// SPDX-License-Identifier: GPL-2.0-only

use super::*;

const CASES: [(Service, &str, u8); 15] = [
    (Service::Trophy2CreateContext, "Bagshr7OQ6Q", 4),
    (Service::Trophy2CreateHandle, "Gz1rmUZpROM", 1),
    (Service::Trophy2RegisterContext, "bIDov3wBu5Q", 3),
    (Service::Trophy2GetGameInfo, "4IzqhhUQ3nk", 4),
    (Service::Trophy2GetGroupInfo, "DoZWauG8mu0", 5),
    (Service::Trophy2GetGroupInfoArray, "+PDSI6WgPRc", 7),
    (Service::Trophy2GetTrophyInfo, "EwNylPdWUTM", 5),
    (Service::Trophy2GetTrophyInfoArray, "y3zHpdZO6ME", 7),
    (Service::Trophy2GetGameIcon, "2QgUy+xJqS0", 4),
    (Service::Trophy2GetGroupIcon, "6IjXJUy6ZnA", 5),
    (Service::Trophy2GetTrophyIcon, "-9LLVU0uvs8", 5),
    (Service::Trophy2RegisterUnlockCallback, "sUXGfNMalIo", 2),
    (Service::Trophy2AbortHandle, "fYapWA9xVmA", 1),
    (Service::Trophy2DestroyHandle, "d8P11CI40KE", 1),
    (Service::Trophy2DestroyContext, "sysY2FHYff4", 1),
];
const OBSERVED: [usize; 5] = [0, 1, 2, 14, 13];

#[test]
fn trophy2_catalog_appends_fifteen_source_roles_and_only_five_observed_pairs() {
    assert_eq!(REGISTRATIONS.len(), 1283);
    assert_eq!(
        REGISTRATIONS[968].service,
        Service::GameIntentGetPropertyValueString
    );
    assert_eq!(
        Service::Trophy2CreateContext as usize,
        Service::GameIntentGetPropertyValueString as usize + 1
    );
    for (index, (service, nid, arguments)) in CASES.into_iter().enumerate() {
        let row = &REGISTRATIONS[969 + index];
        assert_eq!(
            (row.service, row.nid, row.arguments),
            (service, nid, arguments)
        );
        assert_eq!(row.source_name, format!("LibNpTrophy2::Np{service:?}"));
        assert_eq!((row.module, row.library), ("NpTrophy2", "NpTrophy2"));
        assert_eq!((row.module_version, row.library_version), ((1, 1), 1));
        assert_eq!(
            resolve("NpTrophy2", (1, 1), "NpTrophy2", 1, nid),
            Some(service)
        );
        let observed = OBSERVED.iter().position(|candidate| *candidate == index);
        assert_eq!(
            REGISTRATIONS
                .iter()
                .filter(|entry| entry.service == service)
                .count(),
            1 + usize::from(observed.is_some())
        );
        if let Some(position) = observed {
            let prefixed = &REGISTRATIONS[984 + position];
            assert_eq!(
                (prefixed.service, prefixed.nid, prefixed.arguments),
                (service, nid, arguments)
            );
            assert_eq!(
                (prefixed.module, prefixed.library),
                ("libSceNpTrophy2", "libSceNpTrophy2")
            );
            assert_eq!(
                (prefixed.module_version, prefixed.library_version),
                ((1, 1), 1)
            );
            assert_eq!(
                resolve("libSceNpTrophy2", (1, 1), "libSceNpTrophy2", 1, nid),
                Some(service)
            );
        }
    }
    assert_eq!(REGISTRATIONS[974].arguments, 7);
    assert_eq!(REGISTRATIONS[976].arguments, 7);
}

#[test]
fn trophy2_catalog_refuses_unobserved_prefixes_mixed_pairs_and_versions() {
    for (index, (_, nid, _)) in CASES.into_iter().enumerate() {
        if !OBSERVED.contains(&index) {
            assert_eq!(
                resolve("libSceNpTrophy2", (1, 1), "libSceNpTrophy2", 1, nid),
                None
            );
        }
        for (module, library) in [
            ("NpTrophy2", "libSceNpTrophy2"),
            ("libSceNpTrophy2", "NpTrophy2"),
            ("NpTrophy", "NpTrophy"),
        ] {
            assert_eq!(resolve(module, (1, 1), library, 1, nid), None);
        }
        for namespace in ["NpTrophy2", "libSceNpTrophy2"] {
            for (module_version, library_version) in [((2, 1), 1), ((1, 2), 1), ((1, 1), 2)] {
                assert_eq!(
                    resolve(namespace, module_version, namespace, library_version, nid),
                    None
                );
            }
        }
    }
}
