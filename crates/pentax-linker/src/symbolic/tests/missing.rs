// SPDX-License-Identifier: GPL-2.0-or-later

use super::*;

#[test]
fn optional_weak_address_policy_keeps_typed_zero_values_and_never_invents_a_function() {
    for kind in [1, 2] {
        let mut module = consumer();
        module.symbols[0].binding = 2;
        module.symbols[0].kind = kind;
        module.symbols[0].value = 0xfeed;
        module.rela = vec![(TARGET, 1, 1, -3), (TARGET + 16, 1, 6, 77)];
        if kind == 2 {
            module.plt = vec![(TARGET + 32, 1, 7, 77)];
        }
        let bytes = module.build();
        let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), A_BIAS)];
        assert!(matches!(
            plan_symbolic(&inputs, &Default::default()),
            Err(SymbolicError::MissingProvider { .. })
        ));
        let limits = SymbolicLimits {
            missing_weak: MissingWeakPolicy::NullAddressV1,
            ..Default::default()
        };
        let plan = plan_symbolic(&inputs, &limits).unwrap();
        assert_eq!(plan.patches()[0].value(), u64::MAX - 2);
        assert!(plan.patches()[1..].iter().all(|p| p.value() == 0));
        assert!(
            plan.patches()
                .iter()
                .all(|p| matches!(p.resolution(), Resolution::UnresolvedWeak(_)))
        );
        assert!(
            plan_missing_functions(&inputs, &limits, 0)
                .unwrap()
                .functions()
                .is_empty()
        );
        let mut provider = Module::provider();
        provider.symbols[0].kind = kind;
        provider.version += 1;
        let provider = provider.build();
        let conflicting = [
            inputs[0].clone(),
            ModuleInput::new(open_executable(&provider).unwrap(), B_BIAS),
        ];
        assert!(matches!(
            plan_symbolic(&conflicting, &limits),
            Err(SymbolicError::VersionMismatch { .. })
        ));
    }
}

fn consumer() -> Module {
    let mut module = Module::consumer();
    module.rela[0].3 = 0;
    module
}

#[test]
fn missing_functions_reuse_exact_scope_and_deduplicate_relocated_function_identities() {
    let mut first = consumer();
    first.rela.push((TARGET + 16, 1, 6, 123));
    first.plt.push((TARGET + 32, 1, 7, -123));
    let mut second = consumer();
    second.primary = false;
    second.module_id = 31;
    second.library_id = 17;
    second.symbols[0].name = b"AAAAAAAAAAA#R#f".to_vec();
    let first = first.build();
    let second = second.build();
    let inputs = [
        ModuleInput::new(open_executable(&first).unwrap(), A_BIAS),
        ModuleInput::new(open_executable(&second).unwrap(), B_BIAS),
    ];
    assert!(matches!(
        plan_symbolic(&inputs, &Default::default()),
        Err(SymbolicError::MissingProvider { .. })
    ));
    let missing = plan_missing_functions(&inputs, &Default::default(), 1).unwrap();
    assert_eq!(missing.source_modules().len(), 2);
    assert_eq!(missing.functions().len(), 1);
    let function = &missing.functions()[0];
    assert_eq!(function.module_name(), b"fixture-module");
    assert_eq!(function.library_name(), b"fixture-library");
    assert_eq!(
        function.versions(),
        DescriptorVersions {
            module_major: 2,
            module_minor: 3,
            library: 4
        }
    );
    assert_eq!(function.nid().as_bytes(), b"AAAAAAAAAAA");
    assert_eq!(function.reference_count(), 4);
    assert_eq!(function.first_consumer().module.index(), 0);
    assert!(!format!("{missing:?}").contains("fixture-module"));
    assert!(!format!("{function:?}").contains("AAAAAAAAAAA"));
    // The default linker has not acquired any new fallback behavior.
    assert!(matches!(
        plan_symbolic(&inputs, &Default::default()),
        Err(SymbolicError::MissingProvider { .. })
    ));
}

#[test]
fn missing_functions_do_not_replace_real_providers_or_mask_version_conflicts() {
    let bytes = consumer().build();
    for mismatch in [false, true] {
        let mut provider = Module::provider();
        if mismatch {
            provider.version += 1;
        }
        let provider = provider.build();
        let inputs = [
            ModuleInput::new(open_executable(&bytes).unwrap(), A_BIAS),
            ModuleInput::new(open_executable(&provider).unwrap(), B_BIAS),
        ];
        let result = plan_missing_functions(&inputs, &Default::default(), 0);
        if mismatch {
            assert!(matches!(result, Err(SymbolicError::VersionMismatch { .. })));
        } else {
            assert!(result.unwrap().functions().is_empty());
            assert!(plan_symbolic(&inputs, &Default::default()).is_ok());
        }
    }
}

#[test]
fn missing_functions_preserve_provider_ambiguity_and_type_mismatch_errors() {
    let bytes = consumer().build();
    let first = Module::provider().build();
    let inputs = [
        ModuleInput::new(open_executable(&bytes).unwrap(), A_BIAS),
        ModuleInput::new(open_executable(&first).unwrap(), B_BIAS),
        ModuleInput::new(open_executable(&first).unwrap(), B_BIAS + 0x100_0000),
    ];
    assert!(matches!(
        plan_missing_functions(&inputs, &Default::default(), 16),
        Err(SymbolicError::AmbiguousProvider { .. })
    ));
    let mut wrong = Module::provider();
    wrong.symbols[0].kind = 1;
    let wrong = wrong.build();
    let inputs = [
        ModuleInput::new(open_executable(&bytes).unwrap(), A_BIAS),
        ModuleInput::new(open_executable(&wrong).unwrap(), B_BIAS),
    ];
    assert!(matches!(
        plan_missing_functions(&inputs, &Default::default(), 16),
        Err(SymbolicError::UnsupportedSymbol {
            reason: UnsupportedSymbol::TypeMismatch,
            ..
        })
    ));
}

#[test]
fn missing_functions_refuse_object_weak_and_address_addend_substitutions() {
    for case in 0..3 {
        let mut module = consumer();
        let reason = match case {
            0 => {
                module.symbols[0].kind = 1;
                MissingFunctionReason::SymbolType
            }
            1 => {
                module.symbols[0].binding = 2;
                MissingFunctionReason::WeakBinding
            }
            2 => {
                module.rela[0].3 = 1;
                MissingFunctionReason::AddressAddend
            }
            _ => unreachable!(),
        };
        let bytes = module.build();
        let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), A_BIAS)];
        assert!(
            matches!(plan_missing_functions(&inputs, &Default::default(), 16),
            Err(SymbolicError::UnsupportedMissingFunction { reason: actual, .. }) if actual == reason)
        );
    }
}

#[test]
fn missing_function_retention_and_reused_lookup_key_budgets_fail_explicitly() {
    let bytes = consumer().build();
    let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), A_BIAS)];
    assert!(matches!(
        plan_missing_functions(&inputs, &Default::default(), 0),
        Err(SymbolicError::LimitExceeded {
            budget: SymbolicBudget::MissingFunctions,
            requested: 1,
            maximum: 0
        })
    ));
    for (limits, budget) in [
        (
            SymbolicLimits {
                max_lookup_steps: 0,
                ..Default::default()
            },
            SymbolicBudget::LookupSteps,
        ),
        (
            SymbolicLimits {
                max_key_bytes: 0,
                ..Default::default()
            },
            SymbolicBudget::KeyBytes,
        ),
    ] {
        assert!(matches!(plan_missing_functions(&inputs, &limits, 16),
            Err(SymbolicError::LimitExceeded { budget: actual, .. }) if actual == budget));
    }
}

#[test]
fn missing_function_query_does_not_turn_deferred_tls_or_unknown_records_into_functions() {
    let mut module = consumer();
    module.rela = vec![(TARGET, 1, 18, 0), (TARGET + 16, 1, 37, 0)];
    let bytes = module.build();
    let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), A_BIAS)];
    assert!(
        plan_missing_functions(&inputs, &Default::default(), 0)
            .unwrap()
            .functions()
            .is_empty()
    );
    assert_eq!(
        plan_symbolic(&inputs, &Default::default())
            .unwrap()
            .deferred_count(),
        2
    );
}
