// SPDX-License-Identifier: GPL-2.0-or-later

mod fixture;
mod hash;
mod hostile;
mod limits;
mod planning;

use pentax_loader::open_executable;

use self::fixture::{Domain, Tables, address, image, null_symbol, rela, static_image, symbol};
use crate::{
    DynamicAddress, Inspection, Limits, LinkCounts, LinkError, LinkMetadata, LinkTables,
    Relocation, Symbol, SymbolBinding, SymbolName, SymbolType, SymbolVisibility, TableExtent,
    TableKind, inspect,
};

/// Offsets: 0 empty, 1 `puts`, 6 non-UTF-8 bytes, 10 UTF-8 `café`.
const STRINGS: &[u8] = b"\0puts\0\xff\xfeX\0caf\xc3\xa9\0";

const ZERO: Limits = Limits {
    max_table_bytes: 0,
    max_name_bytes: 0,
    max_symbols: 0,
    max_relocations: 0,
};

const DOMAINS: [Domain; 2] = [Domain::Guest, Domain::Dynlib];

fn sample_symbols() -> Vec<u8> {
    [
        null_symbol(),
        symbol(1, 0x12, 0x02, 0, 0, 0),
        symbol(6, 0x21, 0x00, 7, 0x1234, 16),
        symbol(10, 0xbd, 0xf3, 0xfff1, u64::MAX, 1),
    ]
    .concat()
}

/// 16 string bytes, 96 symbol bytes and 48 bytes in each relocation table.
fn sample_tables() -> Tables {
    Tables {
        strings: Some(STRINGS.to_vec()),
        symbols: Some(sample_symbols()),
        rela: Some([rela(0x2000, 0, 8, -8), rela(0x2008, 2, 1, i64::MIN)].concat()),
        plt_rela: Some(
            [
                rela(0x3000, 1, 7, 0),
                rela(0x3008, 3, 0xdead_beef, i64::MAX),
            ]
            .concat(),
        ),
        ..Tables::default()
    }
}

fn inspect_bytes(bytes: &[u8], limits: &Limits) -> Result<Inspection, LinkError> {
    let view = open_executable(bytes)?;
    inspect(&view, limits)
}

fn metadata(domain: Domain, tables: &Tables) -> LinkMetadata {
    match inspect_bytes(&image(domain, tables), &Limits::default()) {
        Ok(Inspection::Dynamic(metadata)) => metadata,
        other => panic!("expected dynamic metadata, got {other:?}"),
    }
}

#[test]
fn raw_records_preserve_fields_and_signed_addends_in_both_domains() {
    for domain in DOMAINS {
        let metadata = metadata(domain, &sample_tables());
        let symbols = metadata.symbols();
        assert_eq!(symbols.len(), 4);

        let puts = &symbols[1];
        assert_eq!(puts.name_offset, 1);
        assert_eq!(
            puts.name.as_ref().map(SymbolName::as_bytes),
            Some(&b"puts"[..])
        );
        assert_eq!((puts.info, puts.other, puts.section_index), (0x12, 2, 0));
        assert_eq!(puts.binding(), SymbolBinding::GLOBAL);
        assert_eq!(puts.symbol_type(), SymbolType::FUNC);
        assert_eq!(puts.visibility(), SymbolVisibility::HIDDEN);
        assert!(puts.is_undefined());

        let object = &symbols[2];
        assert_eq!(
            (object.section_index, object.value, object.size),
            (7, 0x1234, 16)
        );
        assert_eq!(object.binding(), SymbolBinding::WEAK);
        assert_eq!(object.symbol_type(), SymbolType::OBJECT);
        assert!(!object.is_undefined());

        assert_eq!(
            metadata.rela(),
            &[
                Relocation {
                    offset: 0x2000,
                    info: 8,
                    addend: -8
                },
                Relocation {
                    offset: 0x2008,
                    info: (2 << 32) | 1,
                    addend: i64::MIN
                },
            ]
        );
        let relative = metadata.rela()[0];
        assert_eq!((relative.symbol_index(), relative.kind()), (0, 8));
        assert!(!relative.references_symbol());
        let plt = metadata.plt_rela()[1];
        assert_eq!(
            (plt.offset, plt.symbol_index(), plt.kind(), plt.addend),
            (0x3008, 3, 0xdead_beef, i64::MAX)
        );
        assert_eq!(
            metadata.counts(),
            LinkCounts {
                symbols: 4,
                named_symbols: 3,
                rela: 2,
                plt_rela: 2,
                symbol_references: 3,
                table_bytes: 16 + 96 + 48 + 48,
                name_bytes: 4 + 3 + 5,
                sysv_hash_buckets: 0,
                sysv_hash_chains: 0,
            }
        );
    }
}

#[test]
fn address_domains_stay_separate() {
    // Strings are placed first and symbols at data offset 16 in both images.
    for domain in DOMAINS {
        let tables = metadata(domain, &sample_tables()).tables();
        let expected = |offset| match domain {
            Domain::Guest => DynamicAddress::GuestVirtual(address(domain, offset)),
            Domain::Dynlib => DynamicAddress::DynlibOffset(address(domain, offset)),
        };
        assert_eq!(
            tables.strings,
            Some(TableExtent {
                address: expected(0),
                size: 16
            })
        );
        assert_eq!(
            tables.symbols,
            Some(TableExtent {
                address: expected(16),
                size: 96
            })
        );
        // Decoding `puts` proves each address was read from its own backing:
        // dynlib offset zero is not file offset zero (the ELF magic), and the
        // guest address is not a file offset.
        assert_eq!(
            metadata(domain, &sample_tables()).symbols()[1]
                .name
                .as_ref()
                .map(SymbolName::as_bytes),
            Some(&b"puts"[..])
        );
    }
}

#[test]
fn names_are_arbitrary_bytes_with_an_optional_utf8_view() {
    let metadata = metadata(Domain::Dynlib, &sample_tables());
    let raw = metadata.symbols()[2].name.as_ref().unwrap();
    assert_eq!(raw.as_bytes(), b"\xff\xfeX");
    assert_eq!(raw.to_utf8(), None);
    assert_eq!(format!("{raw:?}"), r#"b"\xff\xfeX""#);
    let text = metadata.symbols()[3].name.as_ref().unwrap();
    assert_eq!(text.to_utf8(), Some("caf\u{e9}"));
    assert_eq!(text.len(), 5);
}

#[test]
fn zeroth_symbol_and_unknown_raw_values_are_kept_without_invention() {
    let metadata = metadata(Domain::Guest, &sample_tables());
    assert_eq!(
        metadata.symbols()[0],
        Symbol {
            name_offset: 0,
            name: None,
            info: 0,
            other: 0,
            section_index: 0,
            value: 0,
            size: 0,
        }
    );
    let unusual = &metadata.symbols()[3];
    assert_eq!(
        (unusual.binding().raw(), unusual.binding().name()),
        (11, None)
    );
    assert!(unusual.binding().is_os_specific());
    assert_eq!(
        (unusual.symbol_type().raw(), unusual.symbol_type().name()),
        (13, None)
    );
    assert!(unusual.symbol_type().is_processor_specific());
    assert_eq!(unusual.visibility(), SymbolVisibility::PROTECTED);
    assert_eq!(unusual.visibility().raw(), 3);
    assert_eq!(
        (unusual.other, unusual.section_index, unusual.value),
        (0xf3, 0xfff1, u64::MAX)
    );
}

#[test]
fn absent_dynamic_section_is_distinct_from_empty_tables() {
    assert_eq!(
        inspect_bytes(&static_image(), &Limits::default()),
        Ok(Inspection::NotDynamic)
    );
    let empty = inspect_bytes(&image(Domain::Guest, &Tables::default()), &ZERO).unwrap();
    let metadata = empty.metadata().unwrap();
    assert_eq!(metadata.tables(), LinkTables::default());
    assert_eq!(metadata.counts(), LinkCounts::default());
    assert!(!metadata.symbol_table_present());
}

#[test]
fn relocations_using_symbol_zero_need_no_symbol_table() {
    for domain in DOMAINS {
        let tables = Tables {
            strings: Some(b"\0unused\0".to_vec()),
            rela: Some(rela(0x2000, 0, 8, 0x40).to_vec()),
            plt_rela: Some(rela(0x3000, 0, 7, 0).to_vec()),
            ..Tables::default()
        };
        let metadata = metadata(domain, &tables);
        assert!(!metadata.symbol_table_present());
        // Names are the only consumer of strings, so they are not inspected.
        assert_eq!(metadata.tables().strings, None);
        assert_eq!(metadata.counts().table_bytes, 48);
        assert_eq!(metadata.counts().symbol_references, 0);
        assert_eq!(metadata.rela()[0].addend, 0x40);
    }
}

#[test]
fn unknown_symbol_extent_is_reported_instead_of_inferred() {
    let tables = Tables {
        unsized_symbols: true,
        rela: Some(rela(0x2000, 0, 8, 0).to_vec()),
        ..sample_tables()
    };
    assert_eq!(
        inspect_bytes(&image(Domain::Guest, &tables), &Limits::default()),
        Err(LinkError::UnknownExtent {
            table: TableKind::Symbols,
            address: DynamicAddress::GuestVirtual(address(Domain::Guest, 16)),
        })
    );
}
