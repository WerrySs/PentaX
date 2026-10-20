// SPDX-License-Identifier: GPL-2.0-only
//! Original qualified consumers of the creation-only ACM provider.

use super::*;

#[test]
fn acm_create_qualified_bindings_append_without_moving_old_sites_or_data() {
    let provider = factory();
    assert_eq!(
        (
            provider.registration_count(),
            MODULES.len(),
            LIBRARIES.len()
        ),
        (1283, 66, 67)
    );
    assert_eq!(
        (provider.code_bytes().len(), provider.image().len()),
        (0xfe68, 0xfe78)
    );
    assert_eq!((MAX_REGISTRATIONS, CODE_MAPPED_BYTES), (2048, 0x10000));
    assert_eq!(
        (provider.objects()[0].symbol, provider.objects()[0].rva),
        (1284, 0x10000)
    );
    assert_eq!(
        (provider.objects()[1].symbol, provider.objects()[1].rva),
        (1285, 0x10008)
    );
    assert_eq!(PROGRAM_NAME_BUFFER_RVA, 0x10010);
    assert_eq!(
        &provider.data_bytes()[..8],
        &STACK_GUARD_INITIAL.to_le_bytes()
    );
    assert_eq!(&provider.data_bytes()[8..], &[0; 8]);
    for (index, marker) in provider.markers()[..1281].iter().enumerate() {
        assert_eq!(
            (marker.symbol, marker.service_id, marker.rva),
            (
                index + 1,
                0x5058_4b01 + index as u32,
                0x100 + index as u64 * 8
            )
        );
        assert_eq!(
            &provider.image()[marker.rva as usize..marker.rva as usize + 8],
            &marker.bytes()
        );
    }
    for (index, scope) in [b"Acm".as_slice(), b"libSceAcm".as_slice()]
        .into_iter()
        .enumerate()
    {
        let row = Service::registrations()[1281 + index];
        assert_eq!(
            (row.nid, row.arguments, row.service),
            ("ZIXln2K3XMk", 1, Service::AcmContextCreate)
        );
        let bytes = consumer(scope, scope, &[row]).build().unwrap();
        let linked = link(&bytes, &provider).unwrap();
        assert_eq!(linked.patch_count(), 2); // Consumer plus provider's real name-object relocation.
        let patch = linked
            .patches()
            .iter()
            .find(|patch| patch.consumer().index() == 0)
            .unwrap();
        assert_eq!(patch.value(), PROVIDER_BIAS + 0x2908 + index as u64 * 8);
        assert!(
            matches!(patch.resolution(), Resolution::Definition(reference) if reference.module.index() == 1 && reference.symbol == 1282 + index)
        );
        let marker = &provider.markers()[1281 + index];
        assert_eq!(
            (marker.service_id, marker.rva),
            (0x5058_5002 + index as u32, 0x2908 + index as u64 * 8)
        );
    }
    assert!(matches!(
        build_kernel_provider(provider.image().len() as u64 - 1),
        Err(KernelProviderError::Budget)
    ));
}

#[test]
fn acm_create_refuses_mixed_scopes_versions_nonfunctions_and_unimplemented_roles() {
    let provider = factory();
    let row = Service::registrations()[1281];
    for (module, library) in [
        (b"Acm".as_slice(), b"libSceAcm".as_slice()),
        (b"libSceAcm".as_slice(), b"Acm".as_slice()),
        (b"other".as_slice(), b"Acm".as_slice()),
    ] {
        let bytes = consumer(module, library, &[row]).build().unwrap();
        assert!(matches!(
            link(&bytes, &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    for (major, minor, version) in [(1, 0, 1), (2, 1, 1), (1, 1, 2)] {
        let mut original = consumer(b"Acm", b"Acm", &[row]);
        (original.major, original.minor, original.version) = (major, minor, version);
        assert!(matches!(
            link(&original.build().unwrap(), &provider),
            Err(SymbolicError::VersionMismatch { .. })
        ));
    }
    let mut object = consumer(b"Acm", b"Acm", &[row]);
    object.imports[0].symbol_type = 1;
    object.imports[0].relocation = 6;
    assert!(matches!(
        link(&object.build().unwrap(), &provider),
        Err(SymbolicError::UnsupportedSymbol {
            reason: UnsupportedSymbol::TypeMismatch,
            ..
        })
    ));
    for nid in [b"jBgBjAj02R8", b"tW9W+CAG4FE", b"RLN3gRlXJLE"] {
        let mut original = consumer(b"Acm", b"Acm", &[row]);
        original.imports[0].nid = *nid;
        assert!(matches!(
            link(&original.build().unwrap(), &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
}
