// SPDX-License-Identifier: GPL-2.0-only
//! Original exact qualified imports and append-only marker geometry.
use super::*;

#[test]
fn pad_runtime_binds_seven_new_markers_and_preserves_all_historical_sites() {
    let provider = factory();
    assert_eq!(
        (
            provider.registration_count(),
            MODULES.len(),
            LIBRARIES.len()
        ),
        (1399, 69, 71)
    );
    assert_eq!(provider.code_bytes().len(), 0x2cb8);
    assert_eq!(
        (provider.objects()[0].symbol, provider.objects()[0].rva),
        (1400, 0x10000)
    );
    assert_eq!(
        (provider.objects()[1].symbol, provider.objects()[1].rva),
        (1401, 0x10008)
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
        assert_eq!(
            &provider.code_bytes()[marker.rva as usize..marker.rva as usize + 8],
            &marker.bytes()
        );
    }
    let rows = &Service::registrations()[1318..1325];
    assert_eq!(rows.len(), 7);
    let bytes = consumer(b"libScePad", b"libScePad", rows).build().unwrap();
    let plan = link(&bytes, &provider).unwrap();
    let patches: Vec<_> = plan
        .patches()
        .iter()
        .filter(|patch| patch.consumer().index() == 0)
        .collect();
    assert_eq!(patches.len(), 7);
    for (offset, patch) in patches.iter().enumerate() {
        assert_eq!(
            patch.value(),
            PROVIDER_BIAS + 0x100 + (1318 + offset) as u64 * 8
        );
        assert!(
            matches!(patch.resolution(), Resolution::Definition(reference)
            if reference.module.index() == 1 && reference.symbol == 1319 + offset)
        );
    }
}

#[test]
fn pad_runtime_refuses_unqualified_namespace_version_object_and_neighbor_tokens() {
    let provider = factory();
    for row in &Service::registrations()[1318..1325] {
        for (module, library) in [
            (b"Pad".as_slice(), b"Pad".as_slice()),
            (b"Pad".as_slice(), b"libScePad".as_slice()),
            (b"libScePad".as_slice(), b"Pad".as_slice()),
        ] {
            assert!(matches!(
                link(
                    &consumer(module, library, &[*row]).build().unwrap(),
                    &provider
                ),
                Err(SymbolicError::MissingProvider { .. })
            ));
        }
        for (major, minor, version) in [(1, 0, 1), (2, 1, 1), (1, 1, 2)] {
            let mut original = consumer(b"libScePad", b"libScePad", &[*row]);
            (original.major, original.minor, original.version) = (major, minor, version);
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::VersionMismatch { .. })
            ));
        }
        let mut object = consumer(b"libScePad", b"libScePad", &[*row]);
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
    let mut neighbor = consumer(
        b"libScePad",
        b"libScePad",
        &[Service::registrations()[1318]],
    );
    neighbor.imports[0].nid = *b"vDLMoJLde8A";
    assert!(pentax_linker::NidToken::parse(&neighbor.imports[0].nid).is_ok());
    assert!(matches!(
        link(&neighbor.build().unwrap(), &provider),
        Err(SymbolicError::MissingProvider { .. })
    ));
}
