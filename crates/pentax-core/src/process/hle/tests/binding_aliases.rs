// SPDX-License-Identifier: GPL-2.0-or-later

//! Original consumers exercise finite alias identities through real GOT calls.

use super::super::super::fixture::{Asm, DATA, FixtureLimits, RESULTS, launch};
use super::super::super::provider::{
    LIBC_ALIAS_SYMBOL, NEED_FLAG_RVA, NEED_FLAG_SYMBOL, NEED_FLAG_SYMBOLS, function_for_symbol,
};
use super::*;

const ALIAS_SCOPES: [(&[u8], &[u8], &[u8]); 3] = [
    (b"libSceLibcInternal", b"libSceLibcInternal", b"#E#C"),
    (b"libSceLibcInternal", b"LibcInternal", b"#B#C"),
    (b"LibcInternal", b"libSceLibcInternal", b"#E#B"),
];

fn one_import(module: &'static [u8], library: &'static [u8]) -> Consumer {
    let mut consumer = minimal_consumer();
    consumer.module_name = module;
    consumer.library_name = library;
    consumer.imports = vec![Import::function(KnownFunction::Memcpy)];
    consumer
}

fn word(memory: &crate::memory::GuestMemory, address: u64) -> u64 {
    let mut bytes = [0; 8];
    memory.read(address, &mut bytes).unwrap();
    u64::from_le_bytes(bytes)
}

#[test]
fn finite_libc_alias_metadata_preserves_original_markers_and_data_rows() {
    let provider = providers();
    let view = open_executable(provider.image()).unwrap();
    let inspection = inspect_sce(&view, &Limits::DEFAULT, &SceLimits::DEFAULT).unwrap();
    let symbols = inspection.metadata().unwrap().link().symbols();
    assert_eq!(provider.function_count(), 17);
    assert_eq!(LIBC_ALIAS_SYMBOL, 22);
    for (position, function) in KnownFunction::LIBC_FUNCTIONS.into_iter().enumerate() {
        for (alias, &(_, _, suffix)) in ALIAS_SCOPES.iter().enumerate() {
            let index = LIBC_ALIAS_SYMBOL + position * 3 + alias;
            let symbol = &symbols[index];
            let mut expected = function.nid().to_vec();
            expected.extend_from_slice(suffix);
            assert_eq!(symbol.name.as_ref().unwrap().as_bytes(), expected);
            assert_eq!(symbol.binding(), SymbolBinding::GLOBAL);
            assert_eq!(symbol.visibility(), SymbolVisibility::PROTECTED);
            assert_eq!(symbol.symbol_type(), SymbolType::FUNC);
            assert_eq!(symbol.size, 8);
            assert_eq!(symbol.value, symbols[function.index() + 1].value);
            assert_eq!(function_for_symbol(index), Some(function));
        }
    }
    for (index, symbol) in symbols
        .iter()
        .enumerate()
        .skip(NEED_FLAG_SYMBOL)
        .take(NEED_FLAG_SYMBOLS)
    {
        assert_eq!(symbol.symbol_type(), SymbolType::OBJECT);
        assert_eq!((symbol.value, symbol.size), (NEED_FLAG_RVA, 4));
        assert_eq!(function_for_symbol(index), None);
    }
    for index in [0, 67, usize::MAX] {
        assert_eq!(function_for_symbol(index), None);
    }
}

#[test]
fn finite_libc_aliases_execute_all_thirteen_existing_handlers_at_canonical_sites() {
    const SOURCE: u64 = DATA + 0x100;
    const NEEDLE: u64 = DATA + 0x140;
    const OUTPUT: u64 = DATA + 0x200;
    let calls = [
        (KnownFunction::Memcpy, [OUTPUT, SOURCE, 4], OUTPUT),
        (KnownFunction::Memmove, [OUTPUT + 1, OUTPUT, 4], OUTPUT + 1),
        (
            KnownFunction::Memset,
            [OUTPUT + 0x20, 0x135a, 3],
            OUTPUT + 0x20,
        ),
        (KnownFunction::Memcmp, [SOURCE, SOURCE, 5], 0),
        (KnownFunction::Strcmp, [SOURCE, SOURCE, 0], 0),
        (KnownFunction::Strncmp, [SOURCE, SOURCE, 3], 0),
        (KnownFunction::Strlen, [SOURCE, u64::MAX, u64::MAX], 4),
        (
            KnownFunction::Strcpy,
            [OUTPUT + 0x40, SOURCE, 0],
            OUTPUT + 0x40,
        ),
        (
            KnownFunction::Strncpy,
            [OUTPUT + 0x60, SOURCE, 2],
            OUTPUT + 0x60,
        ),
        (
            KnownFunction::Strcat,
            [OUTPUT + 0x40, NEEDLE, 0],
            OUTPUT + 0x40,
        ),
        (KnownFunction::Strchr, [SOURCE, 0x1_006f, 0], SOURCE + 1),
        (KnownFunction::Strrchr, [SOURCE, b'p' as u64, 0], SOURCE),
        (KnownFunction::Strstr, [SOURCE, NEEDLE, 0], SOURCE + 1),
    ];
    for (alias, &(module, library, _)) in ALIAS_SCOPES.iter().enumerate() {
        let mut asm = Asm::new(CODE);
        for (slot, &(_, arguments, _)) in calls.iter().enumerate() {
            for (register, argument) in [7, 6, 2].into_iter().zip(arguments) {
                asm.mov(register, argument);
            }
            asm.call_slot(slot);
            asm.store_rax(RESULTS as u32 + slot as u32 * 8);
        }
        asm.int3();
        let halt = CONSUMER_BIAS + asm.rva() as u64;
        let mut consumer = Consumer::importing(vec![(CODE, asm.bytes)]);
        consumer.module_name = module;
        consumer.library_name = library;
        consumer.imports = KnownFunction::LIBC.map(Import::function).to_vec();
        let bytes = consumer.build().unwrap();
        let original = bytes.clone();
        let provider = providers();
        let plan = plan_one(&bytes, &provider).unwrap();
        assert_eq!(plan.provider_bindings(), 13);
        assert_eq!(plan.sites().len(), 17);
        for (slot, patch) in plan.link_plan().patches().iter().enumerate() {
            assert_eq!(patch.value(), stub_entry(KnownFunction::LIBC[slot]));
            assert!(matches!(
                patch.resolution(),
                Resolution::Definition(symbol)
                    if symbol.module.index() == 1 && symbol.symbol == LIBC_ALIAS_SYMBOL + slot * 3 + alias
            ));
        }
        let mut launched = launch(&bytes, &FixtureLimits::default()).unwrap();
        launched.gateway.memory.write(SOURCE, b"port\0").unwrap();
        launched.gateway.memory.write(NEEDLE, b"or\0").unwrap();
        launched
            .gateway
            .memory
            .write(OUTPUT, &[0xcc; 0x80])
            .unwrap();
        assert_eq!(launched.gateway.run(), Ok(halt));
        assert_eq!((launched.bindings, launched.applied), (13, 13));
        for (slot, &(function, _, expected)) in calls.iter().enumerate() {
            assert_eq!(
                word(&launched.gateway.memory, DATA + RESULTS + slot as u64 * 8),
                expected
            );
            assert_eq!(launched.gateway.budget.calls(function), 1);
        }
        assert_eq!(launched.gateway.budget.attempts(), 13);
        assert_eq!(launched.gateway.budget.completed(), 13);
        let mut copied = [0; 6];
        launched.gateway.memory.read(OUTPUT, &mut copied).unwrap();
        assert_eq!(&copied, b"pport\xcc");
        let mut filled = [0; 4];
        launched
            .gateway
            .memory
            .read(OUTPUT + 0x20, &mut filled)
            .unwrap();
        assert_eq!(filled, [b'Z', b'Z', b'Z', 0xcc]);
        let mut appended = [0; 7];
        launched
            .gateway
            .memory
            .read(OUTPUT + 0x40, &mut appended)
            .unwrap();
        assert_eq!(&appended, b"portor\0");
        let mut truncated = [0; 3];
        launched
            .gateway
            .memory
            .read(OUTPUT + 0x60, &mut truncated)
            .unwrap();
        assert_eq!(&truncated, b"po\xcc");
        assert_eq!(bytes, original);
    }
}

#[test]
fn finite_libc_aliases_keep_exact_versions_types_and_reject_unreviewed_scopes() {
    let provider = providers();
    for (alias, &(module, library, _)) in ALIAS_SCOPES.iter().enumerate() {
        for (major, minor, version) in [(2, 1, 1), (1, 2, 1), (1, 1, 2)] {
            let mut consumer = one_import(module, library);
            (consumer.major, consumer.minor, consumer.version) = (major, minor, version);
            let bytes = consumer.build().unwrap();
            assert!(matches!(
                plan_one(&bytes, &provider).unwrap_err(),
                HleError::Link(SymbolicError::VersionMismatch {
                    available: DescriptorVersions {
                        module_major: 1,
                        module_minor: 1,
                        library: 1,
                    },
                    ..
                })
            ));
        }
        let mut consumer = one_import(module, library);
        consumer.imports[0].symbol_type = 1;
        consumer.imports[0].relocation = 6;
        let bytes = consumer.build().unwrap();
        assert!(matches!(
            plan_one(&bytes, &provider).unwrap_err(),
            HleError::Link(SymbolicError::UnsupportedSymbol {
                reason: UnsupportedSymbol::TypeMismatch,
                symbol,
            }) if symbol.module.index() == 1 && symbol.symbol == LIBC_ALIAS_SYMBOL + alias
        ));
    }
    let unreviewed: [(&[u8], &[u8]); 4] = [
        (b"libc", b"libSceLibcInternal"),
        (b"libSceLibcInternal", b"libScelibcInternal"),
        (b"libScelibSceLibcInternal", b"LibcInternal"),
        (b"libSceLibcInternal", b"LibcInternalExt"),
    ];
    for (module, library) in unreviewed {
        let bytes = one_import(module, library).build().unwrap();
        assert!(matches!(
            plan_one(&bytes, &provider).unwrap_err(),
            HleError::Link(SymbolicError::MissingProvider { .. })
        ));
    }
    // The finite libc aliases do not widen the two trace-query identities.
    for (module, library) in [
        (
            b"libSceLibcInternal".as_slice(),
            b"LibcInternalExt".as_slice(),
        ),
        (
            b"LibcInternal".as_slice(),
            b"libSceLibcInternalExt".as_slice(),
        ),
    ] {
        let mut consumer = one_import(module, library);
        consumer.imports = vec![Import::function(KnownFunction::HeapGetTraceInfo)];
        let bytes = consumer.build().unwrap();
        assert!(matches!(
            plan_one(&bytes, &provider).unwrap_err(),
            HleError::Link(SymbolicError::MissingProvider { .. })
        ));
    }
}

#[test]
fn finite_libc_alias_competitors_remain_ambiguous_without_provider_priority() {
    let provider = providers();
    let consumer = one_import(b"libSceLibcInternal", b"libSceLibcInternal")
        .build()
        .unwrap();
    let supplied = provider.image().to_vec();
    assert!(matches!(
        plan_with(
            &[(&consumer, CONSUMER_BIAS), (&supplied, SUPPLIED_BIAS)],
            &provider,
        )
        .unwrap_err(),
        HleError::Link(SymbolicError::AmbiguousProvider { first, second, .. })
            if first.module.index() == 1 && second.module.index() == 2
    ));
}
