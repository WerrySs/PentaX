// SPDX-License-Identifier: GPL-2.0-only
//! Original qualified POSIX self consumers; no additional thread semantics.

use super::*;

#[test]
fn posix_self_exact_got_bindings_append_without_moving_existing_marker_sites() {
    let provider = factory();
    assert_eq!(provider.registration_count(), 1315);
    assert_eq!((MODULES.len(), LIBRARIES.len()), (68, 69));
    for (index, marker) in provider.markers()[..1313].iter().enumerate() {
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
    for (index, library) in [
        (1313, b"Posix".as_slice()),
        (1314, b"libScePosix".as_slice()),
    ] {
        let row = Service::registrations()[index];
        assert_eq!(
            (row.service, row.nid, row.arguments),
            (Service::PthreadSelf, "EotR8a3ASf4", 0)
        );
        let original = consumer(b"libkernel", library, &[row]).build().unwrap();
        let linked = link(&original, &provider).unwrap();
        let imports: Vec<_> = linked
            .patches()
            .iter()
            .filter(|p| p.consumer().index() == 0)
            .collect();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].value(), PROVIDER_BIAS + 0x100 + index as u64 * 8);
        assert!(
            matches!(imports[0].resolution(), Resolution::Definition(reference)
            if reference.module.index() == 1 && reference.symbol == index + 1)
        );
        assert_eq!(provider.markers()[index].registration, &row);
    }
}

#[test]
fn posix_self_does_not_generalize_module_versions_symbol_type_or_other_nids() {
    let provider = factory();
    for row in &Service::registrations()[1313..1315] {
        let library = if row.library == "Posix" {
            b"Posix".as_slice()
        } else {
            b"libScePosix".as_slice()
        };
        for module in [
            b"Posix".as_slice(),
            b"libScePosix".as_slice(),
            b"libSceKernel".as_slice(),
        ] {
            let original = consumer(module, library, &[*row]).build().unwrap();
            assert!(matches!(
                link(&original, &provider),
                Err(SymbolicError::MissingProvider { .. })
            ));
        }
        for (major, minor, version) in [(1, 0, 1), (2, 1, 1), (1, 1, 2)] {
            let mut original = consumer(b"libkernel", library, &[*row]);
            (original.major, original.minor, original.version) = (major, minor, version);
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::VersionMismatch { .. })
            ));
        }
        let mut object = consumer(b"libkernel", library, &[*row]);
        object.imports[0].symbol_type = 1;
        object.imports[0].relocation = 6;
        assert!(matches!(
            link(&object.build().unwrap(), &provider),
            Err(SymbolicError::UnsupportedSymbol {
                reason: UnsupportedSymbol::TypeMismatch,
                ..
            })
        ));
        let mut unrelated = consumer(b"libkernel", library, &[*row]);
        unrelated.imports[0].nid = *b"aI+OeCz8xrQ";
        assert!(matches!(
            link(&unrelated.build().unwrap(), &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
}
