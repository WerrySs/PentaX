// SPDX-License-Identifier: GPL-2.0-only
//! Original qualified imports; no proprietary input or namespace fallback.
use super::*;
#[test]
fn np_callback_provider_appends_three_exact_rows_without_moving_old_markers() {
    let provider = factory();
    assert_eq!(provider.registration_count(), 1399);
    for (index, marker) in provider.markers()[..1396].iter().enumerate() {
        assert_eq!(
            (marker.symbol, marker.service_id, marker.rva),
            (
                index + 1,
                0x5058_4b01 + index as u32,
                0x100 + index as u64 * 8
            )
        );
    }
    for (offset, (service, module, library, nid, args)) in [
        (
            Service::NpCheckCallback,
            "NpManager",
            "NpManager",
            "3Zl8BePTh9Y",
            0,
        ),
        (
            Service::NpCheckCallback,
            "libSceNpManager",
            "libSceNpManager",
            "3Zl8BePTh9Y",
            0,
        ),
        (
            Service::NpRegisterStateCallbackA,
            "NpManager",
            "NpManager",
            "qQJfO8HAiaY",
            2,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let row = Service::registrations()[1396 + offset];
        assert_eq!(
            (row.service, row.module, row.library, row.nid, row.arguments),
            (service, module, library, nid, args)
        );
        let bytes = consumer(row.module.as_bytes(), row.library.as_bytes(), &[row])
            .build()
            .unwrap();
        let plan = link(&bytes, &provider).unwrap();
        assert_eq!(
            plan.patches()
                .iter()
                .filter(|p| p.consumer().index() == 0)
                .count(),
            1
        );
    }
}
#[test]
fn np_callback_provider_refuses_unobserved_cross_scopes_versions_and_unregister_stub_identity() {
    let provider = factory();
    let check = Service::registrations()[1396];
    let register = Service::registrations()[1398];
    for (module, library, row) in [
        (
            b"NpManager".as_slice(),
            b"libSceNpManager".as_slice(),
            check,
        ),
        (b"libSceNpManager", b"NpManager", check),
        (b"libSceNpManager", b"libSceNpManager", register),
    ] {
        assert!(matches!(
            link(
                &consumer(module, library, &[row]).build().unwrap(),
                &provider
            ),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    for (major, minor, version) in [(1, 0, 1), (2, 1, 1), (1, 1, 2)] {
        let mut c = consumer(b"NpManager", b"NpManager", &[check]);
        (c.major, c.minor, c.version) = (major, minor, version);
        assert!(matches!(
            link(&c.build().unwrap(), &provider),
            Err(SymbolicError::VersionMismatch { .. })
        ));
    }
    let mut unknown = consumer(b"NpManager", b"NpManager", &[check]);
    unknown.imports[0].nid = *b"M3wFXbYQtAA";
    assert!(matches!(
        link(&unknown.build().unwrap(), &provider),
        Err(SymbolicError::MissingProvider { .. })
    ));
    let mut object = consumer(b"NpManager", b"NpManager", &[check]);
    object.imports[0].symbol_type = 1;
    object.imports[0].relocation = 6;
    assert!(matches!(
        link(&object.build().unwrap(), &provider),
        Err(SymbolicError::UnsupportedSymbol {
            reason: UnsupportedSymbol::TypeMismatch,
            ..
        })
    ));
}
