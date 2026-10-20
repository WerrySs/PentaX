// SPDX-License-Identifier: GPL-2.0-only

use super::*;

const CASES: [(Service, &str, u8); 41] = [
    (Service::UdsInitialize, "sjaobBgqeB4", 1),
    (Service::UdsCreateContext, "5zBnau1uIEo", 4),
    (Service::UdsCreateHandle, "hT0IAEvN+M0", 1),
    (Service::UdsCreateEvent, "p+GcLqwpL9M", 4),
    (Service::UdsPostEvent, "CzkKf7ahIyU", 4),
    (Service::UdsDestroyHandle, "AUIHb7jUX3I", 1),
    (Service::UdsAbortHandle, "jZCqWFgMehE", 1),
    (Service::UdsDestroyContext, "wB7IWzGp2v0", 1),
    (Service::UdsGetMemoryStat, "su7jW3VDDb4", 1),
    (Service::UdsEventEstimateSize, "+s14jq-KGYw", 2),
    (Service::UdsEventToString, "vj6CQGWtEBg", 4),
    (Service::UdsDestroyEvent, "wG+84pnNIuo", 1),
    (Service::UdsRegisterContext, "tpFJ8LIKvPw", 3),
    (Service::UdsCreateEventPropertyObject, "s6W4Zl4Slgk", 1),
    (Service::UdsDestroyEventPropertyObject, "kKUH0Viib3c", 1),
    (Service::UdsEventPropertyObjectSetString, "MfDb+4Nln64", 3),
    (Service::UdsEventPropertyObjectSetInt32, "YE4dbtbz6OE", 3),
    (Service::UdsEventPropertyObjectSetUInt32, "AzD4irAcKE4", 3),
    (Service::UdsEventPropertyObjectSetInt64, "56QLTqx911s", 3),
    (Service::UdsEventPropertyObjectSetUInt64, "xvsP5Yz6FmY", 3),
    (Service::UdsEventPropertyObjectSetFloat32, "lbPlT4+QVcE", 3),
    (Service::UdsEventPropertyObjectSetFloat64, "4Fu8tHW+u-k", 3),
    (Service::UdsEventPropertyObjectSetBool, "Fidd8vWgyVE", 3),
    (Service::UdsEventPropertyObjectSetBinary, "wAcxBDLHj1M", 4),
    (Service::UdsEventPropertyObjectSetObject, "74ASEqxSnkM", 4),
    (Service::UdsEventPropertyObjectSetArray, "Wxbg5x3pTXA", 4),
    (Service::UdsCreateEventPropertyArray, "Hm7qubT3b70", 1),
    (Service::UdsDestroyEventPropertyArray, "W-0xwY0ZMjw", 1),
    (Service::UdsEventPropertyArraySetString, "4llLk7YJRTE", 2),
    (Service::UdsEventPropertyArraySetInt32, "BypQuF113-k", 2),
    (Service::UdsEventPropertyArraySetUInt32, "yMi0xAOpmXM", 2),
    (Service::UdsEventPropertyArraySetInt64, "viVXAwmmYrY", 2),
    (Service::UdsEventPropertyArraySetUInt64, "Qo9qR7v5zO4", 2),
    (Service::UdsEventPropertyArraySetFloat32, "JmgwKm96Lq4", 2),
    (Service::UdsEventPropertyArraySetFloat64, "sbSYZLR5AiE", 2),
    (Service::UdsEventPropertyArraySetBool, "0+l4QSWCM4E", 2),
    (Service::UdsEventPropertyArraySetBinary, "IEdUCV9j2Cw", 3),
    (Service::UdsEventPropertyArraySetObject, "XY14n3jNIpE", 3),
    (Service::UdsEventPropertyArraySetArray, "rdi9BAfDLq8", 3),
    (Service::UdsGetStorageStat, "KmN62tT4U8A", 2),
    (Service::UdsTerminate, "47UAEuQl+iI", 0),
];
const OBSERVED: [usize; 20] = [
    0, 1, 2, 3, 4, 5, 7, 11, 12, 13, 15, 16, 17, 18, 19, 22, 25, 28, 37, 40,
];

#[test]
fn uds_catalog_appends_all_source_roles_and_only_twenty_observed_prefixes() {
    assert_eq!(REGISTRATIONS.len(), 1283);
    assert_eq!(REGISTRATIONS[988].service, Service::Trophy2DestroyHandle);
    assert_eq!(
        Service::UdsInitialize as usize,
        Service::Trophy2DestroyContext as usize + 1
    );
    for (index, (service, nid, arity)) in CASES.into_iter().enumerate() {
        let row = &REGISTRATIONS[989 + index];
        assert_eq!((row.service, row.nid, row.arguments), (service, nid, arity));
        let role = format!("{service:?}");
        let suffix = role.strip_prefix("Uds").unwrap();
        assert_eq!(
            row.source_name,
            format!("LibNpUniversalDataSystem::NpUniversalDataSystem{suffix}")
        );
        assert_eq!(
            (row.module, row.library),
            ("NpUniversalDataSystem", "NpUniversalDataSystem")
        );
        assert_eq!((row.module_version, row.library_version), ((1, 1), 1));
        assert_eq!(
            resolve(row.module, (1, 1), row.library, 1, nid),
            Some(service)
        );
        let observed = OBSERVED.iter().position(|candidate| *candidate == index);
        assert_eq!(
            REGISTRATIONS
                .iter()
                .filter(|row| row.service == service)
                .count(),
            1 + usize::from(observed.is_some())
        );
        if let Some(position) = observed {
            let alias = &REGISTRATIONS[1030 + position];
            assert_eq!(
                (alias.service, alias.nid, alias.arguments),
                (service, nid, arity)
            );
            assert_eq!(
                (alias.module, alias.library),
                ("libSceNpUniversalDataSystem", "libSceNpUniversalDataSystem")
            );
            assert_eq!(
                resolve(alias.module, (1, 1), alias.library, 1, nid),
                Some(service)
            );
        }
    }
    // C semantic arity includes the logging-only FP operand. The value occupies
    // XMM0, never RDX for ObjectSet or RSI for ArraySet; it is not a third GP.
    for index in [20, 21] {
        assert_eq!(REGISTRATIONS[989 + index].arguments, 3);
    }
    for index in [33, 34] {
        assert_eq!(REGISTRATIONS[989 + index].arguments, 2);
    }
    assert_eq!(
        REGISTRATIONS[1024].service,
        Service::UdsEventPropertyArraySetBool
    );
}

#[test]
fn uds_catalog_refuses_unobserved_and_mixed_scope_or_version_aliases() {
    for (index, (_, nid, _)) in CASES.into_iter().enumerate() {
        if !OBSERVED.contains(&index) {
            assert_eq!(
                resolve(
                    "libSceNpUniversalDataSystem",
                    (1, 1),
                    "libSceNpUniversalDataSystem",
                    1,
                    nid
                ),
                None
            );
        }
        for (module, library) in [
            ("NpUniversalDataSystem", "libSceNpUniversalDataSystem"),
            ("libSceNpUniversalDataSystem", "NpUniversalDataSystem"),
            ("UniversalDataSystem", "UniversalDataSystem"),
        ] {
            assert_eq!(resolve(module, (1, 1), library, 1, nid), None);
        }
        for namespace in ["NpUniversalDataSystem", "libSceNpUniversalDataSystem"] {
            for (module_version, library_version) in [((2, 1), 1), ((1, 2), 1), ((1, 1), 2)] {
                assert_eq!(
                    resolve(namespace, module_version, namespace, library_version, nid),
                    None
                );
            }
        }
    }
}
