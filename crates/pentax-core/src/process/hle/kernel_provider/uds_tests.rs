// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use crate::process::hle::fixture::{CODE, CONSUMER_BIAS, Consumer, Import, PROVIDER_BIAS};
use pentax_linker::{
    DescriptorEncoding, ModuleInput, Resolution, SymbolicError, UnsupportedSymbol, plan_symbolic,
};
use pentax_loader::open_executable;

const OBSERVED: [usize; 20] = [
    0, 1, 2, 3, 4, 5, 7, 11, 12, 13, 15, 16, 17, 18, 19, 22, 25, 28, 37, 40,
];

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
fn uds_bindings_cross_index1024_with_full_ids_fixed_pages_and_stable_data() {
    let provider = build_kernel_provider(CODE_MAPPED_BYTES).unwrap();
    assert_eq!(MAX_REGISTRATIONS, 2048);
    assert_eq!(CODE_MAPPED_BYTES, 4 * PAGE_SIZE);
    assert_eq!(provider.registration_count(), 1283);
    assert_eq!((MODULES.len(), LIBRARIES.len()), (66, 67));
    assert_eq!(
        &MODULES[43..45],
        &[
            ("NpUniversalDataSystem", (1, 1)),
            ("libSceNpUniversalDataSystem", (1, 1))
        ]
    );
    assert_eq!(
        &LIBRARIES[44..46],
        &[
            ("NpUniversalDataSystem", 1),
            ("libSceNpUniversalDataSystem", 1)
        ]
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
    for (index, id, rva) in [
        (989, 0x5058_4ede, 0x1fe8),
        (1023, 0x5058_4f00, 0x20f8),
        (1024, 0x5058_4f01, 0x2100),
        (1049, 0x5058_4f1a, 0x21c8),
    ] {
        let marker = provider.markers()[index];
        assert_eq!((marker.service_id, marker.rva), (id, rva));
        assert_eq!(
            u32::from_le_bytes(
                provider.image()[rva as usize + 1..rva as usize + 5]
                    .try_into()
                    .unwrap()
            ),
            id
        );
    }
    assert_eq!(
        provider.markers()[1024].registration.service,
        Service::UdsEventPropertyArraySetBool
    );
    assert_eq!(
        (provider.code_bytes().len(), provider.image().len()),
        (0xfe68, 0xfe78)
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
    assert_eq!(provider.data_bytes().len(), 16);
    assert_eq!(
        &provider.data_bytes()[..8],
        &STACK_GUARD_INITIAL.to_le_bytes()
    );
    assert_eq!(&provider.data_bytes()[8..], &[0; 8]);
    let view = open_executable(provider.image()).unwrap();
    let runtime = view.runtime_plan().unwrap();
    assert_eq!(
        (
            runtime.segments[0].flags.raw(),
            runtime.segments[0].mem_size
        ),
        (5, 4 * PAGE_SIZE)
    );
    assert_eq!(
        (runtime.segments[1].flags.raw(), runtime.segments[1].vaddr),
        (6, 4 * PAGE_SIZE)
    );
    for (namespace, start, end) in [
        (b"NpUniversalDataSystem".as_slice(), 989, 1030),
        (b"libSceNpUniversalDataSystem".as_slice(), 1030, 1050),
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
    let size = provider.image().len() as u64;
    assert!(matches!(
        build_kernel_provider(size - 1),
        Err(KernelProviderError::Budget)
    ));
    assert_eq!(
        build_kernel_provider(size).unwrap().image().len() as u64,
        size
    );
}

#[test]
fn uds_bindings_refuse_unobserved_and_mixed_scopes_versions_and_object_types() {
    let provider = build_kernel_provider(CODE_MAPPED_BYTES).unwrap();
    let initialize = &Service::registrations()[989..990];
    for (module, library) in [
        (
            b"NpUniversalDataSystem".as_slice(),
            b"libSceNpUniversalDataSystem".as_slice(),
        ),
        (
            b"libSceNpUniversalDataSystem".as_slice(),
            b"NpUniversalDataSystem".as_slice(),
        ),
        (
            b"UniversalDataSystem".as_slice(),
            b"UniversalDataSystem".as_slice(),
        ),
    ] {
        assert!(matches!(
            link(
                &consumer(module, library, initialize).build().unwrap(),
                &provider
            ),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    for role in 0..41 {
        if !OBSERVED.contains(&role) {
            let row = &Service::registrations()[989 + role..990 + role];
            assert!(matches!(
                link(
                    &consumer(
                        b"libSceNpUniversalDataSystem",
                        b"libSceNpUniversalDataSystem",
                        row
                    )
                    .build()
                    .unwrap(),
                    &provider
                ),
                Err(SymbolicError::MissingProvider { .. })
            ));
        }
    }
    for namespace in [
        b"NpUniversalDataSystem".as_slice(),
        b"libSceNpUniversalDataSystem".as_slice(),
    ] {
        for (major, minor, version) in [(2, 1, 1), (1, 2, 1), (1, 1, 2)] {
            let mut original = consumer(namespace, namespace, initialize);
            (original.major, original.minor, original.version) = (major, minor, version);
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::VersionMismatch { .. })
            ));
        }
        let mut original = consumer(namespace, namespace, initialize);
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
