// SPDX-License-Identifier: GPL-2.0-only

//! Original synthetic sources; no generated-origin or native authority claimed.
use super::*;

const C_BIAS: u64 = B_BIAS + 0x100_0000;
const D_BIAS: u64 = C_BIAS + 0x100_0000;
const E_BIAS: u64 = D_BIAS + 0x100_0000;

fn input(bytes: &[u8], bias: u64) -> ModuleInput<'_> {
    ModuleInput::new(open_executable(bytes).unwrap(), bias)
}

fn consumer() -> Module {
    let mut module = Module::consumer();
    module.rela[0].3 = 0;
    module
}

fn grant(base: &SymbolicLinkPlan<'_>, provider: usize, symbols: &[usize]) -> FallbackPolicy {
    let module = base.modules()[provider].id();
    FallbackPolicy::new(
        base,
        module,
        base.modules()[provider].source_bytes(),
        &symbols
            .iter()
            .map(|&symbol| SymbolRef { module, symbol })
            .collect::<Vec<_>>(),
        &Default::default(),
    )
    .unwrap()
}

fn imported_guest() -> Module {
    let mut module = Module::provider();
    module.declarations.extend([
        fixture::Declaration {
            role: DescriptorRole::NeededModule,
            encoding: DescriptorEncoding::Classic,
            name: module.module_name.clone(),
            id: 2,
            version: u16::from(module.major) << 8 | u16::from(module.minor),
        },
        fixture::Declaration {
            role: DescriptorRole::ImportLibrary,
            encoding: DescriptorEncoding::Classic,
            name: module.library_name.clone(),
            id: 1,
            version: module.version,
        },
    ]);
    module
}

fn import(module: &mut Module, name: &[u8], at: u64) {
    let mut symbol = module.symbols[0].clone();
    symbol.name = name.to_vec();
    symbol.binding = 1;
    symbol.kind = 2;
    symbol.section = 0;
    symbol.value = 0;
    symbol.size = 0;
    module.symbols.push(symbol);
    module.rela.push((at, module.symbols.len() as u32, 1, 0));
}

fn with_tls(module: &Module) -> Vec<u8> {
    let mut bytes = module.build();
    let count = u16::from_le_bytes(bytes[56..58].try_into().unwrap());
    put(&mut bytes, 56, &(count + 1).to_le_bytes());
    let header = 64 + usize::from(count) * 56;
    put(&mut bytes, header, &7_u32.to_le_bytes());
    put(&mut bytes, header + 4, &4_u32.to_le_bytes());
    for (offset, value) in [
        (8, 0xa00_u64),
        (16, BASE + 0xa00),
        (32, 8),
        (40, 16),
        (48, 8),
    ] {
        put(&mut bytes, header + offset, &value.to_le_bytes());
    }
    bytes
}

#[test]
fn new_missing_delta_and_weak_replacement_share_one_parent_then_both_factories_can_retire() {
    let mut consumer = consumer();
    let mut tls = consumer.symbols[0].clone();
    tls.name = b"DDDDDDDDDDA#B#C".to_vec();
    tls.kind = 6;
    consumer.symbols.push(tls);
    consumer.rela.push((TARGET + 32, 2, 16, 0));
    let consumer = consumer.build();
    let mut fallback = Module::provider();
    let mut second = fallback.symbols[0].clone();
    second.name = b"BBBBBBBBBBA#R#f".to_vec();
    second.value += 16;
    fallback.symbols.push(second);
    let mut tls = fallback.symbols[0].clone();
    tls.name = b"DDDDDDDDDDA#R#f".to_vec();
    tls.kind = 6;
    tls.value = 0;
    fallback.symbols.push(tls);
    let fallback = with_tls(&fallback);
    let base = plan(&consumer, &fallback).unwrap();
    assert!(base.appended_guest().is_none());
    let policy = grant(&base, 1, &[1, 2]);
    let mut guest = imported_guest();
    guest.symbols[0].binding = 2;
    import(&mut guest, b"BBBBBBBBBBA#B#C", TARGET);
    import(&mut guest, b"CCCCCCCCCCA#B#C", TARGET + 8);
    let guest = guest.build();
    let missing = plan_missing_functions_for_extension(
        &base,
        input(&guest, C_BIAS),
        std::slice::from_ref(&policy),
        &Default::default(),
        1,
    )
    .unwrap();
    assert_eq!(missing.source_modules().len(), 3);
    assert!(std::ptr::eq(
        missing.source_modules()[1].source_bytes(),
        &fallback[..]
    ));
    assert!(std::ptr::eq(
        missing.source_modules()[2].source_bytes(),
        &guest[..]
    ));
    assert_eq!(missing.functions().len(), 1);
    assert_eq!(missing.functions()[0].nid().as_bytes(), b"CCCCCCCCCCA");
    assert_eq!(
        missing.functions()[0].first_consumer(),
        SymbolRef {
            module: missing.source_modules()[2].id(),
            symbol: 3,
        }
    );
    assert_eq!(missing.functions()[0].reference_count(), 1);
    assert!(matches!(
        append_symbolic_module_with_fallbacks(
            &base,
            input(&guest, C_BIAS),
            &policy,
            &Default::default()
        ),
        Err(SymbolicExtensionError::Symbolic(
            SymbolicError::MissingProvider { .. }
        ))
    ));
    let mut delta = Module::provider();
    delta.symbols[0].name = b"CCCCCCCCCCA#R#f".to_vec();
    let delta = delta.build();
    let next = append_symbolic_modules_with_fallbacks(
        &base,
        &[input(&guest, C_BIAS), input(&delta, D_BIAS)],
        &[policy],
        &Default::default(),
    )
    .unwrap();
    assert_eq!(next.modules().len(), 4);
    assert_eq!(next.parent_identity(), Some(base.identity()));
    assert_eq!(next.appended_guest(), Some(next.modules()[2].id()));
    assert_eq!(
        next.retired_fallbacks(),
        &[SymbolRef {
            module: base.modules()[1].id(),
            symbol: 1
        }]
    );
    assert_eq!(next.fallback_transitions().len(), 1);
    assert_eq!(
        next.fallback_transitions()[0].new_provider().module.index(),
        2
    );
    assert_eq!(
        next.modules()[2]
            .metadata
            .as_ref()
            .unwrap()
            .link()
            .symbols()[1]
            .binding(),
        crate::SymbolBinding::WEAK
    );
    assert_eq!(next.patches().iter().find(|patch| patch.consumer().index() == 2
        && patch.source().record.symbol_index() == 2).unwrap().value(), B_BIAS + DEFINITION + 16);
    assert_eq!(next.patches().iter().find(|patch| patch.consumer().index() == 2
        && patch.source().record.symbol_index() == 3).unwrap().value(), D_BIAS + DEFINITION);
    assert_eq!(
        plan_tls_bindings(&next, &Default::default())
            .unwrap()
            .bindings()
            .len(),
        1
    );
    assert!(base.retired_fallbacks().is_empty());

    let policies = [grant(&next, 3, &[1]), grant(&next, 1, &[2])];
    let mut real = Module::provider();
    real.symbols[0].name = b"BBBBBBBBBBA#R#f".to_vec();
    let mut second = real.symbols[0].clone();
    second.name = b"CCCCCCCCCCA#R#f".to_vec();
    second.value += 16;
    real.symbols.push(second);
    let real = real.build();
    assert!(
        plan_missing_functions_for_extension(
            &next,
            input(&real, E_BIAS),
            &policies,
            &Default::default(),
            0,
        )
        .unwrap()
        .functions()
        .is_empty()
    );
    let later = append_symbolic_modules_with_fallbacks(
        &next,
        &[input(&real, E_BIAS)],
        &policies,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(later.parent_identity(), Some(next.identity()));
    assert_eq!(later.appended_guest().unwrap().index(), 4);
    assert_eq!(later.retired_fallbacks().len(), 3);
    assert_eq!(later.fallback_transitions().len(), 2);
    assert!(later.fallback_transitions().iter().all(|transition| {
        transition.new_provider().module.index() == 4
            && matches!(transition.old_provider().module.index(), 1 | 3)
    }));
    assert_eq!(
        plan_tls_bindings(&later, &Default::default())
            .unwrap()
            .bindings()
            .len(),
        1
    );
    assert!(
        lookup_module_export(&later, 3, b"CCCCCCCCCCA", &Default::default())
            .result
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        FallbackPolicy::new(
            &later,
            later.modules()[3].id(),
            &delta,
            &[SymbolRef {
                module: later.modules()[3].id(),
                symbol: 1
            }],
            &Default::default()
        ),
        Err(SymbolicExtensionError::InvalidFallback)
    ));
}

#[test]
fn batch_policies_reject_foreign_base_copied_source_and_duplicate_factories() {
    let consumer = consumer().build();
    let fallback = Module::provider().build();
    let base = plan(&consumer, &fallback).unwrap();
    let foreign = plan(&consumer, &fallback).unwrap();
    let guest = Module::provider().build();
    let duplicate = [grant(&base, 1, &[1]), grant(&base, 1, &[1])];
    assert!(matches!(
        append_symbolic_modules_with_fallbacks(
            &base,
            &[input(&guest, C_BIAS)],
            &duplicate,
            &Default::default()
        ),
        Err(SymbolicExtensionError::InvalidFallback)
    ));
    assert!(matches!(
        plan_missing_functions_for_extension(
            &base,
            input(&guest, C_BIAS),
            &duplicate,
            &Default::default(),
            0
        ),
        Err(SymbolicExtensionError::InvalidFallback)
    ));
    let policy = [grant(&foreign, 1, &[1])];
    assert!(matches!(
        append_symbolic_modules_with_fallbacks(
            &base,
            &[input(&guest, C_BIAS)],
            &policy,
            &Default::default()
        ),
        Err(SymbolicExtensionError::FallbackSource)
    ));
    assert!(matches!(
        plan_missing_functions_for_extension(
            &base,
            input(&guest, C_BIAS),
            &policy,
            &Default::default(),
            0
        ),
        Err(SymbolicExtensionError::FallbackSource)
    ));
    let copied = fallback.clone();
    assert!(matches!(
        FallbackPolicy::new(
            &base,
            base.modules()[1].id(),
            &copied,
            &[SymbolRef {
                module: base.modules()[1].id(),
                symbol: 1
            }],
            &Default::default()
        ),
        Err(SymbolicExtensionError::FallbackSource)
    ));
}

#[test]
fn extension_classifier_refuses_noncallable_addend_and_version_substitutions() {
    let consumer = consumer().build();
    let fallback = Module::provider().build();
    let base = plan(&consumer, &fallback).unwrap();
    for variant in 0..5 {
        let mut guest = imported_guest();
        guest.symbols[0].name = b"EEEEEEEEEEA#R#f".to_vec();
        import(
            &mut guest,
            if variant == 3 {
                b"AAAAAAAAAAA#B#C"
            } else {
                b"CCCCCCCCCCA#B#C"
            },
            TARGET,
        );
        match variant {
            0 => guest.symbols[1].kind = 1,
            1 => guest.symbols[1].binding = 2,
            2 => guest.rela[0].3 = 1,
            3 => guest.declarations[0].version += 1,
            _ => guest.symbols[0].name = b"AAAAAAAAAAA#R#f".to_vec(),
        }
        if variant == 4 {
            guest.symbols[0].kind = 1;
        }
        let guest = guest.build();
        let result = plan_missing_functions_for_extension(
            &base,
            input(&guest, C_BIAS),
            &[grant(&base, 1, &[1])],
            &Default::default(),
            1,
        );
        let expected = match variant {
            0 => Some(MissingFunctionReason::SymbolType),
            1 => Some(MissingFunctionReason::WeakBinding),
            2 => Some(MissingFunctionReason::AddressAddend),
            _ => None,
        };
        if let Some(expected) = expected {
            assert!(matches!(result, Err(SymbolicExtensionError::Symbolic(
                SymbolicError::UnsupportedMissingFunction { reason, .. })) if reason == expected));
        } else if variant == 3 {
            assert!(matches!(
                result,
                Err(SymbolicExtensionError::Symbolic(
                    SymbolicError::VersionMismatch { .. }
                ))
            ));
        } else {
            assert!(matches!(
                result,
                Err(SymbolicExtensionError::Symbolic(
                    SymbolicError::UnsupportedSymbol {
                        reason: UnsupportedSymbol::TypeMismatch,
                        ..
                    }
                ))
            ));
        }
    }
}

#[test]
fn delta_is_never_the_real_replacement_target_and_batch_bounds_are_inclusive() {
    let consumer = consumer().build();
    let fallback = Module::provider().build();
    let base = plan(&consumer, &fallback).unwrap();
    let mut guest = imported_guest();
    guest.symbols[0].name = b"EEEEEEEEEEA#R#f".to_vec();
    import(&mut guest, b"CCCCCCCCCCA#B#C", TARGET);
    let guest = guest.build();
    let mut delta = Module::provider();
    let mut second = delta.symbols[0].clone();
    second.name = b"CCCCCCCCCCA#R#f".to_vec();
    delta.symbols.push(second);
    let delta = delta.build();
    let policies = [grant(&base, 1, &[1])];
    assert!(matches!(
        append_symbolic_modules_with_fallbacks(
            &base,
            &[input(&guest, C_BIAS), input(&delta, D_BIAS)],
            &policies,
            &Default::default()
        ),
        Err(SymbolicExtensionError::Symbolic(
            SymbolicError::AmbiguousProvider { .. }
        ))
    ));
    assert!(matches!(
        append_symbolic_modules_with_fallbacks(&base, &[], &[], &Default::default()),
        Err(SymbolicExtensionError::Input)
    ));
    assert!(matches!(
        append_symbolic_modules_with_fallbacks(
            &base,
            &[
                input(&guest, C_BIAS),
                input(&delta, D_BIAS),
                input(&delta, E_BIAS)
            ],
            &[],
            &Default::default()
        ),
        Err(SymbolicExtensionError::Input)
    ));
    for limits in [
        SymbolicExtensionLimits {
            max_preservation_work: 0,
            ..Default::default()
        },
        SymbolicExtensionLimits {
            symbolic: SymbolicLimits {
                max_modules: 3,
                ..Default::default()
            },
            ..Default::default()
        },
    ] {
        assert!(matches!(
            append_symbolic_modules_with_fallbacks(
                &base,
                &[input(&guest, C_BIAS), input(&delta, D_BIAS)],
                &policies,
                &limits
            ),
            Err(SymbolicExtensionError::Budget)
        ));
    }
    assert!(matches!(
        plan_missing_functions_for_extension(
            &base,
            input(&guest, C_BIAS),
            &policies,
            &Default::default(),
            0
        ),
        Err(SymbolicExtensionError::Symbolic(
            SymbolicError::LimitExceeded {
                budget: SymbolicBudget::MissingFunctions,
                ..
            }
        ))
    ));
    let limits = SymbolicExtensionLimits {
        symbolic: SymbolicLimits {
            max_lookup_steps: 0,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(matches!(
        plan_missing_functions_for_extension(&base, input(&guest, C_BIAS), &policies, &limits, 1),
        Err(SymbolicExtensionError::Symbolic(
            SymbolicError::LimitExceeded {
                budget: SymbolicBudget::LookupSteps,
                ..
            }
        ))
    ));
    assert!(base.retired_fallbacks().is_empty());
}

#[test]
fn classifier_and_final_append_reject_unrelated_weak_or_null_predecessor_rebinding() {
    for absent in [false, true] {
        let mut consumer = consumer();
        import(&mut consumer, b"BBBBBBBBBBA#B#C", TARGET + 8);
        consumer.symbols[1].binding = 2;
        let consumer = consumer.build();
        let fallback = Module::provider().build();
        let mut weak = Module::provider();
        weak.symbols[0].name = b"BBBBBBBBBBA#R#f".to_vec();
        weak.symbols[0].binding = 2;
        let weak = weak.build();
        let limits = SymbolicExtensionLimits {
            symbolic: SymbolicLimits {
                missing_weak: MissingWeakPolicy::NullAddressV1,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut inputs = vec![input(&consumer, A_BIAS), input(&fallback, B_BIAS)];
        if !absent {
            inputs.push(input(&weak, C_BIAS));
        }
        let base = plan_symbolic(&inputs, &limits.symbolic).unwrap();
        let mut guest = imported_guest();
        let mut second = guest.symbols[0].clone();
        second.name = b"BBBBBBBBBBA#R#f".to_vec();
        guest.symbols.push(second);
        import(&mut guest, b"CCCCCCCCCCA#B#C", TARGET);
        let guest = guest.build();
        let policies = [grant(&base, 1, &[1])];
        assert!(matches!(
            plan_missing_functions_for_extension(
                &base,
                input(&guest, D_BIAS),
                &policies,
                &limits,
                1
            ),
            Err(SymbolicExtensionError::ChangedPredecessor)
        ));
        let mut delta = Module::provider();
        delta.symbols[0].name = b"CCCCCCCCCCA#R#f".to_vec();
        let delta = delta.build();
        assert!(matches!(
            append_symbolic_modules_with_fallbacks(
                &base,
                &[input(&guest, D_BIAS), input(&delta, E_BIAS)],
                &policies,
                &limits
            ),
            Err(SymbolicExtensionError::ChangedPredecessor)
        ));
        assert!(base.retired_fallbacks().is_empty());
    }
}
