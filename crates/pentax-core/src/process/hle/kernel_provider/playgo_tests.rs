// SPDX-License-Identifier: GPL-2.0-only
//! Original qualified consumers; no package or title input.

use super::*;

#[test]
fn playgo_all_exact_bindings_fit_caps_and_preserve_prior_markers_and_object_rvas() {
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
        &MODULES[62..],
        &[("PlayGo", (1, 0)), ("libScePlayGo", (1, 0))]
    );
    assert_eq!(&LIBRARIES[63..], &[("PlayGo", 1), ("libScePlayGo", 1)]);
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
            provider.markers()[1237].service_id,
            provider.markers()[1237].rva
        ),
        (0x5058_4fd6, 0x27a8)
    );
    assert_eq!(
        (
            provider.markers()[1238].service_id,
            provider.markers()[1238].rva
        ),
        (0x5058_4fd7, 0x27b0)
    );
    assert_eq!(
        (
            provider.markers()[1262].service_id,
            provider.markers()[1262].rva
        ),
        (0x5058_4fef, 0x2870)
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
    let view = open_executable(provider.image()).unwrap();
    let inspection = inspect_sce(&view, &Default::default(), &Default::default()).unwrap();
    let metadata = inspection.metadata().unwrap();
    for (scope, start, end, module_id, library_id) in [
        (b"PlayGo".as_slice(), 1238, 1256, 63, 64),
        (b"libScePlayGo".as_slice(), 1256, 1263, 64, 65),
    ] {
        for (chunk, rows) in Service::registrations()[start..end].chunks(8).enumerate() {
            let mut original = consumer(scope, scope, rows);
            original.minor = 0;
            let bytes = original.build().unwrap();
            let plan = link(&bytes, &provider).unwrap();
            assert_eq!(plan.patch_count(), rows.len() + 1);
            let imports = plan
                .patches()
                .iter()
                .filter(|patch| patch.consumer().index() == 0);
            assert_eq!(imports.clone().count(), rows.len());
            for (offset, patch) in imports.enumerate() {
                let index = start + chunk * 8 + offset;
                assert_eq!(patch.value(), PROVIDER_BIAS + 0x100 + index as u64 * 8);
                assert!(
                    matches!(patch.resolution(), Resolution::Definition(reference) if reference.module.index() == 1 && reference.symbol == index + 1)
                );
                let symbol = &metadata.link().symbols()[index + 1];
                assert_eq!(
                    (symbol.symbol_type(), symbol.binding(), symbol.visibility()),
                    (
                        SymbolType::FUNC,
                        SymbolBinding::GLOBAL,
                        SymbolVisibility::PROTECTED
                    )
                );
                let name = symbol.name.as_ref().unwrap();
                let parsed = pentax_linker::QualifiedNid::parse(name.as_bytes()).unwrap();
                assert_eq!(
                    (parsed.module().value(), parsed.library().value()),
                    (module_id, library_id)
                );
            }
        }
    }
    assert!(matches!(
        build_kernel_provider(provider.image().len() as u64 - 1),
        Err(KernelProviderError::Budget)
    ));
}

#[test]
fn playgo_rejects_unobserved_prefixes_mixed_scopes_versions_and_object_imports() {
    let provider = factory();
    for (index, row) in Service::registrations()[1238..1256].iter().enumerate() {
        if ![0, 1, 2, 3, 6, 7, 8].contains(&index) {
            let mut original = consumer(b"libScePlayGo", b"libScePlayGo", &[*row]);
            original.minor = 0;
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::MissingProvider { .. })
            ));
        }
        for (module, library) in [
            (b"PlayGo".as_slice(), b"libScePlayGo".as_slice()),
            (b"libScePlayGo", b"PlayGo"),
        ] {
            let mut original = consumer(module, library, &[*row]);
            original.minor = 0;
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::MissingProvider { .. })
            ));
        }
        for (major, minor, version) in [(1, 1, 1), (2, 0, 1), (1, 0, 2)] {
            let mut original = consumer(b"PlayGo", b"PlayGo", &[*row]);
            (original.major, original.minor, original.version) = (major, minor, version);
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::VersionMismatch { .. })
            ));
        }
        let mut original = consumer(b"PlayGo", b"PlayGo", &[*row]);
        original.minor = 0;
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
