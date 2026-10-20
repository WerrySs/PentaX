// SPDX-License-Identifier: GPL-2.0-only

//! Original metadata policy fixtures; generated origin is authenticated by core.
use super::*;

const C_BIAS: u64 = B_BIAS + 0x100_0000;

fn consumer() -> Module {
    let mut module = Module::consumer();
    module.rela[0].3 = 0; // A callable fallback never admits a function offset.
    module.plt.push((TARGET + 8, 1, 7, -99)); // JUMP_SLOT ignores its addend.
    module.rela.push((TARGET + 16, 0, 8, 17)); // Unrelated relative value stays exact.
    module
}

fn policy(base: &SymbolicLinkPlan<'_>) -> FallbackPolicy {
    let provider = base.modules()[1].id();
    FallbackPolicy::new(
        base,
        provider,
        base.modules()[1].source_bytes(),
        &[SymbolRef {
            module: provider,
            symbol: 1,
        }],
        &Default::default(),
    )
    .unwrap()
}

#[test]
fn exact_fallback_append_has_opaque_transitions_and_strict_default_remains_ambiguous() {
    let consumer = consumer().build();
    let fallback = Module::provider().build();
    let base = plan(&consumer, &fallback).unwrap();
    let real = Module::provider().build();
    let added = || ModuleInput::new(open_executable(&real).unwrap(), C_BIAS);
    assert!(matches!(
        append_symbolic_module(&base, added(), &Default::default()),
        Err(SymbolicExtensionError::Symbolic(
            SymbolicError::AmbiguousProvider { .. }
        ))
    ));
    let grant = policy(&base);
    assert_eq!(grant.source_identity(), base.identity());
    assert_eq!(grant.provider(), base.modules()[1].id());
    let next =
        append_symbolic_module_with_fallbacks(&base, added(), &grant, &Default::default()).unwrap();
    assert_eq!(next.parent_identity(), Some(base.identity()));
    assert_eq!(next.retired_fallbacks(), grant.symbols());
    assert_eq!(next.fallback_transitions().len(), 2);
    for transition in next.fallback_transitions() {
        assert_eq!(transition.parent_identity(), base.identity());
        assert_eq!(transition.next_identity(), next.identity());
        assert_eq!(transition.old_provider(), grant.symbols()[0]);
        assert_eq!(transition.new_provider().module.index(), 2);
        assert_eq!(transition.old_patch().value(), B_BIAS + DEFINITION);
        assert_eq!(transition.new_patch().value(), C_BIAS + DEFINITION);
        assert_eq!(
            transition.old_patch().address(),
            transition.new_patch().address()
        );
        assert_eq!(
            transition.old_patch().source(),
            transition.new_patch().source()
        );
    }
    assert_eq!(base.patches()[2], next.patches()[2]);
    assert!(base.retired_fallbacks().is_empty());
    assert!(base.fallback_transitions().is_empty());
}

#[test]
fn fallback_policy_rejects_wrong_source_duplicate_weak_object_absolute_and_tight_limits() {
    let consumer = consumer().build();
    let fallback = Module::provider().build();
    let base = plan(&consumer, &fallback).unwrap();
    let grant = policy(&base);
    let copied = fallback.clone();
    assert!(matches!(
        FallbackPolicy::new(
            &base,
            grant.provider(),
            &copied,
            grant.symbols(),
            &Default::default()
        ),
        Err(SymbolicExtensionError::FallbackSource)
    ));
    assert!(matches!(
        FallbackPolicy::new(
            &base,
            grant.provider(),
            &fallback,
            &[grant.symbols()[0]; 2],
            &Default::default()
        ),
        Err(SymbolicExtensionError::InvalidFallback)
    ));
    let separately_planned = plan(&consumer, &fallback).unwrap();
    let real = Module::provider().build();
    assert!(matches!(
        append_symbolic_module_with_fallbacks(
            &separately_planned,
            ModuleInput::new(open_executable(&real).unwrap(), C_BIAS),
            &grant,
            &Default::default()
        ),
        Err(SymbolicExtensionError::FallbackSource)
    ));
    for variant in 0..4 {
        let mut changed = Module::provider();
        match variant {
            0 => changed.symbols[0].binding = 2,
            1 => changed.symbols[0].kind = 1,
            2 => changed.symbols[0].section = 0xfff1,
            _ => changed.symbols[0].kind = 6,
        }
        // No import is needed to inspect and reject an ineligible policy target.
        changed.primary = true;
        let bytes = changed.build();
        let source = plan_symbolic(
            &[ModuleInput::new(open_executable(&bytes).unwrap(), B_BIAS)],
            &Default::default(),
        )
        .unwrap();
        let reference = SymbolRef {
            module: source.modules()[0].id(),
            symbol: 1,
        };
        assert!(matches!(
            FallbackPolicy::new(
                &source,
                reference.module,
                &bytes,
                &[reference],
                &Default::default()
            ),
            Err(SymbolicExtensionError::Symbolic(_))
        ));
    }
    let limits = SymbolicExtensionLimits {
        max_preservation_work: 0,
        ..Default::default()
    };
    assert!(matches!(
        FallbackPolicy::new(&base, grant.provider(), &fallback, grant.symbols(), &limits),
        Err(SymbolicExtensionError::Budget)
    ));
    assert!(matches!(
        append_symbolic_module_with_fallbacks(
            &base,
            ModuleInput::new(open_executable(&real).unwrap(), C_BIAS),
            &grant,
            &limits
        ),
        Err(SymbolicExtensionError::Budget)
    ));
}

#[test]
fn fallback_replacement_rejects_version_type_binding_and_two_strong_candidates() {
    let consumer = consumer().build();
    let fallback = Module::provider().build();
    let base = plan(&consumer, &fallback).unwrap();
    let grant = policy(&base);
    for variant in 0..5 {
        let mut real = Module::provider();
        match variant {
            0 => real.minor += 1,
            1 => real.symbols[0].kind = 1,
            2 => real.symbols[0].binding = 3,
            3 => real.symbols.push(real.symbols[0].clone()),
            _ => real.symbols[0].section = 0xfff1,
        }
        let bytes = real.build();
        let result = append_symbolic_module_with_fallbacks(
            &base,
            ModuleInput::new(open_executable(&bytes).unwrap(), C_BIAS),
            &grant,
            &Default::default(),
        );
        match variant {
            0 => assert!(matches!(
                result,
                Err(SymbolicExtensionError::Symbolic(
                    SymbolicError::VersionMismatch { .. }
                ))
            )),
            1 | 2 | 4 => assert!(matches!(
                result,
                Err(SymbolicExtensionError::Symbolic(
                    SymbolicError::UnsupportedSymbol { .. }
                ))
            )),
            _ => assert!(matches!(
                result,
                Err(SymbolicExtensionError::Symbolic(
                    SymbolicError::AmbiguousProvider { .. }
                ))
            )),
        }
    }
}

#[test]
fn unique_weak_replacement_retains_real_identity_and_strict_append_keeps_the_marker() {
    let consumer = consumer().build();
    let fallback = Module::provider().build();
    let base = plan(&consumer, &fallback).unwrap();
    let mut weak = Module::provider();
    weak.symbols[0].binding = 2;
    let weak = weak.build();
    let added = || ModuleInput::new(open_executable(&weak).unwrap(), C_BIAS);
    let strict = append_symbolic_module(&base, added(), &Default::default()).unwrap();
    assert_eq!(strict.patches(), base.patches());
    assert!(strict.retired_fallbacks().is_empty());
    assert!(strict.fallback_transitions().is_empty());

    let next =
        append_symbolic_module_with_fallbacks(&base, added(), &policy(&base), &Default::default())
            .unwrap();
    let real = SymbolRef {
        module: next.modules()[2].id(),
        symbol: 1,
    };
    assert_eq!(next.fallback_transitions().len(), 2);
    assert_eq!(next.retired_fallbacks(), policy(&base).symbols());
    assert!(std::ptr::eq(next.modules()[2].source_bytes(), &weak[..]));
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
    for transition in next.fallback_transitions() {
        assert_eq!(transition.new_provider(), real);
        assert_eq!(
            transition.new_patch().resolution(),
            Resolution::Definition(real)
        );
        assert_eq!(transition.new_patch().value(), C_BIAS + DEFINITION);
        assert_eq!(transition.old_provider(), policy(&base).symbols()[0]);
    }
    assert_eq!(next.patches()[2], base.patches()[2]);
}

#[test]
fn real_strong_precedence_over_weak_candidates_is_independent_of_symbol_order() {
    let consumer = consumer().build();
    let fallback = Module::provider().build();
    let base = plan(&consumer, &fallback).unwrap();
    for strong_first in [false, true] {
        let mut real = Module::provider();
        let mut weak = real.symbols[0].clone();
        weak.binding = 2;
        weak.value += 16;
        real.symbols[0].value += 32;
        let strong_index = if strong_first {
            real.symbols.extend([weak.clone(), weak]);
            1
        } else {
            real.symbols.insert(0, weak.clone());
            real.symbols.insert(0, weak);
            3
        };
        let real = real.build();
        let next = append_symbolic_module_with_fallbacks(
            &base,
            ModuleInput::new(open_executable(&real).unwrap(), C_BIAS),
            &policy(&base),
            &Default::default(),
        )
        .unwrap();
        for transition in next.fallback_transitions() {
            assert_eq!(transition.new_provider().symbol, strong_index);
            assert_eq!(transition.new_patch().value(), C_BIAS + DEFINITION + 32);
        }
    }
}

#[test]
fn real_weak_only_duplicates_refuse_within_and_across_images() {
    let consumer = consumer().build();
    let fallback = Module::provider().build();
    let mut weak = Module::provider();
    weak.symbols[0].binding = 2;
    let retained = weak.build();
    for across_images in [false, true] {
        let mut inputs = vec![
            ModuleInput::new(open_executable(&consumer).unwrap(), A_BIAS),
            ModuleInput::new(open_executable(&fallback).unwrap(), B_BIAS),
        ];
        if across_images {
            inputs.push(ModuleInput::new(
                open_executable(&retained).unwrap(),
                C_BIAS,
            ));
        }
        let base = plan_symbolic(&inputs, &Default::default()).unwrap();
        let mut added = weak.clone();
        if !across_images {
            added.symbols.push(added.symbols[0].clone());
        }
        let added = added.build();
        assert!(matches!(
            append_symbolic_module_with_fallbacks(
                &base,
                ModuleInput::new(open_executable(&added).unwrap(), C_BIAS + 0x100_0000),
                &policy(&base),
                &Default::default()
            ),
            Err(SymbolicExtensionError::Symbolic(
                SymbolicError::AmbiguousProvider { .. }
            ))
        ));
        assert!(base.retired_fallbacks().is_empty());
    }
}

#[test]
fn added_strong_wins_over_retained_weak_but_wrong_type_never_authorizes_retirement() {
    let consumer = consumer().build();
    let fallback = Module::provider().build();
    let mut weak = Module::provider();
    weak.symbols[0].binding = 2;
    let weak = weak.build();
    let base = plan_symbolic(
        &[
            ModuleInput::new(open_executable(&consumer).unwrap(), A_BIAS),
            ModuleInput::new(open_executable(&fallback).unwrap(), B_BIAS),
            ModuleInput::new(open_executable(&weak).unwrap(), C_BIAS),
        ],
        &Default::default(),
    )
    .unwrap();
    for wrong_type in [false, true] {
        let mut real = Module::provider();
        if wrong_type {
            let mut object = real.symbols[0].clone();
            object.binding = 2;
            object.kind = 1;
            real.symbols.push(object);
        }
        let real = real.build();
        let result = append_symbolic_module_with_fallbacks(
            &base,
            ModuleInput::new(open_executable(&real).unwrap(), C_BIAS + 0x100_0000),
            &policy(&base),
            &Default::default(),
        );
        if wrong_type {
            assert!(matches!(
                result,
                Err(SymbolicExtensionError::Symbolic(
                    SymbolicError::UnsupportedSymbol {
                        reason: UnsupportedSymbol::TypeMismatch,
                        ..
                    }
                ))
            ));
        } else {
            let next = result.unwrap();
            assert_eq!(next.fallback_transitions().len(), 2);
            assert!(next.fallback_transitions().iter().all(|transition| {
                transition.new_provider().module.index() == 3
                    && transition.new_patch().value() == C_BIAS + 0x100_0000 + DEFINITION
            }));
        }
    }
}

#[test]
fn weak_replacement_preserves_version_visibility_and_executable_range_gates() {
    let consumer = consumer().build();
    let fallback = Module::provider().build();
    let base = plan(&consumer, &fallback).unwrap();
    for variant in 0..5 {
        let mut weak = Module::provider();
        weak.symbols[0].binding = 2;
        match variant {
            0 => weak.version += 1,
            1 => weak.symbols[0].other = 4,
            2 => weak.symbols[0].value = BASE + 0x2000,
            3 => weak.flags = 4,
            _ => weak.symbols[0].section = 0xfff1,
        }
        let weak = weak.build();
        let result = append_symbolic_module_with_fallbacks(
            &base,
            ModuleInput::new(open_executable(&weak).unwrap(), C_BIAS),
            &policy(&base),
            &Default::default(),
        );
        match variant {
            0 => assert!(matches!(
                result,
                Err(SymbolicExtensionError::Symbolic(
                    SymbolicError::VersionMismatch { .. }
                ))
            )),
            1 | 4 => assert!(matches!(
                result,
                Err(SymbolicExtensionError::Symbolic(
                    SymbolicError::UnsupportedSymbol { .. }
                ))
            )),
            2 => assert!(matches!(
                result,
                Err(SymbolicExtensionError::Symbolic(
                    SymbolicError::SymbolRange { .. }
                ))
            )),
            _ => assert!(matches!(
                result,
                Err(SymbolicExtensionError::Symbolic(
                    SymbolicError::FunctionNotExecutable { .. }
                ))
            )),
        }
    }
}

#[test]
fn fallback_exception_never_admits_changed_weak_null_or_nonzero_function_addends() {
    for variant in 0..3 {
        let mut consumer = consumer();
        if variant == 0 {
            consumer.symbols[0].binding = 2;
        }
        if variant == 1 {
            consumer.rela[0].3 = 1;
        }
        if variant == 2 {
            let mut optional = consumer.symbols[0].clone();
            optional.name = b"BBBBBBBBBBA#B#C".to_vec();
            optional.binding = 2;
            consumer.symbols.push(optional);
            consumer.rela.push((TARGET + 24, 2, 1, 0));
        }
        let consumer = consumer.build();
        let fallback = Module::provider().build();
        let limits = SymbolicLimits {
            missing_weak: MissingWeakPolicy::NullAddressV1,
            ..Default::default()
        };
        let base = plan_symbolic(
            &[
                ModuleInput::new(open_executable(&consumer).unwrap(), A_BIAS),
                ModuleInput::new(open_executable(&fallback).unwrap(), B_BIAS),
            ],
            &limits,
        )
        .unwrap();
        let mut real = Module::provider();
        if variant == 2 {
            let mut extra = real.symbols[0].clone();
            extra.name = b"BBBBBBBBBBA#R#f".to_vec();
            real.symbols.push(extra);
        }
        let real = real.build();
        let grant = policy(&base);
        assert!(matches!(
            append_symbolic_module_with_fallbacks(
                &base,
                ModuleInput::new(open_executable(&real).unwrap(), C_BIAS),
                &grant,
                &SymbolicExtensionLimits {
                    symbolic: limits,
                    ..Default::default()
                }
            ),
            Err(SymbolicExtensionError::ChangedPredecessor)
        ));
    }
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
fn an_allowed_fallback_change_does_not_admit_an_unrelated_weak_provider_change() {
    let mut consumer = consumer();
    let mut other = consumer.symbols[0].clone();
    other.name = b"BBBBBBBBBBA#B#C".to_vec();
    consumer.symbols.push(other);
    consumer.rela.push((TARGET + 24, 2, 1, 0));
    let consumer = consumer.build();
    let fallback = Module::provider().build();
    let mut weak = Module::provider();
    weak.symbols[0].name = b"BBBBBBBBBBA#R#f".to_vec();
    weak.symbols[0].binding = 2;
    let weak = weak.build();
    let base = plan_symbolic(
        &[
            ModuleInput::new(open_executable(&consumer).unwrap(), A_BIAS),
            ModuleInput::new(open_executable(&fallback).unwrap(), B_BIAS),
            ModuleInput::new(open_executable(&weak).unwrap(), C_BIAS),
        ],
        &Default::default(),
    )
    .unwrap();
    let mut real = Module::provider();
    let mut other = real.symbols[0].clone();
    other.name = b"BBBBBBBBBBA#R#f".to_vec();
    real.symbols.push(other);
    let real = real.build();
    assert!(matches!(
        append_symbolic_module_with_fallbacks(
            &base,
            ModuleInput::new(open_executable(&real).unwrap(), C_BIAS + 0x100_0000),
            &policy(&base),
            &Default::default()
        ),
        Err(SymbolicExtensionError::ChangedPredecessor)
    ));
}

#[test]
fn retirement_survives_future_append_tls_reconstruction_and_module_lookup() {
    let mut consumer = consumer();
    let mut tls = consumer.symbols[0].clone();
    tls.name = b"BBBBBBBBBBA#B#C".to_vec();
    tls.kind = 6;
    consumer.symbols.push(tls);
    consumer.rela.push((TARGET + 32, 2, 16, 0));
    let consumer = consumer.build();
    let mut fallback = Module::provider();
    let mut tls = fallback.symbols[0].clone();
    tls.name = b"BBBBBBBBBBA#R#f".to_vec();
    tls.kind = 6;
    tls.value = 0;
    fallback.symbols.push(tls);
    let fallback = with_tls(&fallback);
    let base = plan(&consumer, &fallback).unwrap();
    let real = Module::provider().build();
    let next = append_symbolic_module_with_fallbacks(
        &base,
        ModuleInput::new(open_executable(&real).unwrap(), C_BIAS),
        &policy(&base),
        &Default::default(),
    )
    .unwrap();
    let bindings = plan_tls_bindings(&next, &Default::default()).unwrap();
    assert_eq!(bindings.bindings().len(), 1);
    assert_eq!(bindings.bindings()[0].definition().module().index(), 1);
    let mut rebuilt = super::super::resolve::Resolver::with_retired(
        next.modules(),
        (&SymbolicLimits::default()).into(),
        next.retired_fallbacks(),
    )
    .unwrap();
    let (value, resolution) = rebuilt
        .resolve(
            SymbolRef {
                module: next.modules()[0].id(),
                symbol: 1,
            },
            true,
        )
        .unwrap();
    assert_eq!(value, C_BIAS + DEFINITION);
    assert!(
        matches!(resolution, Resolution::Definition(reference) if reference.module.index() == 2)
    );
    assert!(
        lookup_module_export(&next, 1, b"AAAAAAAAAAA", &Default::default())
            .result
            .unwrap()
            .is_none()
    );
    assert_eq!(
        lookup_module_export(&next, 2, b"AAAAAAAAAAA", &Default::default())
            .result
            .unwrap()
            .unwrap()
            .address(),
        C_BIAS + DEFINITION
    );
    let mut unrelated = Module::provider();
    unrelated.symbols[0].name = b"CCCCCCCCCCA#R#f".to_vec();
    let unrelated = unrelated.build();
    let later = append_symbolic_module(
        &next,
        ModuleInput::new(open_executable(&unrelated).unwrap(), C_BIAS + 0x100_0000),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(later.retired_fallbacks(), next.retired_fallbacks());
    assert!(later.fallback_transitions().is_empty());
    assert_eq!(
        plan_tls_bindings(&later, &Default::default())
            .unwrap()
            .bindings()
            .len(),
        1
    );
    let duplicate = Module::provider().build();
    assert!(matches!(
        append_symbolic_module(
            &next,
            ModuleInput::new(open_executable(&duplicate).unwrap(), C_BIAS + 0x100_0000),
            &Default::default()
        ),
        Err(SymbolicExtensionError::Symbolic(
            SymbolicError::AmbiguousProvider { .. }
        ))
    ));
}

#[test]
fn retained_real_winner_does_not_authorize_marker_retirement() {
    // With no callable import yet, two retained GLOBAL declarations can be
    // inspected without resolving this key. An added WEAK is not its winner.
    let mut consumer = consumer();
    consumer.symbols.clear();
    consumer.plt.clear();
    consumer.rela.retain(|record| record.1 == 0);
    let consumer = consumer.build();
    let fallback = Module::provider().build();
    let retained = Module::provider().build();
    let base = plan_symbolic(
        &[
            ModuleInput::new(open_executable(&consumer).unwrap(), A_BIAS),
            ModuleInput::new(open_executable(&fallback).unwrap(), B_BIAS),
            ModuleInput::new(open_executable(&retained).unwrap(), C_BIAS),
        ],
        &Default::default(),
    )
    .unwrap();
    let mut weak = Module::provider();
    weak.symbols[0].binding = 2;
    let weak = weak.build();
    let next = append_symbolic_module_with_fallbacks(
        &base,
        ModuleInput::new(open_executable(&weak).unwrap(), C_BIAS + 0x100_0000),
        &policy(&base),
        &Default::default(),
    )
    .unwrap();
    assert!(next.retired_fallbacks().is_empty());
    assert!(next.fallback_transitions().is_empty());
    assert_eq!(next.patches(), base.patches());
}

#[test]
fn weak_retirement_survives_tls_and_future_append_without_authorizing_real_rebinding() {
    let mut consumer = consumer();
    let mut tls = consumer.symbols[0].clone();
    tls.name = b"BBBBBBBBBBA#B#C".to_vec();
    tls.kind = 6;
    consumer.symbols.push(tls);
    consumer.rela.push((TARGET + 32, 2, 16, 0));
    let consumer = consumer.build();
    let mut fallback = Module::provider();
    let mut tls = fallback.symbols[0].clone();
    tls.name = b"BBBBBBBBBBA#R#f".to_vec();
    tls.kind = 6;
    tls.value = 0;
    fallback.symbols.push(tls);
    let fallback = with_tls(&fallback);
    let base = plan(&consumer, &fallback).unwrap();
    let mut weak = Module::provider();
    weak.symbols[0].binding = 2;
    let weak = weak.build();
    let next = append_symbolic_module_with_fallbacks(
        &base,
        ModuleInput::new(open_executable(&weak).unwrap(), C_BIAS),
        &policy(&base),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        plan_tls_bindings(&next, &Default::default())
            .unwrap()
            .bindings()
            .len(),
        1
    );
    let mut rebuilt = super::super::resolve::Resolver::with_retired(
        next.modules(),
        (&SymbolicLimits::default()).into(),
        next.retired_fallbacks(),
    )
    .unwrap();
    let actual = SymbolRef {
        module: next.modules()[2].id(),
        symbol: 1,
    };
    assert_eq!(
        rebuilt
            .resolve(
                SymbolRef {
                    module: next.modules()[0].id(),
                    symbol: 1
                },
                true
            )
            .unwrap(),
        (C_BIAS + DEFINITION, Resolution::Definition(actual))
    );
    assert!(
        lookup_module_export(&next, 1, b"AAAAAAAAAAA", &Default::default())
            .result
            .unwrap()
            .is_none()
    );
    // Module-local lookup retains its independent GLOBAL-only contract.
    assert!(matches!(
        lookup_module_export(&next, 2, b"AAAAAAAAAAA", &Default::default()).result,
        Err(ModuleLookupError::Symbolic(
            SymbolicError::UnsupportedSymbol {
                reason: UnsupportedSymbol::WeakProvider,
                ..
            }
        ))
    ));
    assert!(matches!(
        FallbackPolicy::new(
            &next,
            next.modules()[1].id(),
            &fallback,
            next.retired_fallbacks(),
            &Default::default()
        ),
        Err(SymbolicExtensionError::InvalidFallback)
    ));

    let mut unrelated = Module::provider();
    unrelated.symbols[0].name = b"CCCCCCCCCCA#R#f".to_vec();
    let unrelated = unrelated.build();
    let later = append_symbolic_module(
        &next,
        ModuleInput::new(open_executable(&unrelated).unwrap(), C_BIAS + 0x100_0000),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(later.retired_fallbacks(), next.retired_fallbacks());
    assert_eq!(later.patches(), next.patches());
    assert!(later.fallback_transitions().is_empty());
    assert_eq!(
        plan_tls_bindings(&later, &Default::default())
            .unwrap()
            .bindings()
            .len(),
        1
    );

    let strong = Module::provider().build();
    let added = || ModuleInput::new(open_executable(&strong).unwrap(), C_BIAS + 0x200_0000);
    assert!(matches!(
        append_symbolic_module(&later, added(), &Default::default()),
        Err(SymbolicExtensionError::ChangedPredecessor)
    ));
    let no_new_grants = FallbackPolicy::new(
        &later,
        later.modules()[1].id(),
        &fallback,
        &[],
        &Default::default(),
    )
    .unwrap();
    assert!(matches!(
        append_symbolic_module_with_fallbacks(&later, added(), &no_new_grants, &Default::default()),
        Err(SymbolicExtensionError::ChangedPredecessor)
    ));
}
