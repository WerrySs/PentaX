// SPDX-License-Identifier: GPL-2.0-only
//! Original exact consumers of the finite demonstrated sleep identity.

use super::*;

#[test]
fn posix_usleep_binds_one_real_got_site_and_preserves_all_existing_marker_addresses() {
    let provider = factory();
    assert_eq!(provider.registration_count(), 1318);
    assert_eq!((MODULES.len(), LIBRARIES.len()), (68, 70));
    for (index, marker) in provider.markers()[..1315].iter().enumerate() {
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
    assert_eq!(
        (
            provider.objects()[0].rva,
            provider.objects()[1].rva,
            PROGRAM_NAME_BUFFER_RVA
        ),
        (0x10000, 0x10008, 0x10010)
    );
    let row = Service::registrations()[1315];
    assert_eq!(
        (row.service, row.nid, row.arguments),
        (Service::Usleep, "QcteRwbsnV0", 1)
    );
    let bytes = consumer(b"libkernel", b"libScePosix", &[row])
        .build()
        .unwrap();
    let plan = link(&bytes, &provider).unwrap();
    let imports: Vec<_> = plan
        .patches()
        .iter()
        .filter(|patch| patch.consumer().index() == 0)
        .collect();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].value(), PROVIDER_BIAS + 0x100 + 1315 * 8);
    assert!(
        matches!(imports[0].resolution(), Resolution::Definition(reference) if reference.module.index() == 1 && reference.symbol == 1316)
    );
}

#[test]
fn posix_usleep_refuses_unobserved_spelling_versions_object_and_other_sleep_nid() {
    let provider = factory();
    let row = Service::registrations()[1315];
    for (module, library) in [
        (b"libkernel".as_slice(), b"Posix".as_slice()),
        (b"libScePosix".as_slice(), b"libScePosix".as_slice()),
        (b"Posix".as_slice(), b"libScePosix".as_slice()),
    ] {
        let bytes = consumer(module, library, &[row]).build().unwrap();
        assert!(matches!(
            link(&bytes, &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    for (major, minor, version) in [(1, 0, 1), (2, 1, 1), (1, 1, 2)] {
        let mut original = consumer(b"libkernel", b"libScePosix", &[row]);
        (original.major, original.minor, original.version) = (major, minor, version);
        assert!(matches!(
            link(&original.build().unwrap(), &provider),
            Err(SymbolicError::VersionMismatch { .. })
        ));
    }
    let mut object = consumer(b"libkernel", b"libScePosix", &[row]);
    object.imports[0].symbol_type = 1;
    object.imports[0].relocation = 6;
    assert!(matches!(
        link(&object.build().unwrap(), &provider),
        Err(SymbolicError::UnsupportedSymbol {
            reason: UnsupportedSymbol::TypeMismatch,
            ..
        })
    ));
    let mut unobserved = consumer(b"libkernel", b"libScePosix", &[row]);
    unobserved.imports[0].nid = *b"1jfXLRVzisc";
    assert!(matches!(
        link(&unobserved.build().unwrap(), &provider),
        Err(SymbolicError::MissingProvider { .. })
    ));
}
