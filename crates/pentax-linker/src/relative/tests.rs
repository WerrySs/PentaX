// SPDX-License-Identifier: GPL-2.0-or-later

//! Original in-memory ELF64 fixtures. No external executables or files are used.

use pentax_loader::open_executable;

use crate::{LimitKind, Limits, LinkError, Relocation, TableKind, inspect};

use super::{RelativeRelocationPlan, RelocationError, plan_relative};

const BASE: u64 = 0x10000;
const TARGET: u64 = 0x20000;
const PT_LOAD: u32 = 1;
const PT_SCE_RELRO: u32 = 0x6100_0010;
const FILE_SIZE: usize = 0x800;

#[derive(Debug, Clone, Copy)]
enum Domain {
    Guest,
    Dynlib,
}

const DOMAINS: [Domain; 2] = [Domain::Guest, Domain::Dynlib];

#[derive(Debug, Clone, Copy)]
struct Mapping {
    kind: u32,
    start: u64,
    size: u64,
    flags: u32,
}

fn mapping(start: u64, size: u64) -> Mapping {
    Mapping {
        kind: PT_LOAD,
        start,
        size,
        flags: 6,
    }
}

#[derive(Default)]
struct Fixture {
    rela: Vec<Relocation>,
    plt: Vec<Relocation>,
    mappings: Vec<Mapping>,
    symbols: bool,
    no_dynamic: bool,
}

fn relative(offset: u64, addend: i64) -> Relocation {
    record(offset, 0, 8, addend)
}

fn record(offset: u64, symbol: u32, kind: u32, addend: i64) -> Relocation {
    Relocation {
        offset,
        info: (u64::from(symbol) << 32) | u64::from(kind),
        addend,
    }
}

fn single(offset: u64, addend: i64) -> Fixture {
    Fixture {
        rela: vec![relative(offset, addend)],
        mappings: vec![mapping(TARGET, 0x100)],
        ..Fixture::default()
    }
}

fn plan(bytes: &[u8], bias: u64) -> Result<RelativeRelocationPlan<'_>, RelocationError> {
    let view = open_executable(bytes)?;
    plan_relative(&view, &Limits::default(), bias)
}

#[test]
fn relative_addends_are_word64_data_in_both_address_domains() {
    for domain in DOMAINS {
        for (bias, addend, value) in [
            (0x400000, 0x1234, 0x401234),
            (0x400000, -8, 0x3ffff8),
            (0, -1, u64::MAX),
            (1, -2, u64::MAX),
            (0, i64::MIN, 0x8000_0000_0000_0000),
            (0x8000_0000_0000_0001, i64::MAX, 0),
            (0x1234, i64::MAX, 0x8000_0000_0000_1233),
        ] {
            let bytes = image(domain, &single(TARGET, addend));
            let original = bytes.clone();
            let result = plan(&bytes, bias).unwrap();
            assert_eq!(result.load_bias(), bias);
            assert_eq!(result.patch_count(), 1);
            assert_eq!(result.deferred_count(), 0);
            assert_eq!(result.patches()[0].address, TARGET + bias);
            assert_eq!(result.patches()[0].value, value);
            assert_eq!(result.patches()[0].source.table, TableKind::Rela);
            assert_eq!(result.patches()[0].source.index, 0);
            assert_eq!(result.patches()[0].source.record, relative(TARGET, addend));
            assert_eq!(result.runtime_plan().metadata.entry, BASE);
            assert_eq!(bytes, original);
        }
    }
}

#[test]
fn none_is_no_write_and_unknown_or_symbolic_records_stay_deferred() {
    for domain in DOMAINS {
        let fixture = Fixture {
            // NONE does not require its meaningless offset to be mapped.
            rela: vec![record(u64::MAX, 0, 0, i64::MIN), record(TARGET, 1, 1, -24)],
            plt: vec![
                record(TARGET + 8, 1, 7, 0),
                record(u64::MAX, 0, 0xdead_beef, -7),
            ],
            symbols: true,
            ..Fixture::default()
        };
        let bytes = image(domain, &fixture);
        let view = open_executable(&bytes).unwrap();
        let before = inspect(&view, &Limits::default()).unwrap();
        let result = plan_relative(&view, &Limits::default(), u64::MAX).unwrap();
        assert!(result.patches().is_empty());
        assert_eq!(result.deferred_count(), 3);
        for (deferred, table, index, record) in [
            (&result.deferred()[0], TableKind::Rela, 1, fixture.rela[1]),
            (&result.deferred()[1], TableKind::PltRela, 0, fixture.plt[0]),
            (&result.deferred()[2], TableKind::PltRela, 1, fixture.plt[1]),
        ] {
            assert_eq!(
                (
                    deferred.source.table,
                    deferred.source.index,
                    deferred.source.record
                ),
                (table, index, record)
            );
        }
        // Planning never normalizes names, symbol values, raw types or hashes.
        assert_eq!(inspect(&view, &Limits::default()).unwrap(), before);
        assert_eq!(
            before.metadata().unwrap().symbols()[1]
                .name
                .as_ref()
                .unwrap()
                .as_bytes(),
            b"\xffX"
        );
    }
}

#[test]
fn plt_relative_records_are_planned_and_patches_sort_without_losing_context() {
    let fixture = Fixture {
        rela: vec![relative(TARGET + 16, 16), relative(TARGET, 0)],
        plt: vec![relative(TARGET + 8, 8)],
        mappings: vec![mapping(TARGET, 24)],
        ..Fixture::default()
    };
    for domain in DOMAINS {
        let bytes = image(domain, &fixture);
        let result = plan(&bytes, 0x4000).unwrap();
        assert_eq!(
            result
                .patches()
                .iter()
                .map(|patch| patch.address)
                .collect::<Vec<_>>(),
            [TARGET + 0x4000, TARGET + 0x4008, TARGET + 0x4010]
        );
        assert_eq!(result.patches()[0].source.index, 1);
        assert_eq!(result.patches()[1].source.table, TableKind::PltRela);
        assert_eq!(result.patches()[1].value, 0x4008);
    }
}

#[test]
fn relative_with_nonzero_symbol_is_a_structured_error() {
    for domain in DOMAINS {
        let fixture = Fixture {
            rela: vec![record(TARGET, 1, 8, 42)],
            mappings: vec![mapping(TARGET, 8)],
            symbols: true,
            ..Fixture::default()
        };
        let bytes = image(domain, &fixture);
        match plan(&bytes, 0) {
            Err(RelocationError::RelativeSymbolIndex { source, symbol }) => {
                assert_eq!(symbol, 1);
                assert_eq!(source.record, fixture.rela[0]);
                assert_eq!((source.table, source.index), (TableKind::Rela, 0));
            }
            other => panic!("expected invalid RELATIVE symbol, got {other:?}"),
        }
    }
}

#[test]
fn unaligned_bss_and_initialized_rx_targets_are_valid_staging_locations() {
    let fixture = Fixture {
        rela: vec![relative(BASE + 1, 17), relative(TARGET + 3, -1)],
        mappings: vec![Mapping {
            flags: 5,
            ..mapping(TARGET, 16)
        }],
        ..Fixture::default()
    };
    let mut bytes = image(Domain::Guest, &fixture);
    // The initialized metadata mapping is RX; the BSS-only target mapping is RX too.
    put(&mut bytes, 64 + 4, &5_u32.to_le_bytes());
    let result = plan(&bytes, 0).unwrap();
    assert_eq!(result.patches()[0].address, BASE + 1);
    assert_eq!(result.patches()[1].address, TARGET + 3);
    assert_eq!(result.patches()[1].value, u64::MAX);
}

#[test]
fn adjacent_logical_ranges_can_jointly_cover_an_unaligned_field() {
    for domain in DOMAINS {
        let fixture = Fixture {
            rela: vec![relative(TARGET + 1, 42)],
            // Deliberately in reverse program-header order.
            mappings: vec![mapping(TARGET + 5, 4), mapping(TARGET, 5)],
            ..Fixture::default()
        };
        let bytes = image(domain, &fixture);
        assert_eq!(plan(&bytes, 0).unwrap().patches()[0].value, 42);
    }
}

#[test]
fn eight_adjacent_single_byte_ranges_cover_exactly_one_field() {
    let fixture = Fixture {
        rela: vec![relative(TARGET, 42)],
        mappings: (0..8).map(|offset| mapping(TARGET + offset, 1)).collect(),
        ..Fixture::default()
    };
    // The fixture's extended program-header area remains below its dynamic section.
    let bytes = image(Domain::Guest, &fixture);
    assert_eq!(plan(&bytes, 0).unwrap().patch_count(), 1);
}

#[test]
fn a_one_byte_gap_or_partial_target_is_not_rounded_into_coverage() {
    for mappings in [
        vec![mapping(TARGET, 4), mapping(TARGET + 5, 4)],
        vec![mapping(TARGET + 1, 16)],
        vec![mapping(TARGET, 7)],
        vec![mapping(TARGET + 8, 8)],
        vec![],
    ] {
        let fixture = Fixture {
            mappings,
            ..single(TARGET, 0)
        };
        let bytes = image(Domain::Guest, &fixture);
        assert!(matches!(
            plan(&bytes, 0),
            Err(RelocationError::TargetOutsideSegments { .. })
        ));
    }
}

#[test]
fn guest_page_padding_and_elf_alignment_never_extend_logical_coverage() {
    for target in [TARGET - 8, TARGET + 8, TARGET + 0x3ff8] {
        let fixture = Fixture {
            mappings: vec![mapping(TARGET, 8)],
            ..single(target, 0)
        };
        let mut bytes = image(Domain::Guest, &fixture);
        // BSS-only target range starts at a 64 KiB boundary, so this is a valid
        // ELF alignment constraint. It does not create 64 KiB of logical data.
        put(&mut bytes, 64 + 56 + 48, &0x10000_u64.to_le_bytes());
        assert!(matches!(
            plan(&bytes, 0),
            Err(RelocationError::TargetOutsideSegments { .. })
        ));
    }
}

#[test]
fn standalone_sce_relro_is_logical_storage_for_relative_fields() {
    for domain in DOMAINS {
        let fixture = Fixture {
            mappings: vec![Mapping {
                kind: PT_SCE_RELRO,
                flags: 4,
                ..mapping(TARGET, 8)
            }],
            ..single(TARGET, 0x123)
        };
        let bytes = image(domain, &fixture);
        assert_eq!(plan(&bytes, 0x4000).unwrap().patches()[0].value, 0x4123);
    }
}

#[test]
fn logical_overlap_is_rejected_even_when_no_relative_writes_exist() {
    for mappings in [
        vec![mapping(TARGET, 16), mapping(TARGET + 8, 16)],
        vec![mapping(TARGET, 16), mapping(TARGET, 16)],
        vec![
            mapping(TARGET, 16),
            Mapping {
                kind: PT_SCE_RELRO,
                ..mapping(TARGET + 4, 4)
            },
        ],
    ] {
        let fixture = Fixture {
            mappings,
            ..Fixture::default()
        };
        let bytes = image(Domain::Guest, &fixture);
        assert!(matches!(
            plan(&bytes, 0),
            Err(RelocationError::OverlappingSegments { .. })
        ));
    }
}

#[test]
fn empty_logical_segments_do_not_create_overlap_or_cover_a_target() {
    let fixture = Fixture {
        mappings: vec![mapping(TARGET, 0), mapping(TARGET, 8)],
        ..single(TARGET, 42)
    };
    let bytes = image(Domain::Guest, &fixture);
    assert_eq!(plan(&bytes, 0).unwrap().patch_count(), 1);
    let fixture = Fixture {
        mappings: vec![mapping(TARGET, 0)],
        ..single(TARGET, 42)
    };
    let bytes = image(Domain::Guest, &fixture);
    assert!(matches!(
        plan(&bytes, 0),
        Err(RelocationError::TargetOutsideSegments { .. })
    ));
}

#[test]
fn original_target_overflow_is_distinct_from_data_value_wrapping() {
    for offset in [u64::MAX, u64::MAX - 7] {
        let bytes = image(Domain::Guest, &single(offset, -1));
        assert!(matches!(
            plan(&bytes, 0),
            Err(RelocationError::TargetRangeOverflow { .. })
        ));
    }
}

#[test]
fn bias_addition_and_biased_field_end_are_checked() {
    let bytes = image(Domain::Guest, &single(TARGET, -1));
    for bias in [u64::MAX - TARGET + 1, u64::MAX - TARGET - 7] {
        match plan(&bytes, bias) {
            Err(RelocationError::BiasedTargetOverflow { source, load_bias }) => {
                assert_eq!(source.record.offset, TARGET);
                assert_eq!(load_bias, bias);
            }
            other => panic!("expected biased target overflow, got {other:?}"),
        }
    }
    // The exclusive end may equal u64::MAX; only a further byte would overflow.
    let bias = u64::MAX - TARGET - 8;
    assert_eq!(
        plan(&bytes, bias).unwrap().patches()[0].address,
        u64::MAX - 8
    );
}

#[test]
fn partially_overlapping_and_cross_table_duplicate_writes_are_rejected() {
    for (rela, plt) in [
        (vec![relative(TARGET, 0), relative(TARGET + 7, 1)], vec![]),
        (vec![relative(TARGET + 7, 0)], vec![relative(TARGET, 1)]),
        (vec![relative(TARGET, 0)], vec![relative(TARGET, 1)]),
        (
            vec![
                relative(TARGET, 0),
                relative(TARGET + 16, 1),
                relative(TARGET, 2),
            ],
            vec![],
        ),
    ] {
        let fixture = Fixture {
            rela,
            plt,
            mappings: vec![mapping(TARGET, 32)],
            ..Fixture::default()
        };
        let bytes = image(Domain::Guest, &fixture);
        assert!(matches!(
            plan(&bytes, 0x4000),
            Err(RelocationError::OverlappingPatches { .. })
        ));
    }
}

#[test]
fn overflowing_logical_mapping_is_rejected_by_the_image_bound_runtime_plan() {
    let fixture = Fixture {
        mappings: vec![mapping(u64::MAX - 7, 8)],
        ..Fixture::default()
    };
    let bytes = image(Domain::Guest, &fixture);
    assert!(matches!(
        plan(&bytes, 0),
        Err(RelocationError::Link(LinkError::Load(
            pentax_loader::LoadError::RangeOverflow {
                context: "load segment memory",
                ..
            }
        )))
    ));
}

#[test]
fn exact_logical_start_and_end_are_accepted_without_a_trailing_padding_byte() {
    for target in [TARGET, TARGET + 8] {
        let fixture = Fixture {
            mappings: vec![mapping(TARGET, 16)],
            ..single(target, 42)
        };
        let bytes = image(Domain::Guest, &fixture);
        assert_eq!(plan(&bytes, 0).unwrap().patch_count(), 1);
    }
    let fixture = Fixture {
        mappings: vec![mapping(TARGET, 16)],
        ..single(TARGET + 9, 42)
    };
    let bytes = image(Domain::Guest, &fixture);
    assert!(matches!(
        plan(&bytes, 0),
        Err(RelocationError::TargetOutsideSegments { .. })
    ));
}

#[test]
fn consecutive_same_offset_sequences_with_relative_are_rejected_in_either_table() {
    for kinds in [[8, 8], [8, 1], [1, 8], [8, 0], [0, 8]] {
        for table in [TableKind::Rela, TableKind::PltRela] {
            let records = kinds.map(|kind| record(TARGET, 0, kind, 42)).to_vec();
            let mut fixture = single(TARGET, 0);
            fixture.rela.clear();
            if table == TableKind::Rela {
                fixture.rela = records;
            } else {
                fixture.plt = records;
            }
            let bytes = image(Domain::Guest, &fixture);
            match plan(&bytes, 0) {
                Err(RelocationError::UnsupportedComposition { first, second }) => {
                    assert_eq!(
                        (first.table, first.index, second.table, second.index),
                        (table, 0, table, 1)
                    );
                }
                other => panic!("expected unsupported composition, got {other:?}"),
            }
        }
    }
}

#[test]
fn unsupported_compositions_are_retained_without_inventing_writes() {
    let fixture = Fixture {
        rela: vec![record(TARGET, 0, 1, 4), record(TARGET, 0, 7, 5)],
        ..Fixture::default()
    };
    let bytes = image(Domain::Guest, &fixture);
    let result = plan(&bytes, 0).unwrap();
    assert!(result.patches().is_empty());
    assert_eq!(result.deferred_count(), 2);
}

#[test]
fn budgets_cover_noop_relative_and_deferred_records_together() {
    let fixture = Fixture {
        rela: vec![record(u64::MAX, 0, 0, 0), relative(TARGET, 42)],
        plt: vec![record(u64::MAX, 0, 0xff, -1)],
        mappings: vec![mapping(TARGET, 8)],
        ..Fixture::default()
    };
    let bytes = image(Domain::Guest, &fixture);
    let view = open_executable(&bytes).unwrap();
    for maximum in 0..3 {
        let limits = Limits {
            max_relocations: maximum,
            ..Limits::default()
        };
        assert!(matches!(plan_relative(&view, &limits, 0),
            Err(RelocationError::Link(LinkError::LimitExceeded {
                limit: LimitKind::Relocations, requested: 3, maximum: actual
            })) if actual == maximum));
    }
    let limits = Limits {
        max_relocations: 3,
        ..Limits::default()
    };
    let result = plan_relative(&view, &limits, 0).unwrap();
    assert_eq!((result.patch_count(), result.deferred_count()), (1, 1));
}

#[test]
fn a_later_invalid_target_returns_only_an_error_and_leaves_the_view_unchanged() {
    let fixture = Fixture {
        rela: vec![relative(TARGET, 42), relative(TARGET + 0x100, 43)],
        ..single(TARGET, 0)
    };
    let bytes = image(Domain::Guest, &fixture);
    let original = bytes.clone();
    let view = open_executable(&bytes).unwrap();
    assert!(
        matches!(plan_relative(&view, &Limits::default(), 0), Err(RelocationError::TargetOutsideSegments { source }) if source.index == 1)
    );
    assert_eq!(bytes, original);
    assert_eq!(
        inspect(&view, &Limits::default())
            .unwrap()
            .metadata()
            .unwrap()
            .rela(),
        fixture.rela
    );
}

#[test]
fn static_and_empty_dynamic_images_have_empty_plans_under_zero_limits() {
    for no_dynamic in [true, false] {
        let fixture = Fixture {
            no_dynamic,
            ..Fixture::default()
        };
        let bytes = image(Domain::Guest, &fixture);
        let view = open_executable(&bytes).unwrap();
        let limits = Limits {
            max_table_bytes: 0,
            max_name_bytes: 0,
            max_symbols: 0,
            max_relocations: 0,
        };
        let result = plan_relative(&view, &limits, 0).unwrap();
        assert_eq!((result.patch_count(), result.deferred_count()), (0, 0));
        assert_eq!(result.runtime_plan().dynamic.is_none(), no_dynamic);
    }
}

// Every image maps its complete 0x800-byte metadata file at BASE. Additional
// mappings are BSS-only so their logical geometry is independent of table data.
// Program headers occupy up to 0x340; the dynamic section starts at 0x350 and
// tables start at 0x480. No disk or game bytes enter these fixtures.
fn image(domain: Domain, fixture: &Fixture) -> Vec<u8> {
    const DYNAMIC: usize = 0x350;
    const RELA: usize = 0x480;
    const PLT: usize = 0x600;
    const SYMBOLS: usize = 0x700;
    const STRINGS: usize = 0x780;
    let mut bytes = vec![0; FILE_SIZE];
    bytes[..9].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 9, 2]);
    let dynlib = matches!(domain, Domain::Dynlib);
    put(
        &mut bytes,
        16,
        &(if dynlib { 0xfe10_u16 } else { 3 }).to_le_bytes(),
    );
    put(&mut bytes, 18, &62_u16.to_le_bytes());
    put(&mut bytes, 20, &1_u32.to_le_bytes());
    put(&mut bytes, 24, &BASE.to_le_bytes());
    put(&mut bytes, 32, &64_u64.to_le_bytes());
    put(&mut bytes, 52, &64_u16.to_le_bytes());
    put(&mut bytes, 54, &56_u16.to_le_bytes());
    let headers =
        1 + fixture.mappings.len() + usize::from(!fixture.no_dynamic) + usize::from(dynlib);
    assert!(64 + headers * 56 <= DYNAMIC);
    put(&mut bytes, 56, &(headers as u16).to_le_bytes());
    program_header(
        &mut bytes,
        0,
        PT_LOAD,
        6,
        [0, BASE, FILE_SIZE as u64, FILE_SIZE as u64, 1],
    );
    for (index, mapping) in fixture.mappings.iter().enumerate() {
        program_header(
            &mut bytes,
            index + 1,
            mapping.kind,
            mapping.flags,
            [0, mapping.start, 0, mapping.size, 1],
        );
    }
    let address = |at: usize| {
        if dynlib {
            (at - RELA) as u64
        } else {
            BASE + at as u64
        }
    };
    let tags = if dynlib {
        [
            0x6100_002f,
            0x6100_0031,
            0x6100_0033,
            0x6100_0029,
            0x6100_002d,
            0x6100_002b,
            0x6100_0039,
            0x6100_003f,
            0x6100_003b,
            0x6100_0035,
            0x6100_0037,
        ]
    } else {
        [7, 8, 9, 23, 2, 20, 6, 39, 11, 5, 10]
    };
    let mut dynamic = Vec::<(i64, u64)>::new();
    if !fixture.rela.is_empty() {
        assert!(fixture.rela.len() * 24 <= PLT - RELA);
        encode_records(&mut bytes, RELA, &fixture.rela);
        dynamic.extend([
            (tags[0], address(RELA)),
            (tags[1], fixture.rela.len() as u64 * 24),
            (tags[2], 24),
        ]);
    }
    if !fixture.plt.is_empty() {
        assert!(fixture.plt.len() * 24 <= SYMBOLS - PLT);
        encode_records(&mut bytes, PLT, &fixture.plt);
        dynamic.extend([
            (tags[3], address(PLT)),
            (tags[4], fixture.plt.len() as u64 * 24),
            (tags[5], 7),
        ]);
    }
    if fixture.symbols {
        // Zeroth entry is zero. Symbol one has an arbitrary-byte name and a
        // nonzero raw value, which planning must never alter or resolve.
        let at = SYMBOLS + 24;
        put(&mut bytes, at, &1_u32.to_le_bytes());
        bytes[at + 4] = 0x12;
        bytes[at + 5] = 0xf3;
        put(&mut bytes, at + 8, &u64::MAX.to_le_bytes());
        put(&mut bytes, STRINGS, b"\0\xffX\0");
        dynamic.extend([
            (tags[6], address(SYMBOLS)),
            (tags[7], 48),
            (tags[8], 24),
            (tags[9], address(STRINGS)),
            (tags[10], 4),
        ]);
    }
    dynamic.push((0, 0));
    assert!(DYNAMIC + dynamic.len() * 16 <= RELA);
    for (index, (tag, value)) in dynamic.iter().enumerate() {
        put(&mut bytes, DYNAMIC + index * 16, &tag.to_le_bytes());
        put(&mut bytes, DYNAMIC + index * 16 + 8, &value.to_le_bytes());
    }
    let next = 1 + fixture.mappings.len();
    if !fixture.no_dynamic {
        program_header(
            &mut bytes,
            next,
            2,
            6,
            [
                DYNAMIC as u64,
                BASE + DYNAMIC as u64,
                dynamic.len() as u64 * 16,
                dynamic.len() as u64 * 16,
                8,
            ],
        );
    }
    if dynlib {
        program_header(
            &mut bytes,
            next + usize::from(!fixture.no_dynamic),
            0x6100_0000,
            0,
            [RELA as u64, 0, (FILE_SIZE - RELA) as u64, 0, 0],
        );
    }
    bytes
}

fn encode_records(bytes: &mut [u8], at: usize, records: &[Relocation]) {
    for (index, record) in records.iter().enumerate() {
        let at = at + index * 24;
        put(bytes, at, &record.offset.to_le_bytes());
        put(bytes, at + 8, &record.info.to_le_bytes());
        put(bytes, at + 16, &record.addend.to_le_bytes());
    }
}

fn program_header(
    bytes: &mut [u8],
    index: usize,
    kind: u32,
    flags: u32,
    [offset, vaddr, file_size, mem_size, align]: [u64; 5],
) {
    let at = 64 + index * 56;
    put(bytes, at, &kind.to_le_bytes());
    put(bytes, at + 4, &flags.to_le_bytes());
    put(bytes, at + 8, &offset.to_le_bytes());
    put(bytes, at + 16, &vaddr.to_le_bytes());
    put(bytes, at + 32, &file_size.to_le_bytes());
    put(bytes, at + 40, &mem_size.to_le_bytes());
    put(bytes, at + 48, &align.to_le_bytes());
}

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}
