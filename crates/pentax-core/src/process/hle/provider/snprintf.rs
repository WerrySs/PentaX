// SPDX-License-Identifier: GPL-2.0-or-later

//! Original snprintf identities and layout checks; no formatting engine calls.
use super::*;
use crate::process::hle::fixture::{CODE, CONSUMER_BIAS, Consumer, Import, PROVIDER_BIAS};
use pentax_linker::{
    DescriptorEncoding, ModuleInput, Resolution, SymbolBinding, SymbolType, SymbolVisibility,
    SymbolicError, UnsupportedSymbol, inspect_sce, plan_symbolic,
};
use pentax_loader::open_executable;

type Scope = (&'static [u8], &'static [u8], usize, &'static [u8]);
const SCOPES: [Scope; 4] = [
    (b"LibcInternal", b"LibcInternal", 17, b"#B#B"),
    (b"libSceLibcInternal", b"libSceLibcInternal", 64, b"#E#C"),
    (b"libSceLibcInternal", b"LibcInternal", 65, b"#B#C"),
    (b"LibcInternal", b"libSceLibcInternal", 66, b"#E#B"),
];

fn factory() -> GeneratedProviders {
    build_hle_providers(
        HleProfile::KytyObservedLibcInternalV1,
        &HleLimits::default(),
    )
    .unwrap()
}

fn consumer(module: &'static [u8], library: &'static [u8]) -> Consumer {
    let mut original = Consumer::importing(vec![(CODE, vec![0xcc])]);
    original.module_name = module;
    original.library_name = library;
    original.imports = vec![Import::function(KnownFunction::Snprintf)];
    original
}

fn link<'a>(
    bytes: &'a [u8],
    provider: &'a GeneratedProviders,
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
fn snprintf_appends_one_site_and_moves_dynamic_metadata_clear_of_all_markers() {
    let provider = factory();
    let old = [
        KnownFunction::Memcpy,
        KnownFunction::Memmove,
        KnownFunction::Memset,
        KnownFunction::Memcmp,
        KnownFunction::Strcmp,
        KnownFunction::Strncmp,
        KnownFunction::Strlen,
        KnownFunction::Strcpy,
        KnownFunction::Strncpy,
        KnownFunction::Strcat,
        KnownFunction::Strchr,
        KnownFunction::Strrchr,
        KnownFunction::Strstr,
        KnownFunction::HeapGetTraceInfo,
        KnownFunction::HeapGetTraceInfoSce,
        KnownFunction::CxaAtExit,
    ];
    assert_eq!(KnownFunction::LIBC.len(), 13);
    assert_eq!(KnownFunction::LIBC_FUNCTIONS.len(), 15);
    assert_eq!(KnownFunction::ALL.len(), 17);
    assert_eq!(&KnownFunction::ALL[..16], &old);
    for (index, function) in old.into_iter().enumerate() {
        assert_eq!(function.index(), index);
        assert_eq!(function.service_id(), 0x5058_4801 + index as u32);
        let rva = 0x100 + index * 16;
        assert_eq!(&provider.image()[rva..rva + 8], &stub(function));
        assert_eq!(provider.manifest()[index].stub_rva, rva as u64);
    }
    let function = KnownFunction::Snprintf;
    assert_eq!(KnownFunction::ALL[16], function);
    assert_eq!(KnownFunction::LIBC_FUNCTIONS[14], function);
    assert_eq!(function.nid(), b"eLdDw6l0-bU");
    assert_eq!(function.index(), 16);
    assert_eq!(function.service_id(), 0x5058_4811);
    assert_eq!(function.string_operation(), None);
    assert_eq!(
        &provider.image()[0x200..0x208],
        &[0xb8, 0x11, 0x48, 0x58, 0x50, 0x0f, 0x05, 0xc3]
    );
    assert_eq!(
        (DYNAMIC_AT, DYNAMIC_ENTRIES, STRINGS_AT),
        (0x300, 12, 0x400)
    );
    // The actual ELF header points to the relocated dynamic table, leaving the
    // newly occupied 0x200 marker intact. No AL/R10 forwarding stub is emitted.
    assert_eq!(
        u64::from_le_bytes(provider.image()[128..136].try_into().unwrap()),
        0x300
    );
    assert_eq!(
        u64::from_le_bytes(provider.image()[136..144].try_into().unwrap()),
        0x300
    );
    assert_eq!(STRINGS_AT + string_offsets().3, 0x887);
    assert_eq!(SYMBOLS_AT + SYMBOLS * SYMBOL_SIZE, 0xf48);
    assert_eq!((CODE_SIZE, IMAGE_SIZE), (0x1000, 0x120c));
    assert_eq!(
        (HEAP_TRACE_RVA, NEED_FLAG_RVA, NEED_FLAG_BYTES),
        (0x4000, 0x4208, 4)
    );
    assert_eq!(
        &provider.image()[CODE_SIZE..CODE_SIZE + HEAP_TRACE_BYTES as usize],
        &[0; 520]
    );
    assert_eq!(
        &provider.image()[CODE_SIZE + HEAP_TRACE_BYTES as usize..],
        &1_u32.to_le_bytes()
    );
    let view = open_executable(provider.image()).unwrap();
    let runtime = view.runtime_plan().unwrap();
    assert_eq!(runtime.segments.len(), 2);
    assert_eq!(
        (runtime.segments[0].flags.raw(), runtime.segments[0].vaddr),
        (5, 0)
    );
    assert_eq!(
        (runtime.segments[1].flags.raw(), runtime.segments[1].vaddr),
        (6, 0x4000)
    );
    assert!(runtime.tls.is_none());
    assert!(runtime.relro.is_empty());
    let metadata = inspect_sce(&view, &Default::default(), &Default::default()).unwrap();
    let symbols = metadata.metadata().unwrap().link().symbols();
    assert_eq!(symbols.len(), 67);
    assert_eq!((NEED_FLAG_SYMBOL, LIBC_ALIAS_SYMBOL), (18, 22));
    for (row, symbol) in symbols.iter().enumerate().take(22).skip(18) {
        assert_eq!(symbol.symbol_type(), SymbolType::OBJECT);
        assert_eq!((symbol.value, symbol.size), (0x4208, 4));
        assert_eq!(function_for_symbol(row), None);
    }
    for (_, _, row, suffix) in SCOPES {
        let symbol = &symbols[row];
        let mut name = b"eLdDw6l0-bU".to_vec();
        name.extend_from_slice(suffix);
        assert_eq!(symbol.name.as_ref().unwrap().as_bytes(), name);
        assert_eq!(symbol.binding(), SymbolBinding::GLOBAL);
        assert_eq!(symbol.visibility(), SymbolVisibility::PROTECTED);
        assert_eq!(symbol.symbol_type(), SymbolType::FUNC);
        assert_eq!((symbol.value, symbol.size), (0x200, 8));
        assert_eq!(function_for_symbol(row), Some(function));
    }
    for row in [0, 67, usize::MAX] {
        assert_eq!(function_for_symbol(row), None);
    }
}

#[test]
fn snprintf_exact_four_qualified_identities_link_in_both_descriptor_domains() {
    let provider = factory();
    for (module, library, row, _) in SCOPES {
        for dynlib in [false, true] {
            for encoding in [
                DescriptorEncoding::Classic,
                DescriptorEncoding::Variant1Observed,
            ] {
                let mut original = consumer(module, library);
                original.dynlib = dynlib;
                original.encoding = encoding;
                let bytes = original.build().unwrap();
                let plan = link(&bytes, &provider).unwrap();
                assert_eq!(plan.patch_count(), 1);
                let patch = plan.patches()[0];
                assert_eq!(patch.value(), PROVIDER_BIAS + 0x200);
                assert!(
                    matches!(patch.resolution(), Resolution::Definition(reference)
                    if reference.module.index() == 1 && reference.symbol == row)
                );
            }
        }
    }
}

#[test]
fn snprintf_identity_refuses_other_versions_names_and_object_imports() {
    let provider = factory();
    for (module, library, _, _) in SCOPES {
        for (major, minor, version) in [(2, 1, 1), (1, 2, 1), (1, 1, 2)] {
            let mut original = consumer(module, library);
            (original.major, original.minor, original.version) = (major, minor, version);
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::VersionMismatch { .. })
            ));
        }
        let mut original = consumer(module, library);
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
        (b"libc".as_slice(), b"libc".as_slice()),
        (b"LibcInternal".as_slice(), b"LibcInternalExt".as_slice()),
        (
            b"libSceLibcInternal".as_slice(),
            b"libSceLibcInternalExt".as_slice(),
        ),
        (
            b"libScelibSceLibcInternal".as_slice(),
            b"libSceLibcInternal".as_slice(),
        ),
    ] {
        assert!(matches!(
            link(&consumer(module, library).build().unwrap(), &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
}
