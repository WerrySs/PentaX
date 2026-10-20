// SPDX-License-Identifier: GPL-2.0-only

//! Original generated consumers; metadata and relocation checks only.
use super::*;
use crate::process::hle::fixture::{CODE, CONSUMER_BIAS, Consumer, Import, PROVIDER_BIAS};
use pentax_linker::{
    DescriptorEncoding, ModuleInput, Resolution, SymbolBinding, SymbolType, SymbolVisibility,
    SymbolicError, UnsupportedSymbol, inspect_sce, plan_symbolic,
};
use pentax_loader::open_executable;

#[path = "playgo_tests.rs"]
mod playgo;

#[path = "priority_tests.rs"]
mod priority;

#[path = "system_notice_tests.rs"]
mod system_notice;

#[path = "acm_tests.rs"]
mod acm;

const OLD_REGISTRATIONS: usize = 862;
const SCOPES: [(&[u8], &[u8]); 4] = [
    (b"NpWebApi2", b"NpWebApi2"),
    (b"libSceNpWebApi2", b"NpWebApi2"),
    (b"NpWebApi2", b"libSceNpWebApi2"),
    (b"libSceNpWebApi2", b"libSceNpWebApi2"),
];

fn factory() -> GeneratedKernelProvider {
    build_kernel_provider(MAX_IMAGE_BYTES).unwrap()
}

#[test]
fn share_media_provider_exact_append_and_only_observed_prefix_bindings() {
    let provider = factory();
    assert_eq!(
        (MAX_REGISTRATIONS, CODE_MAPPED_BYTES),
        (2048, 4 * PAGE_SIZE)
    );
    assert_eq!(
        (provider.objects()[0].rva, provider.objects()[1].rva),
        (0x10000, 0x10008)
    );
    assert_eq!(provider.markers()[1212].service_id, 0x5058_4b01 + 1212);
    assert_eq!(
        provider.markers()[1212].registration.source_name,
        "Share::ShareFeaturePermit"
    );
    for (scope, start, end) in [
        (b"Share".as_slice(), 1220, 1234),
        (b"libSceShare".as_slice(), 1234, 1238),
    ] {
        for (chunk, rows) in Service::registrations()[start..end].chunks(8).enumerate() {
            let bytes = consumer(scope, scope, rows).build().unwrap();
            let plan = link(&bytes, &provider).unwrap();
            assert_eq!(plan.patch_count(), rows.len() + 1);
            for (offset, patch) in plan
                .patches()
                .iter()
                .filter(|p| p.consumer().index() == 0)
                .enumerate()
            {
                let index = start + chunk * 8 + offset;
                assert_eq!(patch.value(), PROVIDER_BIAS + 0x100 + index as u64 * 8);
                assert!(
                    matches!(patch.resolution(),Resolution::Definition(r) if r.module.index()==1 && r.symbol==index+1)
                );
            }
        }
    }
    for (index, row) in Service::registrations()[1220..1234].iter().enumerate() {
        if ![5, 6, 7, 10].contains(&index) {
            let bytes = consumer(b"libSceShare", b"libSceShare", &[*row])
                .build()
                .unwrap();
            assert!(matches!(
                link(&bytes, &provider),
                Err(SymbolicError::MissingProvider { .. })
            ));
        }
        for (module, library) in [
            (b"Share".as_slice(), b"libSceShare".as_slice()),
            (b"libSceShare", b"Share"),
        ] {
            let bytes = consumer(module, library, &[*row]).build().unwrap();
            assert!(matches!(
                link(&bytes, &provider),
                Err(SymbolicError::MissingProvider { .. })
            ));
        }
    }
    let row = Service::registrations()[1220];
    for (major, minor, version) in [(2, 1, 1), (1, 0, 1), (1, 1, 2)] {
        let mut bad = consumer(b"Share", b"Share", &[row]);
        (bad.major, bad.minor, bad.version) = (major, minor, version);
        assert!(matches!(
            link(&bad.build().unwrap(), &provider),
            Err(SymbolicError::VersionMismatch { .. })
        ));
    }
    let mut object = consumer(b"Share", b"Share", &[row]);
    object.imports[0].symbol_type = 1;
    object.imports[0].relocation = 6;
    assert!(matches!(
        link(&object.build().unwrap(), &provider),
        Err(SymbolicError::UnsupportedSymbol {
            reason: UnsupportedSymbol::TypeMismatch,
            ..
        })
    ));
    assert!(matches!(
        build_kernel_provider(provider.image().len() as u64 - 1),
        Err(KernelProviderError::Budget)
    ));
}

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
fn content_provider_selector_boundaries_and_wide_actual_symbolic_bindings() {
    let nid = b"0GnN4QCgIfs";
    for (library, module) in [(63, 64), (64, 4096), (65535, 65535)] {
        let (name, length) = qualified_name(nid, library, module);
        assert_eq!(name[length - 1], 0);
        let parsed = pentax_linker::QualifiedNid::parse(&name[..length - 1]).unwrap();
        assert_eq!(
            (parsed.library().value(), parsed.module().value()),
            (library, module)
        );
    }
    let ordinary = factory();
    // Exercise one-, two- and three-digit selectors with unchanged marker RVAs.
    // Namespace metadata no longer competes with executable source capacity.
    for base in [1, 2, 63, 64, 4096] {
        let provider = build_kernel_provider_with_base(MAX_IMAGE_BYTES, base).unwrap();
        for (a, b) in ordinary.markers().iter().zip(provider.markers()) {
            assert_eq!(
                (a.service_id, a.rva, a.bytes()),
                (b.service_id, b.rva, b.bytes())
            );
        }
        let view = open_executable(provider.image()).unwrap();
        let inspection = inspect_sce(&view, &Default::default(), &Default::default()).unwrap();
        let metadata = inspection.metadata().unwrap();
        let descriptors = metadata.descriptors().decoded().unwrap();
        for (index, descriptor) in descriptors.module_info().iter().enumerate() {
            assert_eq!(descriptor.id().value(), base + index as u16);
        }
        for (index, descriptor) in descriptors.export_libraries().iter().enumerate() {
            assert_eq!(descriptor.id().value(), base + index as u16);
        }
        for (index, row) in Service::registrations().iter().enumerate() {
            let name = metadata.link().symbols()[index + 1].name.as_ref().unwrap();
            let parsed = pentax_linker::QualifiedNid::parse(name.as_bytes()).unwrap();
            let module = MODULES
                .iter()
                .position(|m| m.0 == row.module && m.1 == row.module_version)
                .unwrap();
            let library = LIBRARIES
                .iter()
                .position(|l| l.0 == row.library && l.1 == row.library_version)
                .unwrap();
            assert_eq!(
                (parsed.module().value(), parsed.library().value()),
                (base + module as u16, base + library as u16)
            );
        }
        for object in provider.objects() {
            let name = metadata.link().symbols()[object.symbol]
                .name
                .as_ref()
                .unwrap();
            let parsed = pentax_linker::QualifiedNid::parse(name.as_bytes()).unwrap();
            assert_eq!(
                (parsed.module().value(), parsed.library().value()),
                (base, base)
            );
        }
        for (index, row) in Service::registrations()[1216..1220].iter().enumerate() {
            let mut original = consumer(row.module.as_bytes(), row.library.as_bytes(), &[*row]);
            if row.service == Service::ContentSearchInit {
                original.minor = 0;
            }
            let bytes = original.build().unwrap();
            let plan = link(&bytes, &provider).unwrap();
            assert_eq!(plan.patch_count(), 2);
            assert!(plan.patches().iter().any(|p| p.consumer().index() == 0
                && p.value() == PROVIDER_BIAS + 0x100 + (1216 + index) as u64 * 8));
        }
        for index in [1238, 1256] {
            let row = Service::registrations()[index];
            let mut original = consumer(row.module.as_bytes(), row.library.as_bytes(), &[row]);
            original.minor = 0;
            let bytes = original.build().unwrap();
            let plan = link(&bytes, &provider).unwrap();
            assert_eq!(plan.patch_count(), 2);
            assert!(
                plan.patches()
                    .iter()
                    .any(|patch| patch.consumer().index() == 0
                        && patch.value() == PROVIDER_BIAS + 0x100 + index as u64 * 8
                        && matches!(patch.resolution(), Resolution::Definition(reference)
                    if reference.module.index() == 1 && reference.symbol == index + 1))
            );
            let name = metadata.link().symbols()[index + 1].name.as_ref().unwrap();
            let qualified = pentax_linker::QualifiedNid::parse(name.as_bytes()).unwrap();
            assert!(qualified.library().value() >= 64);
            if index == 1256 {
                assert!(qualified.module().value() >= 64);
            }
        }
        assert!(provider.code_bytes().len() as u64 <= CODE_MAPPED_BYTES);
        assert!(matches!(
            build_kernel_provider_with_base(provider.image().len() as u64 - 1, base),
            Err(KernelProviderError::Budget)
        ));
    }
    assert!(matches!(
        build_kernel_provider_with_base(MAX_IMAGE_BYTES, 65535),
        Err(KernelProviderError::InvalidCatalogue)
    ));
}

#[test]
fn content_provider_exact_versions_alias_negatives_and_fixed_site_geometry() {
    let provider = factory();
    assert_eq!((MODULES.len(), LIBRARIES.len()), (66, 67));
    assert_eq!(
        (MAX_REGISTRATIONS, CODE_MAPPED_BYTES),
        (2048, 4 * PAGE_SIZE)
    );
    assert_eq!(
        (provider.objects()[0].rva, provider.objects()[1].rva),
        (0x10000, 0x10008)
    );
    for row in &Service::registrations()[1216..1220] {
        let mut original = consumer(row.module.as_bytes(), row.library.as_bytes(), &[*row]);
        if row.service == Service::ContentSearchInit {
            original.minor = 0;
        }
        let bytes = original.build().unwrap();
        assert_eq!(link(&bytes, &provider).unwrap().patch_count(), 2);
        original.minor = if row.service == Service::ContentSearchInit {
            1
        } else {
            0
        };
        assert!(matches!(
            link(&original.build().unwrap(), &provider),
            Err(SymbolicError::VersionMismatch { .. })
        ));
    }
    for (index, prefix) in [
        (1217, b"libSceContentSearch".as_slice()),
        (1218, b"libSceContentDelete".as_slice()),
    ] {
        let row = Service::registrations()[index];
        let mut original = consumer(prefix, prefix, &[row]);
        if index == 1217 {
            original.minor = 0;
        }
        assert!(matches!(
            link(&original.build().unwrap(), &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    let row = Service::registrations()[1216];
    for (module, library) in [
        (
            b"ContentExport".as_slice(),
            b"libSceContentExport".as_slice(),
        ),
        (b"libSceContentExport", b"ContentExport"),
    ] {
        let bytes = consumer(module, library, &[row]).build().unwrap();
        assert!(matches!(
            link(&bytes, &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    let mut object = consumer(b"ContentExport", b"ContentExport", &[row]);
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

#[test]
fn share_provider_appends_exact_scopes_preserves_old_sites_and_refuses_bad_imports() {
    let provider = factory();
    assert_eq!((MODULES.len(), LIBRARIES.len()), (66, 67));
    assert_eq!(
        &MODULES[56..58],
        &[("Share", (1, 1)), ("libSceShare", (1, 1))]
    );
    assert_eq!(&LIBRARIES[57..59], &[("Share", 1), ("libSceShare", 1)]);
    assert_eq!(
        (MAX_REGISTRATIONS, CODE_MAPPED_BYTES),
        (2048, 4 * PAGE_SIZE)
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
        (provider.objects()[0].symbol, provider.objects()[0].rva),
        (Service::registrations().len() + 1, 0x10000)
    );
    assert_eq!(
        (provider.objects()[1].symbol, provider.objects()[1].rva),
        (Service::registrations().len() + 2, 0x10008)
    );
    assert_eq!(
        &provider.data_bytes()[..8],
        &STACK_GUARD_INITIAL.to_le_bytes()
    );
    assert_eq!(&provider.data_bytes()[8..], &[0; 8]);
    for (scope, start, end) in [
        (b"Share".as_slice(), 1211, 1214),
        (b"libSceShare".as_slice(), 1214, 1216),
    ] {
        let rows = &Service::registrations()[start..end];
        let bytes = consumer(scope, scope, rows).build().unwrap();
        let plan = link(&bytes, &provider).unwrap();
        assert_eq!(plan.patch_count(), rows.len() + 1);
        for (offset, patch) in plan
            .patches()
            .iter()
            .filter(|p| p.consumer().index() == 0)
            .enumerate()
        {
            assert_eq!(
                patch.value(),
                PROVIDER_BIAS + 0x100 + (start + offset) as u64 * 8
            );
            assert!(
                matches!(patch.resolution(), Resolution::Definition(r) if r.module.index()==1 && r.symbol==start+offset+1)
            );
        }
    }
    let permit = Service::registrations()[1212];
    for (module, library) in [
        (b"libSceShare".as_slice(), b"libSceShare".as_slice()),
        (b"Share", b"libSceShare"),
        (b"libSceShare", b"Share"),
    ] {
        let bytes = consumer(module, library, &[permit]).build().unwrap();
        assert!(matches!(
            link(&bytes, &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    for (major, minor, version) in [(2, 1, 1), (1, 2, 1), (1, 1, 2)] {
        let mut bad = consumer(b"Share", b"Share", &[permit]);
        (bad.major, bad.minor, bad.version) = (major, minor, version);
        assert!(matches!(
            link(&bad.build().unwrap(), &provider),
            Err(SymbolicError::VersionMismatch { .. })
        ));
    }
    let mut object = consumer(b"Share", b"Share", &[permit]);
    object.imports[0].symbol_type = 1;
    object.imports[0].relocation = 6;
    assert!(matches!(
        link(&object.build().unwrap(), &provider),
        Err(SymbolicError::UnsupportedSymbol {
            reason: UnsupportedSymbol::TypeMismatch,
            ..
        })
    ));
    assert!(matches!(
        build_kernel_provider(provider.image().len() as u64 - 1),
        Err(KernelProviderError::Budget)
    ));
}

#[test]
fn np_webapi2_provider_appends_descriptors_and_sites_without_moving_old_markers_or_objects() {
    let provider = factory();
    assert_eq!(MODULES.len(), 66);
    assert_eq!(LIBRARIES.len(), 67);
    assert_eq!(
        &MODULES[34..38],
        &[
            ("Http2", (1, 1)),
            ("libSceHttp2", (1, 1)),
            ("NpWebApi2", (1, 1)),
            ("libSceNpWebApi2", (1, 1))
        ]
    );
    assert_eq!(
        &LIBRARIES[35..39],
        &[
            ("Http2", 1),
            ("libSceHttp2", 1),
            ("NpWebApi2", 1),
            ("libSceNpWebApi2", 1)
        ]
    );
    let view = open_executable(provider.image()).unwrap();
    let runtime = view.runtime_plan().unwrap();
    assert_eq!(runtime.segments.len(), 3);
    assert_eq!(
        (
            runtime.segments[0].flags.raw(),
            runtime.segments[0].mem_size
        ),
        (5, CODE_MAPPED_BYTES)
    );
    assert_eq!(
        (runtime.segments[1].flags.raw(), runtime.segments[1].vaddr),
        (6, CODE_MAPPED_BYTES)
    );
    assert!(runtime.tls.is_none());
    assert!(provider.code_bytes().len() as u64 <= CODE_MAPPED_BYTES);
    let metadata = inspect_sce(&view, &Default::default(), &Default::default()).unwrap();
    let metadata = metadata.metadata().unwrap();
    let descriptors = metadata.descriptors().decoded().unwrap();
    assert_eq!(descriptors.module_info().len(), 66);
    assert_eq!(descriptors.export_libraries().len(), 67);
    for (index, descriptor) in descriptors.module_info().iter().enumerate() {
        assert_eq!(descriptor.name().as_bytes(), MODULES[index].0.as_bytes());
        assert_eq!(descriptor.id().value(), index as u16 + 1);
        assert_eq!(
            (
                u16::from(descriptor.version_major()),
                u16::from(descriptor.version_minor())
            ),
            MODULES[index].1
        );
    }
    for (index, descriptor) in descriptors.export_libraries().iter().enumerate() {
        assert_eq!(descriptor.name().as_bytes(), LIBRARIES[index].0.as_bytes());
        assert_eq!(descriptor.id().value(), index as u16 + 1);
        assert_eq!(descriptor.version(), LIBRARIES[index].1);
    }
    let symbols = metadata.link().symbols();
    assert_eq!(symbols.len(), Service::registrations().len() + 3);
    assert_eq!(
        provider.objects()[0].symbol,
        Service::registrations().len() + 1
    );
    assert_eq!(
        provider.objects()[1].symbol,
        Service::registrations().len() + 2
    );
    assert_eq!(provider.objects()[0].rva, STACK_GUARD_RVA);
    assert_eq!(provider.objects()[1].rva, PROGRAM_NAME_OBJECT_RVA);
    assert_eq!(STACK_GUARD_RVA, 4 * PAGE_SIZE);
    assert_eq!(PROGRAM_NAME_BUFFER_RVA, 4 * PAGE_SIZE + 16);
    for (index, marker) in provider.markers().iter().enumerate() {
        assert_eq!(marker.symbol, index + 1);
        assert_eq!(marker.service_id, 0x5058_4b01 + index as u32);
        assert_eq!(marker.rva, 0x100 + index as u64 * 8);
        assert_eq!(
            &provider.image()[marker.rva as usize..marker.rva as usize + 8],
            &marker.bytes()
        );
        let symbol = &symbols[marker.symbol];
        assert_eq!(symbol.symbol_type(), SymbolType::FUNC);
        assert_eq!(symbol.binding(), SymbolBinding::GLOBAL);
        assert_eq!(symbol.visibility(), SymbolVisibility::PROTECTED);
        assert_eq!((symbol.value, symbol.size), (marker.rva, 8));
        assert_eq!(
            marker
                .registration
                .source_name
                .starts_with("LibNpWebApi2::"),
            (OLD_REGISTRATIONS..938).contains(&index)
        );
    }
    let first = provider.markers()[OLD_REGISTRATIONS];
    let last = provider.markers()[937];
    assert_eq!((first.service_id, first.rva), (0x5058_4e5f, 0x1bf0));
    assert_eq!((last.service_id, last.rva), (0x5058_4eaa, 0x1e48));
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
fn np_webapi2_all_public_functions_bind_exact_qualified_scopes() {
    let provider = factory();
    let rows = Service::registrations()[OLD_REGISTRATIONS..938]
        .iter()
        .step_by(4)
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 19);
    for (scope, (module, library)) in SCOPES.into_iter().enumerate() {
        for (group_index, group) in rows.chunks(9).enumerate() {
            for dynlib in [false, true] {
                for encoding in [
                    DescriptorEncoding::Classic,
                    DescriptorEncoding::Variant1Observed,
                ] {
                    let mut original = consumer(module, library, group);
                    original.dynlib = dynlib;
                    original.encoding = encoding;
                    let bytes = original.build().unwrap();
                    let source = link(&bytes, &provider).unwrap();
                    // The provider has its own real program-name relocation.
                    assert_eq!(source.patch_count(), group.len() + 1);
                    let provider_patch = source
                        .patches()
                        .iter()
                        .find(|patch| patch.consumer().index() == 1)
                        .unwrap();
                    assert_eq!(
                        provider_patch.address(),
                        PROVIDER_BIAS + PROGRAM_NAME_OBJECT_RVA
                    );
                    assert_eq!(
                        provider_patch.value(),
                        PROVIDER_BIAS + PROGRAM_NAME_BUFFER_RVA
                    );
                    let imports = source
                        .patches()
                        .iter()
                        .filter(|patch| patch.consumer().index() == 0);
                    assert_eq!(imports.clone().count(), group.len());
                    for (index, patch) in imports.enumerate() {
                        let row = OLD_REGISTRATIONS + (group_index * 9 + index) * 4 + scope;
                        let marker = provider.markers()[row];
                        assert_eq!(patch.value(), PROVIDER_BIAS + marker.rva);
                        assert!(
                            matches!(patch.resolution(), Resolution::Definition(reference)
                            if reference.module.index() == 1 && reference.symbol == row + 1)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn np_webapi2_provider_refuses_other_versions_types_and_namespace_spellings() {
    let provider = factory();
    let rows = &Service::registrations()[OLD_REGISTRATIONS..OLD_REGISTRATIONS + 1];
    for (module, library) in SCOPES {
        for (major, minor, version) in [(2, 1, 1), (1, 2, 1), (1, 1, 2)] {
            let mut original = consumer(module, library, rows);
            (original.major, original.minor, original.version) = (major, minor, version);
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::VersionMismatch { .. })
            ));
        }
        let mut original = consumer(module, library, rows);
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
    for (module, library) in [
        (b"NpWebApi".as_slice(), b"NpWebApi2".as_slice()),
        (b"NpWebApi2".as_slice(), b"NpWebApi".as_slice()),
        (
            b"libSceNpCppWebApi".as_slice(),
            b"libSceNpWebApi2".as_slice(),
        ),
        (
            b"libScelibSceNpWebApi2".as_slice(),
            b"libSceNpWebApi2".as_slice(),
        ),
    ] {
        assert!(matches!(
            link(&consumer(module, library, rows).build().unwrap(), &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
}
