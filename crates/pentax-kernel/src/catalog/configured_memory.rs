// SPDX-License-Identifier: GPL-2.0-only
// Exact public KytyPS5 f186b35706ddbc3faf5513f326e2335d224802f7
// libKernel.cpp:2961 and memory.cpp:3507..3518; see NOTICE.
use super::{Registration, Service};

pub(super) const CONFIGURED_MEMORY_REGISTRATIONS: [Registration; 1] = [Registration {
    service: Service::ConfiguredFlexibleMemorySize,
    nid: "n1-v6FgU7MQ",
    source_name: "KernelConfiguredFlexibleMemorySize",
    arguments: 1,
    module: "libkernel",
    module_version: (1, 1),
    library: "libkernel",
    library_version: 1,
}];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{REGISTRATIONS, resolve};

    #[test]
    fn configured_memory_appends_one_exact_pointer_argument_without_namespace_aliases() {
        assert_eq!(REGISTRATIONS.len(), 1318);
        assert_eq!(&REGISTRATIONS[1317..], &CONFIGURED_MEMORY_REGISTRATIONS);
        assert_eq!(
            Service::ConfiguredFlexibleMemorySize as usize,
            Service::UserGetPlatformPrivacyWs1 as usize + 1
        );
        let row = CONFIGURED_MEMORY_REGISTRATIONS[0];
        assert_eq!(
            (row.arguments, row.source_name),
            (1, "KernelConfiguredFlexibleMemorySize")
        );
        assert_eq!(
            resolve("libkernel", (1, 1), "libkernel", 1, "n1-v6FgU7MQ"),
            Some(row.service)
        );
        for (module, library, major, minor, version, nid) in [
            ("libSceKernel", "libkernel", 1, 1, 1, row.nid),
            ("libkernel", "Posix", 1, 1, 1, row.nid),
            ("libkernel", "libkernel", 1, 0, 1, row.nid),
            ("libkernel", "libkernel", 2, 1, 1, row.nid),
            ("libkernel", "libkernel", 1, 1, 2, row.nid),
            ("libkernel", "libkernel", 1, 1, 1, "n1-v6FgU7MR"),
        ] {
            assert_eq!(resolve(module, (major, minor), library, version, nid), None);
        }
    }
}
