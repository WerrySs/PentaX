// SPDX-License-Identifier: GPL-2.0-only

//! Original split-provider consumer and exact mspace publication metadata.
use super::*;
use crate::process::hle::catalog::KnownFunction;
use crate::process::hle::fixture::{CODE, CONSUMER_BIAS, Consumer, Import, PROVIDER_BIAS};
use crate::process::hle::{GeneratedProviders, HleLimits, HleProfile, build_hle_providers};
use pentax_linker::{
    DescriptorEncoding, ModuleInput, Resolution, SymbolBinding, SymbolType, SymbolVisibility,
    SymbolicError, UnsupportedSymbol, inspect_sce, plan_symbolic,
};
use pentax_loader::open_executable;

const OLD_REGISTRATIONS: usize = 938;
const KERNEL_BIAS: u64 = 0x40_0000;

fn factories() -> (GeneratedKernelProvider, GeneratedProviders) {
    (
        build_kernel_provider(CODE_MAPPED_BYTES).unwrap(),
        build_hle_providers(
            HleProfile::KytyObservedLibcInternalV1,
            &HleLimits::default(),
        )
        .unwrap(),
    )
}

fn consumer(module: &'static [u8], library: &'static [u8]) -> Consumer {
    let mut original = Consumer::importing(vec![(CODE, vec![0xcc])]);
    original.module_name = module;
    original.library_name = library;
    original.imports = vec![
        Import {
            nid: *b"-hn1tcVHq5Q",
            symbol_type: 2,
            binding: 1,
            relocation: 7,
            addend: 0,
        },
        Import::function(KnownFunction::Snprintf),
    ];
    original
}

fn link<'a>(
    bytes: &'a [u8],
    kernel: &'a GeneratedKernelProvider,
    libc: &'a GeneratedProviders,
) -> Result<pentax_linker::SymbolicLinkPlan<'a>, SymbolicError> {
    plan_symbolic(
        &[
            ModuleInput::new(open_executable(bytes).unwrap(), CONSUMER_BIAS),
            ModuleInput::new(open_executable(kernel.image()).unwrap(), KERNEL_BIAS),
            ModuleInput::new(open_executable(libc.image()).unwrap(), PROVIDER_BIAS),
        ],
        &Default::default(),
    )
}

#[test]
fn mspace_provider_appends_exact_sites_and_descriptors_with_stable_old_rvas() {
    let (provider, libc) = factories();
    assert_eq!(provider.registration_count(), 1281);
    assert_eq!(provider.markers().len(), 1281);
    assert_eq!((MODULES.len(), LIBRARIES.len()), (64, 65));
    assert_eq!(
        &MODULES[36..39],
        &[
            ("NpWebApi2", (1, 1)),
            ("libSceNpWebApi2", (1, 1)),
            ("libSceLibcInternal", (1, 1))
        ]
    );
    assert_eq!(
        &LIBRARIES[37..40],
        &[
            ("NpWebApi2", 1),
            ("libSceNpWebApi2", 1),
            ("libSceLibcInternal", 1)
        ]
    );
    let view = open_executable(provider.image()).unwrap();
    let runtime = view.runtime_plan().unwrap();
    assert_eq!(runtime.segments.len(), 2);
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
    assert_eq!(provider.code_bytes().len(), 0xfdb8);
    assert_eq!(provider.image().len(), 0xfdc8);
    let metadata = inspect_sce(&view, &Default::default(), &Default::default()).unwrap();
    let metadata = metadata.metadata().unwrap();
    let descriptors = metadata.descriptors().decoded().unwrap();
    for (index, module) in descriptors.module_info().iter().enumerate() {
        assert_eq!(module.id().value(), index as u16 + 1);
        assert_eq!(module.name().as_bytes(), MODULES[index].0.as_bytes());
        assert_eq!(
            (
                u16::from(module.version_major()),
                u16::from(module.version_minor())
            ),
            MODULES[index].1
        );
    }
    for (index, library) in descriptors.export_libraries().iter().enumerate() {
        assert_eq!(library.id().value(), index as u16 + 1);
        assert_eq!(library.name().as_bytes(), LIBRARIES[index].0.as_bytes());
        assert_eq!(library.version(), LIBRARIES[index].1);
    }
    let symbols = metadata.link().symbols();
    assert_eq!(symbols.len(), 1053);
    for (index, marker) in provider.markers().iter().enumerate() {
        assert_eq!(marker.symbol, index + 1);
        assert_eq!(marker.rva, 0x100 + index as u64 * 8);
        assert_eq!(marker.service_id, 0x5058_4b01 + index as u32);
        assert_eq!(
            &provider.image()[marker.rva as usize..marker.rva as usize + 8],
            &marker.bytes()
        );
        if (OLD_REGISTRATIONS..961).contains(&index) {
            let symbol = &symbols[marker.symbol];
            assert_eq!(symbol.binding(), SymbolBinding::GLOBAL);
            assert_eq!(symbol.visibility(), SymbolVisibility::PROTECTED);
            assert_eq!(symbol.symbol_type(), SymbolType::FUNC);
            assert_eq!((symbol.value, symbol.size), (marker.rva, 8));
            assert_eq!(
                symbol.name.as_ref().unwrap().as_bytes(),
                format!("{}#o#n", marker.registration.nid).as_bytes()
            );
        }
    }
    let first = provider.markers()[938];
    let last = provider.markers()[960];
    assert_eq!(
        (first.registration.service, first.service_id, first.rva),
        (Service::MspaceCreate, 0x5058_4eab, 0x1e50)
    );
    assert_eq!(
        (last.registration.service, last.service_id, last.rva),
        (Service::PafMspaceMallocStatsFast, 0x5058_4ec1, 0x1f00)
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
    // Mspace stays in the kernel factory; the separately retained libc factory
    // keeps its seventeen sites and the prior snprintf marker/data geometry.
    assert_eq!(libc.function_count(), 17);
    assert_eq!(libc.image().len(), 0x120c);
    assert_eq!(
        &libc.image()[0x200..0x208],
        &[0xb8, 0x11, 0x48, 0x58, 0x50, 0x0f, 0x05, 0xc3]
    );
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
fn mspace_and_snprintf_share_one_observed_scope_without_shadowing_distinct_providers() {
    let (kernel, libc) = factories();
    for dynlib in [false, true] {
        for encoding in [
            DescriptorEncoding::Classic,
            DescriptorEncoding::Variant1Observed,
        ] {
            let mut original = consumer(b"libSceLibcInternal", b"libSceLibcInternal");
            original.dynlib = dynlib;
            original.encoding = encoding;
            let bytes = original.build().unwrap();
            let plan = link(&bytes, &kernel, &libc).unwrap();
            // Two consumer imports plus the kernel's program-name relocation.
            assert_eq!(plan.patch_count(), 3);
            assert_eq!(plan.patches()[0].value(), KERNEL_BIAS + 0x1e50);
            assert!(
                matches!(plan.patches()[0].resolution(), Resolution::Definition(reference)
                if reference.module.index() == 1 && reference.symbol == 939)
            );
            assert_eq!(plan.patches()[1].value(), PROVIDER_BIAS + 0x200);
            assert!(
                matches!(plan.patches()[1].resolution(), Resolution::Definition(reference)
                if reference.module.index() == 2 && reference.symbol == 64)
            );
            assert_eq!(
                kernel
                    .markers()
                    .iter()
                    .filter(|marker| marker.registration.nid == "eLdDw6l0-bU")
                    .count(),
                0
            );
            assert_eq!(
                libc.manifest()
                    .iter()
                    .filter(|entry| entry.function.nid() == b"-hn1tcVHq5Q")
                    .count(),
                0
            );
        }
    }
}

#[test]
fn mspace_split_provider_rejects_unconfirmed_scopes_versions_and_object_imports() {
    let (kernel, libc) = factories();
    for (module, library) in [
        (b"LibcInternal".as_slice(), b"LibcInternal".as_slice()),
        (b"LibcInternal".as_slice(), b"libSceLibcInternal".as_slice()),
        (b"libSceLibcInternal".as_slice(), b"LibcInternal".as_slice()),
        (b"libc".as_slice(), b"libc".as_slice()),
    ] {
        assert!(matches!(
            link(&consumer(module, library).build().unwrap(), &kernel, &libc),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    for (major, minor, version) in [(2, 1, 1), (1, 2, 1), (1, 1, 2)] {
        let mut original = consumer(b"libSceLibcInternal", b"libSceLibcInternal");
        (original.major, original.minor, original.version) = (major, minor, version);
        assert!(matches!(
            link(&original.build().unwrap(), &kernel, &libc),
            Err(SymbolicError::VersionMismatch { .. })
        ));
    }
    let mut original = consumer(b"libSceLibcInternal", b"libSceLibcInternal");
    original.imports[0].symbol_type = 1;
    original.imports[0].relocation = 6;
    assert!(matches!(link(&original.build().unwrap(), &kernel, &libc),
        Err(SymbolicError::UnsupportedSymbol { reason: UnsupportedSymbol::TypeMismatch, symbol })
            if symbol.module.index() == 1 && symbol.symbol == 939));
}
