// SPDX-License-Identifier: GPL-2.0-only
//! Original exact WS1 consumers; no NID-only or namespace fallback.
use super::*;

const MODULE: &[u8] = b"libSceUserService";
const LIBRARY: &[u8] = b"libSceUserServicePlatformPrivacyWs1";

#[test]
fn privacy_ws1_binds_one_two_argument_got_site_without_moving_previous_markers() {
    let provider = factory();
    assert_eq!(
        (
            provider.registration_count(),
            MODULES.len(),
            LIBRARIES.len()
        ),
        (1318, 68, 70)
    );
    assert_eq!(LIBRARIES[69], ("libSceUserServicePlatformPrivacyWs1", 1));
    for (index, marker) in provider.markers()[..1316].iter().enumerate() {
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
    let row = Service::registrations()[1316];
    assert_eq!(
        (row.service, row.nid, row.arguments),
        (Service::UserGetPlatformPrivacyWs1, "D-CzAxQL0XI", 2)
    );
    let bytes = consumer(MODULE, LIBRARY, &[row]).build().unwrap();
    let plan = link(&bytes, &provider).unwrap();
    let imports: Vec<_> = plan
        .patches()
        .iter()
        .filter(|patch| patch.consumer().index() == 0)
        .collect();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].value(), PROVIDER_BIAS + 0x100 + 1316 * 8);
    assert!(
        matches!(imports[0].resolution(), Resolution::Definition(reference)
        if reference.module.index() == 1 && reference.symbol == 1317)
    );
    assert!(matches!(
        build_kernel_provider(provider.image().len() as u64 - 1),
        Err(KernelProviderError::Budget)
    ));
}

#[test]
fn privacy_ws1_refuses_other_namespaces_versions_types_and_neighbor_nids() {
    let provider = factory();
    let row = Service::registrations()[1316];
    for (module, library) in [
        (b"UserService".as_slice(), b"UserService".as_slice()),
        (MODULE, b"libSceUserService".as_slice()),
        (b"UserService".as_slice(), LIBRARY),
        (MODULE, b"UserServicePlatformPrivacyWs1".as_slice()),
    ] {
        let bytes = consumer(module, library, &[row]).build().unwrap();
        assert!(matches!(
            link(&bytes, &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    for (major, minor, version) in [(1, 0, 1), (2, 1, 1), (1, 1, 2)] {
        let mut original = consumer(MODULE, LIBRARY, &[row]);
        (original.major, original.minor, original.version) = (major, minor, version);
        assert!(matches!(
            link(&original.build().unwrap(), &provider),
            Err(SymbolicError::VersionMismatch { .. })
        ));
    }
    let mut object = consumer(MODULE, LIBRARY, &[row]);
    object.imports[0].symbol_type = 1;
    object.imports[0].relocation = 6;
    assert!(matches!(
        link(&object.build().unwrap(), &provider),
        Err(SymbolicError::UnsupportedSymbol {
            reason: UnsupportedSymbol::TypeMismatch,
            ..
        })
    ));
    let mut neighbor = consumer(MODULE, LIBRARY, &[row]);
    neighbor.imports[0].nid = *b"Iem2arLgE+I";
    assert!(matches!(
        link(&neighbor.build().unwrap(), &provider),
        Err(SymbolicError::MissingProvider { .. })
    ));
}
