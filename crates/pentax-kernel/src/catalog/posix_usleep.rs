// SPDX-License-Identifier: GPL-2.0-only
// Public KernelUsleep ABI: KytyPS5@f186b35706ddbc3faf5513f326e2335d224802f7,
// src/kernel/pthread.cpp blob bdb125ab7e77969821928985da0dfaff4c929070.
// The finite observed library spelling below is a PentaX compatibility binding,
// not an assertion that the source registers a POSIX usleep wrapper. See NOTICE.

use super::{Registration, Service};

pub(super) const POSIX_USLEEP_REGISTRATIONS: [Registration; 1] = [Registration {
    service: Service::Usleep,
    nid: "QcteRwbsnV0",
    source_name: "LibKernel::KernelUsleep",
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
    fn usleep_appends_only_the_demonstrated_posix_library_identity() {
        assert_eq!(REGISTRATIONS.len(), 1369);
        assert_eq!(&REGISTRATIONS[1315..1316], &POSIX_USLEEP_REGISTRATIONS);
        assert_eq!(
            resolve("libkernel", (1, 1), "libScePosix", 1, "QcteRwbsnV0"),
            Some(Service::Usleep)
        );
        for (module, library, major, minor, version, nid) in [
            ("Posix", "libScePosix", 1, 1, 1, "QcteRwbsnV0"),
            ("libScePosix", "libScePosix", 1, 1, 1, "QcteRwbsnV0"),
            ("libkernel", "Posix", 1, 1, 1, "QcteRwbsnV0"),
            ("libkernel", "libScePosix", 1, 0, 1, "QcteRwbsnV0"),
            ("libkernel", "libScePosix", 2, 1, 1, "QcteRwbsnV0"),
            ("libkernel", "libScePosix", 1, 1, 2, "QcteRwbsnV0"),
            ("libkernel", "libScePosix", 1, 1, 1, "1jfXLRVzisc"),
            ("libkernel", "libScePosix", 1, 1, 1, "usleep"),
        ] {
            assert_eq!(resolve(module, (major, minor), library, version, nid), None);
        }
        for nid in ["QcteRwbsnV0", "1jfXLRVzisc"] {
            assert_eq!(
                resolve("libkernel", (1, 1), "libkernel", 1, nid),
                Some(Service::Usleep)
            );
        }
    }
}
