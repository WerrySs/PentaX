// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: finite appended aliases; see NOTICE.
// Public KytyPS5@f186b35706ddbc3faf5513f326e2335d224802f7,
// src/libs/libKernel.cpp blob c6fb3d76c9df124274d28f51da9b1fd3d9183b61.

use super::{Registration, Service};

// Keep the original libkernel-library rows and every established marker fixed.
// The POSIX wrapper delegates to the same selected-thread implementation.
pub(super) const POSIX_SELF_REGISTRATIONS: [Registration; 2] = {
    let canonical = Registration {
        service: Service::PthreadSelf,
        nid: "EotR8a3ASf4",
        source_name: "Posix::pthread_self",
        arguments: 0,
        module: "libkernel",
        module_version: (1, 1),
        library: "Posix",
        library_version: 1,
    };
    [
        canonical,
        Registration {
            library: "libScePosix",
            ..canonical
        },
    ]
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{REGISTRATIONS, resolve};

    #[test]
    fn posix_self_appends_only_two_exact_zero_argument_bindings() {
        assert_eq!(REGISTRATIONS.len(), 1317);
        assert_eq!(&REGISTRATIONS[1313..1315], &POSIX_SELF_REGISTRATIONS);
        for library in ["Posix", "libScePosix"] {
            let matching: Vec<_> = REGISTRATIONS
                .iter()
                .filter(|r| {
                    r.module == "libkernel" && r.library == library && r.nid == "EotR8a3ASf4"
                })
                .collect();
            assert_eq!(matching.len(), 1);
            assert_eq!(
                (matching[0].service, matching[0].arguments),
                (Service::PthreadSelf, 0)
            );
            assert_eq!(
                resolve("libkernel", (1, 1), library, 1, "EotR8a3ASf4"),
                Some(Service::PthreadSelf)
            );
            for (module, major, minor, version) in [
                ("Posix", 1, 1, 1),
                ("libScePosix", 1, 1, 1),
                ("libkernel", 1, 0, 1),
                ("libkernel", 2, 1, 1),
                ("libkernel", 1, 1, 2),
            ] {
                assert_eq!(
                    resolve(module, (major, minor), library, version, "EotR8a3ASf4"),
                    None
                );
            }
            assert_eq!(
                resolve("libkernel", (1, 1), library, 1, "aI+OeCz8xrQ"),
                None
            );
        }
        assert_eq!(
            resolve("libkernel", (1, 1), "libkernel", 1, "EotR8a3ASf4"),
            Some(Service::PthreadSelf)
        );
        assert_eq!(
            resolve("libkernel", (1, 1), "Posix", 1, "pthread_self"),
            None
        );
    }
}
