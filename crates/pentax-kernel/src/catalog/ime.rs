// SPDX-License-Identifier: GPL-2.0-only

//! Finite pinned IME FUNC identities; all arguments use the GP register bank.

use super::{Registration, Service};

const EXPORTS: [(Service, &str, &str, u8); 13] = [
    (Service::ImeSetText, "ieCNrVrzKd4", "Ime::ImeSetText", 2),
    (Service::ImeSetCaret, "WLxUN2WMim8", "Ime::ImeSetCaret", 1),
    (Service::ImeParamInit, "WmYDzdC4EHI", "Ime::ImeParamInit", 1),
    (
        Service::ImeGetPanelSize,
        "ziPDcIjO0Vk",
        "Ime::ImeGetPanelSize",
        3,
    ),
    (Service::ImeOpen, "RPydv-Jr1bc", "Ime::ImeOpen", 2),
    (Service::ImeUpdate, "-4GCfYdNF1s", "Ime::ImeUpdate", 1),
    (Service::ImeClose, "TmVP8LzcFcY", "Ime::ImeClose", 0),
    (
        Service::ImeSetTextGeometry,
        "TXYHFRuL8UY",
        "Ime::ImeSetTextGeometry",
        2,
    ),
    (
        Service::ImeKeyboardOpen,
        "eaFXjfJv3xs",
        "Ime::ImeKeyboardOpen",
        2,
    ),
    (
        Service::ImeKeyboardClose,
        "PMVehSlfZ94",
        "Ime::ImeKeyboardClose",
        1,
    ),
    (
        Service::ImeKeyboardGetResourceId,
        "dKadqZFgKKQ",
        "Ime::ImeKeyboardGetResourceId",
        2,
    ),
    (
        Service::ImeKeyboardGetInfo,
        "VkqLPArfFdc",
        "Ime::ImeKeyboardGetInfo",
        2,
    ),
    (
        Service::ImeKeyboardSetMode,
        "ua+13Hk9kKs",
        "Ime::ImeKeyboardSetMode",
        2,
    ),
];

// Only Update and KeyboardOpen have independently observed full-prefix imports.
pub(super) const IME_REGISTRATIONS: [Registration; 15] = {
    let base = Registration {
        service: Service::ImeSetText,
        nid: "ieCNrVrzKd4",
        source_name: "Ime::ImeSetText",
        arguments: 2,
        module: "Ime",
        module_version: (1, 1),
        library: "Ime",
        library_version: 1,
    };
    let mut entries = [base; 15];
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
    entries[13] = Registration {
        module: "libSceIme",
        library: "libSceIme",
        ..entries[5]
    };
    entries[14] = Registration {
        module: "libSceIme",
        library: "libSceIme",
        ..entries[8]
    };
    entries
};

#[cfg(test)]
mod tests {
    use super::super::{REGISTRATIONS, resolve};
    use super::*;

    #[test]
    fn ime_exact_catalogue_preserves_order_gp_arities_and_observed_aliases() {
        assert_eq!(
            Service::ImeSetText as usize,
            Service::FontSelectRendererFt as usize + 1
        );
        for (index, (service, nid, source_name, arguments)) in EXPORTS.into_iter().enumerate() {
            let row = &REGISTRATIONS[1183 + index];
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
                ("Ime", "Ime", (1, 1), 1)
            );
            assert_eq!(resolve("Ime", (1, 1), "Ime", 1, nid), Some(service));
            assert_eq!(
                REGISTRATIONS
                    .iter()
                    .filter(|row| row.service == service)
                    .count(),
                if matches!(index, 5 | 8) { 2 } else { 1 }
            );
        }
        for (alias, canonical) in [(1196, 1188), (1197, 1191)] {
            let row = &REGISTRATIONS[alias];
            assert_eq!(
                (row.service, row.nid, row.arguments),
                (
                    REGISTRATIONS[canonical].service,
                    REGISTRATIONS[canonical].nid,
                    REGISTRATIONS[canonical].arguments
                )
            );
            assert_eq!((row.module, row.library), ("libSceIme", "libSceIme"));
            assert_eq!(
                resolve(row.module, (1, 1), row.library, 1, row.nid),
                Some(row.service)
            );
        }
    }

    #[test]
    fn ime_refuses_unobserved_prefixes_mixed_names_and_versions() {
        for (index, (_, nid, _, _)) in EXPORTS.into_iter().enumerate() {
            if !matches!(index, 5 | 8) {
                assert_eq!(resolve("libSceIme", (1, 1), "libSceIme", 1, nid), None);
            }
            for (module, library) in [
                ("Ime", "libSceIme"),
                ("libSceIme", "Ime"),
                ("ImeKeyboard", "ImeKeyboard"),
            ] {
                assert_eq!(resolve(module, (1, 1), library, 1, nid), None);
            }
            for scope in ["Ime", "libSceIme"] {
                for (version, library_version) in [((2, 1), 1), ((1, 2), 1), ((1, 1), 2)] {
                    assert_eq!(resolve(scope, version, scope, library_version, nid), None);
                }
            }
        }
    }
}
