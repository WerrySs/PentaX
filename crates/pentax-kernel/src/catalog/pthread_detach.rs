// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: one exact appended POSIX detach import.
// Source: KytyPS5@209dffe src/libs/libKernel.cpp and kernel/pthread.cpp;
// see NOTICE. No alternate module, spelling or version is inferred.
use super::{Registration, Service};

pub(super) const PTHREAD_DETACH_REGISTRATIONS: [Registration; 1] = [Registration {
    service: Service::PosixPthreadDetach,
    nid: "+U1R4WtXvoc",
    source_name: "Posix::pthread_detach",
    arguments: 1,
    module: "libkernel",
    module_version: (1, 1),
    library: "libScePosix",
    library_version: 1,
}];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{REGISTRATIONS, resolve};

    #[test]
    fn observed_posix_detach_appends_one_exact_errno_domain_binding() {
        assert_eq!(
            REGISTRATIONS.get(1390),
            Some(&PTHREAD_DETACH_REGISTRATIONS[0])
        );
        assert_eq!(
            REGISTRATIONS
                .iter()
                .filter(|row| row.nid == "+U1R4WtXvoc")
                .count(),
            1
        );
        assert_eq!(
            resolve("libkernel", (1, 1), "libScePosix", 1, "+U1R4WtXvoc"),
            Some(Service::PosixPthreadDetach)
        );
        assert_eq!(
            resolve("libkernel", (1, 1), "libkernel", 1, "4qGrR6eoP9Y"),
            Some(Service::PthreadDetach)
        );
        for (module, library, module_version, library_version) in [
            ("libkernel", "Posix", (1, 1), 1),
            ("libkernel", "libkernel", (1, 1), 1),
            ("libSceKernel", "libScePosix", (1, 1), 1),
            ("libkernel", "libScePosix", (1, 0), 1),
            ("libkernel", "libScePosix", (1, 1), 2),
        ] {
            assert_eq!(
                resolve(
                    module,
                    module_version,
                    library,
                    library_version,
                    "+U1R4WtXvoc"
                ),
                None
            );
        }
    }
}
