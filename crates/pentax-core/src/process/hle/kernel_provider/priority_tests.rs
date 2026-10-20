// SPDX-License-Identifier: GPL-2.0-only
//! Original exact priority consumers; no application input or scheduling claim.

use super::*;

#[test]
fn priority_exact_bindings_append_without_moving_previous_sites_or_objects() {
    let provider = factory();
    assert_eq!(
        (
            provider.registration_count(),
            MODULES.len(),
            LIBRARIES.len()
        ),
        (1281, 64, 65)
    );
    assert_eq!(
        (MAX_REGISTRATIONS, CODE_MAPPED_BYTES),
        (2048, 4 * PAGE_SIZE)
    );
    assert_eq!(
        (provider.code_bytes().len(), provider.image().len()),
        (0xfdb8, 0xfdc8)
    );
    for index in 1262..1272 {
        let marker = provider.markers()[index];
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
            provider.markers()[1262].service_id,
            provider.markers()[1262].rva
        ),
        (0x5058_4fef, 0x2870)
    );
    assert_eq!(
        (
            provider.markers()[1263].service_id,
            provider.markers()[1263].rva
        ),
        (0x5058_4ff0, 0x2878)
    );
    assert_eq!(
        (
            provider.markers()[1271].service_id,
            provider.markers()[1271].rva
        ),
        (0x5058_4ff8, 0x28b8)
    );
    assert_eq!(
        (provider.objects()[0].symbol, provider.objects()[0].rva),
        (1282, 0x10000)
    );
    assert_eq!(
        (provider.objects()[1].symbol, provider.objects()[1].rva),
        (1283, 0x10008)
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
    for (scope, start, end) in [
        (b"libkernel".as_slice(), 1263, 1267),
        (b"Posix".as_slice(), 1267, 1272),
    ] {
        let rows = &Service::registrations()[start..end];
        let bytes = consumer(b"libkernel", scope, rows).build().unwrap();
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
            let symbol = &metadata.link().symbols()[index + 1];
            assert_eq!(
                (symbol.symbol_type(), symbol.binding(), symbol.visibility()),
                (
                    SymbolType::FUNC,
                    SymbolBinding::GLOBAL,
                    SymbolVisibility::PROTECTED
                )
            );
        }
    }
    assert!(matches!(
        build_kernel_provider(provider.image().len() as u64 - 1),
        Err(KernelProviderError::Budget)
    ));
}

#[test]
fn priority_refuses_extra_posix_spellings_wrong_namespace_versions_and_object_types() {
    let provider = factory();
    for row in &Service::registrations()[1263..1272] {
        let other = if row.library == "libkernel" {
            b"Posix".as_slice()
        } else {
            b"libkernel".as_slice()
        };
        for (module, library) in [
            (b"libkernel".as_slice(), other),
            (b"libkernel".as_slice(), b"libScePosix".as_slice()),
            (b"libSceKernel".as_slice(), row.library.as_bytes()),
        ] {
            let bytes = consumer(module, library, &[*row]).build().unwrap();
            assert!(matches!(
                link(&bytes, &provider),
                Err(SymbolicError::MissingProvider { .. })
            ));
        }
        for (major, minor, version) in [(1, 0, 1), (2, 1, 1), (1, 1, 2)] {
            let mut original = consumer(b"libkernel", row.library.as_bytes(), &[*row]);
            (original.major, original.minor, original.version) = (major, minor, version);
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::VersionMismatch { .. })
            ));
        }
        let mut original = consumer(b"libkernel", row.library.as_bytes(), &[*row]);
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
