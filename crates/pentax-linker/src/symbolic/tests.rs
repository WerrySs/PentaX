// SPDX-License-Identifier: GPL-2.0-or-later

mod atomic_delta;
mod eligibility;
mod extension;
mod fallback;
mod missing;
mod module_lookup;
use super::fixture;
use super::*;
use crate::{
    DT_SCE_IMPORT_LIB, DT_SCE_NEEDED_MODULE_1, DT_SCE_ORIGINAL_FILENAME,
    DT_SCE_ORIGINAL_FILENAME_1, DescriptorEncoding, DescriptorRole, RelocationError, TableKind,
};
use fixture::*;
use pentax_loader::open_executable;

fn plan<'a>(a: &'a [u8], b: &'a [u8]) -> Result<SymbolicLinkPlan<'a>, SymbolicError> {
    plan_symbolic(
        &[
            ModuleInput::new(open_executable(a).unwrap(), A_BIAS),
            ModuleInput::new(open_executable(b).unwrap(), B_BIAS),
        ],
        &SymbolicLimits::default(),
    )
}

#[test]
fn scoped_two_module_values_sources_and_deferred_are_image_bound() {
    for dynlib in [false, true] {
        let mut a = Module::consumer();
        let mut b = Module::provider();
        a.dynlib = dynlib;
        b.dynlib = dynlib;
        a.rela = vec![
            (TARGET, 1, 1, -8),
            (TARGET + 16, 1, 6, 123),
            (TARGET + 32, 0, 8, -1),
            (TARGET + 48, 1, 37, 0),
        ];
        a.plt = vec![(TARGET + 64, 1, 7, -123)];
        let a = a.build();
        let b = b.build();
        let result = plan(&a, &b).unwrap();
        assert_eq!(result.modules().len(), 2);
        assert_eq!(result.modules()[1].runtime_plan().metadata.entry, 0);
        assert_eq!(result.primary_entry(), A_BIAS + BASE);
        assert_eq!(result.modules()[1].load_bias(), B_BIAS);
        assert_eq!(
            result
                .patches()
                .iter()
                .map(SymbolicPatch::value)
                .collect::<Vec<_>>(),
            [
                B_BIAS + DEFINITION - 8,
                B_BIAS + DEFINITION,
                A_BIAS - 1,
                B_BIAS + DEFINITION
            ]
        );
        assert_eq!(
            result.patches()[0].resolution(),
            Resolution::Definition(SymbolRef {
                module: ModuleId(1),
                symbol: 1
            })
        );
        assert_eq!(result.patches()[0].consumer(), ModuleId(0));
        assert_eq!(result.patches()[0].source().table, TableKind::Rela);
        assert_eq!(result.patches()[3].source().table, TableKind::PltRela);
        assert_eq!(result.deferred_count(), 1);
        assert_eq!(result.deferred()[0].module(), ModuleId(0));
        assert_eq!(result.deferred()[0].source().record.kind(), 37);
        assert_eq!(result.patches()[0].address(), A_BIAS + TARGET);
        assert!(a[0x900..0x908].iter().all(|&byte| byte == 0));
    }
}

#[test]
fn classic_observed_and_cross_role_encodings_bind_by_decoded_semantics() {
    let encodings = [
        DescriptorEncoding::Classic,
        DescriptorEncoding::Variant1Observed,
    ];
    for dynlib in [false, true] {
        for am in encodings {
            for al in encodings {
                for bm in encodings {
                    for bl in encodings {
                        let mut a = Module::consumer();
                        let mut b = Module::provider();
                        a.dynlib = dynlib;
                        b.dynlib = dynlib;
                        a.module_encoding = am;
                        a.library_encoding = al;
                        b.module_encoding = bm;
                        b.library_encoding = bl;
                        let a = a.build();
                        let b = b.build();
                        let result = plan(&a, &b).unwrap();
                        assert_eq!(result.patch_count(), 1);
                        assert_eq!(result.patches()[0].value(), B_BIAS + DEFINITION - 8);
                        assert_eq!(
                            result.patches()[0].resolution(),
                            Resolution::Definition(SymbolRef {
                                module: ModuleId(1),
                                symbol: 1
                            })
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn unequal_module_versions_match_hardcoded_raw_words_in_both_encodings() {
    for dynlib in [false, true] {
        for encoding in [
            DescriptorEncoding::Classic,
            DescriptorEncoding::Variant1Observed,
        ] {
            let mut a = Module::consumer();
            let mut b = Module::provider();
            a.dynlib = dynlib;
            b.dynlib = dynlib;
            a.module_encoding = encoding;
            b.module_encoding = encoding;
            let a = a.build();
            let mut b = b.build();
            // Independent literal words: ID=2/31, version=0x0203, name=1.
            // They pin major=2/minor=3 without using the fixture's packer.
            assert_eq!(
                u64::from_le_bytes(a[0xc88..0xc90].try_into().unwrap()),
                0x0002_0203_0000_0001
            );
            assert_eq!(
                u64::from_le_bytes(b[0xc58..0xc60].try_into().unwrap()),
                0x001f_0203_0000_0001
            );
            let result = plan(&a, &b).unwrap();
            let decoded = result.modules()[1]
                .metadata
                .as_ref()
                .unwrap()
                .descriptors()
                .decoded()
                .unwrap();
            assert_eq!(decoded.module_info()[0].version_major(), 2);
            assert_eq!(decoded.module_info()[0].version_minor(), 3);
            put(&mut b, 0xc58, &0x001f_0407_0000_0001_u64.to_le_bytes());
            assert!(matches!(
                plan(&a, &b),
                Err(SymbolicError::VersionMismatch {
                    requested: DescriptorVersions {
                        module_major: 2,
                        module_minor: 3,
                        library: 4
                    },
                    available: DescriptorVersions {
                        module_major: 4,
                        module_minor: 7,
                        library: 4
                    },
                    ..
                })
            ));
        }
    }
}

#[test]
fn every_same_role_encoding_conflict_is_rejected_before_name_decoding() {
    for dynlib in [false, true] {
        for (provider, role) in [
            (false, DescriptorRole::NeededModule),
            (false, DescriptorRole::ImportLibrary),
            (true, DescriptorRole::ModuleInfo),
            (true, DescriptorRole::ExportLibrary),
        ] {
            let mut a = Module::consumer();
            let mut b = Module::provider();
            a.dynlib = dynlib;
            b.dynlib = dynlib;
            let module = if provider { &mut b } else { &mut a };
            // An unreadable name/ID word cannot poison the earlier role census.
            module
                .extra
                .push((role.tag(DescriptorEncoding::Variant1Observed), u64::MAX));
            let a = a.build();
            let b = b.build();
            assert!(
                matches!(plan(&a, &b), Err(SymbolicError::UnsupportedDescriptors { module })
                if module.index() == usize::from(provider))
            );
        }
    }
}

#[test]
fn opaque_filename_words_do_not_read_names_and_count_in_aggregate_records() {
    for dynlib in [false, true] {
        let mut a = Module::consumer();
        let mut b = Module::provider();
        a.dynlib = dynlib;
        b.dynlib = dynlib;
        a.extra.extend([
            (DT_SCE_ORIGINAL_FILENAME, u64::MAX),
            (DT_SCE_ORIGINAL_FILENAME_1, u64::MAX),
        ]);
        let exact_names = 2 * (a.module_name.len() + a.library_name.len()) as u64;
        let a = a.build();
        let b = b.build();
        let inputs = [
            ModuleInput::new(open_executable(&a).unwrap(), A_BIAS),
            ModuleInput::new(open_executable(&b).unwrap(), B_BIAS),
        ];
        let mut limits = SymbolicLimits::default();
        limits.sce.max_metadata_name_bytes = exact_names;
        limits.sce.max_descriptor_records = 6;
        let result = plan_symbolic(&inputs, &limits).unwrap();
        let filenames = result.modules()[0]
            .metadata
            .as_ref()
            .unwrap()
            .original_filenames();
        assert_eq!(filenames.len(), 2);
        assert!(filenames.iter().all(|record| record.raw() == u64::MAX));
        limits.sce.max_descriptor_records = 5;
        assert!(matches!(
            plan_symbolic(&inputs, &limits),
            Err(SymbolicError::LimitExceeded {
                budget: SymbolicBudget::DescriptorRecords,
                requested: 6,
                maximum: 5,
            })
        ));
    }
}

#[test]
fn filename_only_sections_need_no_string_table_or_name_allowance() {
    for dynlib in [false, true] {
        let mut a = Module::consumer();
        a.dynlib = dynlib;
        a.symbols.clear();
        a.rela.clear();
        a.extra.extend([
            (DT_SCE_ORIGINAL_FILENAME, u64::MAX),
            (DT_SCE_ORIGINAL_FILENAME_1, u64::MAX),
        ]);
        let mut a = a.build();
        // Remove table and descriptor declarations; only the two opaque
        // markers remain meaningful. No descriptor name can be read here.
        for index in 0..7 {
            put(&mut a, 0xc00 + index * 16, &0x1234_i64.to_le_bytes());
        }
        let input = [ModuleInput::new(open_executable(&a).unwrap(), A_BIAS)];
        let limits = SymbolicLimits {
            inspection: crate::Limits {
                max_table_bytes: 0,
                max_name_bytes: 0,
                max_symbols: 0,
                max_relocations: 0,
            },
            sce: crate::SceLimits {
                max_descriptor_records: 2,
                max_metadata_name_bytes: 0,
                ..crate::SceLimits::default()
            },
            ..SymbolicLimits::default()
        };
        let result = plan_symbolic(&input, &limits).unwrap();
        assert_eq!(result.patch_count(), 0);
        let metadata = result.modules()[0].metadata.as_ref().unwrap();
        assert_eq!(metadata.metadata_name_bytes(), 0);
        assert_eq!(metadata.table_bytes(), 0);
        assert_eq!(metadata.original_filenames().len(), 2);
        let limits = SymbolicLimits {
            sce: crate::SceLimits {
                max_descriptor_records: 1,
                ..limits.sce
            },
            ..limits
        };
        assert!(matches!(
            plan_symbolic(&input, &limits),
            Err(SymbolicError::LimitExceeded {
                budget: SymbolicBudget::DescriptorRecords,
                requested: 2,
                maximum: 1,
            })
        ));
    }
}

#[test]
fn observed_roles_and_both_filename_tags_preflight_before_any_symbol_decode() {
    for dynlib in [false, true] {
        let mut a = Module::consumer();
        let mut b = Module::provider();
        a.dynlib = dynlib;
        b.dynlib = dynlib;
        a.module_encoding = DescriptorEncoding::Variant1Observed;
        a.library_encoding = DescriptorEncoding::Variant1Observed;
        b.module_encoding = DescriptorEncoding::Variant1Observed;
        b.library_encoding = DescriptorEncoding::Variant1Observed;
        a.extra.extend([
            (DT_SCE_ORIGINAL_FILENAME, u64::MAX),
            (DT_SCE_ORIGINAL_FILENAME_1, u64::MAX),
        ]);
        let mut a = a.build();
        let b = b.build();
        let address = u64::from_le_bytes(a[0xc28..0xc30].try_into().unwrap());
        let symbol_at = if dynlib {
            0x200 + address as usize
        } else {
            (address - BASE) as usize
        };
        put(&mut a, symbol_at + 24, &u32::MAX.to_le_bytes());
        let inputs = [
            ModuleInput::new(open_executable(&a).unwrap(), A_BIAS),
            ModuleInput::new(open_executable(&b).unwrap(), B_BIAS),
        ];
        let limits = SymbolicLimits {
            sce: crate::SceLimits {
                max_descriptor_records: 5,
                ..crate::SceLimits::default()
            },
            ..SymbolicLimits::default()
        };
        assert!(matches!(
            plan_symbolic(&inputs, &limits),
            Err(SymbolicError::LimitExceeded {
                budget: SymbolicBudget::DescriptorRecords,
                requested: 6,
                maximum: 5,
            })
        ));
    }
}

#[test]
fn missing_weak_is_an_error_and_undefined_value_is_never_an_address() {
    let mut a = Module::consumer();
    let mut b = Module::provider();
    a.symbols[0].binding = 2;
    a.symbols[0].value = DEFINITION;
    b.module_name = b"other".to_vec();
    let a = a.build();
    let b = b.build();
    assert!(matches!(
        plan(&a, &b),
        Err(SymbolicError::MissingProvider {
            consumer: SymbolRef { symbol: 1, .. }
        })
    ));
}

#[test]
fn duplicate_providers_are_ambiguous_even_if_value_bits_match() {
    let a = Module::consumer().build();
    let b = Module::provider().build();
    let c = Module::provider().build();
    let result = plan_symbolic(
        &[
            ModuleInput::new(open_executable(&a).unwrap(), A_BIAS),
            ModuleInput::new(open_executable(&b).unwrap(), B_BIAS),
            ModuleInput::new(open_executable(&c).unwrap(), B_BIAS + 0x4000),
        ],
        &SymbolicLimits::default(),
    );
    assert!(matches!(
        result,
        Err(SymbolicError::AmbiguousProvider {
            first: SymbolRef {
                module: ModuleId(1),
                symbol: 1
            },
            second: SymbolRef {
                module: ModuleId(2),
                symbol: 1
            },
            ..
        })
    ));
}

#[test]
fn identity_is_exact_names_versions_nid_and_type_not_selector_numbers() {
    let a = Module::consumer().build();
    for change in 0..6 {
        let mut b = Module::provider();
        match change {
            0 => b.module_name.push(b'x'),
            1 => b.library_name.push(b'x'),
            2 => b.major += 1,
            3 => b.minor += 1,
            4 => b.version += 1,
            _ => b.symbols[0].name = b"BAAAAAAAAAA#R#f".to_vec(),
        }
        let available = DescriptorVersions {
            module_major: b.major,
            module_minor: b.minor,
            library: b.version,
        };
        let b = b.build();
        let result = plan(&a, &b);
        if (2..=4).contains(&change) {
            assert!(
                matches!(result, Err(SymbolicError::VersionMismatch {
                consumer: SymbolRef { module: ModuleId(0), symbol: 1 },
                requested: DescriptorVersions { module_major: 2, module_minor: 3, library: 4 },
                available: actual, provider: SymbolRef { module: ModuleId(1), symbol: 1 },
            }) if actual == available),
                "change {change}"
            );
        } else {
            assert!(
                matches!(result, Err(SymbolicError::MissingProvider { .. })),
                "change {change}"
            );
        }
    }
    let mut b = Module::provider();
    b.symbols[0].kind = 1;
    let b = b.build();
    assert!(matches!(
        plan(&a, &b),
        Err(SymbolicError::UnsupportedSymbol {
            reason: UnsupportedSymbol::TypeMismatch,
            ..
        })
    ));
}

#[test]
fn version_diagnostics_are_bounded_deterministic_and_never_bind_other_versions() {
    let a = Module::consumer().build();
    let mut b = Module::provider();
    b.major = 9;
    let b = b.build();
    let mut c = Module::provider();
    c.major = 1;
    c.symbols[0].kind = 1;
    let cbytes = c.build();
    let result = plan_symbolic(
        &[
            ModuleInput::new(open_executable(&a).unwrap(), A_BIAS),
            ModuleInput::new(open_executable(&b).unwrap(), B_BIAS),
            ModuleInput::new(open_executable(&cbytes).unwrap(), B_BIAS + 0x4000),
        ],
        &SymbolicLimits::default(),
    );
    // The first version in sorted order is diagnosed even though its type
    // also differs; it is explicitly an available record, not a provider choice.
    assert!(matches!(
        result,
        Err(SymbolicError::VersionMismatch {
            requested: DescriptorVersions {
                module_major: 2,
                module_minor: 3,
                library: 4
            },
            available: DescriptorVersions {
                module_major: 1,
                module_minor: 3,
                library: 4
            },
            provider: SymbolRef {
                module: ModuleId(2),
                symbol: 1
            },
            ..
        })
    ));
    // An exact-version wrong type wins over any alternate-version report.
    c.major = 2;
    let cbytes = c.build();
    let result = plan_symbolic(
        &[
            ModuleInput::new(open_executable(&a).unwrap(), A_BIAS),
            ModuleInput::new(open_executable(&b).unwrap(), B_BIAS),
            ModuleInput::new(open_executable(&cbytes).unwrap(), B_BIAS + 0x4000),
        ],
        &SymbolicLimits::default(),
    );
    assert!(matches!(
        result,
        Err(SymbolicError::UnsupportedSymbol {
            reason: UnsupportedSymbol::TypeMismatch,
            ..
        })
    ));
}

#[test]
fn import_selector_must_exist_in_its_own_role() {
    let mut a = Module::consumer();
    a.library_id = 9;
    // The same numeric ID in an export declaration cannot satisfy an import.
    a.extra.push((crate::DT_SCE_EXPORT_LIB, 1_u64 << 48));
    let a = a.build();
    let b = Module::provider().build();
    assert!(matches!(
        plan(&a, &b),
        Err(SymbolicError::MissingDeclaration {
            role: DescriptorRole::ImportLibrary,
            selector: 1,
            ..
        })
    ));
}

#[test]
fn bad_qualified_names_and_same_role_conflicts_are_explicit() {
    let mut a = Module::consumer();
    a.symbols[0].name = b"plain_name".to_vec();
    let a = a.build();
    let b = Module::provider().build();
    assert!(matches!(
        plan(&a, &b),
        Err(SymbolicError::QualifiedName { .. })
    ));
    let mut a = Module::consumer();
    a.extra.push((DT_SCE_NEEDED_MODULE_1, u64::MAX));
    let a = a.build();
    assert!(matches!(
        plan(&a, &b),
        Err(SymbolicError::UnsupportedDescriptors {
            module: ModuleId(0)
        })
    ));
}

#[test]
fn unsupported_binding_visibility_and_other_bits_are_rejected() {
    let a = Module::consumer().build();
    for (binding, other, reason) in [
        (10, 0, UnsupportedSymbol::Binding),
        (1, 4, UnsupportedSymbol::Visibility),
        (1, 8, UnsupportedSymbol::OtherBits),
    ] {
        let mut b = Module::provider();
        b.symbols[0].binding = binding;
        b.symbols[0].other = other;
        let b = b.build();
        assert!(
            matches!(plan(&a,&b),Err(SymbolicError::UnsupportedSymbol{reason:r,..}) if r==reason)
        );
    }
}

#[test]
fn weak_address_definitions_bind_real_storage_and_strong_precedence_is_order_independent() {
    for kind in [1, 2] {
        let mut consumer = Module::consumer();
        consumer.symbols[0].kind = kind;
        let consumer = consumer.build();
        let mut weak = Module::provider();
        weak.symbols[0].kind = kind;
        weak.symbols[0].binding = 2;
        let weak = weak.build();
        let mut strong = Module::provider();
        strong.symbols[0].kind = kind;
        strong.symbols[0].value += 16;
        let strong = strong.build();
        let result = plan(&consumer, &weak).unwrap();
        assert_eq!(result.patches()[0].value(), B_BIAS + DEFINITION - 8);
        for weak_first in [false, true] {
            let ordered = if weak_first {
                [&weak, &strong]
            } else {
                [&strong, &weak]
            };
            let inputs = [
                ModuleInput::new(open_executable(&consumer).unwrap(), A_BIAS),
                ModuleInput::new(open_executable(ordered[0]).unwrap(), B_BIAS),
                ModuleInput::new(open_executable(ordered[1]).unwrap(), B_BIAS + 0x100_0000),
            ];
            let result = plan_symbolic(&inputs, &Default::default()).unwrap();
            let expected_bias = B_BIAS + if weak_first { 0x100_0000 } else { 0 };
            assert_eq!(
                result.patches()[0].value(),
                expected_bias + DEFINITION + 16 - 8
            );
        }
        for duplicate in [&weak, &strong] {
            let inputs = [
                ModuleInput::new(open_executable(&consumer).unwrap(), A_BIAS),
                ModuleInput::new(open_executable(duplicate).unwrap(), B_BIAS),
                ModuleInput::new(open_executable(duplicate).unwrap(), B_BIAS + 0x100_0000),
            ];
            assert!(matches!(
                plan_symbolic(&inputs, &Default::default()),
                Err(SymbolicError::AmbiguousProvider { .. })
            ));
        }
    }
}

#[test]
fn hidden_provider_is_not_exported_and_nid_only_fallback_does_not_exist() {
    let a = Module::consumer().build();
    let mut b = Module::provider();
    b.symbols[0].other = 2;
    let b = b.build();
    assert!(matches!(
        plan(&a, &b),
        Err(SymbolicError::MissingProvider { .. })
    ));
}

#[test]
fn common_reserved_tls_and_ifunc_have_typed_unsupported_diagnostics() {
    let a = Module::consumer().build();
    for (section, kind, reason) in [
        (0xfff2, 1, UnsupportedSymbol::Section),
        (0xffff, 2, UnsupportedSymbol::Section),
        (1, 6, UnsupportedSymbol::Type),
        (1, 10, UnsupportedSymbol::Type),
    ] {
        let mut b = Module::provider();
        b.symbols[0].section = section;
        b.symbols[0].kind = kind;
        // Ensure type mismatch does not mask unsupported definition semantics.
        let mut consumer = Module::consumer();
        consumer.symbols[0].kind = kind;
        let consumer = consumer.build();
        let b = b.build();
        assert!(
            matches!(plan(&consumer,&b),Err(SymbolicError::UnsupportedSymbol{reason:r,..}) if r==reason)
        );
    }
    assert!(!a.is_empty());
}

#[test]
fn absolute_object_uses_unbiased_data_bits_and_r64_wraps() {
    let mut a = Module::consumer();
    let mut b = Module::provider();
    a.symbols[0].kind = 1;
    a.rela[0].3 = 3;
    b.symbols[0].kind = 1;
    b.symbols[0].section = 0xfff1;
    b.symbols[0].value = u64::MAX;
    let a = a.build();
    let b = b.build();
    let result = plan(&a, &b).unwrap();
    assert_eq!(result.patches()[0].value(), 2);
    assert!(matches!(
        result.patches()[0].resolution(),
        Resolution::Absolute(SymbolRef {
            module: ModuleId(1),
            symbol: 1
        })
    ));
}

#[test]
fn local_absolute_notype_and_zero_symbol_index_do_not_require_providers() {
    let mut a = Module::consumer();
    a.symbols[0].binding = 0;
    a.symbols[0].kind = 0;
    a.symbols[0].section = 0xfff1;
    a.symbols[0].value = 9;
    a.rela = vec![
        (TARGET, 1, 1, -10),
        (TARGET + 8, 0, 1, -1),
        (TARGET + 16, 0, 6, 999),
        (TARGET + 24, 0, 7, 777),
    ];
    let a = a.build();
    let b = Module::provider().build();
    let result = plan(&a, &b).unwrap();
    assert_eq!(
        result
            .patches()
            .iter()
            .map(SymbolicPatch::value)
            .collect::<Vec<_>>(),
        [u64::MAX, u64::MAX, 0, 0]
    );
    assert_eq!(
        result.patches()[1].resolution(),
        Resolution::ZeroSymbolIndex
    );
}

#[test]
fn same_image_nonpreemptible_definitions_work_default_definitions_do_not() {
    for (binding, visibility) in [(0, 0), (1, 2), (1, 3)] {
        let mut a = Module::consumer();
        a.symbols[0].binding = binding;
        a.symbols[0].other = visibility;
        a.symbols[0].section = 1;
        a.symbols[0].value = DEFINITION;
        let a = a.build();
        let b = Module::provider().build();
        let result = plan(&a, &b).unwrap();
        assert_eq!(result.patches()[0].value(), A_BIAS + DEFINITION - 8);
        assert!(matches!(
            result.patches()[0].resolution(),
            Resolution::Definition(SymbolRef {
                module: ModuleId(0),
                symbol: 1
            })
        ));
    }
    let mut a = Module::consumer();
    a.symbols[0].section = 1;
    a.symbols[0].value = DEFINITION;
    let a = a.build();
    let b = Module::provider().build();
    assert!(matches!(
        plan(&a, &b),
        Err(SymbolicError::UnsupportedSymbol {
            reason: UnsupportedSymbol::PreemptibleDefinition,
            ..
        })
    ));
}

#[test]
fn ordinary_definition_range_includes_bss_but_excludes_padding_and_overflow() {
    let a = Module::consumer().build();
    let mut b = Module::provider();
    b.symbols[0].value = BASE + 0x1100;
    b.symbols[0].size = 16;
    let good = b.build();
    assert!(plan(&a, &good).is_ok());
    for (value, size) in [(BASE + 0x1400, 0), (BASE + 0x13ff, 2), (u64::MAX, 8)] {
        b.symbols[0].value = value;
        b.symbols[0].size = size;
        let b = b.build();
        assert!(matches!(
            plan(&a, &b),
            Err(SymbolicError::SymbolRange { .. })
        ));
    }
}

#[test]
fn jump_slot_requires_executable_function_and_rejects_absolute_function() {
    let mut a = Module::consumer();
    a.rela[0].2 = 7;
    let a = a.build();
    let mut b = Module::provider();
    b.flags = 6;
    let bbytes = b.build();
    assert!(matches!(
        plan(&a, &bbytes),
        Err(SymbolicError::FunctionNotExecutable { .. })
    ));
    b.flags = 5;
    b.symbols[0].section = 0xfff1;
    let bbytes = b.build();
    assert!(matches!(
        plan(&a, &bbytes),
        Err(SymbolicError::UnsupportedSymbol {
            reason: UnsupportedSymbol::AbsoluteFunction,
            ..
        })
    ));
}

#[test]
fn symbolic_targets_use_full_logical_ranges_including_bss_not_page_padding() {
    let b = Module::provider().build();
    let mut a = Module::consumer();
    a.rela[0].0 = BASE + 0x1101;
    let good = a.build();
    assert!(plan(&good, &b).is_ok());
    for (target, overflow) in [
        (BASE + 0x13fc, false),
        (BASE + 0x2000, false),
        (u64::MAX - 3, true),
    ] {
        a.rela[0].0 = target;
        let a = a.build();
        assert!(
            matches!(
                plan(&a, &b),
                Err(SymbolicError::Relocation {
                    error: RelocationError::TargetRangeOverflow { .. },
                    ..
                })
            ) == overflow
        );
        if !overflow {
            assert!(matches!(
                plan(&a, &b),
                Err(SymbolicError::Relocation {
                    error: RelocationError::TargetOutsideSegments { .. },
                    ..
                })
            ));
        }
    }
}

#[test]
fn relative_symbol_errors_and_combined_patch_overlap_are_preserved() {
    let mut a = Module::consumer();
    a.rela[0].2 = 8;
    let b = Module::provider().build();
    let bad = a.build();
    assert!(matches!(
        plan(&bad, &b),
        Err(SymbolicError::Relocation {
            error: RelocationError::RelativeSymbolIndex { .. },
            ..
        })
    ));
    a.rela = vec![(TARGET, 1, 1, 0)];
    a.plt = vec![(TARGET + 4, 0, 8, 0)];
    let bad = a.build();
    assert!(matches!(
        plan(&bad, &b),
        Err(SymbolicError::OverlappingPatches {
            first_module: ModuleId(0),
            second_module: ModuleId(0),
            ..
        })
    ));
}

#[test]
fn consecutive_supported_composition_is_rejected_even_with_none_or_deferred() {
    let b = Module::provider().build();
    for other in [0, 1, 37] {
        let mut a = Module::consumer();
        a.rela.push((TARGET, 0, other, 0));
        let a = a.build();
        assert!(matches!(
            plan(&a, &b),
            Err(SymbolicError::UnsupportedComposition { .. })
        ));
    }
}

#[test]
fn primary_is_required_and_must_own_its_executable_entry() {
    assert!(matches!(
        plan_symbolic(&[], &SymbolicLimits::default()),
        Err(SymbolicError::EmptySession)
    ));
    let mut a = Module::consumer();
    a.flags = 6;
    let b = Module::provider().build();
    let a = a.build();
    assert!(matches!(
        plan(&a, &b),
        Err(SymbolicError::PrimaryEntryOutsideExecutable { .. })
    ));
    let mut a = Module::consumer().build();
    put(&mut a, 24, &(BASE + 0x4000).to_le_bytes());
    assert!(matches!(
        plan(&a, &b),
        Err(SymbolicError::PrimaryEntryOutsideExecutable { .. })
    ));
}

#[test]
fn global_page_ownership_and_mapping_bias_overflow_are_checked() {
    let a = Module::consumer().build();
    let b = Module::provider().build();
    let inputs = [
        ModuleInput::new(open_executable(&a).unwrap(), A_BIAS),
        ModuleInput::new(open_executable(&b).unwrap(), A_BIAS + 0x2000),
    ];
    assert!(matches!(
        plan_symbolic(&inputs, &SymbolicLimits::default()),
        Err(SymbolicError::OverlappingModulePages { .. })
    ));
    let inputs = [ModuleInput::new(open_executable(&a).unwrap(), u64::MAX)];
    assert!(matches!(
        plan_symbolic(&inputs, &SymbolicLimits::default()),
        Err(SymbolicError::BiasedMappingOverflow { .. })
    ));
}

#[test]
fn aggregate_extent_rejection_precedes_any_full_table_copy_or_decode() {
    let mut a = Module::consumer().build();
    let b = Module::provider().build();
    // Corrupt an original st_name. Extent preflight must reject the combined
    // table budget before any symbol decoding can observe that bad field.
    let symbols = u64::from_le_bytes(a[0xc00 + 2 * 16 + 8..0xc00 + 3 * 16].try_into().unwrap());
    put(
        &mut a,
        (symbols - BASE) as usize + 24,
        &u32::MAX.to_le_bytes(),
    );
    let mut limits = SymbolicLimits::default();
    limits.inspection.max_table_bytes = 190;
    let inputs = [
        ModuleInput::new(open_executable(&a).unwrap(), A_BIAS),
        ModuleInput::new(open_executable(&b).unwrap(), B_BIAS),
    ];
    let result = plan_symbolic(&inputs, &limits);
    assert!(
        matches!(
            result,
            Err(SymbolicError::LimitExceeded {
                budget: SymbolicBudget::TableBytes,
                ..
            })
        ),
        "{result:?}"
    );
}

#[test]
fn all_session_budgets_are_inclusive_and_enforced() {
    let a = Module::consumer().build();
    let b = Module::provider().build();
    let inputs = [
        ModuleInput::new(open_executable(&a).unwrap(), A_BIAS),
        ModuleInput::new(open_executable(&b).unwrap(), B_BIAS),
    ];
    for budget in [
        SymbolicBudget::Modules,
        SymbolicBudget::DynamicEntries,
        SymbolicBudget::DescriptorRecords,
        SymbolicBudget::TableBytes,
        SymbolicBudget::Symbols,
        SymbolicBudget::Relocations,
        SymbolicBudget::Exports,
        SymbolicBudget::Patches,
        SymbolicBudget::KeyBytes,
        SymbolicBudget::LookupSteps,
    ] {
        let mut limits = SymbolicLimits::default();
        match budget {
            SymbolicBudget::Modules => limits.max_modules = 1,
            SymbolicBudget::DynamicEntries => limits.sce.max_dynamic_entries = 1,
            SymbolicBudget::DescriptorRecords => limits.sce.max_descriptor_records = 3,
            SymbolicBudget::TableBytes => limits.inspection.max_table_bytes = 190,
            SymbolicBudget::Symbols => limits.inspection.max_symbols = 3,
            SymbolicBudget::Relocations => limits.inspection.max_relocations = 0,
            SymbolicBudget::Exports => limits.max_exports = 0,
            SymbolicBudget::Patches => limits.max_patches = 0,
            SymbolicBudget::KeyBytes => limits.max_key_bytes = 0,
            SymbolicBudget::LookupSteps => limits.max_lookup_steps = 0,
            SymbolicBudget::MissingFunctions => {
                unreachable!("bound belongs to the explicit missing-function query")
            }
        }
        let result = plan_symbolic(&inputs, &limits);
        // A single image can itself exceed an aggregate allowance; generic
        // inspection's typed per-table errors remain valid in that case.
        assert!(result.is_err(), "{budget:?}");
    }
    let limits = SymbolicLimits {
        max_modules: 2,
        max_exports: 1,
        max_patches: 1,
        ..SymbolicLimits::default()
    };
    assert!(plan_symbolic(&inputs, &limits).is_ok());
}

#[test]
fn symbol_and_metadata_name_allowances_are_shared_across_images() {
    let a = Module::consumer().build();
    let b = Module::provider().build();
    let inputs = [
        ModuleInput::new(open_executable(&a).unwrap(), A_BIAS),
        ModuleInput::new(open_executable(&b).unwrap(), B_BIAS),
    ];
    let mut limits = SymbolicLimits::default();
    limits.inspection.max_name_bytes = 20;
    assert!(matches!(
        plan_symbolic(&inputs, &limits),
        Err(SymbolicError::Sce {
            module: ModuleId(1),
            ..
        })
    ));
    let mut limits = SymbolicLimits::default();
    limits.sce.max_metadata_name_bytes = 30;
    assert!(matches!(
        plan_symbolic(&inputs, &limits),
        Err(SymbolicError::Sce {
            module: ModuleId(1),
            ..
        })
    ));
}

#[test]
fn lookup_budget_includes_successful_resolution_cache_insertion() {
    let a = Module::consumer().build();
    let b = Module::provider().build();
    let inputs = [
        ModuleInput::new(open_executable(&a).unwrap(), A_BIAS),
        ModuleInput::new(open_executable(&b).unwrap(), B_BIAS),
    ];
    // Two descriptor comparisons + provider entry; cache lookup + two
    // import descriptor comparisons + provider lookup + candidate visit;
    // the ninth operation inserts the successfully resolved cache entry.
    let limits = SymbolicLimits {
        max_lookup_steps: 8,
        ..SymbolicLimits::default()
    };
    assert!(matches!(
        plan_symbolic(&inputs, &limits),
        Err(SymbolicError::LimitExceeded {
            budget: SymbolicBudget::LookupSteps,
            requested: 9,
            maximum: 8
        })
    ));
    let limits = SymbolicLimits {
        max_lookup_steps: 9,
        ..limits
    };
    assert!(plan_symbolic(&inputs, &limits).is_ok());
}

#[test]
fn metadata_only_strings_are_included_in_aggregate_preflight() {
    let mut a = Module::consumer();
    a.symbols.clear();
    a.rela.clear();
    let mut bytes = a.build();
    // Convert symbol table tags to unrecognized tags so descriptor names
    // are the only consumer of strings (the original bytes stay synthetic).
    for index in 2..5 {
        put(&mut bytes, 0xc00 + index * 16, &0x1234_i64.to_le_bytes());
    }
    let b = Module::provider().build();
    let inputs = [
        ModuleInput::new(open_executable(&bytes).unwrap(), A_BIAS),
        ModuleInput::new(open_executable(&b).unwrap(), B_BIAS),
    ];
    let mut limits = SymbolicLimits::default();
    limits.inspection.max_table_bytes = 120;
    assert!(matches!(
        plan_symbolic(&inputs, &limits),
        Err(SymbolicError::LimitExceeded {
            budget: SymbolicBudget::TableBytes,
            ..
        })
    ));
}

#[test]
fn missing_descriptor_is_distinct_from_a_missing_provider() {
    let mut a = Module::consumer().build();
    // Import library record is the final non-null dynamic record.
    let start = 0xc00;
    let mut index = 0;
    loop {
        let at = start + index * 16;
        let tag = i64::from_le_bytes(a[at..at + 8].try_into().unwrap());
        if tag == DT_SCE_IMPORT_LIB {
            put(&mut a, at, &0x1234_i64.to_le_bytes());
            break;
        }
        assert_ne!(tag, 0);
        index += 1;
    }
    let b = Module::provider().build();
    assert!(matches!(
        plan(&a, &b),
        Err(SymbolicError::MissingDeclaration {
            role: DescriptorRole::ImportLibrary,
            ..
        })
    ));
}
