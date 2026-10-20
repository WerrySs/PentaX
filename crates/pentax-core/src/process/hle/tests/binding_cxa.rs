// SPDX-License-Identifier: GPL-2.0-or-later

//! Exact C++ exit-registration identity; no memory/string handler is invoked.
use super::super::super::provider::{
    HEAP_TRACE_RVA, LIBC_ALIAS_SYMBOL, NEED_FLAG_RVA, NEED_FLAG_SYMBOL, function_for_symbol,
};
use super::*;

const SCOPES: [(&[u8], &[u8], usize); 4] = [
    (b"LibcInternal", b"LibcInternal", 16),
    (b"libSceLibcInternal", b"libSceLibcInternal", 61),
    (b"libSceLibcInternal", b"LibcInternal", 62),
    (b"LibcInternal", b"libSceLibcInternal", 63),
];

fn consumer(module: &'static [u8], library: &'static [u8]) -> Consumer {
    let mut consumer = minimal_consumer();
    consumer.module_name = module;
    consumer.library_name = library;
    consumer.imports = vec![Import::function(KnownFunction::CxaAtExit)];
    consumer
}

#[test]
fn cxa_identity_appends_one_marker_without_moving_existing_services_or_data() {
    let factory = providers();
    let old_functions = [
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
    ];
    assert_eq!(KnownFunction::LIBC.len(), 13);
    assert_eq!(KnownFunction::LIBC_FUNCTIONS.len(), 15);
    assert_eq!(KnownFunction::ALL.len(), 17);
    for (index, function) in old_functions.into_iter().enumerate() {
        assert_eq!(KnownFunction::ALL[index], function);
        assert_eq!(function.index(), index);
        assert_eq!(function.service_id(), 0x5058_4801 + index as u32);
        let rva = 0x100 + index * 0x10;
        assert_eq!(&factory.image()[rva..rva + 8], &stub(function));
    }
    let function = KnownFunction::CxaAtExit;
    assert_eq!(function.nid(), b"tsvEmnenz48");
    assert_eq!(function.index(), 15);
    assert_eq!(function.service_id(), 0x5058_4810);
    assert_eq!(function.string_operation(), None);
    assert_eq!(&factory.image()[0x1f0..0x1f8], &stub(function));
    assert_eq!((HEAP_TRACE_RVA, NEED_FLAG_RVA), (0x4000, 0x4208));
    assert_eq!((NEED_FLAG_SYMBOL, LIBC_ALIAS_SYMBOL), (18, 22));
    let view = open_executable(factory.image()).unwrap();
    let metadata = inspect_sce(&view, &Limits::DEFAULT, &SceLimits::DEFAULT).unwrap();
    let symbols = metadata.metadata().unwrap().link().symbols();
    assert_eq!(symbols.len(), 67);
    assert_eq!(SYMBOLS_AT + symbols.len() * SYMBOL_SIZE, 0xf48);
    for (_, _, row) in SCOPES {
        let symbol = &symbols[row];
        assert_eq!(symbol.binding(), SymbolBinding::GLOBAL);
        assert_eq!(symbol.visibility(), SymbolVisibility::PROTECTED);
        assert_eq!(symbol.symbol_type(), SymbolType::FUNC);
        assert_eq!((symbol.value, symbol.size), (0x1f0, 8));
        assert_eq!(function_for_symbol(row), Some(function));
    }
    assert_eq!(function_for_symbol(17), Some(KnownFunction::Snprintf));
    for row in 18..22 {
        assert_eq!(function_for_symbol(row), None);
    }
    assert_eq!(function_for_symbol(67), None);
}

#[test]
fn cxa_exact_four_scopes_bind_one_canonical_site_in_both_descriptor_domains() {
    let factory = providers();
    for (module, library, row) in SCOPES {
        for dynlib in [false, true] {
            for encoding in [
                DescriptorEncoding::Classic,
                DescriptorEncoding::Variant1Observed,
            ] {
                let mut input = consumer(module, library);
                input.dynlib = dynlib;
                input.encoding = encoding;
                let bytes = input.build().unwrap();
                let plan = plan_one(&bytes, &factory).unwrap();
                assert_eq!(plan.provider_bindings(), 1);
                let patch = plan.link_plan().patches()[0];
                assert_eq!(patch.value(), PROVIDER_BIAS + 0x1f0);
                assert!(
                    matches!(patch.resolution(), Resolution::Definition(reference)
                    if reference.module.index() == 1 && reference.symbol == row)
                );
                let site = &plan.sites()[15];
                assert_eq!(site.function, KnownFunction::CxaAtExit);
                assert_eq!(
                    (site.entry, site.trap, site.resume),
                    (
                        PROVIDER_BIAS + 0x1f0,
                        PROVIDER_BIAS + 0x1f5,
                        PROVIDER_BIAS + 0x1f7
                    )
                );
                let prepared = prepare_hle_session(&plan, &SessionLoadLimits::default()).unwrap();
                assert_eq!(prepared.provider_bindings(), 1);
                assert_eq!(prepared.applied_relocations(), 1);
            }
        }
    }
}

#[test]
fn cxa_identity_does_not_normalize_other_scopes_versions_nids_or_object_types() {
    let factory = providers();
    for (module, library, row) in SCOPES {
        for (major, minor, version) in [(2, 1, 1), (1, 2, 1), (1, 1, 2)] {
            let mut input = consumer(module, library);
            (input.major, input.minor, input.version) = (major, minor, version);
            let bytes = input.build().unwrap();
            assert!(matches!(
                plan_one(&bytes, &factory),
                Err(HleError::Link(SymbolicError::VersionMismatch { .. }))
            ));
        }
        let mut input = consumer(module, library);
        input.imports[0].symbol_type = 1;
        input.imports[0].relocation = 6;
        let bytes = input.build().unwrap();
        assert!(
            matches!(plan_one(&bytes, &factory), Err(HleError::Link(SymbolicError::UnsupportedSymbol {
            reason: UnsupportedSymbol::TypeMismatch, symbol,
        })) if symbol.module.index() == 1 && symbol.symbol == row)
        );
        let mut input = consumer(module, library);
        input.imports[0].nid = *b"AAAAAAAAAAA";
        assert!(matches!(
            plan_one(&input.build().unwrap(), &factory),
            Err(HleError::Link(SymbolicError::MissingProvider { .. }))
        ));
    }
    let unreviewed: [(&[u8], &[u8]); 5] = [
        (b"libc", b"libc"),
        (b"LibcInternal", b"LibcInternalExt"),
        (b"libSceLibcInternal", b"libSceLibcInternalExt"),
        (b"libScelibcInternal", b"LibcInternal"),
        (b"libScelibSceLibcInternal", b"libSceLibcInternal"),
    ];
    for (module, library) in unreviewed {
        assert!(matches!(
            plan_one(&consumer(module, library).build().unwrap(), &factory),
            Err(HleError::Link(SymbolicError::MissingProvider { .. }))
        ));
    }
}
