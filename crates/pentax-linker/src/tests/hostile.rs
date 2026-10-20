// SPDX-License-Identifier: GPL-2.0-or-later

use std::panic::catch_unwind;

use super::fixture::{
    Domain, SCE_SYMENT, SCE_SYMTAB, SCE_SYMTABSZ, Tables, assemble, image, null_symbol, rela,
    symbol,
};
use super::{DOMAINS, inspect_bytes, sample_tables};
use crate::{Limits, LinkError, TableKind};

fn failure(domain: Domain, tables: &Tables) -> LinkError {
    inspect_bytes(&image(domain, tables), &Limits::default()).unwrap_err()
}

/// A null entry followed by one global symbol named at `name_offset`.
fn named(strings: Option<&[u8]>, name_offset: u32) -> Tables {
    Tables {
        strings: strings.map(<[u8]>::to_vec),
        symbols: Some([null_symbol(), symbol(name_offset, 0x10, 0, 0, 0, 0)].concat()),
        ..Tables::default()
    }
}

#[test]
fn string_offsets_and_terminators_are_checked() {
    for domain in DOMAINS {
        for (strings, name_offset, table_size) in [
            (&b"\0a\0"[..], 3, 3),
            (&b"\0a\0"[..], u32::MAX, 3),
            (&b""[..], 1, 0),
        ] {
            assert_eq!(
                failure(domain, &named(Some(strings), name_offset)),
                LinkError::NameOffsetOutOfRange {
                    symbol: 1,
                    name_offset,
                    table_size
                }
            );
        }
        // The image has zero padding right after this table. Scanning stays
        // inside the declared size, so the name is still unterminated.
        assert_eq!(
            failure(domain, &named(Some(&b"\0abc"[..]), 1)),
            LinkError::UnterminatedName {
                symbol: 1,
                name_offset: 1
            }
        );
        assert_eq!(
            failure(domain, &named(None, 1)),
            LinkError::MissingStringTable {
                symbol: 1,
                name_offset: 1
            }
        );
        // Unnamed symbols need no string table at all.
        assert!(inspect_bytes(&image(domain, &named(None, 0)), &Limits::default()).is_ok());
    }
}

#[test]
fn symbol_references_are_checked_but_never_resolved() {
    let without = Tables {
        rela: Some([rela(0, 0, 8, 0), rela(8, 1, 6, 0)].concat()),
        ..Tables::default()
    };
    assert_eq!(
        failure(Domain::Guest, &without),
        LinkError::MissingSymbolTable {
            table: TableKind::Rela,
            relocation: 1,
            symbol: 1
        }
    );

    let two = Some([null_symbol(), symbol(0, 0x10, 0, 0, 0, 0)].concat());
    let plt = Tables {
        symbols: two.clone(),
        plt_rela: Some([rela(0, 1, 7, 0), rela(8, 2, 7, 0)].concat()),
        ..Tables::default()
    };
    assert_eq!(
        failure(Domain::Dynlib, &plt),
        LinkError::SymbolIndexOutOfRange {
            table: TableKind::PltRela,
            relocation: 1,
            symbol: 2,
            symbol_count: 2
        }
    );
    let maximum = Tables {
        symbols: two,
        rela: Some(rela(0, u32::MAX, 1, 0).to_vec()),
        ..Tables::default()
    };
    assert_eq!(
        failure(Domain::Guest, &maximum),
        LinkError::SymbolIndexOutOfRange {
            table: TableKind::Rela,
            relocation: 0,
            symbol: u32::MAX,
            symbol_count: 2
        }
    );
    // A present but empty table establishes a count of zero.
    let empty = Tables {
        symbols: Some(Vec::new()),
        rela: Some(rela(0, 1, 1, 0).to_vec()),
        ..Tables::default()
    };
    assert_eq!(
        failure(Domain::Dynlib, &empty),
        LinkError::SymbolIndexOutOfRange {
            table: TableKind::Rela,
            relocation: 0,
            symbol: 1,
            symbol_count: 0
        }
    );
}

#[test]
fn partial_records_are_rejected_before_decoding() {
    let entries = [(SCE_SYMTAB, 0), (SCE_SYMTABSZ, 25), (SCE_SYMENT, 24)];
    let bytes = assemble(Domain::Dynlib, &entries, &[0; 32]);
    assert!(matches!(
        inspect_bytes(&bytes, &Limits::default()),
        Err(LinkError::Load(_))
    ));
}

#[test]
fn hostile_mutations_and_truncations_return_errors_without_panics() {
    let limits = Limits::default();
    for domain in DOMAINS {
        let baseline = image(domain, &sample_tables());
        for index in 0..baseline.len() {
            let original = baseline[index];
            for value in [0, 0xff, original ^ 0x80, original.wrapping_add(1)] {
                let mut bytes = baseline.clone();
                bytes[index] = value;
                assert!(
                    catch_unwind(|| inspect_bytes(&bytes, &limits)).is_ok(),
                    "{domain:?} byte {index:#x} = {value:#x}"
                );
            }
        }
        // The load segment covers the whole file, so every truncation fails.
        for length in 0..baseline.len() {
            let truncated = &baseline[..length];
            let outcome = catch_unwind(|| inspect_bytes(truncated, &limits));
            assert!(
                matches!(outcome, Ok(Err(_))),
                "{domain:?} length {length:#x}"
            );
        }
    }
}
