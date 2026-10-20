// SPDX-License-Identifier: GPL-2.0-only

//! Original metadata and placement-bound guest words; no title data or CPU entry.
use super::*;
use crate::process::hle::fixture::{CODE, CONSUMER_BIAS, Consumer, Import};
use pentax_linker::{
    DescriptorEncoding, ModuleInput, Resolution, SymbolBinding, SymbolType, SymbolVisibility,
    SymbolicError, UnsupportedSymbol, inspect_sce, plan_symbolic,
};
use pentax_loader::open_executable;

const BIAS: u64 = 0x3000000;
const EXACT_SCOPES: [(&[u8], &[u8], &[u8]); 5] = [
    (b"Json2", b"Json2", b"#B#B"),
    (b"libSceJson2", b"libSceJson2", b"#C#C"),
    (b"libSceJson2", b"Json2", b"#B#C"),
    (b"Json2", b"libSceJson2", b"#C#B"),
    (b"libSceJson", b"libSceJson2", b"#C#D"),
];
// Independent public identity/role expectations plus the first appended row.
// The fifth scope is admitted only for the trusted observed public NID rows.
const PRIMITIVE_CASES: [(&[u8; 11], JsonFunction, bool, usize); 32] = [
    (b"UeuWT+yNdCQ", JsonFunction::ValueBoolCtor, true, 15),
    (b"0lLK8+kDqmE", JsonFunction::ValueIntCtor, true, 20),
    (b"CbrT3dwDILo", JsonFunction::ValueTypeCtor, false, 25),
    (b"5yHuiWXo2gg", JsonFunction::ValueSetBool, true, 29),
    (b"QxVVYhP-mvg", JsonFunction::ValueSetInt, true, 34),
    (b"SIe1ZmW7e7s", JsonFunction::ValueSetUInt, true, 39),
    (b"IKQimvG9Wqs", JsonFunction::ValueSetType, true, 44),
    (b"SHtAad20YYM", JsonFunction::ValueGetType, true, 49),
    (b"zTwZdI8AZ5Y", JsonFunction::ValueGetBoolean, true, 54),
    (b"DIxvoy7Ngvk", JsonFunction::ValueGetInteger, true, 59),
    (b"3qrge7L-AU4", JsonFunction::ValueGetReal, true, 64),
    (b"fSb2oQTNrgA", JsonFunction::ValueCopyCtor, true, 69),
    (b"4zrm6VrgIAw", JsonFunction::ValueAssign, true, 74),
    (b"WTtYf+cNnXI", JsonFunction::ValueDtor, true, 79),
    (b"0eUrW9JAxM0", JsonFunction::ValueDtor, false, 84),
    (b"FIjXN2TkuTs", JsonFunction::ValueClearMethod, false, 88),
    (b"RBw+4NukeGQ", JsonFunction::ValueCount, true, 92),
    (b"OcAgPxcq5Vk", JsonFunction::MemAllocatorDtor, true, 97),
    (b"sOmU4vnx3s0", JsonFunction::ValueDoubleCtor, true, 102),
    (b"BSmWDIkV4w4", JsonFunction::ValueSetDouble, true, 107),
    (b"qSmqLXXCPas", JsonFunction::StringCtor, true, 112),
    (b"9KUZFjI1IxA", JsonFunction::StringCStringCtor, true, 117),
    (b"cG1VE2HMl6c", JsonFunction::StringDtor, true, 122),
    (b"cn9svYGWKDQ", JsonFunction::StringAssign, false, 127),
    (b"L1KAkYWml-M", JsonFunction::StringCStr, true, 131),
    (b"EUH+EmT-v9E", JsonFunction::StringLength, true, 136),
    (b"b9V6fmppLXY", JsonFunction::ValueCStringCtor, true, 141),
    (b"sZIoMRGO+jk", JsonFunction::ValueStringCtor, true, 146),
    (b"6l3Bv2gysNc", JsonFunction::ValueSetString, true, 151),
    (b"epJ6x2LV0kU", JsonFunction::ValueGetString, true, 156),
    (b"Ncel8t2Rrpc", JsonFunction::ValueToString, true, 161),
    (b"R7FDWtcN6f8", JsonFunction::ValueSerialize, true, 166),
];

fn factory(bias: u64) -> GeneratedJsonProvider {
    build_json_provider(ProviderPlacement { load_bias: bias }, IMAGE_BYTES as u64).unwrap()
}
fn consumer(module: &'static [u8], library: &'static [u8]) -> Consumer {
    let mut consumer = Consumer::importing(vec![(CODE, vec![0xcc])]);
    consumer.module_name = module;
    consumer.library_name = library;
    consumer.imports = vec![Import {
        nid: *CONSTRUCTOR_NID,
        symbol_type: 2,
        binding: 1,
        relocation: 7,
        addend: 0,
    }];
    consumer
}
fn link<'a>(
    input: &'a [u8],
    provider: &'a GeneratedJsonProvider,
) -> Result<pentax_linker::SymbolicLinkPlan<'a>, SymbolicError> {
    plan_symbolic(
        &[
            ModuleInput::new(open_executable(input).unwrap(), CONSUMER_BIAS),
            ModuleInput::new(
                open_executable(provider.image()).unwrap(),
                provider.load_bias(),
            ),
        ],
        &Default::default(),
    )
}

#[test]
fn json_provider_has_typed_exports_and_no_terminal_symbol_or_elf_initializer() {
    let provider = factory(BIAS);
    assert_eq!(provider.function_count(), 224);
    assert_eq!(provider.mapped_bytes(), 2 * PAGE_SIZE);
    assert_eq!(provider.load_bias(), BIAS);
    let view = open_executable(provider.image()).unwrap();
    let runtime = view.runtime_plan().unwrap();
    assert_eq!(runtime.metadata.entry, 0);
    assert!(runtime.tls.is_none());
    assert!(runtime.relro.is_empty());
    assert_eq!(runtime.segments.len(), 2);
    let code = &runtime.segments[0];
    let table = &runtime.segments[1];
    assert_eq!(
        (
            code.vaddr,
            code.mem_size,
            code.flags.raw(),
            code.file_bytes.len()
        ),
        (0, PAGE_SIZE, 5, CODE_FILE_BYTES)
    );
    assert_eq!(
        (
            table.vaddr,
            table.mem_size,
            table.flags.raw(),
            table.file_bytes.len()
        ),
        (PAGE_SIZE, PAGE_SIZE, 4, 104)
    );
    assert_eq!(runtime.metadata.program_headers[2].kind, 2);
    assert!(
        !runtime
            .dynamic
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .any(|entry| matches!(
                entry.tag,
                7 | 8 | 9 | 12 | 17 | 18 | 19 | 23 | 25 | 26 | 27 | 28
            ))
    );
    let metadata = inspect_sce(&view, &Default::default(), &Default::default()).unwrap();
    let metadata = metadata.metadata().unwrap();
    let descriptors = metadata.descriptors().decoded().unwrap();
    assert_eq!(descriptors.module_info().len(), 3);
    assert_eq!(descriptors.export_libraries().len(), 2);
    assert!(descriptors.needed_modules().is_empty());
    assert!(descriptors.import_libraries().is_empty());
    for (index, module) in descriptors.module_info().iter().enumerate() {
        assert_eq!(module.name().as_bytes(), MODULES[index]);
        assert_eq!(
            (
                module.version_major(),
                module.version_minor(),
                module.id().value()
            ),
            (1, 1, index as u16 + 1)
        );
    }
    for (index, library) in descriptors.export_libraries().iter().enumerate() {
        assert_eq!(library.name().as_bytes(), LIBRARIES[index]);
        assert_eq!(
            (library.version(), library.id().value()),
            (1, index as u16 + 1)
        );
    }
    let symbols = metadata.link().symbols();
    assert_eq!(symbols.len(), 225);
    assert_eq!(
        (symbols[0].info, symbols[0].value, symbols[0].size),
        (0, 0, 0)
    );
    for (symbol, (_, _, suffix)) in symbols[1..].iter().zip(EXACT_SCOPES) {
        let mut expected = CONSTRUCTOR_NID.to_vec();
        expected.extend_from_slice(suffix);
        assert_eq!(symbol.name.as_ref().unwrap().as_bytes(), expected);
        assert_eq!(symbol.binding(), SymbolBinding::GLOBAL);
        assert_eq!(symbol.visibility(), SymbolVisibility::PROTECTED);
        assert_eq!(symbol.symbol_type(), SymbolType::FUNC);
        assert_eq!((symbol.value, symbol.size), (0x100, 8));
    }
    for symbol in 1..=5 {
        assert_eq!(provider.export_marker(symbol), Some(provider.markers()[0]));
    }
    for symbol in 6..=14 {
        assert_eq!(provider.export_marker(symbol), Some(provider.markers()[2]));
    }
    for symbol in [0, 225, usize::MAX] {
        assert!(provider.export_marker(symbol).is_none());
    }
    assert!(!symbols.iter().any(|symbol| symbol.value == 0x110));
    assert!(metadata.link().rela().is_empty());
    assert!(metadata.link().plt_rela().is_empty());
    assert_eq!(provider.markers().len(), 61);
    for marker in provider.markers() {
        assert_eq!(
            &provider.image()[marker.rva as usize..marker.rva as usize + 8],
            &marker.bytes()
        );
    }
    assert_eq!(
        provider.markers()[0],
        JsonMarker {
            function: JsonFunction::MemAllocatorCtor,
            service_id: 0x5058_4a01,
            rva: 0x100,
        }
    );
    assert_eq!(
        provider.markers()[1],
        JsonMarker {
            function: JsonFunction::VirtualCall,
            service_id: 0x5058_4a02,
            rva: 0x110,
        }
    );
    assert_eq!(
        provider.markers()[2],
        JsonMarker {
            function: JsonFunction::ValueCtor,
            service_id: 0x5058_4a03,
            rva: 0x120,
        }
    );
    assert_eq!(
        (CODE_FILE_BYTES, SYMBOLS_AT, provider.image().len()),
        (0x3800, 0x1800, 0x3868)
    );
    assert_eq!((DYNAMIC_AT, STRINGS_AT), (0x600, 0x800));
    assert_eq!(STRINGS_AT + STRINGS_BYTES, 0x161e);
    assert_eq!(SYMBOLS_AT + symbols.len() * SYMBOL_BYTES, 0x2d18);
    for (row, symbol) in symbols.iter().enumerate().skip(1) {
        let marker = provider.export_marker(row).unwrap();
        assert_eq!(symbol.binding(), SymbolBinding::GLOBAL);
        assert_eq!(symbol.visibility(), SymbolVisibility::PROTECTED);
        assert_eq!(symbol.symbol_type(), SymbolType::FUNC);
        assert_eq!((symbol.value, symbol.size), (marker.rva, 8));
        assert_ne!(marker.function, JsonFunction::VirtualCall);
    }
}

#[test]
fn json_initializer_constructor_preserves_exact_scopes_and_separate_lifecycle_identity() {
    let provider = factory(BIAS);
    let marker = provider.markers()[34];
    assert_eq!(
        marker,
        JsonMarker {
            function: JsonFunction::InitializerCtor,
            service_id: 0x5058_4a23,
            rva: 0x320,
        }
    );
    let view = open_executable(provider.image()).unwrap();
    let inspected = inspect_sce(&view, &Default::default(), &Default::default()).unwrap();
    let symbols = inspected.metadata().unwrap().link().symbols();
    for (row, module, library, name) in [
        (
            171,
            b"Json2".as_slice(),
            b"Json2".as_slice(),
            b"cK6bYHf-Q5E#B#B".as_slice(),
        ),
        (
            172,
            b"libSceJson".as_slice(),
            b"libSceJson2".as_slice(),
            b"cK6bYHf-Q5E#C#D".as_slice(),
        ),
    ] {
        assert_eq!(symbols[row].name.as_ref().unwrap().as_bytes(), name);
        assert_eq!(provider.export_marker(row), Some(marker));
        assert_eq!((symbols[row].value, symbols[row].size), (0x320, 8));
        let mut original = consumer(module, library);
        original.imports[0].nid = *b"cK6bYHf-Q5E";
        let bytes = original.build().unwrap();
        let source = link(&bytes, &provider).unwrap();
        assert!(
            matches!(source.patches()[0].resolution(), Resolution::Definition(reference)
            if reference.module.index() == 1 && reference.symbol == row)
        );
        assert_eq!(source.patches()[0].value(), BIAS + 0x320);
        for (major, minor, version) in [(2, 1, 1), (1, 2, 1), (1, 1, 2)] {
            let mut wrong = original.clone();
            (wrong.major, wrong.minor, wrong.version) = (major, minor, version);
            assert!(matches!(
                link(&wrong.build().unwrap(), &provider),
                Err(SymbolicError::VersionMismatch { .. })
            ));
        }
        let mut object = original.clone();
        object.imports[0].symbol_type = 1;
        object.imports[0].relocation = 6;
        assert!(matches!(
            link(&object.build().unwrap(), &provider),
            Err(SymbolicError::UnsupportedSymbol {
                reason: UnsupportedSymbol::TypeMismatch,
                ..
            })
        ));
        let mut unported = original.clone();
        unported.imports[0].nid = *b"IXW-z8pggfg";
        assert!(matches!(
            link(&unported.build().unwrap(), &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    for (module, library) in [
        (b"libSceJson2".as_slice(), b"libSceJson2".as_slice()),
        (b"libSceJson2".as_slice(), b"Json2".as_slice()),
        (b"Json2".as_slice(), b"libSceJson2".as_slice()),
        (b"libSceJson".as_slice(), b"Json2".as_slice()),
    ] {
        let mut original = consumer(module, library);
        original.imports[0].nid = *b"cK6bYHf-Q5E";
        assert!(matches!(
            link(&original.build().unwrap(), &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    // The old last String marker and all old rows retain their exact identities.
    assert_eq!(
        provider.markers()[33],
        JsonMarker {
            function: JsonFunction::ValueSerialize,
            service_id: 0x5058_4a22,
            rva: 0x310,
        }
    );
    assert_eq!(
        symbols[170].name.as_ref().unwrap().as_bytes(),
        b"R7FDWtcN6f8#C#D"
    );
    assert_eq!(symbols[170].value, 0x310);
}

#[test]
fn json_container_manifest_appends_exact_public_functions_and_refuses_unobserved_authority() {
    let expected = [
        (b"OJPTonqdg0I", JsonFunction::ObjectCtor),
        (b"a+W7HHlwpBs", JsonFunction::ObjectCopyCtor),
        (b"5JmzZt8twAo", JsonFunction::ObjectDtor),
        (b"ERuf9y0DY84", JsonFunction::ObjectIndex),
        (b"JP-PtKMiI1E", JsonFunction::ArrayCtor),
        (b"HJ8GpRT1aiw", JsonFunction::ArrayDtor),
        (b"zQtLRTqceMY", JsonFunction::ArrayPushBack),
        (b"3xUXnmUkXfo", JsonFunction::ValueObjectCtor),
        (b"iZeYfOxtMRg", JsonFunction::ValueArrayCtor),
        (b"dFCphqnd+a4", JsonFunction::ValueSetObject),
        (b"IlsmvBtMkak", JsonFunction::ValueGetObject),
        (b"ONT8As5R1ug", JsonFunction::ValueGetArray),
        (b"HwDt5lD9Bfo", JsonFunction::ValueIndexString),
        (b"XlWbvieLj2M", JsonFunction::ValueIndexUInt),
        (b"wLsJlmgEIaI", JsonFunction::ValueReferValueKey),
        (b"bcH5EnFE2xY", JsonFunction::ArrayBegin),
        (b"WXF2ihRF+B8", JsonFunction::ArrayEnd),
        (b"5AZPp99ogrc", JsonFunction::ArrayIteratorNotEqual),
        (b"wcgr5mte7T8", JsonFunction::ArrayIteratorDereference),
        (b"w5+VCznos5E", JsonFunction::ArrayIteratorIncrement),
        (b"9yLjn46Ypfs", JsonFunction::ArrayIteratorDtor),
        (b"S5JxQnoGF3E", JsonFunction::ParserParse),
    ];
    let provider = factory(BIAS);
    for (index, (nid, function)) in expected.into_iter().enumerate() {
        let marker = provider.markers()[35 + index];
        assert_eq!(
            (marker.function, marker.service_id, marker.rva),
            (
                function,
                0x5058_4a24 + index as u32,
                0x330 + index as u64 * 16
            )
        );
        for (scope, module, library) in [
            (0, b"Json2".as_slice(), b"Json2".as_slice()),
            (1, b"libSceJson".as_slice(), b"libSceJson2".as_slice()),
        ] {
            let mut original = consumer(module, library);
            original.imports[0].nid = *nid;
            let bytes = original.build().unwrap();
            let source = link(&bytes, &provider).unwrap();
            assert_eq!(source.patches()[0].value(), BIAS + marker.rva);
            assert!(
                matches!(source.patches()[0].resolution(), Resolution::Definition(reference)
                if reference.module.index() == 1 && reference.symbol == 173 + index * 2 + scope)
            );
            assert_eq!(
                provider.export_marker(173 + index * 2 + scope),
                Some(marker)
            );
            original.minor = 2;
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::VersionMismatch { .. })
            ));
            original.minor = 1;
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
        let mut unobserved = consumer(b"libSceJson2", b"libSceJson2");
        unobserved.imports[0].nid = *nid;
        assert!(matches!(
            link(&unobserved.build().unwrap(), &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    assert_eq!(provider.export_marker(172).unwrap().rva, 0x320);
    assert_eq!(provider.export_marker(170).unwrap().rva, 0x310);
    assert_eq!(provider.table_rva(), PAGE_SIZE);
    assert_eq!(provider.empty_cstr_rva(), PAGE_SIZE + 96);
}

#[test]
fn json_table_contains_two_zero_words_and_six_checked_guest_terminal_addresses() {
    let first = factory(BIAS);
    let second = factory(BIAS + 0x10000);
    assert_eq!(
        &first.image()[..CODE_FILE_BYTES],
        &second.image()[..CODE_FILE_BYTES]
    );
    for provider in [&first, &second] {
        assert_eq!(provider.table_rva(), PAGE_SIZE);
        assert_eq!(
            provider.address_point(),
            provider.load_bias() + PAGE_SIZE + 16
        );
        let words = provider
            .table_bytes()
            .chunks_exact(8)
            .map(|bytes| u64::from_le_bytes(bytes.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(words.len(), 8);
        assert_eq!(&words[..2], &[0, 0]);
        assert_eq!(&words[2..], &[provider.load_bias() + 0x110; 6]);
        assert_eq!(provider.table_bytes().len(), 64);
        assert_eq!(provider.data_bytes().len(), 104);
        assert_eq!(&provider.data_bytes()[..64], provider.table_bytes());
        assert_eq!(&provider.data_bytes()[64..], &[0; 40]);
        assert_eq!(provider.empty_string_rva(), PAGE_SIZE + 88);
        assert_eq!(provider.empty_cstr_rva(), PAGE_SIZE + 96);
        assert_eq!(&provider.data_bytes()[88..96], &[0; 8]);
        assert_eq!(provider.data_bytes()[96], 0);
        assert_eq!(&provider.data_bytes()[97..], &[0; 7]);
        for (function, rva) in [
            (JsonFunction::ValueGetBoolean, PAGE_SIZE + 64),
            (JsonFunction::ValueGetInteger, PAGE_SIZE + 72),
            (JsonFunction::ValueGetReal, PAGE_SIZE + 80),
        ] {
            assert_eq!(provider.scalar_default_rva(function), Some(rva));
            let offset = (rva - PAGE_SIZE) as usize;
            assert_eq!(&provider.data_bytes()[offset..offset + 8], &[0; 8]);
        }
        for marker in provider.markers() {
            if !matches!(
                marker.function,
                JsonFunction::ValueGetBoolean
                    | JsonFunction::ValueGetInteger
                    | JsonFunction::ValueGetReal
            ) {
                assert_eq!(provider.scalar_default_rva(marker.function), None);
            }
        }
    }
    assert_ne!(first.table_bytes(), second.table_bytes());
}

#[test]
fn json_constructor_binds_only_five_exact_scopes_versions_and_function_types() {
    let provider = factory(BIAS);
    for (index, (module, library, _)) in EXACT_SCOPES.into_iter().enumerate() {
        for dynlib in [false, true] {
            for encoding in [
                DescriptorEncoding::Classic,
                DescriptorEncoding::Variant1Observed,
            ] {
                let mut original = consumer(module, library);
                original.dynlib = dynlib;
                original.encoding = encoding;
                let bytes = original.build().unwrap();
                let source = link(&bytes, &provider).unwrap();
                assert_eq!(source.patch_count(), 1);
                let patch = source.patches()[0];
                assert_eq!(patch.value(), BIAS + 0x100);
                assert!(
                    matches!(patch.resolution(), Resolution::Definition(reference)
                    if reference.module.index() == 1 && reference.symbol == index + 1)
                );
                assert_eq!(source.deferred_count(), 0);
            }
        }
        for (major, minor, version) in [(2, 1, 1), (1, 2, 1), (1, 1, 2)] {
            let mut original = consumer(module, library);
            (original.major, original.minor, original.version) = (major, minor, version);
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::VersionMismatch { .. })
            ));
        }
        let mut object = consumer(module, library);
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
    for (module, library) in [
        (b"Json".as_slice(), b"Json".as_slice()),
        (b"libSceJson".as_slice(), b"Json2".as_slice()),
        (b"libc".as_slice(), b"Json".as_slice()),
        (b"Json2".as_slice(), b"libSceJson".as_slice()),
        (b"libScelibSceJson2".as_slice(), b"libSceJson2".as_slice()),
    ] {
        assert!(matches!(
            link(&consumer(module, library).build().unwrap(), &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
}

#[test]
fn json_value_constructor_aliases_bind_the_same_typed_marker_without_moving_allocator_rows() {
    let provider = factory(BIAS);
    let view = open_executable(provider.image()).unwrap();
    let metadata = inspect_sce(&view, &Default::default(), &Default::default()).unwrap();
    let symbols = metadata.metadata().unwrap().link().symbols();
    assert_eq!(VALUE_CONSTRUCTOR_NIDS, [b"qBMjqyBn3OM", b"-wa17B7TGnw"]);
    for (nid_index, nid) in VALUE_CONSTRUCTOR_NIDS.into_iter().enumerate() {
        for (scope_index, (module, library, suffix)) in
            EXACT_SCOPES[..4].iter().copied().enumerate()
        {
            let row = 6 + nid_index * 4 + scope_index;
            let symbol = &symbols[row];
            let mut name = nid.to_vec();
            name.extend_from_slice(suffix);
            assert_eq!(symbol.name.as_ref().unwrap().as_bytes(), name);
            assert_eq!(symbol.binding(), SymbolBinding::GLOBAL);
            assert_eq!(symbol.visibility(), SymbolVisibility::PROTECTED);
            assert_eq!(symbol.symbol_type(), SymbolType::FUNC);
            assert_eq!((symbol.value, symbol.size), (0x120, 8));
            assert_eq!(provider.export_marker(row), Some(provider.markers()[2]));
            for dynlib in [false, true] {
                for encoding in [
                    DescriptorEncoding::Classic,
                    DescriptorEncoding::Variant1Observed,
                ] {
                    let mut original = consumer(module, library);
                    original.imports[0].nid = *nid;
                    original.dynlib = dynlib;
                    original.encoding = encoding;
                    let bytes = original.build().unwrap();
                    let source = link(&bytes, &provider).unwrap();
                    assert_eq!(source.patch_count(), 1);
                    assert_eq!(source.patches()[0].value(), BIAS + 0x120);
                    assert!(
                        matches!(source.patches()[0].resolution(), Resolution::Definition(reference)
                        if reference.module.index() == 1 && reference.symbol == row)
                    );
                }
            }
        }
    }
    let symbol = &symbols[14];
    assert_eq!(symbol.name.as_ref().unwrap().as_bytes(), b"qBMjqyBn3OM#C#D");
    assert_eq!((symbol.value, symbol.size), (0x120, 8));
    assert_eq!(symbol.symbol_type(), SymbolType::FUNC);
    assert_eq!(provider.export_marker(14), Some(provider.markers()[2]));
    for dynlib in [false, true] {
        for encoding in [
            DescriptorEncoding::Classic,
            DescriptorEncoding::Variant1Observed,
        ] {
            let mut original = consumer(b"libSceJson", b"libSceJson2");
            original.imports[0].nid = *b"qBMjqyBn3OM";
            original.dynlib = dynlib;
            original.encoding = encoding;
            let bytes = original.build().unwrap();
            let source = link(&bytes, &provider).unwrap();
            assert_eq!(source.patches()[0].value(), BIAS + 0x120);
            assert!(
                matches!(source.patches()[0].resolution(), Resolution::Definition(reference)
                if reference.module.index() == 1 && reference.symbol == 14)
            );
        }
    }
    for (row, symbol) in symbols.iter().enumerate().take(6).skip(1) {
        assert_eq!(symbol.value, 0x100);
        assert_eq!(
            provider.export_marker(row).unwrap().function,
            JsonFunction::MemAllocatorCtor
        );
    }
    assert!(
        !(1..=provider.function_count())
            .any(|row| provider.export_marker(row).unwrap().function == JsonFunction::VirtualCall)
    );
}

#[test]
fn json_value_constructor_aliases_refuse_unconfirmed_scopes_wrong_versions_and_object_imports() {
    let provider = factory(BIAS);
    for nid in VALUE_CONSTRUCTOR_NIDS {
        for (scope, &(module, library, _)) in EXACT_SCOPES.iter().enumerate() {
            if scope == 4 && nid == VALUE_CONSTRUCTOR_NIDS[1] {
                continue;
            }
            for (major, minor, version) in [(2, 1, 1), (1, 2, 1), (1, 1, 2)] {
                let mut original = consumer(module, library);
                original.imports[0].nid = *nid;
                (original.major, original.minor, original.version) = (major, minor, version);
                assert!(matches!(
                    link(&original.build().unwrap(), &provider),
                    Err(SymbolicError::VersionMismatch { .. })
                ));
            }
            let mut original = consumer(module, library);
            original.imports[0].nid = *nid;
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
            (b"Json".as_slice(), b"Json".as_slice()),
            (b"libc".as_slice(), b"Json".as_slice()),
        ] {
            let mut original = consumer(module, library);
            original.imports[0].nid = *nid;
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::MissingProvider { .. })
            ));
        }
    }
    let mut unconfirmed = consumer(b"libSceJson", b"libSceJson2");
    unconfirmed.imports[0].nid = *b"-wa17B7TGnw";
    assert!(matches!(
        link(&unconfirmed.build().unwrap(), &provider),
        Err(SymbolicError::MissingProvider { .. })
    ));
}

#[test]
fn json_provider_refuses_bad_placements_and_checks_the_inclusive_file_budget() {
    assert_eq!(
        build_json_provider(
            ProviderPlacement { load_bias: BIAS },
            IMAGE_BYTES as u64 - 1
        )
        .unwrap_err(),
        KernelProviderError::Budget
    );
    assert_eq!(factory(BIAS).image().len(), IMAGE_BYTES);
    assert_eq!(
        build_json_provider(ProviderPlacement { load_bias: BIAS }, 0x440).unwrap_err(),
        KernelProviderError::Budget
    );
    assert_eq!(
        build_json_provider(ProviderPlacement { load_bias: BIAS }, 0x840).unwrap_err(),
        KernelProviderError::Budget
    );
    for bias in [
        BIAS + 1,
        ADDRESS_LIMIT,
        ADDRESS_LIMIT - PAGE_SIZE,
        u64::MAX - PAGE_SIZE + 1,
    ] {
        assert_eq!(
            build_json_provider(ProviderPlacement { load_bias: bias }, IMAGE_BYTES as u64)
                .unwrap_err(),
            KernelProviderError::InvalidCatalogue
        );
    }
    let highest = factory(ADDRESS_LIMIT - 2 * PAGE_SIZE);
    assert_eq!(highest.address_point(), ADDRESS_LIMIT - PAGE_SIZE + 16);
    assert_eq!(
        u64::from_le_bytes(highest.table_bytes()[16..24].try_into().unwrap()),
        ADDRESS_LIMIT - 2 * PAGE_SIZE + 0x110
    );
}

#[test]
fn json_primitive_manifest_keeps_old_sites_and_binds_grouped_exact_public_imports() {
    let provider = factory(BIAS);
    let functions = [
        JsonFunction::ValueBoolCtor,
        JsonFunction::ValueIntCtor,
        JsonFunction::ValueTypeCtor,
        JsonFunction::ValueSetBool,
        JsonFunction::ValueSetInt,
        JsonFunction::ValueSetUInt,
        JsonFunction::ValueSetType,
        JsonFunction::ValueGetType,
        JsonFunction::ValueGetBoolean,
        JsonFunction::ValueGetInteger,
        JsonFunction::ValueGetReal,
        JsonFunction::ValueCopyCtor,
        JsonFunction::ValueAssign,
        JsonFunction::ValueDtor,
        JsonFunction::ValueClearMethod,
        JsonFunction::ValueCount,
        JsonFunction::MemAllocatorDtor,
        JsonFunction::ValueDoubleCtor,
        JsonFunction::ValueSetDouble,
        JsonFunction::StringCtor,
        JsonFunction::StringCStringCtor,
        JsonFunction::StringDtor,
        JsonFunction::StringAssign,
        JsonFunction::StringCStr,
        JsonFunction::StringLength,
        JsonFunction::ValueCStringCtor,
        JsonFunction::ValueStringCtor,
        JsonFunction::ValueSetString,
        JsonFunction::ValueGetString,
        JsonFunction::ValueToString,
        JsonFunction::ValueSerialize,
    ];
    for (index, function) in functions.into_iter().enumerate() {
        let marker = provider.markers()[index + 3];
        assert_eq!(marker.function, function);
        assert_eq!(marker.rva, 0x130 + index as u64 * 16);
        assert_eq!(marker.service_id, 0x5058_4a04 + index as u32);
    }
    let view = open_executable(provider.image()).unwrap();
    let metadata = inspect_sce(&view, &Default::default(), &Default::default()).unwrap();
    let symbols = metadata.metadata().unwrap().link().symbols();
    for (scope, (module, library, suffix)) in EXACT_SCOPES.into_iter().enumerate() {
        let available = PRIMITIVE_CASES
            .into_iter()
            .filter(|entry| scope != 4 || entry.2)
            .collect::<Vec<_>>();
        // Nine qualified imports fit the established original consumer's fixed
        // tables. Grouping validates every identity without per-function suites.
        for group in available.chunks(9) {
            for dynlib in [false, true] {
                for encoding in [
                    DescriptorEncoding::Classic,
                    DescriptorEncoding::Variant1Observed,
                ] {
                    let mut original = consumer(module, library);
                    original.dynlib = dynlib;
                    original.encoding = encoding;
                    original.imports = group
                        .iter()
                        .map(|entry| Import {
                            nid: *entry.0,
                            symbol_type: 2,
                            binding: 1,
                            relocation: 7,
                            addend: 0,
                        })
                        .collect();
                    let bytes = original.build().unwrap();
                    let source = link(&bytes, &provider).unwrap();
                    assert_eq!(source.patch_count(), group.len());
                    for (patch, &(nid, function, _, first_row)) in
                        source.patches().iter().zip(group)
                    {
                        let row = first_row + scope;
                        let marker = provider.export_marker(row).unwrap();
                        assert_eq!(marker.function, function);
                        assert_eq!(patch.value(), BIAS + marker.rva);
                        assert!(
                            matches!(patch.resolution(), Resolution::Definition(reference)
                            if reference.module.index() == 1 && reference.symbol == row)
                        );
                        let mut name = nid.to_vec();
                        name.extend_from_slice(suffix);
                        assert_eq!(symbols[row].name.as_ref().unwrap().as_bytes(), name);
                    }
                }
            }
        }
    }
}

#[test]
fn json_primitive_fifth_scope_is_per_nid_and_wrong_types_or_versions_refuse() {
    let provider = factory(BIAS);
    for &(nid, _, observed, _) in &PRIMITIVE_CASES {
        if observed {
            continue;
        }
        let mut original = consumer(b"libSceJson", b"libSceJson2");
        original.imports[0].nid = *nid;
        assert!(matches!(
            link(&original.build().unwrap(), &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    let mut object = consumer(b"libSceJson", b"libSceJson2");
    object.imports[0].nid = *b"zTwZdI8AZ5Y";
    object.imports[0].symbol_type = 1;
    object.imports[0].relocation = 6;
    assert!(matches!(
        link(&object.build().unwrap(), &provider),
        Err(SymbolicError::UnsupportedSymbol {
            reason: UnsupportedSymbol::TypeMismatch,
            ..
        })
    ));
    let mut version = consumer(b"libSceJson", b"libSceJson2");
    version.imports[0].nid = *b"OcAgPxcq5Vk";
    version.minor = 2;
    assert!(matches!(
        link(&version.build().unwrap(), &provider),
        Err(SymbolicError::VersionMismatch { .. })
    ));
}

#[test]
fn json_lifecycle_manifest_preserves_old_sites_and_adds_only_exact_reviewed_scopes() {
    let provider = factory(BIAS);
    for (index, (nid, function)) in [
        (b"Cxwy7wHq4J0", JsonFunction::InitializerInitializeV1),
        (b"PR5k1penBLM", JsonFunction::InitializerTerminate),
        (b"RujUxbr3haM", JsonFunction::InitializerDtor),
        (
            b"00oCq0RwSAY",
            JsonFunction::InitializerSetGlobalNullAccessCallback,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let marker = provider.markers()[57 + index];
        assert_eq!(
            (marker.function, marker.rva, marker.service_id),
            (
                function,
                0x490 + index as u64 * 16,
                0x5058_4a3a + index as u32
            )
        );
        for (scope, module, library) in [
            (0, b"Json2".as_slice(), b"Json2".as_slice()),
            (1, b"libSceJson".as_slice(), b"libSceJson2".as_slice()),
        ] {
            let mut original = consumer(module, library);
            original.imports[0].nid = *nid;
            let bytes = original.build().unwrap();
            let source = link(&bytes, &provider).unwrap();
            assert_eq!(source.patches()[0].value(), BIAS + marker.rva);
            assert!(
                matches!(source.patches()[0].resolution(), Resolution::Definition(reference)
                if reference.module.index() == 1 && reference.symbol == 217 + 2 * index + scope)
            );
            original.minor = 2;
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::VersionMismatch { .. })
            ));
            original.minor = 1;
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
        let mut unobserved = consumer(b"libSceJson2", b"libSceJson2");
        unobserved.imports[0].nid = *nid;
        assert!(matches!(
            link(&unobserved.build().unwrap(), &provider),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    assert_eq!(
        provider.export_marker(216).unwrap(),
        JsonMarker {
            function: JsonFunction::ParserParse,
            rva: 0x480,
            service_id: 0x5058_4a39,
        }
    );
    assert_eq!(provider.markers()[34].rva, 0x320);
    assert_eq!(
        (
            provider.table_rva(),
            provider.empty_string_rva(),
            provider.empty_cstr_rva()
        ),
        (0x4000, 0x4058, 0x4060)
    );
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
#[test]
fn json_table_maps_as_readonly_nx_with_real_owned_guest_marker_targets() {
    use crate::memory::native::{NativeLoadLimits, prepare_in};
    use pentax_platform::native_memory::{
        NativeAddressSpace, NativeMemoryError, NativeMemoryLimits,
    };
    let owner = NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x40000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    let provider = factory(base + 0x10000);
    let bytes = consumer(b"libSceJson", b"libSceJson2").build().unwrap();
    let source = plan_symbolic(
        &[
            ModuleInput::new(open_executable(&bytes).unwrap(), base),
            ModuleInput::new(
                open_executable(provider.image()).unwrap(),
                provider.load_bias(),
            ),
        ],
        &Default::default(),
    )
    .unwrap();
    let mut memory = prepare_in(&source, &NativeLoadLimits::default(), Some(owner)).unwrap();
    let paused = memory.pause().unwrap();
    let table_address = provider.load_bias() + provider.table_rva();
    let mut data = [0; DATA_BYTES];
    paused.read(table_address, &mut data).unwrap();
    assert_eq!(data.as_slice(), provider.data_bytes());
    let table = &data[..TABLE_BYTES];
    assert_eq!(table, provider.table_bytes());
    assert_eq!(&data[TABLE_BYTES..], &[0; 40]);
    assert!(matches!(
        paused.preflight_write(table_address, DATA_BYTES as u64),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    assert!(matches!(
        paused.preflight_gpu_write(table_address, DATA_BYTES as u64),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    assert!(matches!(
        paused.preflight_fetch_source(table_address, DATA_BYTES as u64),
        Err(NativeMemoryError::PermissionDenied(_))
    ));
    let terminal = provider.markers()[1];
    for word in table[16..].chunks_exact(8) {
        let target = u64::from_le_bytes(word.try_into().unwrap());
        assert_eq!(target, provider.load_bias() + terminal.rva);
        let mut marker = [0; 8];
        paused.fetch_source(target, &mut marker).unwrap();
        assert_eq!(marker, terminal.bytes());
    }
    for function in [
        JsonFunction::ValueGetBoolean,
        JsonFunction::ValueGetInteger,
        JsonFunction::ValueGetReal,
    ] {
        let address = provider.load_bias() + provider.scalar_default_rva(function).unwrap();
        let mut value = [0; 8];
        paused.read(address, &mut value).unwrap();
        assert_eq!(value, [0; 8]);
        paused.preflight_non_executable(address, 8).unwrap();
        assert!(matches!(
            paused.preflight_write(address, 8),
            Err(NativeMemoryError::PermissionDenied(_))
        ));
    }
    for (rva, length) in [
        (provider.empty_string_rva(), 8),
        (provider.empty_cstr_rva(), 1),
    ] {
        let address = provider.load_bias() + rva;
        let mut bytes = [0xa7; 8];
        paused.read(address, &mut bytes[..length]).unwrap();
        assert_eq!(&bytes[..length], &[0; 8][..length]);
        paused
            .preflight_non_executable(address, length as u64)
            .unwrap();
        assert!(matches!(
            paused.preflight_write(address, length as u64),
            Err(NativeMemoryError::PermissionDenied(_))
        ));
        assert!(matches!(
            paused.preflight_gpu_write(address, length as u64),
            Err(NativeMemoryError::PermissionDenied(_))
        ));
        assert!(matches!(
            paused.preflight_fetch_source(address, length as u64),
            Err(NativeMemoryError::PermissionDenied(_))
        ));
    }
}
