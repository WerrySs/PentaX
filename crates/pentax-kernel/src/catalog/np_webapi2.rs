// SPDX-License-Identifier: GPL-2.0-only

//! Original exact-identity expectations from pinned public libNet.cpp.
use super::*;

const CASES: [(Service, &str, u8); 19] = [
    (Service::NpWebApi2Initialize, "+o9816YQhqQ", 2),
    (Service::NpWebApi2GetMemoryPoolStats, "Xweb+naPZ8Y", 2),
    (Service::NpWebApi2PushEventCreateHandle, "WV1GwM32NgY", 1),
    (Service::NpWebApi2CreateUserContext, "sk54bi6FtYM", 2),
    (Service::NpWebApi2DeleteUserContext, "9X9+cneTGUU", 1),
    (Service::NpWebApi2CreateRequest, "3EI-OSJ65Xc", 6),
    (Service::NpWebApi2SendRequest, "lQOCF84lvzw", 4),
    (Service::NpWebApi2ReadData, "OOY9+ObfKec", 3),
    (Service::NpWebApi2AbortRequest, "zpiPsH7dbFQ", 1),
    (Service::NpWebApi2DeleteRequest, "vvzWO-DvG1s", 1),
    (Service::NpWebApi2AddHttpRequestHeader, "egOOvrnF6mI", 3),
    (
        Service::NpWebApi2GetHttpResponseHeaderValueLength,
        "HwP3aM+c85c",
        3,
    ),
    (
        Service::NpWebApi2GetHttpResponseHeaderValue,
        "hksbskNToEA",
        4,
    ),
    (Service::NpWebApi2PushEventDeleteHandle, "fIATVMo4Y1w", 2),
    (
        Service::NpWebApi2PushEventDeletePushContext,
        "QafxeZM3WK4",
        2,
    ),
    (Service::NpWebApi2PushEventCreateFilter, "MsaFhR+lPE4", 6),
    (
        Service::NpWebApi2PushEventRegisterCallback,
        "fY3QqeNkF8k",
        4,
    ),
    (Service::NpWebApi2CheckTimeout, "3Tt9zL3tkoc", 0),
    (Service::NpWebApi2Terminate, "bEvXpcEk200", 1),
];
const SCOPES: [(&str, &str); 4] = [
    ("NpWebApi2", "NpWebApi2"),
    ("libSceNpWebApi2", "NpWebApi2"),
    ("NpWebApi2", "libSceNpWebApi2"),
    ("libSceNpWebApi2", "libSceNpWebApi2"),
];

#[test]
fn np_webapi2_appends_exact_public_nids_source_names_and_abi_counts() {
    assert_eq!(REGISTRATIONS[861].service, Service::Http2SetTimeOut);
    assert_eq!(REGISTRATIONS[861].nid, "VYMxTcBqSE0");
    assert_eq!(
        Service::NpWebApi2Initialize as usize,
        Service::Http2SetTimeOut as usize + 1
    );
    assert_eq!(
        Service::NpWebApi2Terminate as usize,
        Service::NpWebApi2Initialize as usize + 18
    );
    assert!(
        REGISTRATIONS[..862]
            .iter()
            .all(|entry| !entry.source_name.starts_with("LibNpWebApi2::"))
    );
    for (index, (service, nid, arguments)) in CASES.into_iter().enumerate() {
        let rows = REGISTRATIONS
            .iter()
            .filter(|entry| entry.service == service)
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 4);
        for (scope, (module, library)) in SCOPES.into_iter().enumerate() {
            let row = &REGISTRATIONS[862 + index * 4 + scope];
            assert_eq!(row.service, service);
            assert_eq!(row.nid, nid);
            assert_eq!(row.arguments, arguments);
            assert_eq!(row.source_name, format!("LibNpWebApi2::{service:?}"));
            assert_eq!((row.module, row.library), (module, library));
            assert_eq!((row.module_version, row.library_version), ((1, 1), 1));
            assert_eq!(resolve(module, (1, 1), library, 1, nid), Some(service));
        }
    }
}

#[test]
fn np_webapi2_refuses_other_names_versions_and_unregistered_nids() {
    for (_, nid, _) in CASES {
        for (module, library) in SCOPES {
            for (module_version, library_version) in [((2, 1), 1), ((1, 2), 1), ((1, 1), 2)] {
                assert_eq!(
                    resolve(module, module_version, library, library_version, nid),
                    None
                );
            }
        }
        for (module, library) in [
            ("NpWebApi", "NpWebApi2"),
            ("NpWebApi2", "NpWebApi"),
            ("libSceNpCppWebApi", "libSceNpWebApi2"),
            ("libScelibSceNpWebApi2", "libSceNpWebApi2"),
            ("libSceNpWebApi2", "libkernel"),
        ] {
            assert_eq!(resolve(module, (1, 1), library, 1, nid), None);
        }
    }
    assert_eq!(
        resolve("NpWebApi2", (1, 1), "NpWebApi2", 1, "AAAAAAAAAAA"),
        None
    );
}
