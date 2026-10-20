// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use crate::process::hle::fixture::{CODE, CONSUMER_BIAS, Consumer, Import, PROVIDER_BIAS};
use pentax_linker::{
    DescriptorEncoding, ModuleInput, Resolution, SymbolicError, UnsupportedSymbol, plan_symbolic,
};
use pentax_loader::open_executable;

fn consumer(module: &'static [u8], library: &'static [u8], rows: &[Registration]) -> Consumer {
    let mut original = Consumer::importing(vec![(CODE, vec![0xcc])]);
    original.module_name = module;
    original.library_name = library;
    original.imports = rows
        .iter()
        .map(|row| Import {
            nid: row.nid.as_bytes().try_into().unwrap(),
            symbol_type: 2,
            binding: 1,
            relocation: 7,
            addend: 0,
        })
        .collect();
    original
}

fn link<'a>(
    bytes: &'a [u8],
    provider: &'a GeneratedKernelProvider,
) -> Result<pentax_linker::SymbolicLinkPlan<'a>, SymbolicError> {
    plan_symbolic(
        &[
            ModuleInput::new(open_executable(bytes).unwrap(), CONSUMER_BIAS),
            ModuleInput::new(open_executable(provider.image()).unwrap(), PROVIDER_BIAS),
        ],
        &Default::default(),
    )
}

#[test]
fn trophy2_all_twenty_qualified_bindings_preserve_prior_sites_and_data_geometry() {
    let provider = build_kernel_provider(CODE_MAPPED_BYTES).unwrap();
    assert_eq!(provider.registration_count(), 1283);
    assert_eq!((MODULES.len(), LIBRARIES.len()), (66, 67));
    assert_eq!(
        &MODULES[41..43],
        &[("NpTrophy2", (1, 1)), ("libSceNpTrophy2", (1, 1))]
    );
    assert_eq!(
        &LIBRARIES[42..44],
        &[("NpTrophy2", 1), ("libSceNpTrophy2", 1)]
    );
    for (index, marker) in provider.markers().iter().enumerate() {
        assert_eq!(marker.symbol, index + 1);
        assert_eq!(marker.rva, 0x100 + index as u64 * 8);
        assert_eq!(marker.service_id, 0x5058_4b01 + index as u32);
        assert_eq!(
            &provider.image()[marker.rva as usize..marker.rva as usize + 8],
            &marker.bytes()
        );
    }
    assert_eq!(
        (
            provider.markers()[969].service_id,
            provider.markers()[969].rva
        ),
        (0x5058_4eca, 0x1f48)
    );
    assert_eq!(
        (
            provider.markers()[988].service_id,
            provider.markers()[988].rva
        ),
        (0x5058_4edd, 0x1fe0)
    );
    assert_eq!(provider.markers()[974].registration.arguments, 7);
    assert_eq!(provider.markers()[976].registration.arguments, 7);
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
        (provider.code_bytes().len(), provider.image().len()),
        (0xfe68, 0xfe78)
    );
    assert!(provider.image().len() as u64 <= CODE_MAPPED_BYTES);
    for (namespace, start, end) in [
        (b"NpTrophy2".as_slice(), 969, 984),
        (b"libSceNpTrophy2".as_slice(), 984, 989),
    ] {
        for (group_index, rows) in Service::registrations()[start..end].chunks(9).enumerate() {
            for dynlib in [false, true] {
                for encoding in [
                    DescriptorEncoding::Classic,
                    DescriptorEncoding::Variant1Observed,
                ] {
                    let mut original = consumer(namespace, namespace, rows);
                    original.dynlib = dynlib;
                    original.encoding = encoding;
                    let bytes = original.build().unwrap();
                    let plan = link(&bytes, &provider).unwrap();
                    assert_eq!(plan.patch_count(), rows.len() + 1);
                    for (index, patch) in plan.patches().iter().take(rows.len()).enumerate() {
                        let marker_index = start + group_index * 9 + index;
                        assert_eq!(
                            patch.value(),
                            PROVIDER_BIAS + 0x100 + marker_index as u64 * 8
                        );
                        assert!(
                            matches!(patch.resolution(), Resolution::Definition(reference)
                            if reference.module.index() == 1 && reference.symbol == marker_index + 1)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn trophy2_bindings_refuse_unobserved_or_mixed_scopes_versions_and_object_types() {
    let provider = build_kernel_provider(CODE_MAPPED_BYTES).unwrap();
    let create = &Service::registrations()[969..970];
    for (module, library) in [
        (b"NpTrophy2".as_slice(), b"libSceNpTrophy2".as_slice()),
        (b"libSceNpTrophy2".as_slice(), b"NpTrophy2".as_slice()),
        (b"NpTrophy".as_slice(), b"NpTrophy".as_slice()),
    ] {
        assert!(matches!(
            link(
                &consumer(module, library, create).build().unwrap(),
                &provider
            ),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    for role in 3..13 {
        let rows = &Service::registrations()[969 + role..970 + role];
        assert!(matches!(
            link(
                &consumer(b"libSceNpTrophy2", b"libSceNpTrophy2", rows)
                    .build()
                    .unwrap(),
                &provider
            ),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    for namespace in [b"NpTrophy2".as_slice(), b"libSceNpTrophy2".as_slice()] {
        for (major, minor, version) in [(2, 1, 1), (1, 2, 1), (1, 1, 2)] {
            let mut original = consumer(namespace, namespace, create);
            (original.major, original.minor, original.version) = (major, minor, version);
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::VersionMismatch { .. })
            ));
        }
        let mut original = consumer(namespace, namespace, create);
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
