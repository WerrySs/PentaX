// SPDX-License-Identifier: GPL-2.0-only

//! Exact pinned SystemService notice, splash, string and power FUNC identities.

use super::{Registration, Service};

const EXPORTS: [(Service, &str, &str, u8); 7] = [
    (
        Service::SystemHideSplashScreen,
        "Vo5V8KAwCmk",
        "SystemService::SystemServiceHideSplashScreen",
        0,
    ),
    (
        Service::SystemParamGetString,
        "SsC-m-S9JTA",
        "SystemService::SystemServiceParamGetString",
        3,
    ),
    (
        Service::SystemGetNoticeScreenSkipFlag,
        "3RQ5aQfnstU",
        "SystemService::SystemServiceGetNoticeScreenSkipFlag",
        1,
    ),
    (
        Service::SystemDisableNoticeScreenSkipFlagAutoSet,
        "8Lo6Zv94aho",
        "SystemService::SystemServiceDisableNoticeScreenSkipFlagAutoSet",
        0,
    ),
    (
        Service::SystemSetNoticeScreenSkipFlag,
        "Q3utJvma4Mo",
        "SystemService::SystemServiceSetNoticeScreenSkipFlag",
        0,
    ),
    (
        Service::SystemPowerTick,
        "XbbJC3E+L5M",
        "SystemService::SystemServicePowerTick",
        0,
    ),
    (
        Service::SystemReportAbnormalTermination,
        "3s8cHiCBKBE",
        "SystemService::SystemServiceReportAbnormalTermination",
        1,
    ),
];

// Independently observed full-prefix identities, in the frozen append order.
const OBSERVED: [usize; 2] = [2, 0];

pub(super) const SYSTEM_NOTICE_REGISTRATIONS: [Registration; 9] = {
    let base = Registration {
        service: Service::SystemHideSplashScreen,
        nid: "Vo5V8KAwCmk",
        source_name: "SystemService::SystemServiceHideSplashScreen",
        arguments: 0,
        module: "SystemService",
        module_version: (1, 1),
        library: "SystemService",
        library_version: 1,
    };
    let mut entries = [base; 9];
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
    index = 0;
    while index < OBSERVED.len() {
        entries[EXPORTS.len() + index] = Registration {
            module: "libSceSystemService",
            library: "libSceSystemService",
            ..entries[OBSERVED[index]]
        };
        index += 1;
    }
    entries
};

#[cfg(test)]
mod tests {
    use super::super::{REGISTRATIONS, resolve};
    use super::*;

    #[test]
    fn system_notice_appends_seven_exact_roles_with_source_labels_and_gp_arities() {
        assert_eq!(REGISTRATIONS.len(), 1281);
        assert_eq!(
            Service::SystemHideSplashScreen as usize,
            Service::PosixThreadSetSchedParam as usize + 1
        );
        for (index, (service, nid, name, arguments)) in EXPORTS.into_iter().enumerate() {
            let row = &REGISTRATIONS[1272 + index];
            assert_eq!(
                (row.service, row.nid, row.source_name, row.arguments),
                (service, nid, name, arguments)
            );
            assert_eq!(
                (
                    row.module,
                    row.library,
                    row.module_version,
                    row.library_version
                ),
                ("SystemService", "SystemService", (1, 1), 1)
            );
            assert_eq!(
                resolve("SystemService", (1, 1), "SystemService", 1, nid),
                Some(service)
            );
            assert_eq!(
                REGISTRATIONS
                    .iter()
                    .filter(|entry| entry.service == service)
                    .count(),
                if OBSERVED.contains(&index) { 2 } else { 1 }
            );
        }
        for (index, canonical) in OBSERVED.into_iter().enumerate() {
            let row = &REGISTRATIONS[1279 + index];
            assert_eq!(
                (row.service, row.nid, row.arguments),
                (
                    REGISTRATIONS[1272 + canonical].service,
                    REGISTRATIONS[1272 + canonical].nid,
                    REGISTRATIONS[1272 + canonical].arguments
                )
            );
            assert_eq!(
                (
                    row.module,
                    row.library,
                    row.module_version,
                    row.library_version
                ),
                ("libSceSystemService", "libSceSystemService", (1, 1), 1)
            );
            assert_eq!(
                resolve(row.module, (1, 1), row.library, 1, row.nid),
                Some(row.service)
            );
        }
    }

    #[test]
    fn system_notice_refuses_unobserved_prefixes_mixed_axes_and_other_versions() {
        for (index, (_, nid, _, _)) in EXPORTS.into_iter().enumerate() {
            if !OBSERVED.contains(&index) {
                assert_eq!(
                    resolve("libSceSystemService", (1, 1), "libSceSystemService", 1, nid),
                    None
                );
            }
            for (module, library) in [
                ("SystemService", "libSceSystemService"),
                ("libSceSystemService", "SystemService"),
            ] {
                assert_eq!(resolve(module, (1, 1), library, 1, nid), None);
            }
            for scope in ["SystemService", "libSceSystemService"] {
                for (module_version, library_version) in [((1, 0), 1), ((2, 1), 1), ((1, 1), 2)] {
                    assert_eq!(
                        resolve(scope, module_version, scope, library_version, nid),
                        None
                    );
                }
            }
        }
    }
}
