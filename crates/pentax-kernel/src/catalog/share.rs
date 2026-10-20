// SPDX-License-Identifier: GPL-2.0-only
//! Exact pinned Share identities; no host sharing or lifecycle is implied.

use super::{Registration, Service};

const EXPORTS: [(Service, &str, &str, u8); 3] = [
    (
        Service::ShareInitialize,
        "nBDD66kiFW8",
        "LibShare::ShareInitialize",
        3,
    ),
    (
        Service::ShareFeaturePermit,
        "YBiIdcDPrxs",
        "Share::ShareFeaturePermit",
        1,
    ),
    (
        Service::ShareTerminate,
        "0IL1keINExQ",
        "LibShare::ShareTerminate",
        0,
    ),
];

pub(super) const SHARE_REGISTRATIONS: [Registration; 5] = {
    let base = Registration {
        service: Service::ShareInitialize,
        nid: "nBDD66kiFW8",
        source_name: "LibShare::ShareInitialize",
        arguments: 3,
        module: "Share",
        module_version: (1, 1),
        library: "Share",
        library_version: 1,
    };
    let mut entries = [base; 5];
    let mut index = 0;
    while index < EXPORTS.len() {
        let (service, nid, source_name, arguments) = EXPORTS[index];
        entries[index] = Registration {
            service,
            nid,
            source_name,
            arguments,
            ..base
        };
        index += 1;
    }
    // Only these two full-prefix identities were independently observed.
    entries[3] = Registration {
        module: "libSceShare",
        library: "libSceShare",
        ..entries[0]
    };
    entries[4] = Registration {
        module: "libSceShare",
        library: "libSceShare",
        ..entries[2]
    };
    entries
};

#[cfg(test)]
mod tests {
    use super::super::{REGISTRATIONS, resolve};
    use super::*;

    #[test]
    fn share_catalogue_preserves_exact_roles_and_only_two_observed_aliases() {
        assert_eq!(REGISTRATIONS.len(), 1281);
        assert_eq!(
            Service::ShareInitialize as usize,
            Service::KeyboardGetKey2Char as usize + 1
        );
        for (index, (service, nid, name, count)) in EXPORTS.into_iter().enumerate() {
            let row = &REGISTRATIONS[1211 + index];
            assert_eq!(
                (row.service, row.nid, row.source_name, row.arguments),
                (service, nid, name, count)
            );
            assert_eq!(
                (
                    row.module,
                    row.module_version,
                    row.library,
                    row.library_version
                ),
                ("Share", (1, 1), "Share", 1)
            );
            assert_eq!(resolve("Share", (1, 1), "Share", 1, nid), Some(service));
            assert_eq!(
                resolve("libSceShare", (1, 1), "libSceShare", 1, nid),
                if index == 1 { None } else { Some(service) }
            );
            assert_eq!(
                REGISTRATIONS
                    .iter()
                    .filter(|r| r.service == service)
                    .count(),
                if index == 1 { 1 } else { 2 }
            );
            for (module, library) in [("Share", "libSceShare"), ("libSceShare", "Share")] {
                assert_eq!(resolve(module, (1, 1), library, 1, nid), None);
            }
            for scope in ["Share", "libSceShare"] {
                for (version, library) in [((2, 1), 1), ((1, 2), 1), ((1, 1), 2)] {
                    assert_eq!(resolve(scope, version, scope, library, nid), None);
                }
            }
        }
        for (alias, canonical) in [(1214, 1211), (1215, 1213)] {
            assert_eq!(
                REGISTRATIONS[alias].service,
                REGISTRATIONS[canonical].service
            );
            assert_eq!(
                REGISTRATIONS[alias].arguments,
                REGISTRATIONS[canonical].arguments
            );
        }
    }
}
