// SPDX-License-Identifier: GPL-2.0-only
//! Original qualified consumers of finite SystemService registrations.

use super::*;

#[test]
fn system_notice_exact_bindings_preserve_old_sites_objects_and_provider_caps() {
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
        (MAX_REGISTRATIONS, CODE_MAPPED_BYTES),
        (2048, 4 * PAGE_SIZE)
    );
    assert_eq!(
        (provider.code_bytes().len(), provider.image().len()),
        (0xfe68, 0xfe78)
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
            &provider.image()[marker.rva as usize..marker.rva as usize + 8],
            &marker.bytes()
        );
    }
    assert_eq!(
        (
            provider.markers()[1271].service_id,
            provider.markers()[1271].rva
        ),
        (0x5058_4ff8, 0x28b8)
    );
    assert_eq!(
        (
            provider.markers()[1272].service_id,
            provider.markers()[1272].rva
        ),
        (0x5058_4ff9, 0x28c0)
    );
    assert_eq!(
        (
            provider.markers()[1280].service_id,
            provider.markers()[1280].rva
        ),
        (0x5058_5001, 0x2900)
    );
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
    for (scope, start, end) in [
        (b"SystemService".as_slice(), 1272, 1279),
        (b"libSceSystemService".as_slice(), 1279, 1281),
    ] {
        let rows = &Service::registrations()[start..end];
        let bytes = consumer(scope, scope, rows).build().unwrap();
        let plan = link(&bytes, &provider).unwrap();
        assert_eq!(plan.patch_count(), rows.len() + 1);
        let imports = plan
            .patches()
            .iter()
            .filter(|patch| patch.consumer().index() == 0);
        assert_eq!(imports.clone().count(), rows.len());
        for (offset, patch) in imports.enumerate() {
            let index = start + offset;
            assert_eq!(patch.value(), PROVIDER_BIAS + 0x100 + index as u64 * 8);
            assert!(
                matches!(patch.resolution(), Resolution::Definition(reference) if reference.module.index() == 1 && reference.symbol == index + 1)
            );
        }
    }
    assert!(matches!(
        build_kernel_provider(provider.image().len() as u64 - 1),
        Err(KernelProviderError::Budget)
    ));
}

#[test]
fn system_notice_refuses_unobserved_prefixes_mixed_axes_versions_and_nonfunction_imports() {
    let provider = factory();
    for (index, row) in Service::registrations()[1272..1279].iter().enumerate() {
        if !matches!(index, 0 | 2) {
            let bytes = consumer(b"libSceSystemService", b"libSceSystemService", &[*row])
                .build()
                .unwrap();
            assert!(matches!(
                link(&bytes, &provider),
                Err(SymbolicError::MissingProvider { .. })
            ));
        }
        for (module, library) in [
            (
                b"SystemService".as_slice(),
                b"libSceSystemService".as_slice(),
            ),
            (
                b"libSceSystemService".as_slice(),
                b"SystemService".as_slice(),
            ),
        ] {
            let bytes = consumer(module, library, &[*row]).build().unwrap();
            assert!(matches!(
                link(&bytes, &provider),
                Err(SymbolicError::MissingProvider { .. })
            ));
        }
        for (major, minor, version) in [(1, 0, 1), (2, 1, 1), (1, 1, 2)] {
            let mut original = consumer(b"SystemService", b"SystemService", &[*row]);
            (original.major, original.minor, original.version) = (major, minor, version);
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::VersionMismatch { .. })
            ));
        }
        let mut original = consumer(b"SystemService", b"SystemService", &[*row]);
        original.imports[0].symbol_type = 1;
        original.imports[0].relocation = 6;
        assert!(matches!(
            link(&original.build().unwrap(), &provider),
            Err(SymbolicError::UnsupportedSymbol {
                reason: UnsupportedSymbol::TypeMismatch,
                ..
            })
        ));
    }
}
