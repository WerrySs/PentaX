// SPDX-License-Identifier: GPL-2.0-or-later

//! Observed `_1` descriptors, per-role encoding selection, same-role conflicts
//! and opaque original-filename markers.

use super::fixture::{Builder, DOMAINS, library, module, strtab, symbol};
use super::{GLOBAL_FUNC, NID, WEAK_FUNC, failure, metadata, run, source};
use crate::{
    AttributeRole, DT_SCE_EXPORT_LIB_1, DT_SCE_EXPORT_LIB_ATTR, DT_SCE_IMPORT_LIB,
    DT_SCE_IMPORT_LIB_1, DT_SCE_IMPORT_LIB_ATTR, DT_SCE_MODULE_ATTR, DT_SCE_MODULE_INFO_1,
    DT_SCE_NEEDED_MODULE, DT_SCE_NEEDED_MODULE_1, DT_SCE_ORIGINAL_FILENAME,
    DT_SCE_ORIGINAL_FILENAME_1, DescriptorEncoding, DescriptorRole, Limits, NidToken, QualifiedNid,
    RoleConflict, SceDescriptors, SceError, SceLimits, SelectorId, SelectorMatch,
};

const CLASSIC: DescriptorEncoding = DescriptorEncoding::Classic;
const OBSERVED: DescriptorEncoding = DescriptorEncoding::Variant1Observed;
const ROLES: [DescriptorRole; 4] = [
    DescriptorRole::ModuleInfo,
    DescriptorRole::NeededModule,
    DescriptorRole::ExportLibrary,
    DescriptorRole::ImportLibrary,
];
/// One encoding per role, alternating, for fixtures that mix roles.
const MIXED: [DescriptorEncoding; 4] = [OBSERVED, CLASSIC, OBSERVED, CLASSIC];

const fn encoding(observed: bool) -> DescriptorEncoding {
    if observed { OBSERVED } else { CLASSIC }
}

/// A canonical qualified import name whose selectors are the given IDs.
fn import_name(library_id: u16, module_id: u16) -> Vec<u8> {
    let nid = NidToken::parse(NID).unwrap();
    let name = QualifiedNid::new(nid, SelectorId::new(library_id), SelectorId::new(module_id));
    name.to_bytes()
}

#[test]
fn observed_roles_decode_every_field_in_both_domains() {
    let declared = import_name(0xfedc, 0x0102);
    let missing_module = import_name(0xfedc, 0x0103);
    let missing_library = import_name(7, 0x0102);
    let names: [&[u8]; 7] = [
        b"libSelf",
        b"libDep",
        b"libExport",
        b"libImport",
        &declared,
        &missing_module,
        &missing_library,
    ];
    let (strings, at) = strtab(&names);
    let symbols = [
        [0; 24],
        symbol(at[4], GLOBAL_FUNC, 0, 0),
        symbol(at[5], WEAK_FUNC, 0, 0),
        symbol(at[6], GLOBAL_FUNC, 0, 0),
    ];
    // Each role has its own name offset, version and ID. Older-numbered
    // attribute tags sit beside the modern descriptors.
    let values = [
        (DT_SCE_IMPORT_LIB_1, library(at[3], 0xa5c3, 0xfedc)),
        (DT_SCE_IMPORT_LIB_ATTR, 0xffff_0000_0000_0009),
        (DT_SCE_MODULE_INFO_1, module(at[0], 0xa5, 0xc3, 0x0001)),
        (DT_SCE_MODULE_ATTR, 0),
        (DT_SCE_NEEDED_MODULE_1, module(at[1], 0x03, 0x7f, 0x0102)),
        (DT_SCE_EXPORT_LIB_1, library(at[2], 0x0001, 0)),
        (DT_SCE_EXPORT_LIB_ATTR, 1),
    ];
    for domain in DOMAINS {
        let mut builder = Builder::new(domain);
        builder.pad(16).strings(&strings).symbols(&symbols);
        let first = builder.entries.len();
        for (tag, value) in values {
            builder.entry(tag, value);
        }
        let metadata = metadata(&builder.build());
        let SceDescriptors::Decoded(decoded) = metadata.descriptors() else {
            panic!("expected decoded descriptors");
        };
        for role in ROLES {
            assert_eq!(decoded.encoding(role), Some(OBSERVED), "{role}");
        }

        let [own] = decoded.module_info() else {
            panic!("one module info");
        };
        let expected = source(first + 2, DT_SCE_MODULE_INFO_1, values[2].1);
        assert_eq!(own.source(), expected);
        assert_eq!(own.encoding(), OBSERVED);
        assert_eq!(own.name().as_bytes(), b"libSelf");
        // The major version is the high byte of the raw u16.
        assert_eq!(own.raw_version(), 0xa5c3);
        assert_eq!((own.version_major(), own.version_minor()), (0xa5, 0xc3));
        assert_eq!(own.id().value(), 1);

        let [needed] = decoded.needed_modules() else {
            panic!("one needed module");
        };
        assert_eq!(needed.name().as_bytes(), b"libDep");
        assert_eq!(needed.raw_version(), 0x037f);
        assert_eq!(needed.id().value(), 0x0102);

        let [export] = decoded.export_libraries() else {
            panic!("one export library");
        };
        assert_eq!(export.name().as_bytes(), b"libExport");
        assert_eq!((export.version(), export.id().value()), (1, 0));

        let [import] = decoded.import_libraries() else {
            panic!("one import library");
        };
        assert_eq!(import.source().index, first);
        assert_eq!(import.encoding(), OBSERVED);
        assert_eq!(import.name().as_bytes(), b"libImport");
        // A library version is one opaque u16, never split.
        assert_eq!((import.version(), import.id().value()), (0xa5c3, 0xfedc));

        // Every decoded field reconstructs the retained raw word.
        for record in [own, needed] {
            let (offset, id) = (record.name_offset(), record.id().value());
            let split = module(offset, record.version_major(), record.version_minor(), id);
            assert_eq!(split, record.source().value);
            assert_eq!(library(offset, record.raw_version(), id), split);
        }
        for record in [export, import] {
            let rebuilt = library(record.name_offset(), record.version(), record.id().value());
            assert_eq!(rebuilt, record.source().value);
        }

        let attributes: Vec<_> = metadata
            .attributes()
            .iter()
            .map(|attribute| (attribute.role(), attribute.raw()))
            .collect();
        assert_eq!(
            attributes,
            [
                (AttributeRole::ImportLibrary, 0xffff_0000_0000_0009),
                (AttributeRole::Module, 0),
                (AttributeRole::ExportLibrary, 1),
            ]
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
                )
            })
            .collect();
        assert_eq!(
            associations,
            [
                (1, SelectorMatch::Declared(0), SelectorMatch::Declared(0)),
                (2, SelectorMatch::Declared(0), SelectorMatch::Missing),
                (3, SelectorMatch::Missing, SelectorMatch::Declared(0)),
            ]
        );
    }
}

#[test]
fn each_role_selects_its_own_encoding() {
    let import = import_name(5, 5);
    let names: [&[u8]; 2] = [b"lib", &import];
    let (strings, at) = strtab(&names);
    // Every combination of per-role encodings. Equal IDs in different roles
    // stay unrelated, and associations use whichever encodings the two import
    // roles selected.
    for mask in 0..16_u32 {
        let pick = |bit: u32| encoding(mask & (1 << bit) != 0);
        let encodings = [pick(0), pick(1), pick(2), pick(3)];
        for domain in DOMAINS {
            let mut builder = Builder::new(domain);
            builder
                .strings(&strings)
                .symbols(&[[0; 24], symbol(at[1], GLOBAL_FUNC, 0, 0)]);
            for (role, encoding) in ROLES.into_iter().zip(encodings) {
                builder.entry(role.tag(encoding), library(at[0], 1, 5));
            }
            let metadata = metadata(&builder.build());
            let decoded = metadata.descriptors().decoded().unwrap();
            for (role, encoding) in ROLES.into_iter().zip(encodings) {
                assert_eq!(decoded.encoding(role), Some(encoding), "{mask:04b}");
            }
            assert_eq!(decoded.module_info()[0].encoding(), encodings[0]);
            assert_eq!(decoded.import_libraries()[0].encoding(), encodings[3]);
            let [association] = metadata.import_associations().unwrap() else {
                panic!("one association");
            };
            assert!(association.is_declared(), "{mask:04b}");
        }
    }
}

#[test]
fn each_same_role_conflict_is_raw_and_suppresses_every_association() {
    let import = import_name(7, 7);
    let names: [&[u8]; 2] = [b"lib", &import];
    let (strings, at) = strtab(&names);
    for conflicting in ROLES {
        let (classic_tag, observed_tag) = (conflicting.tag(CLASSIC), conflicting.tag(OBSERVED));
        // Equal and disjoint IDs across the two sides; the classic side also
        // repeats its ID. Unreadable name offsets prove nothing is named.
        for observed_id in [7, 0xfedc] {
            let classic = library(u32::MAX, 0x0101, 7);
            let observed = library(u32::MAX, 0xa5c3, observed_id);
            for domain in DOMAINS {
                let mut builder = Builder::new(domain);
                builder
                    .strings(&strings)
                    .symbols(&[[0; 24], symbol(at[1], GLOBAL_FUNC, 0, 0)]);
                let first = builder.entries.len();
                builder
                    .entry(observed_tag, observed)
                    .entry(classic_tag, classic)
                    .entry(classic_tag, classic)
                    .entry(observed_tag, u64::MAX);
                // The other roles mix encodings and remain decodable.
                for (role, encoding) in ROLES.into_iter().zip(MIXED) {
                    if role != conflicting {
                        builder.entry(role.tag(encoding), library(at[0], 1, 7));
                    }
                }
                let metadata = metadata(&builder.build());
                let SceDescriptors::Conflicted(conflicted) = metadata.descriptors() else {
                    panic!("expected a conflict");
                };
                let [conflict] = conflicted.conflicts() else {
                    panic!("one conflicting role");
                };
                assert_eq!(conflict.role(), conflicting);
                assert_eq!(
                    conflict.classic(),
                    [
                        source(first + 1, classic_tag, classic),
                        source(first + 2, classic_tag, classic),
                    ]
                );
                assert_eq!(
                    conflict.variant1(),
                    [
                        source(first, observed_tag, observed),
                        source(first + 3, observed_tag, u64::MAX),
                    ]
                );

                let decoded = conflicted.decoded();
                for (role, encoding) in ROLES.into_iter().zip(MIXED) {
                    let expected = (role != conflicting).then_some(encoding);
                    assert_eq!(decoded.encoding(role), expected, "{role}");
                }
                // Only the three other roles were named.
                assert_eq!(metadata.metadata_name_bytes(), 3 * 3);
                assert_eq!(metadata.descriptors().decoded(), None);
                // The qualified import is listed, but nothing is associated.
                assert_eq!(metadata.qualified_symbols().len(), 1);
                assert_eq!(metadata.import_associations(), None);
            }
        }
    }
}

#[test]
fn every_role_may_conflict_without_reading_names() {
    for domain in DOMAINS {
        let mut builder = Builder::new(domain);
        // Roles appear in reverse order and there is no string table.
        for role in ROLES.into_iter().rev() {
            builder
                .entry(role.tag(CLASSIC), u64::MAX)
                .entry(role.tag(OBSERVED), u64::MAX);
        }
        let metadata = metadata(&builder.build());
        let SceDescriptors::Conflicted(conflicted) = metadata.descriptors() else {
            panic!("expected conflicts");
        };
        let roles: Vec<_> = conflicted
            .conflicts()
            .iter()
            .map(RoleConflict::role)
            .collect();
        assert_eq!(roles, ROLES);
        for conflict in conflicted.conflicts() {
            assert_eq!(conflict.classic().len(), 1);
            assert_eq!(conflict.variant1().len(), 1);
        }
        for role in ROLES {
            assert_eq!(conflicted.decoded().encoding(role), None);
        }
        assert_eq!(metadata.metadata_strings(), None);
        assert_eq!(
            (metadata.table_bytes(), metadata.metadata_name_bytes()),
            (0, 0)
        );
        assert_eq!(metadata.import_associations(), None);
    }
}

#[test]
fn observed_roles_keep_the_duplicate_id_policy() {
    let (strings, at) = strtab(&[b"one", b"two"]);
    for role in ROLES {
        let tag = role.tag(OBSERVED);
        let first = library(at[0], 1, 0xfedc);
        // The repeat is rejected before its unreadable name is scanned.
        let second = library(u32::MAX, 2, 0xfedc);
        for domain in DOMAINS {
            let mut builder = Builder::new(domain);
            builder
                .strings(&strings)
                .entry(tag, first)
                .entry(tag, second);
            let error = failure(&builder.build());
            assert_eq!(
                error,
                SceError::DuplicateDescriptorId {
                    role,
                    encoding: OBSERVED,
                    id: 0xfedc,
                    first: source(2, tag, first),
                    second: source(3, tag, second),
                }
            );
            if role == DescriptorRole::NeededModule {
                let text = error.to_string();
                assert!(text.starts_with("DT_SCE_NEEDED_MODULE_1 ID 65244 "));
            }
        }
    }
}

#[test]
fn observed_names_keep_every_string_table_check() {
    for domain in DOMAINS {
        let image = |strings: Option<&[u8]>, value| {
            let mut builder = Builder::new(domain);
            if let Some(strings) = strings {
                builder.strings(strings);
            }
            builder.entry(DT_SCE_EXPORT_LIB_1, value);
            builder.build()
        };
        let value = library(1, 1, 0);
        assert_eq!(
            failure(&image(None, value)),
            SceError::MissingStringTable {
                source: source(0, DT_SCE_EXPORT_LIB_1, value),
                name_offset: 1,
            }
        );
        // With every bit set, only the low 32 bits are a table offset.
        assert_eq!(
            failure(&image(Some(&b"\0abc"[..]), u64::MAX)),
            SceError::NameOffsetOutOfRange {
                source: source(2, DT_SCE_EXPORT_LIB_1, u64::MAX),
                name_offset: u64::from(u32::MAX),
                table_size: 4,
            }
        );
        assert_eq!(
            failure(&image(Some(&b"\0abc"[..]), value)),
            SceError::UnterminatedName {
                source: source(2, DT_SCE_EXPORT_LIB_1, value),
                name_offset: 1,
            }
        );
    }
}

#[test]
fn filename_markers_alone_are_opaque_and_read_nothing() {
    let values = [
        (DT_SCE_ORIGINAL_FILENAME_1, u64::MAX),
        (DT_SCE_ORIGINAL_FILENAME, 0x0000_0001_0000_0001),
        (DT_SCE_ORIGINAL_FILENAME_1, 0xffff_ffff_0000_0000),
    ];
    for domain in DOMAINS {
        for strings in [None, Some(&b"\0a\0"[..])] {
            let mut builder = Builder::new(domain);
            if let Some(strings) = strings {
                builder.strings(strings);
            }
            let first = builder.entries.len();
            for (tag, value) in values {
                builder.entry(tag, value);
            }
            let metadata = metadata(&builder.build());
            assert_eq!(metadata.descriptors(), &SceDescriptors::Absent);
            let markers: Vec<_> = metadata
                .original_filenames()
                .iter()
                .map(|marker| (marker.source(), marker.raw()))
                .collect();
            let expected: Vec<_> = values
                .iter()
                .enumerate()
                .map(|(offset, &(tag, value))| (source(first + offset, tag, value), value))
                .collect();
            assert_eq!(markers, expected);
            // No name is looked up, so no string table is read or charged.
            assert_eq!(metadata.metadata_strings(), None);
            assert_eq!(
                (metadata.table_bytes(), metadata.metadata_name_bytes()),
                (0, 0)
            );
            assert_eq!(metadata.import_associations(), None);
        }
    }
}

#[test]
fn filename_markers_never_select_or_taint_descriptors() {
    let import = import_name(1, 2);
    let names: [&[u8]; 2] = [b"lib", &import];
    let (strings, at) = strtab(&names);
    let markers: [&[(i64, u64)]; 3] = [
        &[(DT_SCE_ORIGINAL_FILENAME_1, u64::MAX)],
        &[(DT_SCE_ORIGINAL_FILENAME, u64::MAX)],
        &[
            (DT_SCE_ORIGINAL_FILENAME, 1),
            (DT_SCE_ORIGINAL_FILENAME_1, 2),
        ],
    ];
    for domain in DOMAINS {
        for encoding in [CLASSIC, OBSERVED] {
            let library_tag = DescriptorRole::ImportLibrary.tag(encoding);
            let module_tag = DescriptorRole::NeededModule.tag(encoding);
            for marker in markers {
                let mut builder = Builder::new(domain);
                builder
                    .strings(&strings)
                    .symbols(&[[0; 24], symbol(at[1], GLOBAL_FUNC, 0, 0)]);
                for &(tag, value) in marker {
                    builder.entry(tag, value);
                }
                builder
                    .entry(library_tag, library(at[0], 1, 1))
                    .entry(module_tag, module(at[0], 1, 0, 2));
                let metadata = metadata(&builder.build());
                let decoded = metadata.descriptors().decoded().unwrap();
                for role in [DescriptorRole::NeededModule, DescriptorRole::ImportLibrary] {
                    assert_eq!(decoded.encoding(role), Some(encoding));
                }
                assert_eq!(metadata.original_filenames().len(), marker.len());
                // Only the two descriptor names were read.
                assert_eq!(metadata.metadata_name_bytes(), 6);
                assert!(metadata.import_associations().unwrap()[0].is_declared());
            }
        }
    }
}

#[test]
fn inspecting_any_encoding_state_does_not_mutate() {
    let import = import_name(0xfedc, 1);
    let names: [&[u8]; 2] = [b"lib", &import];
    let (strings, at) = strtab(&names);
    let states: [&[(i64, u64)]; 3] = [
        // An observed role beside a classic role.
        &[
            (DT_SCE_IMPORT_LIB_1, library(at[0], 0xa5c3, 0xfedc)),
            (DT_SCE_NEEDED_MODULE, module(at[0], 1, 0, 1)),
        ],
        // A same-role conflict.
        &[
            (DT_SCE_IMPORT_LIB, u64::MAX),
            (DT_SCE_IMPORT_LIB_1, u64::MAX),
        ],
        // Filename markers only.
        &[
            (DT_SCE_ORIGINAL_FILENAME, u64::MAX),
            (DT_SCE_ORIGINAL_FILENAME_1, u64::MAX),
        ],
    ];
    for domain in DOMAINS {
        for state in states {
            let mut builder = Builder::new(domain);
            builder
                .strings(&strings)
                .symbols(&[[0; 24], symbol(at[1], GLOBAL_FUNC, 0, 0)]);
            for &(tag, value) in state {
                builder.entry(tag, value);
            }
            let bytes = builder.build();
            let original = bytes.clone();
            let first = run(&bytes, &Limits::default(), &SceLimits::default()).unwrap();
            let second = run(&bytes, &Limits::default(), &SceLimits::default()).unwrap();
            assert_eq!(bytes, original);
            assert_eq!(first, second);
        }
    }
}
