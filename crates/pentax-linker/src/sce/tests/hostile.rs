// SPDX-License-Identifier: GPL-2.0-or-later

use pentax_loader::open_executable;

use super::fixture::{
    BASE, Builder, DOMAINS, Domain, assemble, library, static_image, strtab, symbol,
};
use super::{DT_NEEDED, GLOBAL_FUNC, failure, run, source};
use crate::{
    DT_SCE_IMPORT_LIB, DT_SCE_IMPORT_LIB_1, DT_SCE_MODULE_ATTR, DT_SCE_MODULE_INFO_1,
    DT_SCE_ORIGINAL_FILENAME, DT_SCE_ORIGINAL_FILENAME_1, LimitKind, Limits, LinkError,
    SceDescriptors, SceError, SceInspection, SceLimitKind, SceLimits, inspect,
};

const UNKNOWN_TAG: i64 = 0x6fff_fff0;

#[test]
fn metadata_strings_and_relocations_are_budgeted_before_generic_decoding() {
    for domain in DOMAINS {
        for tag in [
            DT_SCE_IMPORT_LIB,
            DT_SCE_IMPORT_LIB_1,
            DT_SCE_ORIGINAL_FILENAME_1,
        ] {
            let mut builder = Builder::new(domain);
            builder.strings(b"\0ab\0");
            builder.entry(tag, library(1, 1, 0));
            let mut relocation = [0; 24];
            // A generic decoder would reject symbol index one without a symbol
            // table. The known aggregate byte excess must be rejected first.
            relocation[8..16].copy_from_slice(&((1_u64 << 32) | 1).to_le_bytes());
            let at = builder.place(&relocation);
            let (table, size, width) = match domain {
                Domain::Guest => (7, 8, 9),
                Domain::Dynlib => (0x6100_002f, 0x6100_0031, 0x6100_0033),
            };
            builder.entry(table, at).entry(size, 24).entry(width, 24);
            let link_limits = Limits {
                max_table_bytes: 24,
                ..Limits::default()
            };
            let result = run(&builder.build(), &link_limits, &SceLimits::default());
            if tag == DT_SCE_ORIGINAL_FILENAME_1 {
                // An opaque marker needs no name, so the string table is not
                // planned or charged and generic decoding reports its own error.
                assert!(matches!(
                    result,
                    Err(SceError::Link(LinkError::MissingSymbolTable {
                        symbol: 1,
                        ..
                    }))
                ));
            } else {
                assert_eq!(
                    result,
                    Err(SceError::Link(LinkError::LimitExceeded {
                        limit: LimitKind::TableBytes,
                        requested: 28,
                        maximum: 24,
                    }))
                );
            }
        }
    }
}

fn sce_limits(adjust: impl FnOnce(&mut SceLimits)) -> SceLimits {
    let mut limits = SceLimits::DEFAULT;
    adjust(&mut limits);
    limits
}

fn records(count: u64) -> Vec<u8> {
    let mut dynamic = Vec::new();
    for _ in 0..count {
        dynamic.extend_from_slice(&UNKNOWN_TAG.to_le_bytes());
        dynamic.extend_from_slice(&0_u64.to_le_bytes());
    }
    dynamic
}

#[test]
fn giant_dynamic_payload_is_rejected_before_parsing() {
    let declared = SceLimits::DEFAULT.max_dynamic_entries + 1;
    // No DT_NULL: the loader could only report that after a full parse.
    let bytes = assemble(Domain::Guest, &records(declared), &[]);
    assert_eq!(
        run(&bytes, &Limits::default(), &SceLimits::default()),
        Err(SceError::LimitExceeded {
            limit: SceLimitKind::DynamicEntries,
            requested: declared,
            maximum: 65_536,
        })
    );
    let view = open_executable(&bytes).unwrap();
    assert!(matches!(
        inspect(&view, &Limits::default()),
        Err(LinkError::Load(_))
    ));
}

#[test]
fn dynamic_record_limit_counts_declared_records_including_null_and_tail() {
    let mut builder = Builder::new(Domain::Dynlib);
    builder.entry(UNKNOWN_TAG, 1).entry(UNKNOWN_TAG, 2);
    let bytes = builder.build();
    let limited =
        |max_dynamic_entries| sce_limits(|limits| limits.max_dynamic_entries = max_dynamic_entries);
    assert!(run(&bytes, &Limits::default(), &limited(3)).is_ok());
    assert_eq!(
        run(&bytes, &Limits::default(), &limited(2)),
        Err(SceError::LimitExceeded {
            limit: SceLimitKind::DynamicEntries,
            requested: 3,
            maximum: 2,
        })
    );

    // Records after DT_NULL are part of the declared payload.
    let mut dynamic = records(1);
    dynamic.extend_from_slice(&[0; 16]);
    dynamic.extend(records(2));
    let trailing = assemble(Domain::Guest, &dynamic, &[]);
    assert!(run(&trailing, &Limits::default(), &limited(4)).is_ok());
    assert!(matches!(
        run(&trailing, &Limits::default(), &limited(3)),
        Err(SceError::LimitExceeded { requested: 4, .. })
    ));

    // A zero limit rejects any dynamic section but not a static image.
    assert!(run(&bytes, &Limits::default(), &limited(0)).is_err());
    assert_eq!(
        run(&static_image(), &Limits::default(), &limited(0)),
        Ok(SceInspection::NotDynamic)
    );
}

#[test]
fn zero_budgets_admit_an_empty_dynamic_section() {
    let zero_link = Limits {
        max_table_bytes: 0,
        max_name_bytes: 0,
        max_symbols: 0,
        max_relocations: 0,
    };
    let zero_sce = SceLimits {
        max_dynamic_entries: 1,
        max_descriptor_records: 0,
        max_metadata_name_bytes: 0,
        max_single_metadata_name_bytes: 0,
    };
    for domain in DOMAINS {
        let bytes = Builder::new(domain).build();
        let inspection = run(&bytes, &zero_link, &zero_sce).unwrap();
        let metadata = inspection.metadata().unwrap();
        assert_eq!(metadata.descriptors(), &SceDescriptors::Absent);
        assert_eq!(
            (metadata.table_bytes(), metadata.metadata_name_bytes()),
            (0, 0)
        );
    }
}

#[test]
fn record_budget_is_checked_before_names_or_output() {
    let mut builder = Builder::new(Domain::Guest);
    // None of these names could be read: there is no string table.
    builder
        .entry(DT_NEEDED, 0xffff_ffff)
        .entry(DT_SCE_IMPORT_LIB, library(0xffff_fff0, 1, 0))
        .entry(DT_SCE_MODULE_ATTR, 1);
    let bytes = builder.build();
    let limited = |max_descriptor_records| {
        sce_limits(|limits| limits.max_descriptor_records = max_descriptor_records)
    };
    assert_eq!(
        run(&bytes, &Limits::default(), &limited(2)),
        Err(SceError::LimitExceeded {
            limit: SceLimitKind::DescriptorRecords,
            requested: 3,
            maximum: 2,
        })
    );
    assert_eq!(
        run(&bytes, &Limits::default(), &limited(3)),
        Err(SceError::MissingStringTable {
            source: source(0, DT_NEEDED, 0xffff_ffff),
            name_offset: 0xffff_ffff,
        })
    );

    // Observed _1 records count against the same budget before their names.
    let mut variants = Builder::new(Domain::Dynlib);
    variants
        .entry(DT_SCE_MODULE_INFO_1, 0)
        .entry(DT_SCE_IMPORT_LIB_1, 0);
    let bytes = variants.build();
    assert!(matches!(
        run(&bytes, &Limits::default(), &limited(1)),
        Err(SceError::LimitExceeded {
            limit: SceLimitKind::DescriptorRecords,
            requested: 2,
            ..
        })
    ));
    assert!(matches!(
        run(&bytes, &Limits::default(), &limited(2)),
        Err(SceError::MissingStringTable { .. })
    ));
}

#[test]
fn filenames_and_conflicts_count_toward_the_record_budget() {
    let limited = |max_descriptor_records| {
        sce_limits(|limits| limits.max_descriptor_records = max_descriptor_records)
    };
    let mut builder = Builder::new(Domain::Guest);
    builder
        .entry(DT_SCE_ORIGINAL_FILENAME, u64::MAX)
        .entry(DT_SCE_IMPORT_LIB_1, u64::MAX)
        .entry(DT_SCE_ORIGINAL_FILENAME_1, 1)
        .entry(DT_SCE_IMPORT_LIB, u64::MAX)
        .entry(DT_SCE_MODULE_ATTR, 0);
    let bytes = builder.build();
    assert_eq!(
        run(&bytes, &Limits::default(), &limited(4)),
        Err(SceError::LimitExceeded {
            limit: SceLimitKind::DescriptorRecords,
            requested: 5,
            maximum: 4,
        })
    );
    // At the exact budget both conflict sides are retained without names.
    let inspection = run(&bytes, &Limits::default(), &limited(5)).unwrap();
    let metadata = inspection.metadata().unwrap();
    assert!(matches!(
        metadata.descriptors(),
        SceDescriptors::Conflicted(_)
    ));
    assert_eq!(metadata.original_filenames().len(), 2);

    // A lone filename marker is still a retained record.
    for tag in [DT_SCE_ORIGINAL_FILENAME, DT_SCE_ORIGINAL_FILENAME_1] {
        let mut builder = Builder::new(Domain::Dynlib);
        builder.entry(tag, u64::MAX);
        assert_eq!(
            run(&builder.build(), &Limits::default(), &limited(0)),
            Err(SceError::LimitExceeded {
                limit: SceLimitKind::DescriptorRecords,
                requested: 1,
                maximum: 0,
            })
        );
    }
}

#[test]
fn name_offsets_and_terminators_stay_inside_the_declared_table() {
    for domain in DOMAINS {
        let check = |strings: Option<&[u8]>, tag: i64, value: u64, expected: SceError| {
            let mut builder = Builder::new(domain);
            if let Some(strings) = strings {
                builder.strings(strings);
            }
            builder.entry(tag, value);
            assert_eq!(failure(&builder.build()), expected);
        };
        // The string table's address and size tags precede the descriptor.
        let entry = 2;
        // Zero bytes follow this table in the image; the scan must not use them.
        check(
            Some(&b"\0abc"[..]),
            DT_SCE_IMPORT_LIB,
            library(1, 1, 0),
            SceError::UnterminatedName {
                source: source(entry, DT_SCE_IMPORT_LIB, library(1, 1, 0)),
                name_offset: 1,
            },
        );
        for name_offset in [4, u64::from(u32::MAX)] {
            let value = library(name_offset as u32, 1, 0);
            check(
                Some(&b"\0abc"[..]),
                DT_SCE_IMPORT_LIB,
                value,
                SceError::NameOffsetOutOfRange {
                    source: source(entry, DT_SCE_IMPORT_LIB, value),
                    name_offset,
                    table_size: 4,
                },
            );
        }
        // DT_NEEDED offsets are the complete u64 value.
        check(
            Some(&b"\0a\0"[..]),
            DT_NEEDED,
            1 << 40,
            SceError::NameOffsetOutOfRange {
                source: source(entry, DT_NEEDED, 1 << 40),
                name_offset: 1 << 40,
                table_size: 3,
            },
        );
        check(
            None,
            DT_SCE_IMPORT_LIB,
            library(1, 1, 0),
            SceError::MissingStringTable {
                source: source(0, DT_SCE_IMPORT_LIB, library(1, 1, 0)),
                name_offset: 1,
            },
        );
    }
}

#[test]
fn single_and_total_name_budgets_charge_every_alias() {
    let (strings, at) = strtab(&[b"abcde"]);
    let mut builder = Builder::new(Domain::Dynlib);
    builder
        .strings(&strings)
        .entry(DT_SCE_IMPORT_LIB, library(at[0], 1, 0))
        .entry(DT_SCE_IMPORT_LIB, library(at[0], 1, 1));
    let bytes = builder.build();
    let limited = |total, single| {
        sce_limits(|limits| {
            limits.max_metadata_name_bytes = total;
            limits.max_single_metadata_name_bytes = single;
        })
    };
    let first = source(2, DT_SCE_IMPORT_LIB, library(at[0], 1, 0));
    let second = source(3, DT_SCE_IMPORT_LIB, library(at[0], 1, 1));
    assert_eq!(
        run(&bytes, &Limits::default(), &limited(100, 4)),
        Err(SceError::NameTooLong {
            source: first,
            name_offset: 1,
            maximum: 4,
        })
    );
    assert_eq!(
        run(&bytes, &Limits::default(), &limited(9, 5)),
        Err(SceError::NameBudgetExceeded {
            source: second,
            name_offset: 1,
            used: 5,
            maximum: 9,
        })
    );
    let exact = run(&bytes, &Limits::default(), &limited(10, 5)).unwrap();
    assert_eq!(exact.metadata().unwrap().metadata_name_bytes(), 10);

    // Zero budgets still admit empty names and fail on the first byte.
    assert!(matches!(
        run(&bytes, &Limits::default(), &limited(0, 4096)),
        Err(SceError::NameBudgetExceeded {
            used: 0,
            maximum: 0,
            ..
        })
    ));
    assert!(matches!(
        run(&bytes, &Limits::default(), &limited(4096, 0)),
        Err(SceError::NameTooLong { maximum: 0, .. })
    ));
    let mut empty = Builder::new(Domain::Guest);
    empty
        .strings(&strings)
        .entry(DT_SCE_IMPORT_LIB, library(0, 1, 0));
    let inspection = run(&empty.build(), &Limits::default(), &limited(0, 0)).unwrap();
    let classic = inspection
        .metadata()
        .unwrap()
        .descriptors()
        .decoded()
        .unwrap();
    assert!(classic.import_libraries()[0].name().is_empty());
}

#[test]
fn repeated_string_tails_fail_at_the_budget() {
    let mut strings = vec![0];
    strings.extend(std::iter::repeat_n(b'a', 1000));
    strings.push(0);
    let mut builder = Builder::new(Domain::Dynlib);
    builder.strings(&strings);
    // Successive offsets into one long tail.
    for id in 0..100_u16 {
        builder.entry(DT_SCE_IMPORT_LIB, library(1 + u32::from(id), 1, id));
    }
    let limits = sce_limits(|limits| limits.max_metadata_name_bytes = 5000);
    assert_eq!(
        run(&builder.build(), &Limits::default(), &limits),
        Err(SceError::NameBudgetExceeded {
            source: source(7, DT_SCE_IMPORT_LIB, library(6, 1, 5)),
            name_offset: 6,
            used: 1000 + 999 + 998 + 997 + 996,
            maximum: 5000,
        })
    );

    // An unterminated tail longer than one name stops at the per-name limit.
    let mut unterminated = vec![0];
    unterminated.extend(std::iter::repeat_n(b'a', 10_000));
    let mut builder = Builder::new(Domain::Guest);
    builder
        .strings(&unterminated)
        .entry(DT_SCE_IMPORT_LIB, library(1, 1, 0));
    assert_eq!(
        failure(&builder.build()),
        SceError::NameTooLong {
            source: source(2, DT_SCE_IMPORT_LIB, library(1, 1, 0)),
            name_offset: 1,
            maximum: 4096,
        }
    );
}

#[test]
fn string_tables_keep_their_address_domain_checks() {
    // A guest string table must be initialized; BSS cannot supply names.
    let declare = |at| {
        let mut builder = Builder::new(Domain::Guest);
        builder
            .strings_at(at, 4)
            .entry(DT_SCE_IMPORT_LIB, library(1, 1, 0));
        builder.build()
    };
    let bss = BASE + declare(BASE).len() as u64;
    assert!(matches!(
        failure(&declare(bss)),
        SceError::Link(LinkError::Load(_))
    ));

    // A dynlib offset is a slice of PT_SCE_DYNLIBDATA only, never the file.
    let mut builder = Builder::new(Domain::Dynlib);
    builder.place(b"\0abc\0");
    builder
        .strings_at(0x40, 5)
        .entry(DT_SCE_IMPORT_LIB, library(1, 1, 0));
    assert!(matches!(
        failure(&builder.build()),
        SceError::Link(LinkError::Load(_))
    ));
}

#[test]
fn generic_link_limits_still_apply() {
    let (strings, at) = strtab(&[b"x"]);
    let mut builder = Builder::new(Domain::Guest);
    builder
        .strings(&strings)
        .symbols(&[[0; 24], symbol(at[0], GLOBAL_FUNC, 0, 0)]);
    let limits = Limits {
        max_symbols: 1,
        ..Limits::default()
    };
    assert_eq!(
        run(&builder.build(), &limits, &SceLimits::default()),
        Err(SceError::Link(LinkError::LimitExceeded {
            limit: LimitKind::Symbols,
            requested: 2,
            maximum: 1,
        }))
    );
}
