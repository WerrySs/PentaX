// SPDX-License-Identifier: GPL-2.0-or-later

mod fixture;
mod hostile;
mod selectors;
mod variants;

use pentax_loader::open_executable;

use self::fixture::{
    Builder, DOMAINS, Domain, address, library, module, static_image, strtab, symbol,
};
use crate::{
    AttributeRole, DT_SCE_EXPORT_LIB, DT_SCE_EXPORT_LIB_ATTR, DT_SCE_IMPORT_LIB,
    DT_SCE_IMPORT_LIB_ATTR, DT_SCE_MODULE_ATTR, DT_SCE_MODULE_INFO, DT_SCE_NEEDED_MODULE,
    DescriptorDirection, DescriptorEncoding, DescriptorNamespace, DescriptorRole, DynamicAddress,
    DynamicSource, Inspection, LimitKind, Limits, LinkError, QualifiedField, QualifiedNameError,
    QualifiedNid, SceDescriptors, SceError, SceInspection, SceLimits, SceMetadata, SelectorMatch,
    SymbolBinding, SymbolSection, TableExtent, TokenError, inspect, inspect_sce,
};

const DT_NEEDED: i64 = 1;
const GLOBAL_FUNC: u8 = 0x12;
const WEAK_FUNC: u8 = 0x22;
const LOCAL_FUNC: u8 = 0x02;
/// OS-specific binding 10 with type FUNC.
const OS_BINDING_FUNC: u8 = 0xa2;
const SHN_ABS: u16 = 0xfff1;
const SHN_COMMON: u16 = 0xfff2;

/// Synthetic canonical tokens. Neither is a hash of any real name.
const NID: &[u8] = b"3T1rzimi0Ew";
const OTHER_NID: &[u8] = b"pentaxNid0A";

fn run(bytes: &[u8], link: &Limits, sce: &SceLimits) -> Result<SceInspection, SceError> {
    let view = open_executable(bytes).map_err(LinkError::from)?;
    inspect_sce(&view, link, sce)
}

fn metadata(bytes: &[u8]) -> SceMetadata {
    match run(bytes, &Limits::default(), &SceLimits::default()) {
        Ok(SceInspection::Dynamic(metadata)) => *metadata,
        other => panic!("expected SCE metadata, got {other:?}"),
    }
}

fn failure(bytes: &[u8]) -> SceError {
    run(bytes, &Limits::default(), &SceLimits::default()).unwrap_err()
}

fn qualified(nid: &[u8], library: &[u8], module: &[u8]) -> Vec<u8> {
    [nid, b"#", library, b"#", module].concat()
}

fn source(index: usize, tag: i64, value: u64) -> DynamicSource {
    DynamicSource { index, tag, value }
}

#[test]
fn classic_descriptors_decode_out_of_order_without_symbols() {
    let names: [&[u8]; 4] = [b"libSceSelf", b"libSceDep", b"\xffDep\xfe", b"libDep.prx"];
    let (strings, at) = strtab(&names);
    for domain in DOMAINS {
        let mut builder = Builder::new(domain);
        builder.pad(24);
        let strings_at = builder.place(&strings);
        // Descriptor tags precede the string table and interleave roles.
        let values = [
            (DT_SCE_IMPORT_LIB, library(at[2], 0xbeef, 0x0102)),
            (DT_NEEDED, u64::from(at[3])),
            (DT_SCE_MODULE_INFO, module(at[0], 0x12, 0x34, 0)),
            (DT_SCE_NEEDED_MODULE, module(at[1], 0xff, 0x01, 0xffff)),
            (DT_SCE_EXPORT_LIB, library(at[0], 0x0001, 0x8000)),
        ];
        for (tag, value) in values {
            builder.entry(tag, value);
        }
        builder.strings_at(strings_at, strings.len() as u64);
        let metadata = metadata(&builder.build());

        assert!(!metadata.link().symbol_table_present());
        let extent = TableExtent {
            address: match domain {
                Domain::Guest => DynamicAddress::GuestVirtual(address(domain, 24)),
                Domain::Dynlib => DynamicAddress::DynlibOffset(24),
            },
            size: strings.len() as u64,
        };
        assert_eq!(metadata.metadata_strings(), Some(extent));
        assert_eq!(metadata.table_bytes(), strings.len() as u64);
        assert_eq!(metadata.metadata_name_bytes(), 9 + 10 + 10 + 5 + 10);

        let SceDescriptors::Decoded(classic) = metadata.descriptors() else {
            panic!("expected decoded descriptors");
        };
        for role in [
            DescriptorRole::ModuleInfo,
            DescriptorRole::NeededModule,
            DescriptorRole::ExportLibrary,
            DescriptorRole::ImportLibrary,
        ] {
            assert_eq!(classic.encoding(role), Some(DescriptorEncoding::Classic));
        }
        let [own] = classic.module_info() else {
            panic!("one module info");
        };
        assert_eq!(own.source(), source(2, DT_SCE_MODULE_INFO, values[2].1));
        assert_eq!(own.encoding(), DescriptorEncoding::Classic);
        assert_eq!(own.name().as_bytes(), b"libSceSelf");
        assert_eq!((own.version_major(), own.version_minor()), (0x12, 0x34));
        assert_eq!(own.raw_version(), 0x1234);
        assert_eq!(own.id().value(), 0);

        let [needed] = classic.needed_modules() else {
            panic!("one needed module");
        };
        assert_eq!(needed.name_offset(), at[1]);
        assert_eq!((needed.version_major(), needed.version_minor()), (0xff, 1));
        assert_eq!(needed.id().value(), 0xffff);

        let [import] = classic.import_libraries() else {
            panic!("one import library");
        };
        assert_eq!(import.source().index, 0);
        assert_eq!(import.name().as_bytes(), b"\xffDep\xfe");
        assert_eq!(import.name().to_utf8(), None);
        assert_eq!(format!("{:?}", import.name()), r#"b"\xffDep\xfe""#);
        // The version is the complete u16, not a major/minor split.
        assert_eq!((import.version(), import.id().value()), (0xbeef, 0x0102));

        let [export] = classic.export_libraries() else {
            panic!("one export library");
        };
        assert_eq!(export.name().as_bytes(), b"libSceSelf");
        assert_eq!((export.version(), export.id().value()), (1, 0x8000));

        // DT_NEEDED stays raw and separate, with no extension rewriting.
        let [dependency] = metadata.needed() else {
            panic!("one DT_NEEDED");
        };
        assert_eq!(dependency.source(), source(1, DT_NEEDED, u64::from(at[3])));
        assert_eq!(dependency.name().as_bytes(), b"libDep.prx");
        assert_eq!(metadata.import_associations(), Some(&[][..]));
    }
}

#[test]
fn roles_have_separate_namespaces_and_directions() {
    let classic = DescriptorEncoding::Classic;
    let observed = DescriptorEncoding::Variant1Observed;
    for (role, classic_tag, observed_tag) in [
        (DescriptorRole::ModuleInfo, 0x6100_000d, 0x6100_0043),
        (DescriptorRole::NeededModule, 0x6100_000f, 0x6100_0045),
        (DescriptorRole::ExportLibrary, 0x6100_0013, 0x6100_0047),
        (DescriptorRole::ImportLibrary, 0x6100_0015, 0x6100_0049),
    ] {
        assert_eq!(role.tag(classic), classic_tag, "{role}");
        assert_eq!(role.tag(observed), observed_tag, "{role}");
    }
    assert_eq!(
        DescriptorRole::NeededModule.namespace(),
        DescriptorNamespace::Module
    );
    assert_eq!(
        DescriptorRole::ExportLibrary.namespace(),
        DescriptorNamespace::Library
    );
    assert_eq!(
        DescriptorRole::ModuleInfo.direction(),
        DescriptorDirection::Export
    );
    assert_eq!(
        DescriptorRole::ImportLibrary.direction(),
        DescriptorDirection::Import
    );
}

#[test]
fn equal_ids_in_distinct_scopes_are_independent() {
    let (strings, at) = strtab(&[b"m", b"l"]);
    let mut builder = Builder::new(Domain::Dynlib);
    builder
        .strings(&strings)
        .entry(DT_SCE_MODULE_INFO, module(at[0], 1, 1, 7))
        .entry(DT_SCE_NEEDED_MODULE, module(at[0], 1, 1, 7))
        .entry(DT_SCE_EXPORT_LIB, library(at[1], 1, 7))
        .entry(DT_SCE_IMPORT_LIB, library(at[1], 1, 7));
    let metadata = metadata(&builder.build());
    let classic = metadata.descriptors().decoded().unwrap();
    assert_eq!(
        classic.module_info()[0].id(),
        classic.import_libraries()[0].id()
    );
    assert_eq!(classic.needed_modules().len(), 1);
    assert_eq!(classic.export_libraries().len(), 1);
}

#[test]
fn duplicate_scoped_ids_are_rejected_even_when_identical() {
    let (strings, at) = strtab(&[b"libA", b"libB"]);
    for (first, second) in [
        (library(at[0], 1, 3), library(at[0], 1, 3)),
        (library(at[0], 1, 3), library(at[1], 2, 3)),
    ] {
        let mut builder = Builder::new(Domain::Guest);
        builder
            .strings(&strings)
            .entry(DT_SCE_IMPORT_LIB, first)
            .entry(DT_SCE_EXPORT_LIB, first)
            .entry(DT_SCE_IMPORT_LIB, second);
        assert_eq!(
            failure(&builder.build()),
            SceError::DuplicateDescriptorId {
                role: DescriptorRole::ImportLibrary,
                encoding: DescriptorEncoding::Classic,
                id: 3,
                first: source(2, DT_SCE_IMPORT_LIB, first),
                second: source(4, DT_SCE_IMPORT_LIB, second),
            }
        );
    }
    let mut builder = Builder::new(Domain::Dynlib);
    builder
        .strings(&strings)
        .entry(DT_SCE_NEEDED_MODULE, module(at[0], 1, 0, 9))
        .entry(DT_SCE_NEEDED_MODULE, module(at[1], 2, 0, 9));
    let error = failure(&builder.build());
    assert!(matches!(
        error,
        SceError::DuplicateDescriptorId {
            role: DescriptorRole::NeededModule,
            id: 9,
            ..
        }
    ));
    assert!(error.to_string().contains("DT_SCE_NEEDED_MODULE ID 9"));
}

#[test]
fn attributes_are_opaque_verbatim_and_need_no_strings() {
    let values = [
        (DT_SCE_MODULE_ATTR, 0x0000_0000_0000_0000),
        (DT_SCE_IMPORT_LIB_ATTR, 0xffff_0000_dead_0009),
        (DT_SCE_EXPORT_LIB_ATTR, 0x0001_8000_0000_0001),
        (DT_SCE_IMPORT_LIB_ATTR, 0xffff_0000_dead_0009),
    ];
    for domain in DOMAINS {
        let mut builder = Builder::new(domain);
        for (tag, value) in values {
            builder.entry(tag, value);
        }
        let metadata = metadata(&builder.build());
        // Attributes alone do not select a descriptor family.
        assert_eq!(metadata.descriptors(), &SceDescriptors::Absent);
        assert_eq!(metadata.metadata_strings(), None);
        let attributes = metadata.attributes();
        assert_eq!(attributes.len(), 4);
        for (index, (attribute, (tag, value))) in attributes.iter().zip(values).enumerate() {
            assert_eq!(attribute.source(), source(index, tag, value));
            assert_eq!(attribute.raw(), value);
        }
        assert_eq!(attributes[0].role(), AttributeRole::Module);
        assert_eq!(attributes[1].role(), AttributeRole::ImportLibrary);
        assert_eq!(attributes[2].role(), AttributeRole::ExportLibrary);
        assert_eq!(metadata.import_associations(), None);
    }
}

#[test]
fn absent_descriptors_are_distinct_from_not_dynamic() {
    assert_eq!(
        run(&static_image(), &Limits::default(), &SceLimits::default()),
        Ok(SceInspection::NotDynamic)
    );
    let mut builder = Builder::new(Domain::Guest);
    builder.strings(b"\0unused\0").entry(0x6fff_fff0, 5);
    let metadata = metadata(&builder.build());
    assert_eq!(metadata.descriptors(), &SceDescriptors::Absent);
    assert_eq!(metadata.descriptors().decoded(), None);
    // No decoded name needs the string table, so it is not read or charged.
    assert_eq!(metadata.metadata_strings(), None);
    assert_eq!(
        (metadata.table_bytes(), metadata.metadata_name_bytes()),
        (0, 0)
    );
    assert!(metadata.needed().is_empty());
    assert!(metadata.attributes().is_empty());
}

#[test]
fn qualified_symbols_associate_only_undefined_global_or_weak_imports() {
    let good = qualified(NID, b"A", b"B");
    let missing_module = qualified(OTHER_NID, b"BA", b"C");
    let missing_library = qualified(NID, b"Q", b"B");
    let leading_zero = qualified(NID, b"AA", b"B");
    let names: [&[u8]; 8] = [
        b"puts",
        &good,
        &missing_module,
        &missing_library,
        b"bad#A",
        &leading_zero,
        b"libc",
        b"",
    ];
    let (strings, at) = strtab(&names);
    let symbols = [
        [0; 24],
        symbol(at[0], GLOBAL_FUNC, 0, 0),          // 1 ordinary name
        symbol(at[1], GLOBAL_FUNC, 0, 0),          // 2 both declared
        symbol(at[2], WEAK_FUNC, 0, 0),            // 3 module missing
        symbol(at[3], GLOBAL_FUNC, 0, 0),          // 4 library missing
        symbol(at[1], GLOBAL_FUNC, 5, 0x1000),     // 5 defined
        symbol(at[1], GLOBAL_FUNC, SHN_ABS, 0x10), // 6 absolute
        symbol(at[1], LOCAL_FUNC, 0, 0),           // 7 local reference
        symbol(at[4], GLOBAL_FUNC, 0, 0),          // 8 malformed
        symbol(at[1], GLOBAL_FUNC, 0, 0xdead),     // 9 SHN_UNDEF, nonzero value
        symbol(at[1], OS_BINDING_FUNC, 0, 0),      // 10 OS-specific binding
        symbol(at[1], GLOBAL_FUNC, SHN_COMMON, 8), // 11 common
        symbol(at[5], GLOBAL_FUNC, 0, 0),          // 12 leading zero selector
        symbol(at[1], GLOBAL_FUNC, 0xff10, 0),     // 13 reserved index
    ];
    for domain in DOMAINS {
        let mut builder = Builder::new(domain);
        builder
            .pad(40)
            .strings(&strings)
            .symbols(&symbols)
            .entry(DT_SCE_IMPORT_LIB, library(at[6], 1, 0))
            .entry(DT_SCE_IMPORT_LIB, library(at[6], 1, 64))
            .entry(DT_SCE_NEEDED_MODULE, module(at[7], 1, 1, 1))
            // Same IDs in export scopes must not satisfy import selectors.
            .entry(DT_SCE_EXPORT_LIB, library(at[6], 1, 16))
            .entry(DT_SCE_MODULE_INFO, module(at[6], 1, 1, 2));
        let metadata = metadata(&builder.build());
        // Strings are shared with the symbol table and counted once.
        assert_eq!(metadata.table_bytes(), metadata.link().counts().table_bytes);
        assert_eq!(
            metadata.table_bytes(),
            (strings.len() + symbols.len() * 24) as u64
        );
        // An empty module name is preserved, not replaced.
        let classic = metadata.descriptors().decoded().unwrap();
        assert!(classic.needed_modules()[0].name().is_empty());

        let listed: Vec<_> = metadata
            .qualified_symbols()
            .iter()
            .map(|symbol| (symbol.symbol(), symbol.section()))
            .collect();
        assert_eq!(
            listed,
            [
                (2, SymbolSection::Undefined),
                (3, SymbolSection::Undefined),
                (4, SymbolSection::Undefined),
                (5, SymbolSection::Section(5)),
                (6, SymbolSection::Absolute),
                (7, SymbolSection::Undefined),
                (8, SymbolSection::Undefined),
                (9, SymbolSection::Undefined),
                (10, SymbolSection::Undefined),
                (11, SymbolSection::Common),
                (12, SymbolSection::Undefined),
                (13, SymbolSection::Reserved(0xff10)),
            ]
        );
        let qualified = metadata.qualified_symbols();
        assert_eq!(
            qualified[6].name(),
            Err(QualifiedNameError::DelimiterCount { count: 1 })
        );
        assert_eq!(
            qualified[10].name(),
            Err(QualifiedNameError::Field {
                field: QualifiedField::Library,
                error: TokenError::LeadingZero,
            })
        );
        assert_eq!(qualified[8].binding().raw(), 10);
        // The original name bytes stay with the raw symbol.
        assert_eq!(
            metadata.link().symbols()[2]
                .name
                .as_ref()
                .unwrap()
                .as_bytes(),
            &good[..]
        );

        let associations: Vec<_> = metadata
            .import_associations()
            .unwrap()
            .iter()
            .map(|association| {
                (
                    association.symbol(),
                    association.library(),
                    association.module(),
                    association.is_declared(),
                )
            })
            .collect();
        assert_eq!(
            associations,
            [
                (
                    2,
                    SelectorMatch::Declared(0),
                    SelectorMatch::Declared(0),
                    true
                ),
                (3, SelectorMatch::Declared(1), SelectorMatch::Missing, false),
                (4, SelectorMatch::Missing, SelectorMatch::Declared(0), false),
                (
                    9,
                    SelectorMatch::Declared(0),
                    SelectorMatch::Declared(0),
                    true
                ),
            ]
        );
        let first = metadata.import_associations().unwrap()[0];
        assert_eq!(first.name(), QualifiedNid::parse(&good).unwrap());
        assert_eq!(first.name().nid().as_bytes(), NID);
    }
}

#[test]
fn qualified_symbols_are_recorded_without_descriptors() {
    let name = qualified(NID, b"A", b"A");
    let names: [&[u8]; 2] = [&name, b"plain"];
    let (strings, at) = strtab(&names);
    let mut builder = Builder::new(Domain::Guest);
    builder.strings(&strings).symbols(&[
        [0; 24],
        symbol(at[0], GLOBAL_FUNC, 0, 0),
        symbol(at[1], WEAK_FUNC, 0, 0),
    ]);
    let metadata = metadata(&builder.build());
    assert_eq!(metadata.descriptors(), &SceDescriptors::Absent);
    assert_eq!(metadata.qualified_symbols().len(), 1);
    assert_eq!(
        metadata.qualified_symbols()[0].binding(),
        SymbolBinding::GLOBAL
    );
    // Without classic descriptors no association is attempted.
    assert_eq!(metadata.import_associations(), None);
}

#[test]
fn metadata_only_string_table_is_charged_in_the_table_budget() {
    let (strings, at) = strtab(&[b"libSceTiny"]);
    let mut builder = Builder::new(Domain::Dynlib);
    builder
        .strings(&strings)
        .entry(DT_SCE_IMPORT_LIB, library(at[0], 1, 1));
    let bytes = builder.build();
    let size = strings.len() as u64;
    let limited = |max_table_bytes| Limits {
        max_table_bytes,
        ..Limits::default()
    };
    assert_eq!(
        run(&bytes, &limited(size - 1), &SceLimits::default()),
        Err(SceError::Link(LinkError::LimitExceeded {
            limit: LimitKind::TableBytes,
            requested: size,
            maximum: size - 1,
        }))
    );
    let exact = run(&bytes, &limited(size), &SceLimits::default()).unwrap();
    assert_eq!(exact.metadata().unwrap().table_bytes(), size);

    // The generic inspection keeps its own behaviour: without symbols, it
    // does not inspect or list the string table.
    let view = open_executable(&bytes).unwrap();
    let Ok(Inspection::Dynamic(link)) = inspect(&view, &limited(0)) else {
        panic!("generic inspection must succeed with no tables");
    };
    assert_eq!(link.tables().strings, None);
}

#[test]
fn inspection_does_not_mutate_and_is_repeatable() {
    let name = qualified(NID, b"A", b"A");
    let names: [&[u8]; 2] = [b"lib", &name];
    let (strings, at) = strtab(&names);
    let mut builder = Builder::new(Domain::Guest);
    builder
        .strings(&strings)
        .symbols(&[[0; 24], symbol(at[1], GLOBAL_FUNC, 0, 0)])
        .entry(DT_SCE_IMPORT_LIB, library(at[0], 1, 0))
        .entry(DT_SCE_NEEDED_MODULE, module(at[0], 1, 1, 0));
    let bytes = builder.build();
    let original = bytes.clone();
    let first = run(&bytes, &Limits::default(), &SceLimits::default()).unwrap();
    let second = run(&bytes, &Limits::default(), &SceLimits::default()).unwrap();
    assert_eq!(bytes, original);
    assert_eq!(first, second);
    let association = first.metadata().unwrap().import_associations().unwrap()[0];
    assert!(association.is_declared());
}
