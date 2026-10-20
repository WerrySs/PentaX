// SPDX-License-Identifier: GPL-2.0-or-later

//! Original role-eligibility fixtures; no software artifact supplies bytes.

use super::*;

const ENCODINGS: [DescriptorEncoding; 2] = [
    DescriptorEncoding::Classic,
    DescriptorEncoding::Variant1Observed,
];

fn configure(module: &mut Module, dynlib: bool, encoding: DescriptorEncoding) {
    module.dynlib = dynlib;
    module.module_encoding = encoding;
    module.library_encoding = encoding;
}

fn import_pair(module: &mut Module, module_id: u16, library_id: u16) {
    module.declarations.extend([
        Declaration {
            role: DescriptorRole::NeededModule,
            encoding: module.module_encoding,
            name: b"import-only-module".to_vec(),
            id: module_id,
            version: 0x0907,
        },
        Declaration {
            role: DescriptorRole::ImportLibrary,
            encoding: module.library_encoding,
            name: b"import-only-library".to_vec(),
            id: library_id,
            version: 11,
        },
    ]);
}

fn import_only_neighbor(module: &mut Module) {
    let mut symbol = module.symbols[0].clone();
    symbol.name = b"AAAAAAAAAAA#B#C".to_vec();
    symbol.value = DEFINITION + 32;
    module.symbols.push(symbol);
    import_pair(module, 2, 1);
}

fn assert_exclusion(plan: &SymbolicLinkPlan<'_>, module: usize, symbol: usize) {
    assert_eq!(plan.provider_exclusion_count(), 1);
    let evidence = &plan.provider_exclusions()[0];
    assert_eq!(
        evidence.symbol(),
        SymbolRef {
            module: ModuleId(module),
            symbol
        }
    );
    assert_eq!(
        evidence.reason(),
        ProviderExclusionReason::NotOfferedImportScope
    );
}

#[test]
fn protected_function_and_object_exports_bind_in_both_domains_and_encodings() {
    for dynlib in [false, true] {
        for encoding in ENCODINGS {
            for kind in [1, 2] {
                let mut a = Module::consumer();
                let mut b = Module::provider();
                configure(&mut a, dynlib, encoding);
                configure(&mut b, dynlib, encoding);
                a.symbols[0].kind = kind;
                b.symbols[0].kind = kind;
                b.symbols[0].other = 3;
                a.rela = vec![(TARGET, 1, 6, 123)];
                if kind == 2 {
                    a.plt = vec![(TARGET + 16, 1, 7, -1)];
                }
                let a = a.build();
                let b = b.build();
                let result = plan(&a, &b).unwrap();
                assert_eq!(result.patch_count(), if kind == 2 { 2 } else { 1 });
                for patch in result.patches() {
                    assert_eq!(patch.value(), B_BIAS + DEFINITION);
                    assert_eq!(
                        patch.resolution(),
                        Resolution::Definition(SymbolRef {
                            module: ModuleId(1),
                            symbol: 1,
                        })
                    );
                }
                assert!(result.provider_exclusions().is_empty());
            }
        }
    }
}

#[test]
fn protected_self_reference_stays_local_despite_an_external_matching_definition() {
    for encoding in ENCODINGS {
        let mut a = Module::provider();
        a.primary = true;
        a.module_encoding = encoding;
        a.library_encoding = encoding;
        a.symbols[0].other = 3;
        a.rela = vec![(TARGET, 1, 1, 5)];
        let mut b = a.clone();
        b.primary = false;
        b.rela.clear();
        b.symbols[0].value += 16;
        let a = a.build();
        let b = b.build();
        let result = plan(&a, &b).unwrap();
        assert_eq!(result.patches()[0].value(), A_BIAS + DEFINITION + 5);
        assert_eq!(
            result.patches()[0].resolution(),
            Resolution::Definition(SymbolRef {
                module: ModuleId(0),
                symbol: 1,
            })
        );
    }
}

#[test]
fn selected_weak_protected_provider_resolves_its_actual_definition() {
    for encoding in ENCODINGS {
        let mut a = Module::consumer();
        let mut b = Module::provider();
        configure(&mut a, true, encoding);
        configure(&mut b, true, encoding);
        b.symbols[0].binding = 2;
        b.symbols[0].other = 3;
        let a = a.build();
        let b = b.build();
        let result = plan(&a, &b).unwrap();
        assert_eq!(result.patches()[0].value(), B_BIAS + DEFINITION - 8);
    }
}

#[test]
fn import_only_definition_is_unoffered_beside_a_valid_protected_export() {
    for dynlib in [false, true] {
        for encoding in ENCODINGS {
            for visibility in [0, 3] {
                let mut a = Module::consumer();
                let mut b = Module::provider();
                configure(&mut a, dynlib, encoding);
                configure(&mut b, dynlib, encoding);
                b.symbols[0].other = 3;
                import_only_neighbor(&mut b);
                b.symbols[1].other = visibility;
                let a_bytes = a.build();
                let b_bytes = b.build();
                let original = b_bytes.clone();
                let result = plan(&a_bytes, &b_bytes).unwrap();
                assert_exclusion(&result, 1, 2);
                assert_eq!(result.patches()[0].value(), B_BIAS + DEFINITION - 8);
                assert_eq!(b_bytes, original);

                // Its import-role identity never exposes the defined address,
                // even though it shares the neighboring export's NID/type.
                a.module_name = b"import-only-module".to_vec();
                a.library_name = b"import-only-library".to_vec();
                a.major = 9;
                a.minor = 7;
                a.version = 11;
                let foreign_request = a.build();
                assert!(matches!(
                    plan(&foreign_request, &b_bytes),
                    Err(SymbolicError::MissingProvider { .. })
                ));
            }
        }
    }
}

#[test]
fn both_complete_pairs_use_owned_identity_even_with_different_import_names_and_versions() {
    for dynlib in [false, true] {
        for encoding in ENCODINGS {
            let mut a = Module::consumer();
            let mut b = Module::provider();
            configure(&mut a, dynlib, encoding);
            configure(&mut b, dynlib, encoding);
            b.symbols[0].other = 3;
            import_pair(&mut b, 31, 17);
            let a_bytes = a.build();
            let b_bytes = b.build();
            let result = plan(&a_bytes, &b_bytes).unwrap();
            assert_eq!(result.provider_exclusion_count(), 0);
            assert_eq!(result.patches()[0].value(), B_BIAS + DEFINITION - 8);
            a.module_name = b"import-only-module".to_vec();
            a.library_name = b"import-only-library".to_vec();
            a.major = 9;
            a.minor = 7;
            a.version = 11;
            let a = a.build();
            assert!(matches!(
                plan(&a, &b_bytes),
                Err(SymbolicError::MissingProvider { .. })
            ));
        }
    }
}

#[test]
fn incomplete_owned_pairs_with_complete_import_pairs_still_report_the_missing_owned_role() {
    for dynlib in [false, true] {
        for encoding in ENCODINGS {
            for own_module in [false, true] {
                let a = Module::consumer().build();
                let mut b = Module::provider();
                configure(&mut b, dynlib, encoding);
                if own_module {
                    b.library_id = 18;
                } else {
                    b.module_id = 30;
                }
                import_pair(&mut b, 31, 17);
                let b = b.build();
                assert!(
                    matches!(plan(&a, &b), Err(SymbolicError::MissingDeclaration {
                    symbol: SymbolRef { module: ModuleId(1), symbol: 1 }, role, selector,
                }) if role == if own_module { DescriptorRole::ExportLibrary } else { DescriptorRole::ModuleInfo }
                    && selector == if own_module { 17 } else { 31 })
                );
            }
        }
    }
}

#[test]
fn no_pair_and_partial_import_pairs_do_not_become_exclusions() {
    for encoding in ENCODINGS {
        for import_roles in 0..3 {
            let a = Module::consumer().build();
            let mut b = Module::provider();
            configure(&mut b, false, encoding);
            b.module_id = 30;
            b.library_id = 18;
            import_pair(&mut b, 31, 17);
            if import_roles == 0 {
                b.declarations.clear();
            } else {
                b.declarations.remove(if import_roles == 1 { 1 } else { 0 });
            }
            let b = b.build();
            assert!(matches!(
                plan(&a, &b),
                Err(SymbolicError::MissingDeclaration {
                    role: DescriptorRole::ModuleInfo,
                    selector: 31,
                    ..
                })
            ));
        }
    }
}

#[test]
fn provider_exclusion_does_not_change_defined_reference_resolution() {
    for encoding in ENCODINGS {
        let mut a = Module::consumer();
        let mut b = Module::provider();
        configure(&mut b, true, encoding);
        a.rela.clear();
        import_only_neighbor(&mut b);
        b.rela = vec![(TARGET, 2, 6, 0)];
        let a = a.build();
        let b = b.build();
        assert!(matches!(
            plan(&a, &b),
            Err(SymbolicError::UnsupportedSymbol {
                symbol: SymbolRef {
                    module: ModuleId(1),
                    symbol: 2
                },
                reason: UnsupportedSymbol::PreemptibleDefinition,
            })
        ));
    }
}

#[test]
fn undefined_nonzero_value_is_import_and_owned_protected_abs_object_zero_is_a_definition() {
    for dynlib in [false, true] {
        for encoding in ENCODINGS {
            let mut a = Module::consumer();
            let mut b = Module::provider();
            configure(&mut a, dynlib, encoding);
            configure(&mut b, dynlib, encoding);
            a.symbols[0].kind = 1;
            a.symbols[0].value = DEFINITION + 32;
            a.rela = vec![(TARGET, 1, 6, 0)];
            b.symbols[0].kind = 1;
            b.symbols[0].other = 3;
            b.symbols[0].section = 0xfff1;
            b.symbols[0].value = 0;
            b.symbols[0].size = 0;
            let a = a.build();
            let b = b.build();
            let result = plan(&a, &b).unwrap();
            assert_eq!(result.patches()[0].value(), 0);
            assert_eq!(
                result.patches()[0].resolution(),
                Resolution::Absolute(SymbolRef {
                    module: ModuleId(1),
                    symbol: 1,
                })
            );
            assert_eq!(result.provider_exclusion_count(), 0);
        }
    }
}

#[test]
fn excluded_candidates_and_diagnostics_consume_aggregate_budgets_without_key_copies() {
    for encoding in ENCODINGS {
        let mut a = Module::consumer();
        a.symbols.clear();
        a.rela.clear();
        let mut b = Module::provider();
        configure(&mut b, true, encoding);
        // One import-only candidate; own declarations exist but do not match.
        b.symbols[0].name = b"AAAAAAAAAAA#B#C".to_vec();
        import_pair(&mut b, 2, 1);
        let a = a.build();
        let b = b.build();
        let inputs = [
            ModuleInput::new(open_executable(&a).unwrap(), A_BIAS),
            ModuleInput::new(open_executable(&b).unwrap(), B_BIAS),
        ];
        let mut limits = SymbolicLimits {
            max_key_bytes: 0,
            max_exports: 0,
            ..SymbolicLimits::default()
        };
        assert!(matches!(
            plan_symbolic(&inputs, &limits),
            Err(SymbolicError::LimitExceeded {
                budget: SymbolicBudget::Exports,
                requested: 1,
                maximum: 0,
            })
        ));
        limits.max_exports = 1;
        limits.max_lookup_steps = 0;
        assert!(matches!(
            plan_symbolic(&inputs, &limits),
            Err(SymbolicError::LimitExceeded {
                budget: SymbolicBudget::LookupSteps,
                requested: 1,
                maximum: 0,
            })
        ));
        // Four role comparisons, then one charged diagnostic allocation.
        limits.max_lookup_steps = 4;
        assert!(matches!(
            plan_symbolic(&inputs, &limits),
            Err(SymbolicError::LimitExceeded {
                budget: SymbolicBudget::LookupSteps,
                requested: 5,
                maximum: 4,
            })
        ));
        limits.max_lookup_steps = 5;
        let result = plan_symbolic(&inputs, &limits).unwrap();
        assert_exclusion(&result, 1, 1);

        // The export allowance belongs to the session, not each image.
        let inputs = [
            ModuleInput::new(open_executable(&a).unwrap(), A_BIAS),
            ModuleInput::new(open_executable(&b).unwrap(), B_BIAS),
            ModuleInput::new(open_executable(&b).unwrap(), B_BIAS + 0x4000),
        ];
        limits.max_lookup_steps = 10;
        assert!(matches!(
            plan_symbolic(&inputs, &limits),
            Err(SymbolicError::LimitExceeded {
                budget: SymbolicBudget::Exports,
                requested: 2,
                maximum: 1,
            })
        ));
        limits.max_exports = 2;
        let result = plan_symbolic(&inputs, &limits).unwrap();
        assert_eq!(result.provider_exclusion_count(), 2);
        assert_eq!(result.provider_exclusions()[1].symbol().module.index(), 2);
    }
}

#[test]
fn malformed_names_conflicts_and_duplicate_ids_cannot_hide_behind_exclusion() {
    for encoding in ENCODINGS {
        for failure in 0..3 {
            let a = Module::consumer().build();
            let mut b = Module::provider();
            configure(&mut b, true, encoding);
            import_only_neighbor(&mut b);
            match failure {
                0 => b.symbols[1].name = b"AAAAAAAAAAA##C".to_vec(),
                1 => b.extra.push((
                    DescriptorRole::NeededModule.tag(if encoding == DescriptorEncoding::Classic {
                        DescriptorEncoding::Variant1Observed
                    } else {
                        DescriptorEncoding::Classic
                    }),
                    u64::MAX,
                )),
                _ => b.declarations.push(b.declarations[0].clone()),
            }
            let b = b.build();
            let result = plan(&a, &b);
            match failure {
                0 => assert!(matches!(
                    result,
                    Err(SymbolicError::QualifiedName {
                        symbol: SymbolRef {
                            module: ModuleId(1),
                            symbol: 2
                        },
                        ..
                    })
                )),
                1 => assert!(matches!(
                    result,
                    Err(SymbolicError::UnsupportedDescriptors {
                        module: ModuleId(1)
                    })
                )),
                _ => assert!(matches!(
                    result,
                    Err(SymbolicError::Sce {
                        module: ModuleId(1),
                        ..
                    })
                )),
            }
        }
    }
}

#[test]
fn images_without_export_libraries_keep_their_existing_provider_scan_scope() {
    let mut a = Module::consumer();
    a.rela.clear();
    a.symbols[0].section = 1;
    a.symbols[0].name = b"malformed##C".to_vec();
    let a = a.build();
    let b = Module::provider().build();
    let result = plan(&a, &b).unwrap();
    assert_eq!(result.patch_count(), 0);
    assert_eq!(result.provider_exclusion_count(), 0);
}

#[test]
fn protected_selected_providers_keep_type_other_section_range_and_executable_checks() {
    for encoding in ENCODINGS {
        for failure in 0..5 {
            let mut a = Module::consumer();
            let mut b = Module::provider();
            configure(&mut b, false, encoding);
            b.symbols[0].other = 3;
            let reason = match failure {
                0 => {
                    b.symbols[0].kind = 1;
                    Some(UnsupportedSymbol::TypeMismatch)
                }
                1 => {
                    b.symbols[0].other |= 8;
                    Some(UnsupportedSymbol::OtherBits)
                }
                2 => {
                    b.symbols[0].section = 0xfff2;
                    Some(UnsupportedSymbol::Section)
                }
                3 => {
                    b.symbols[0].value = BASE + 0x13ff;
                    None
                }
                _ => {
                    b.flags = 4;
                    a.rela[0].2 = 7;
                    None
                }
            };
            let a = a.build();
            let b = b.build();
            let result = plan(&a, &b);
            if let Some(expected) = reason {
                assert!(
                    matches!(result, Err(SymbolicError::UnsupportedSymbol { reason, .. }) if reason == expected)
                );
            } else if failure == 3 {
                assert!(matches!(result, Err(SymbolicError::SymbolRange { .. })));
            } else {
                assert!(matches!(
                    result,
                    Err(SymbolicError::FunctionNotExecutable { .. })
                ));
            }
        }
    }
}
