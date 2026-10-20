// SPDX-License-Identifier: GPL-2.0-only
// KytyPS5@f186b35706ddbc3faf5513f326e2335d224802f7 src/libs/libAudio.cpp,
// blob2069a82b636778b652d4d6f936a57c3e8dd040ed: canonical Acm1.1/library1.

use super::{Registration, Service};

pub(super) const ACM_REGISTRATIONS: [Registration; 2] = {
    let canonical = Registration {
        service: Service::AcmContextCreate,
        nid: "ZIXln2K3XMk",
        source_name: "Acm::AcmContextCreate",
        arguments: 1,
        module: "Acm",
        module_version: (1, 1),
        library: "Acm",
        library_version: 1,
    };
    [
        canonical,
        Registration {
            module: "libSceAcm",
            library: "libSceAcm",
            ..canonical
        },
    ]
};

#[cfg(test)]
mod tests {
    use super::super::{REGISTRATIONS, resolve};
    use super::*;

    #[test]
    fn acm_create_appends_only_two_exact_scopes_and_one_output_argument() {
        assert_eq!(
            Service::AcmContextCreate as usize,
            Service::SystemReportAbnormalTermination as usize + 1
        );
        assert_eq!(REGISTRATIONS.len(), 1283);
        assert_eq!(&REGISTRATIONS[1281..], &ACM_REGISTRATIONS);
        for (index, name) in ["Acm", "libSceAcm"].into_iter().enumerate() {
            let entry = &ACM_REGISTRATIONS[index];
            assert_eq!((entry.module, entry.library), (name, name));
            assert_eq!(
                (entry.module_version, entry.library_version, entry.arguments),
                ((1, 1), 1, 1)
            );
            assert_eq!(
                (entry.nid, entry.source_name),
                ("ZIXln2K3XMk", "Acm::AcmContextCreate")
            );
            assert_eq!(
                resolve(name, (1, 1), name, 1, "ZIXln2K3XMk"),
                Some(Service::AcmContextCreate)
            );
        }
        assert_eq!(
            REGISTRATIONS
                .iter()
                .filter(|entry| entry.service == Service::AcmContextCreate)
                .count(),
            2
        );
        for (module, library, major, minor, version) in [
            ("Acm", "libSceAcm", 1, 1, 1),
            ("libSceAcm", "Acm", 1, 1, 1),
            ("Acm", "Acm", 1, 0, 1),
            ("Acm", "Acm", 2, 1, 1),
            ("Acm", "Acm", 1, 1, 2),
            ("other", "Acm", 1, 1, 1),
        ] {
            assert_eq!(
                resolve(module, (major, minor), library, version, "ZIXln2K3XMk"),
                None
            );
        }
        for absent in ["jBgBjAj02R8", "tW9W+CAG4FE", "RLN3gRlXJLE"] {
            assert_eq!(resolve("Acm", (1, 1), "Acm", 1, absent), None);
            assert_eq!(resolve("libSceAcm", (1, 1), "libSceAcm", 1, absent), None);
        }
    }
}
