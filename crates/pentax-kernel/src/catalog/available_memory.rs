// SPDX-License-Identifier: GPL-2.0-only
// Exact KytyPS5-Performance 209dffe9032d72749257ce18680d506b891284de
// src/libs/libKernel.cpp:3241 and src/kernel/memory.cpp:4404..4424; see NOTICE.
use super::{Registration, Service};

pub(super) const AVAILABLE_MEMORY_REGISTRATIONS: [Registration; 1] = [Registration {
    service: Service::AvailableFlexibleMemorySize,
    nid: "aNz11fnnzi4",
    source_name: "KernelAvailableFlexibleMemorySize",
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
    fn available_memory_appends_only_the_observed_pointer_binding() {
        let row = AVAILABLE_MEMORY_REGISTRATIONS[0];
        assert_eq!(REGISTRATIONS.get(1391), Some(&row));
        assert_eq!(
            row.service as usize,
            Service::PosixPthreadDetach as usize + 1
        );
        assert_eq!(
            (row.arguments, row.source_name),
            (1, "KernelAvailableFlexibleMemorySize")
        );
        assert_eq!(
            resolve("libkernel", (1, 1), "libkernel", 1, row.nid),
            Some(row.service)
        );
        for (module, library, version, library_version, nid) in [
            ("libSceKernel", "libkernel", (1, 1), 1, row.nid),
            ("libkernel", "Posix", (1, 1), 1, row.nid),
            ("libkernel", "libkernel", (1, 0), 1, row.nid),
            ("libkernel", "libkernel", (2, 1), 1, row.nid),
            ("libkernel", "libkernel", (1, 1), 2, row.nid),
            ("libkernel", "libkernel", (1, 1), 1, "aNz11fnnziA"),
        ] {
            assert_eq!(
                resolve(module, version, library, library_version, nid),
                None
            );
        }
    }
}
