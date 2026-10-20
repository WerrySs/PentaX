// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use crate::process::hle::fixture::{CODE, CONSUMER_BIAS, Consumer, Import, PROVIDER_BIAS};
use pentax_linker::{ModuleInput, Resolution, SymbolicError, UnsupportedSymbol, plan_symbolic};
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
fn font_all_exact_bindings_fit_existing_caps_and_preserve_old_sites_and_objects() {
    let provider = build_kernel_provider(MAX_IMAGE_BYTES).unwrap();
    assert_eq!(
        (MAX_REGISTRATIONS, CODE_MAPPED_BYTES),
        (2048, 4 * PAGE_SIZE)
    );
    assert_eq!((MODULES.len(), LIBRARIES.len()), (66, 67));
    assert_eq!(
        &MODULES[47..51],
        &[
            ("Font", (1, 1)),
            ("libSceFont", (1, 1)),
            ("FontFt", (1, 1)),
            ("libSceFontFt", (1, 1))
        ]
    );
    assert_eq!(
        &LIBRARIES[48..52],
        &[
            ("Font", 1),
            ("libSceFont", 1),
            ("FontFt", 1),
            ("libSceFontFt", 1)
        ]
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
            provider.markers()[1056].service_id,
            provider.markers()[1056].rva
        ),
        (0x5058_4f21, 0x2200)
    );
    assert_eq!(
        (
            provider.markers()[1057].service_id,
            provider.markers()[1057].rva
        ),
        (0x5058_4f22, 0x2208)
    );
    assert_eq!(
        (
            provider.markers()[1182].service_id,
            provider.markers()[1182].rva
        ),
        (0x5058_4f9f, 0x25f0)
    );
    assert_eq!(
        (provider.objects()[0].symbol, provider.objects()[0].rva),
        (Service::registrations().len() + 1, 0x10000)
    );
    assert_eq!(
        (provider.objects()[1].symbol, provider.objects()[1].rva),
        (Service::registrations().len() + 2, 0x10008)
    );
    assert_eq!(PROGRAM_NAME_BUFFER_RVA, 0x10010);
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
    for (scope, start, end) in [
        (b"Font".as_slice(), 1057, 1127),
        (b"FontFt".as_slice(), 1127, 1129),
        (b"libSceFont".as_slice(), 1129, 1181),
        (b"libSceFontFt".as_slice(), 1181, 1183),
    ] {
        for (group, rows) in Service::registrations()[start..end].chunks(9).enumerate() {
            let bytes = consumer(scope, scope, rows).build().unwrap();
            let plan = link(&bytes, &provider).unwrap();
            assert_eq!(plan.patch_count(), rows.len() + 1);
            assert_eq!(
                plan.patches()
                    .iter()
                    .filter(|patch| patch.consumer().index() == 0)
                    .count(),
                rows.len()
            );
            for (offset, patch) in plan
                .patches()
                .iter()
                .filter(|patch| patch.consumer().index() == 0)
                .enumerate()
            {
                let index = start + group * 9 + offset;
                assert_eq!(patch.value(), PROVIDER_BIAS + 0x100 + index as u64 * 8);
                assert!(
                    matches!(patch.resolution(), Resolution::Definition(reference)
                    if reference.module.index() == 1 && reference.symbol == index + 1)
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
fn font_rejects_unobserved_renderer_nids_mixed_scopes_versions_and_object_types() {
    let provider = build_kernel_provider(MAX_IMAGE_BYTES).unwrap();
    for index in [1090, 1092] {
        let row = &Service::registrations()[index..index + 1];
        assert_eq!(row[0].service, Service::FontRenderCharGlyphImageHorizontal);
        assert!(matches!(
            link(
                &consumer(b"libSceFont", b"libSceFont", row).build().unwrap(),
                &provider
            ),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    for (scope, prefixed, index) in [
        (b"Font".as_slice(), b"libSceFont".as_slice(), 1085),
        (b"FontFt".as_slice(), b"libSceFontFt".as_slice(), 1127),
    ] {
        let row = &Service::registrations()[index..index + 1];
        for (module, library) in [(scope, prefixed), (prefixed, scope)] {
            assert!(matches!(
                link(&consumer(module, library, row).build().unwrap(), &provider),
                Err(SymbolicError::MissingProvider { .. })
            ));
        }
        for namespace in [scope, prefixed] {
            for (major, minor, version) in [(2, 1, 1), (1, 2, 1), (1, 1, 2)] {
                let mut original = consumer(namespace, namespace, row);
                (original.major, original.minor, original.version) = (major, minor, version);
                assert!(matches!(
                    link(&original.build().unwrap(), &provider),
                    Err(SymbolicError::VersionMismatch { .. })
                ));
            }
            let mut original = consumer(namespace, namespace, row);
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
}
