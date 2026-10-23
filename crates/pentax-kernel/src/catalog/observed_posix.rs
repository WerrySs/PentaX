// SPDX-License-Identifier: GPL-2.0-only
// Public KytyPS5 f186b357 libKernel.cpp:1902,2170-2171,2230 and
// kernel/pthread.cpp:4294 bind these NIDs to the POSIX wrappers.
// Exact observed libkernel/library spelling is a PentaX compatibility binding;
// it does not turn the wrappers into kernel-error-domain services.
use super::{Registration, Service};

pub(super) const OBSERVED_POSIX_REGISTRATIONS: [Registration; 3] = [
    Registration {
        service: Service::PosixNanosleep,
        nid: "NhpspxdjEKU",
        source_name: "nanosleep",
        arguments: 2,
        module: "libkernel",
        module_version: (1, 1),
        library: "libkernel",
        library_version: 1,
    },
    Registration {
        service: Service::PosixNanosleep,
        nid: "yS8U2TGCe1A",
        source_name: "nanosleep",
        arguments: 2,
        module: "libkernel",
        module_version: (1, 1),
        library: "libkernel",
        library_version: 1,
    },
    Registration {
        service: Service::PosixCondWait,
        nid: "Op8TBGY5KHg",
        source_name: "pthread_cond_wait",
        arguments: 2,
        module: "libkernel",
        module_version: (1, 1),
        library: "libkernel",
        library_version: 1,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{REGISTRATIONS, resolve};
    #[test]
    fn observed_posix_aliases_append_exact_scopes_and_preserve_wrapper_domains() {
        assert_eq!(REGISTRATIONS.len(), 1402);
        assert_eq!(&REGISTRATIONS[1399..], &OBSERVED_POSIX_REGISTRATIONS);
        assert_eq!(
            REGISTRATIONS[1398].service,
            Service::NpRegisterStateCallbackA
        );
        for row in OBSERVED_POSIX_REGISTRATIONS {
            assert_eq!(row.arguments, 2);
            for library in ["libkernel", "Posix", "libScePosix"] {
                assert_eq!(
                    resolve("libkernel", (1, 1), library, 1, row.nid),
                    Some(row.service)
                );
            }
            for (module, mv, library, lv) in [
                ("libSceKernel", (1, 1), "libkernel", 1),
                ("libkernel", (1, 0), "libkernel", 1),
                ("libkernel", (2, 1), "libkernel", 1),
                ("libkernel", (1, 1), "libkernel", 2),
                ("libkernel", (1, 1), "other", 1),
            ] {
                assert_eq!(resolve(module, mv, library, lv, row.nid), None);
            }
            assert_eq!(
                REGISTRATIONS
                    .iter()
                    .filter(|r| r.nid == row.nid
                        && r.module == "libkernel"
                        && r.library == "libkernel")
                    .count(),
                1
            );
        }
        assert_ne!(Service::PosixNanosleep, Service::Nanosleep);
        assert_ne!(Service::PosixCondWait, Service::CondWait);
    }
}
