// SPDX-License-Identifier: GPL-2.0-only

//! Exact authorized209dffe libPlayGo.cpp identities, module1.0/library1.

use super::{Registration, Service};

const EXPORTS: [(Service, &str, &str, u8); 18] = [
    (
        Service::PlayGoInitialize,
        "ts6GlZOKRrE",
        "PlayGo::PlayGoInitialize",
        1,
    ),
    (
        Service::PlayGoTerminate,
        "MPe0EeBGM-E",
        "PlayGo::PlayGoTerminate",
        0,
    ),
    (Service::PlayGoOpen, "M1Gma1ocrGE", "PlayGo::PlayGoOpen", 2),
    (
        Service::PlayGoClose,
        "Uco1I0dlDi8",
        "PlayGo::PlayGoClose",
        1,
    ),
    (
        Service::PlayGoGetLocus,
        "uWIYLFkkwqk",
        "PlayGo::PlayGoGetLocus",
        4,
    ),
    (
        Service::PlayGoGetInstallSpeed,
        "rvBSfTimejE",
        "PlayGo::PlayGoGetInstallSpeed",
        2,
    ),
    (
        Service::PlayGoSetInstallSpeed,
        "4AAcTU9R3XM",
        "PlayGo::PlayGoSetInstallSpeed",
        2,
    ),
    (
        Service::PlayGoGetChunkId,
        "73fF1MFU8hA",
        "PlayGo::PlayGoGetChunkId",
        4,
    ),
    (
        Service::PlayGoGetToDoList,
        "Nn7zKwnA5q0",
        "PlayGo::PlayGoGetToDoList",
        4,
    ),
    (
        Service::PlayGoSetToDoList,
        "gUPGiOQ1tmQ",
        "PlayGo::PlayGoSetToDoList",
        3,
    ),
    (
        Service::PlayGoPrefetch,
        "-Q1-u1a7p0g",
        "PlayGo::PlayGoPrefetch",
        4,
    ),
    (
        Service::PlayGoGetEta,
        "v6EZ-YWRdMs",
        "PlayGo::PlayGoGetEta",
        4,
    ),
    (
        Service::PlayGoGetLanguageMask,
        "3OMbYZBaa50",
        "PlayGo::PlayGoGetLanguageMask",
        2,
    ),
    (
        Service::PlayGoGetProgress,
        "-RJWNMK3fC8",
        "PlayGo::PlayGoGetProgress",
        4,
    ),
    (
        Service::PlayGoGetOptionalChunk,
        "g4AZyxpSAlA",
        "PlayGo::PlayGoGetOptionalChunk",
        3,
    ),
    (
        Service::PlayGoPrefetchOptionalChunk,
        "HVAa744ecdw",
        "PlayGo::PlayGoPrefetchOptionalChunk",
        3,
    ),
    (
        Service::PlayGoGetInstallChunkId,
        "8-e7E989rCU",
        "PlayGo::PlayGoGetInstallChunkId",
        4,
    ),
    (
        Service::PlayGoGetSupportedOptionalChunk,
        "IfiN+-oeVWI",
        "PlayGo::PlayGoGetSupportedOptionalChunk",
        3,
    ),
];
const OBSERVED: [usize; 7] = [0, 1, 2, 3, 6, 7, 8];

pub(super) const PLAYGO_REGISTRATIONS: [Registration; 25] = {
    let base = Registration {
        service: Service::PlayGoInitialize,
        nid: "ts6GlZOKRrE",
        source_name: "PlayGo::PlayGoInitialize",
        arguments: 1,
        module: "PlayGo",
        module_version: (1, 0),
        library: "PlayGo",
        library_version: 1,
    };
    let mut entries = [base; 25];
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
            module: "libScePlayGo",
            library: "libScePlayGo",
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
    fn playgo_appends_eighteen_source_roles_and_only_seven_observed_aliases() {
        assert_eq!(
            Service::PlayGoInitialize as usize,
            Service::ShareSetCaptureSource as usize + 1
        );
        for (index, (service, nid, source_name, arguments)) in EXPORTS.into_iter().enumerate() {
            let row = &REGISTRATIONS[1238 + index];
            assert_eq!(
                (row.service, row.nid, row.source_name, row.arguments),
                (service, nid, source_name, arguments)
            );
            assert_eq!(
                (
                    row.module,
                    row.library,
                    row.module_version,
                    row.library_version
                ),
                ("PlayGo", "PlayGo", (1, 0), 1)
            );
            assert_eq!(resolve("PlayGo", (1, 0), "PlayGo", 1, nid), Some(service));
            assert_eq!(
                REGISTRATIONS
                    .iter()
                    .filter(|entry| entry.service == service)
                    .count(),
                if OBSERVED.contains(&index) { 2 } else { 1 }
            );
        }
        for (index, canonical) in OBSERVED.into_iter().enumerate() {
            let row = &REGISTRATIONS[1256 + index];
            assert_eq!(
                (row.service, row.nid, row.arguments),
                (
                    REGISTRATIONS[1238 + canonical].service,
                    REGISTRATIONS[1238 + canonical].nid,
                    REGISTRATIONS[1238 + canonical].arguments
                )
            );
            assert_eq!((row.module, row.library), ("libScePlayGo", "libScePlayGo"));
            assert_eq!(
                resolve(row.module, (1, 0), row.library, 1, row.nid),
                Some(row.service)
            );
        }
    }

    #[test]
    fn playgo_rejects_unobserved_siblings_mixed_axes_and_other_versions() {
        for (index, (_, nid, _, _)) in EXPORTS.into_iter().enumerate() {
            if !OBSERVED.contains(&index) {
                assert_eq!(
                    resolve("libScePlayGo", (1, 0), "libScePlayGo", 1, nid),
                    None
                );
            }
            for (module, library) in [("PlayGo", "libScePlayGo"), ("libScePlayGo", "PlayGo")] {
                assert_eq!(resolve(module, (1, 0), library, 1, nid), None);
            }
            for scope in ["PlayGo", "libScePlayGo"] {
                for (version, library_version) in [((1, 1), 1), ((2, 0), 1), ((1, 0), 2)] {
                    assert_eq!(resolve(scope, version, scope, library_version, nid), None);
                }
            }
        }
    }
}
