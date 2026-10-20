// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn a_new_strong_definition_cannot_rebind_an_existing_weak_provider_or_null_import() {
    for absent in [false, true] {
        let mut consumer = Module::consumer();
        consumer.symbols[0].binding = 2;
        let consumer = consumer.build();
        let mut weak = Module::provider();
        weak.symbols[0].binding = 2;
        let weak = weak.build();
        let limits = SymbolicLimits {
            missing_weak: MissingWeakPolicy::NullAddressV1,
            ..Default::default()
        };
        let mut inputs = vec![ModuleInput::new(
            open_executable(&consumer).unwrap(),
            A_BIAS,
        )];
        if !absent {
            inputs.push(ModuleInput::new(open_executable(&weak).unwrap(), B_BIAS));
        }
        let base = plan_symbolic(&inputs, &limits).unwrap();
        let old = base.patches().to_vec();
        let strong = Module::provider().build();
        let result = append_symbolic_module(
            &base,
            ModuleInput::new(open_executable(&strong).unwrap(), B_BIAS + 0x100_0000),
            &SymbolicExtensionLimits {
                symbolic: limits,
                ..Default::default()
            },
        );
        assert!(matches!(
            result,
            Err(SymbolicExtensionError::ChangedPredecessor)
        ));
        assert_eq!(base.patches(), old);
    }
}

#[test]
fn appended_module_keeps_existing_source_imports_and_deferred_records() {
    let mut consumer = Module::consumer();
    consumer.rela.push((TARGET + 48, 1, 37, 0));
    let consumer = consumer.build();
    let provider = Module::provider().build();
    let base = plan(&consumer, &provider).unwrap();
    let mut added = Module::provider();
    added.symbols[0].name = b"BBBBBBBBBBA#R#f".to_vec();
    let added = added.build();
    let next = append_symbolic_module(
        &base,
        ModuleInput::new(open_executable(&added).unwrap(), B_BIAS + 0x100_0000),
        &Default::default(),
    )
    .unwrap();
    assert_ne!(base.identity(), next.identity());
    assert_eq!(next.parent_identity(), Some(base.identity()));
    assert_eq!(next.clone().identity(), next.identity());
    assert_eq!(next.modules().len(), 3);
    assert!(std::ptr::eq(
        next.modules()[0].source_bytes(),
        base.modules()[0].source_bytes()
    ));
    assert!(
        base.patches().iter().eq(next
            .patches()
            .iter()
            .filter(|patch| patch.consumer().index() < 2))
    );
    assert!(
        base.deferred().iter().eq(next
            .deferred()
            .iter()
            .filter(|record| record.module().index() < 2))
    );
}

#[test]
fn a_newly_ambiguous_existing_import_cannot_rebind_live_predecessor_state() {
    let consumer = Module::consumer().build();
    let provider = Module::provider().build();
    let base = plan(&consumer, &provider).unwrap();
    let old = base.patches().to_vec();
    let duplicate = Module::provider().build();
    assert!(matches!(
        append_symbolic_module(
            &base,
            ModuleInput::new(open_executable(&duplicate).unwrap(), B_BIAS + 0x100_0000),
            &Default::default()
        ),
        Err(SymbolicExtensionError::Symbolic(
            SymbolicError::AmbiguousProvider { .. }
        ))
    ));
    assert_eq!(base.patches(), old);
    assert_eq!(base.parent_identity(), None);
}

#[test]
fn source_append_refuses_module_and_preservation_work_exhaustion() {
    let consumer = Module::consumer().build();
    let provider = Module::provider().build();
    let base = plan(&consumer, &provider).unwrap();
    let added = Module::provider().build();
    let input = || ModuleInput::new(open_executable(&added).unwrap(), B_BIAS + 0x100_0000);
    let mut limits = SymbolicExtensionLimits::default();
    limits.symbolic.max_modules = 2;
    assert!(matches!(
        append_symbolic_module(&base, input(), &limits),
        Err(SymbolicExtensionError::Budget)
    ));
    limits.symbolic.max_modules = 3;
    limits.max_preservation_work = 0;
    assert!(matches!(
        append_symbolic_module(&base, input(), &limits),
        Err(SymbolicExtensionError::Budget)
    ));
}
