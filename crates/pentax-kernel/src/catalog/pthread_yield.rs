// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: one exact observed cooperative yield import.
// Source: KytyPS5@209dffe src/libs/libKernel.cpp and kernel/pthread.cpp;
// see NOTICE. The source signature is void PthreadYield(void).
use super::{Registration, Service};

pub(super) const PTHREAD_YIELD_REGISTRATIONS: [Registration; 1] = [Registration {
    service: Service::PthreadYield,
    nid: "T72hz6ffq08",
    source_name: "PthreadYield",
    arguments: 0,
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
    fn observed_yield_has_one_exact_zero_argument_binding_without_unobserved_aliases() {
        assert_eq!(
            REGISTRATIONS.get(1395),
            Some(&PTHREAD_YIELD_REGISTRATIONS[0])
        );
        assert_eq!(
            REGISTRATIONS
                .iter()
                .filter(|row| row.service == Service::PthreadYield)
                .count(),
            1
        );
        assert_eq!(
            resolve("libkernel", (1, 1), "libkernel", 1, "T72hz6ffq08"),
            Some(Service::PthreadYield)
        );
        for (module, version, library, library_version, nid) in [
            ("libkernel", (1, 1), "libkernel", 1, "6XG4B33N09g"),
            ("libkernel", (1, 1), "libScePosix", 1, "T72hz6ffq08"),
            ("libkernel", (1, 1), "Posix", 1, "T72hz6ffq08"),
            ("libSceKernel", (1, 1), "libkernel", 1, "T72hz6ffq08"),
            ("libkernel", (1, 0), "libkernel", 1, "T72hz6ffq08"),
            ("libkernel", (1, 1), "libkernel", 2, "T72hz6ffq08"),
        ] {
            assert_eq!(
                resolve(module, version, library, library_version, nid),
                None
            );
        }
    }
}
