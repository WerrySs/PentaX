// SPDX-License-Identifier: GPL-2.0-only

//! Original public-name/NID and exact admitted-scope expectations.
use super::*;

const CASES: [(Service, &str, u8); 23] = [
    (Service::MspaceCreate, "-hn1tcVHq5Q", 4),
    (Service::MspaceDestroy, "W6SiVSiCDtI", 1),
    (Service::MspaceMalloc, "OJjm-QOIHlI", 2),
    (Service::MspaceFree, "Vla-Z+eXlxo", 2),
    (Service::MspaceCalloc, "LYo3GhIlB38", 3),
    (Service::MspaceRealloc, "gigoVHZvVPE", 3),
    (Service::MspacePosixMemalign, "qWESlyXMI3E", 4),
    (Service::MspaceMemalign, "iF1iQHzxBJU", 3),
    (Service::MspaceAlignedAlloc, "ljkqMcC4-mk", 3),
    (Service::MspaceReallocalign, "p6lrRW8-MLY", 4),
    (Service::MspaceMallocStats, "mfHdJTIvhuo", 2),
    (Service::MspaceMallocStatsFast, "k04jLXu3+Ic", 2),
    (Service::MspaceMallocUsableSize, "fEoW6BJsPt4", 1),
    (Service::PafMspaceCreate, "6hdfGRKHefs", 5),
    (Service::PafMspaceDestroy, "qB5nGjWa-bk", 1),
    (Service::PafMspaceMalloc, "QuZzFJD5Hrw", 3),
    (Service::PafMspaceMemalign, "PKJcFUfhKtw", 4),
    (Service::PafMspaceFree, "9mMuuhXMwqQ", 3),
    (Service::PafMspaceCalloc, "-lZdT34nAAE", 4),
    (Service::PafMspaceRealloc, "u32UXVridxQ", 4),
    (Service::PafMspaceReallocalign, "4SvlEtd0j40", 5),
    (Service::PafMspaceMallocStats, "mO8NB8whKy8", 2),
    (Service::PafMspaceMallocStatsFast, "OmG3YPCBLJs", 2),
];

#[test]
fn mspace_catalog_appends_exact_public_roles_nids_and_argument_counts_once() {
    assert_eq!(REGISTRATIONS[937].service, Service::NpWebApi2Terminate);
    assert_eq!(
        Service::MspaceCreate as usize,
        Service::NpWebApi2Terminate as usize + 1
    );
    assert_eq!(
        Service::PafMspaceMallocStatsFast as usize,
        Service::MspaceCreate as usize + 22
    );
    for (index, (service, nid, arguments)) in CASES.into_iter().enumerate() {
        let entry = &REGISTRATIONS[938 + index];
        assert_eq!(entry.service, service);
        assert_eq!(entry.nid, nid);
        assert_eq!(entry.source_name, format!("sceLibc{service:?}"));
        assert_eq!(entry.arguments, arguments);
        assert_eq!(
            (entry.module, entry.library),
            ("libSceLibcInternal", "libSceLibcInternal")
        );
        assert_eq!((entry.module_version, entry.library_version), ((1, 1), 1));
        assert_eq!(
            REGISTRATIONS
                .iter()
                .filter(|entry| entry.service == service)
                .count(),
            1
        );
        assert_eq!(
            resolve("libSceLibcInternal", (1, 1), "libSceLibcInternal", 1, nid),
            Some(service)
        );
    }
}

#[test]
fn mspace_catalog_does_not_infer_canonical_mixed_or_other_version_aliases() {
    for (_, nid, _) in CASES {
        for (module, library) in [
            ("LibcInternal", "LibcInternal"),
            ("LibcInternal", "libSceLibcInternal"),
            ("libSceLibcInternal", "LibcInternal"),
            ("libc", "libc"),
            ("libSceLibcInternal", "libSceLibcInternalExt"),
        ] {
            assert_eq!(resolve(module, (1, 1), library, 1, nid), None);
        }
        for (module_version, library_version) in [((2, 1), 1), ((1, 2), 1), ((1, 1), 2)] {
            assert_eq!(
                resolve(
                    "libSceLibcInternal",
                    module_version,
                    "libSceLibcInternal",
                    library_version,
                    nid
                ),
                None
            );
        }
    }
    assert_eq!(
        resolve(
            "libSceLibcInternal",
            (1, 1),
            "libSceLibcInternal",
            1,
            "AAAAAAAAAAA"
        ),
        None
    );
}
