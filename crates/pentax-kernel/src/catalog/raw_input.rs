// SPDX-License-Identifier: GPL-2.0-only

//! Exact pinned Mouse/Keyboard FUNC identities; disconnected source profile.

use super::{Registration, Service};

const EXPORTS: [(Service, &str, &str, u8, bool); 10] = [
    (
        Service::MouseClose,
        "cAnT0Rw-IwU",
        "Mouse::MouseClose",
        1,
        false,
    ),
    (
        Service::MouseInit,
        "Qs0wWulgl7U",
        "Mouse::MouseInit",
        0,
        false,
    ),
    (
        Service::MouseOpen,
        "RaqxZIf6DvE",
        "Mouse::MouseOpen",
        4,
        false,
    ),
    (
        Service::MouseRead,
        "x8qnXqh-tiM",
        "Mouse::MouseRead",
        3,
        false,
    ),
    (
        Service::KeyboardInit,
        "wadT3QBCGY0",
        "Keyboard::KeyboardInit",
        0,
        true,
    ),
    (
        Service::KeyboardOpen,
        "HJ+KnEHcaxI",
        "Keyboard::KeyboardOpen",
        4,
        true,
    ),
    (
        Service::KeyboardClose,
        "0LWei+c7RNc",
        "Keyboard::KeyboardClose",
        1,
        true,
    ),
    (
        Service::KeyboardReadState,
        "6HpE68bzX6M",
        "Keyboard::KeyboardReadState",
        2,
        true,
    ),
    (
        Service::KeyboardRead,
        "xybbGMCr738",
        "Keyboard::KeyboardRead",
        3,
        true,
    ),
    (
        Service::KeyboardGetKey2Char,
        "yO9JwdRhtSA",
        "Keyboard::KeyboardGetKey2Char",
        6,
        true,
    ),
];

// Only Mouse Init/Open/Read have separately observed full-prefix imports.
pub(super) const RAW_INPUT_REGISTRATIONS: [Registration; 13] = {
    let base = Registration {
        service: Service::MouseClose,
        nid: "cAnT0Rw-IwU",
        source_name: "Mouse::MouseClose",
        arguments: 1,
        module: "Mouse",
        module_version: (1, 1),
        library: "Mouse",
        library_version: 1,
    };
    let mut entries = [base; 13];
    let mut index = 0;
    while index < EXPORTS.len() {
        let (service, nid, source_name, arguments, keyboard) = EXPORTS[index];
        let scope = if keyboard { "Keyboard" } else { "Mouse" };
        entries[index] = Registration {
            service,
            nid,
            source_name,
            arguments,
            module: scope,
            library: scope,
            ..base
        };
        index += 1;
    }
    entries[10] = Registration {
        module: "libSceMouse",
        library: "libSceMouse",
        ..entries[1]
    };
    entries[11] = Registration {
        module: "libSceMouse",
        library: "libSceMouse",
        ..entries[2]
    };
    entries[12] = Registration {
        module: "libSceMouse",
        library: "libSceMouse",
        ..entries[3]
    };
    entries
};

#[cfg(test)]
mod tests {
    use super::super::{REGISTRATIONS, resolve};
    use super::*;

    #[test]
    fn raw_input_exact_catalogue_appends_ten_roles_and_three_observed_mouse_aliases() {
        assert_eq!(REGISTRATIONS.len(), 1283);
        assert_eq!(
            Service::MouseClose as usize,
            Service::ImeKeyboardSetMode as usize + 1
        );
        for (index, (service, nid, name, arguments, keyboard)) in EXPORTS.into_iter().enumerate() {
            let row = &REGISTRATIONS[1198 + index];
            let scope = if keyboard { "Keyboard" } else { "Mouse" };
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
                (scope, scope, (1, 1), 1)
            );
            assert_eq!(resolve(scope, (1, 1), scope, 1, nid), Some(service));
            assert_eq!(
                REGISTRATIONS
                    .iter()
                    .filter(|row| row.service == service)
                    .count(),
                if matches!(index, 1..=3) { 2 } else { 1 }
            );
        }
        for (alias, canonical) in [(1208, 1199), (1209, 1200), (1210, 1201)] {
            let row = &REGISTRATIONS[alias];
            assert_eq!(
                (row.service, row.nid, row.arguments),
                (
                    REGISTRATIONS[canonical].service,
                    REGISTRATIONS[canonical].nid,
                    REGISTRATIONS[canonical].arguments
                )
            );
            assert_eq!((row.module, row.library), ("libSceMouse", "libSceMouse"));
            assert_eq!(
                resolve(row.module, (1, 1), row.library, 1, row.nid),
                Some(row.service)
            );
        }
    }

    #[test]
    fn raw_input_refuses_unobserved_prefixes_mixed_names_and_versions() {
        for (index, (_, nid, _, _, keyboard)) in EXPORTS.into_iter().enumerate() {
            let (scope, prefixed) = if keyboard {
                ("Keyboard", "libSceKeyboard")
            } else {
                ("Mouse", "libSceMouse")
            };
            if !matches!(index, 1..=3) {
                assert_eq!(resolve(prefixed, (1, 1), prefixed, 1, nid), None);
            }
            for (module, library) in [(scope, prefixed), (prefixed, scope)] {
                assert_eq!(resolve(module, (1, 1), library, 1, nid), None);
            }
            for (version, library_version) in [((2, 1), 1), ((1, 2), 1), ((1, 1), 2)] {
                assert_eq!(resolve(scope, version, scope, library_version, nid), None);
                assert_eq!(
                    resolve(prefixed, version, prefixed, library_version, nid),
                    None
                );
            }
        }
    }
}
