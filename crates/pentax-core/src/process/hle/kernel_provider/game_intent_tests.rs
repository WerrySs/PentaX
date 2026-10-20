// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use crate::process::hle::fixture::{CODE, CONSUMER_BIAS, Consumer, Import, PROVIDER_BIAS};
use pentax_linker::{
    DescriptorEncoding, ModuleInput, Resolution, SymbolicError, UnsupportedSymbol, plan_symbolic,
};
use pentax_loader::open_executable;

const NIDS: [[u8; 11]; 4] = [
    *b"m87BHxt-H60",
    *b"0HBYxYAjmf0",
    *b"jEIXUAr9XE8",
    *b"rPl0INNc-M8",
];

fn consumer(module: &'static [u8], library: &'static [u8]) -> Consumer {
    let mut original = Consumer::importing(vec![(CODE, vec![0xcc])]);
    original.module_name = module;
    original.library_name = library;
    original.imports = NIDS
        .into_iter()
        .map(|nid| Import {
            nid,
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
fn game_intent_all_eight_bindings_append_after_stable_old_sites_and_data() {
    let provider = build_kernel_provider(CODE_MAPPED_BYTES).unwrap();
    assert_eq!(provider.registration_count(), 1281);
    assert_eq!((MODULES.len(), LIBRARIES.len()), (64, 65));
    assert_eq!(
        &MODULES[39..41],
        &[("NpGameIntent", (1, 1)), ("libSceNpGameIntent", (1, 1))]
    );
    assert_eq!(
        &LIBRARIES[40..42],
        &[("NpGameIntent", 1), ("libSceNpGameIntent", 1)]
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
            provider.markers()[961].service_id,
            provider.markers()[961].rva
        ),
        (0x5058_4ec2, 0x1f08)
    );
    assert_eq!(
        (
            provider.markers()[968].service_id,
            provider.markers()[968].rva
        ),
        (0x5058_4ec9, 0x1f40)
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
        (provider.code_bytes().len(), provider.image().len()),
        (0xfdb8, 0xfdc8)
    );
    for (scope, namespace) in [b"NpGameIntent".as_slice(), b"libSceNpGameIntent".as_slice()]
        .into_iter()
        .enumerate()
    {
        for dynlib in [false, true] {
            for encoding in [
                DescriptorEncoding::Classic,
                DescriptorEncoding::Variant1Observed,
            ] {
                let mut original = consumer(namespace, namespace);
                original.dynlib = dynlib;
                original.encoding = encoding;
                let bytes = original.build().unwrap();
                let plan = link(&bytes, &provider).unwrap();
                // Four consumer imports plus the provider's program-name relocation.
                assert_eq!(plan.patch_count(), 5);
                for (index, patch) in plan.patches().iter().take(4).enumerate() {
                    let marker_index = 961 + index * 2 + scope;
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

#[test]
fn game_intent_bindings_refuse_mixed_scopes_other_versions_and_object_imports() {
    let provider = build_kernel_provider(CODE_MAPPED_BYTES).unwrap();
    for (module, library) in [
        (b"NpGameIntent".as_slice(), b"libSceNpGameIntent".as_slice()),
        (b"libSceNpGameIntent".as_slice(), b"NpGameIntent".as_slice()),
        (b"GameIntent".as_slice(), b"GameIntent".as_slice()),
    ] {
        assert!(matches!(
            link(&consumer(module, library).build().unwrap(), &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    for namespace in [b"NpGameIntent".as_slice(), b"libSceNpGameIntent".as_slice()] {
        for (major, minor, version) in [(2, 1, 1), (1, 2, 1), (1, 1, 2)] {
            let mut original = consumer(namespace, namespace);
            (original.major, original.minor, original.version) = (major, minor, version);
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::VersionMismatch { .. })
            ));
        }
        let mut original = consumer(namespace, namespace);
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
