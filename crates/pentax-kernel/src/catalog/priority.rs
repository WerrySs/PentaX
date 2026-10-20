// SPDX-License-Identifier: GPL-2.0-only

//! Exact priority identities from authorized209dffe libKernel.cpp.
//! The appended POSIX rows do not inherit extra spelling aliases.

use super::{Registration, Service};

const EXPORTS: [(Service, &str, &str, u8, &str); 9] = [
    (
        Service::PthreadGetprio,
        "1tKyG7RlMJo",
        "LibKernel::PthreadGetprio",
        2,
        "libkernel",
    ),
    (
        Service::PthreadSetprio,
        "W0Hpm2X0uPE",
        "LibKernel::PthreadSetprio",
        2,
        "libkernel",
    ),
    (
        Service::PosixSchedGetPriorityMax,
        "CBNtXOoef-E",
        "Posix::sched_get_priority_max",
        1,
        "libkernel",
    ),
    (
        Service::PosixSchedGetPriorityMin,
        "m0iS6jNsXds",
        "Posix::sched_get_priority_min",
        1,
        "libkernel",
    ),
    (
        Service::PosixThreadSetprio,
        "a2P9wYGeZvc",
        "Posix::pthread_setprio",
        2,
        "Posix",
    ),
    (
        Service::PosixThreadGetSchedParam,
        "FIs3-UQT9sg",
        "Posix::pthread_getschedparam",
        3,
        "Posix",
    ),
    (
        Service::PosixThreadGetSchedParam,
        "P41kTWUS3EI",
        "Posix::pthread_getschedparam",
        3,
        "Posix",
    ),
    (
        Service::PosixThreadSetSchedParam,
        "Xs9hdiD7sAA",
        "Posix::pthread_setschedparam",
        3,
        "Posix",
    ),
    (
        Service::PosixThreadSetSchedParam,
        "oIRFTjoILbg",
        "Posix::pthread_setschedparam",
        3,
        "Posix",
    ),
];

pub(super) const PRIORITY_REGISTRATIONS: [Registration; 9] = {
    let base = Registration {
        service: Service::PthreadGetprio,
        nid: "1tKyG7RlMJo",
        source_name: "LibKernel::PthreadGetprio",
        arguments: 2,
        module: "libkernel",
        module_version: (1, 1),
        library: "libkernel",
        library_version: 1,
    };
    let mut entries = [base; 9];
    let mut index = 0;
    while index < EXPORTS.len() {
        let (service, nid, source_name, arguments, library) = EXPORTS[index];
        entries[index] = Registration {
            service,
            nid,
            source_name,
            arguments,
            library,
            ..base
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
    fn priority_appends_nine_source_rows_with_seven_stable_roles_and_gp_arities() {
        assert_eq!(REGISTRATIONS.len(), 1281);
        assert_eq!(
            Service::PthreadGetprio as usize,
            Service::PlayGoGetSupportedOptionalChunk as usize + 1
        );
        assert_eq!(
            Service::PosixThreadSetSchedParam as usize,
            Service::PthreadGetprio as usize + 6
        );
        for (index, (service, nid, name, arguments, library)) in EXPORTS.into_iter().enumerate() {
            let row = &REGISTRATIONS[1263 + index];
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
                ("libkernel", library, (1, 1), 1)
            );
            assert_eq!(resolve("libkernel", (1, 1), library, 1, nid), Some(service));
            assert_eq!(
                REGISTRATIONS
                    .iter()
                    .filter(|entry| entry.nid == nid)
                    .count(),
                1
            );
        }
        assert_eq!(REGISTRATIONS[1268].service, REGISTRATIONS[1269].service);
        assert_eq!(REGISTRATIONS[1270].service, REGISTRATIONS[1271].service);
    }

    #[test]
    fn priority_refuses_unregistered_posix_spellings_and_other_versions() {
        for (_, nid, _, _, library) in EXPORTS {
            let other = if library == "libkernel" {
                "Posix"
            } else {
                "libkernel"
            };
            assert_eq!(resolve("libkernel", (1, 1), other, 1, nid), None);
            assert_eq!(resolve("libkernel", (1, 1), "libScePosix", 1, nid), None);
            assert_eq!(resolve("libSceKernel", (1, 1), library, 1, nid), None);
            for (module_version, library_version) in [((1, 0), 1), ((2, 1), 1), ((1, 1), 2)] {
                assert_eq!(
                    resolve("libkernel", module_version, library, library_version, nid),
                    None
                );
            }
        }
    }
}
