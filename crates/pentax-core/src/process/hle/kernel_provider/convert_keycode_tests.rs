// SPDX-License-Identifier: GPL-2.0-only
//! Original qualified linking and append-only keyboard-type provider proofs.
use super::*;

#[test]
fn keyboard_type_binds_one_version_one_zero_marker_and_rejects_scope_type_and_version_fallback() {
    let provider = factory();
    let row = Service::registrations()[1325];
    assert_eq!(row.service, Service::ConvertKeycodeGetImeKeyboardType);
    assert_eq!(
        (
            provider.registration_count(),
            MODULES.len(),
            LIBRARIES.len()
        ),
        (1381, 69, 71)
    );
    assert_eq!(provider.code_bytes().len(), 0x2c28);
    assert_eq!(
        (provider.objects()[0].symbol, provider.objects()[0].rva),
        (1382, 0x10000)
    );
    for (index, marker) in provider.markers().iter().enumerate() {
        assert_eq!(
            (marker.symbol, marker.service_id, marker.rva),
            (
                index + 1,
                0x5058_4b01 + index as u32,
                0x100 + index as u64 * 8
            )
        );
    }
    let original = || {
        let mut value = consumer(b"libSceConvertKeycode", b"libSceConvertKeycode", &[row]);
        value.minor = 0;
        value
    };
    let bytes = original().build().unwrap();
    let plan = link(&bytes, &provider).unwrap();
    let patches: Vec<_> = plan
        .patches()
        .iter()
        .filter(|patch| patch.consumer().index() == 0)
        .collect();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].value(), PROVIDER_BIAS + 0x100 + 1325 * 8);
    for (module, library) in [
        (b"ConvertKeycode".as_slice(), b"ConvertKeycode".as_slice()),
        (
            b"libSceConvertKeycode".as_slice(),
            b"ConvertKeycode".as_slice(),
        ),
        (
            b"ConvertKeycode".as_slice(),
            b"libSceConvertKeycode".as_slice(),
        ),
    ] {
        let mut value = original();
        value.module_name = module;
        value.library_name = library;
        assert!(matches!(
            link(&value.build().unwrap(), &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    for (major, minor, version) in [(1, 1, 1), (2, 0, 1), (1, 0, 2)] {
        let mut value = original();
        (value.major, value.minor, value.version) = (major, minor, version);
        assert!(matches!(
            link(&value.build().unwrap(), &provider),
            Err(SymbolicError::VersionMismatch { .. })
        ));
    }
    let mut value = original();
    value.imports[0].symbol_type = 1;
    value.imports[0].relocation = 6;
    assert!(matches!(
        link(&value.build().unwrap(), &provider),
        Err(SymbolicError::UnsupportedSymbol {
            reason: UnsupportedSymbol::TypeMismatch,
            ..
        })
    ));
}
