// SPDX-License-Identifier: GPL-2.0-or-later

// Reuse PentaX's original module encoder, with an independently added PT_TLS.
use super::super::fixture;

use super::*;
use crate::{
    DescriptorEncoding, DescriptorRole, ModuleInput, SymbolicBudget, SymbolicLimits,
    UnsupportedSymbol, plan_symbolic,
};
use fixture::*;
use pentax_loader::open_executable;

fn pair() -> (Module, Module) {
    let mut consumer = Module::consumer();
    let mut provider = Module::provider();
    consumer.symbols[0].kind = 6;
    consumer.rela = vec![(TARGET, 1, 16, 0), (TARGET + 8, 1, 17, -1)];
    consumer.plt = vec![(TARGET + 16, 1, 18, 2)];
    provider.symbols[0].kind = 6;
    provider.symbols[0].value = 0;
    (consumer, provider)
}

fn image(module: &Module, tls: Option<(u64, u64)>) -> Vec<u8> {
    let mut bytes = module.build();
    if let Some((initialized, memory)) = tls {
        let count = u16::from_le_bytes(bytes[56..58].try_into().unwrap());
        put(&mut bytes, 56, &(count + 1).to_le_bytes());
        let header = 64 + usize::from(count) * 56;
        put(&mut bytes, header, &7_u32.to_le_bytes());
        put(&mut bytes, header + 4, &4_u32.to_le_bytes());
        for (offset, value) in [
            (8, 0xa00_u64),
            (16, BASE + 0xa00),
            (32, initialized),
            (40, memory),
            (48, 8),
        ] {
            put(&mut bytes, header + offset, &value.to_le_bytes());
        }
    }
    bytes
}

fn base<'a>(images: &[&'a [u8]]) -> Result<SymbolicLinkPlan<'a>, SymbolicError> {
    let inputs: Vec<_> = images
        .iter()
        .enumerate()
        .map(|(index, bytes)| {
            ModuleInput::new(
                open_executable(bytes).unwrap(),
                A_BIAS + index as u64 * (B_BIAS - A_BIAS),
            )
        })
        .collect();
    plan_symbolic(&inputs, &SymbolicLimits::default())
}

fn bind<'p, 'i>(base: &'p SymbolicLinkPlan<'i>) -> Result<TlsBindingPlan<'p, 'i>, TlsBindingError> {
    plan_tls_bindings(base, &TlsBindingLimits::default())
}

fn resolution_error(result: Result<TlsBindingPlan<'_, '_>, TlsBindingError>) -> SymbolicError {
    match result {
        Err(TlsBindingError::Resolution { error, .. }) => *error,
        other => panic!("expected a TLS resolution failure, got {other:?}"),
    }
}

#[test]
fn typed_zero_offsets_foreign_providers_sources_and_domains_are_retained() {
    for dynlib in [false, true] {
        for consumer_encoding in [
            DescriptorEncoding::Classic,
            DescriptorEncoding::Variant1Observed,
        ] {
            for provider_encoding in [
                DescriptorEncoding::Classic,
                DescriptorEncoding::Variant1Observed,
            ] {
                for offset in [0, 8] {
                    let (mut consumer, mut provider) = pair();
                    consumer.dynlib = dynlib;
                    provider.dynlib = dynlib;
                    consumer.module_encoding = consumer_encoding;
                    consumer.library_encoding = provider_encoding;
                    // SHN_UNDEF controls import status even with nonzero value bits.
                    consumer.symbols[0].value = u64::MAX;
                    provider.module_encoding = provider_encoding;
                    provider.library_encoding = consumer_encoding;
                    provider.symbols[0].value = offset;
                    let consumer = image(&consumer, None);
                    let provider = image(&provider, Some((16, 64)));
                    let original_consumer = consumer.clone();
                    let original_provider = provider.clone();
                    let base = base(&[&consumer, &provider]).unwrap();
                    // GLOBAL STT_TLS is indexed without ordinary resolution.
                    assert_eq!(base.patch_count(), 0);
                    assert_eq!(base.deferred_count(), 3);
                    let tls = bind(&base).unwrap();
                    assert!(std::ptr::eq(tls.base_plan(), &base));
                    assert_eq!(
                        tls.policy(),
                        TlsBindingPolicy::GenericAmd64ExactDeclarationV1
                    );
                    assert!(tls.remaining_deferred().is_empty());
                    for (index, binding) in tls.bindings().iter().enumerate() {
                        assert_eq!(binding.deferred_index(), index);
                        assert_eq!(binding.consumer(), base.modules()[0].id());
                        assert_eq!(binding.source(), base.deferred()[index].source());
                        assert_eq!(binding.address(), A_BIAS + TARGET + index as u64 * 8);
                        assert_eq!(binding.definition().module().index(), 1);
                        assert_eq!(binding.definition().symbol().unwrap().symbol, 1);
                        assert_eq!(binding.definition().offset().get(), offset);
                        assert_eq!(binding.definition().size(), 8);
                        assert_eq!(binding.definition().template_mem_size(), 64);
                        assert_eq!(
                            binding.definition().program_header_index(),
                            if dynlib { 3 } else { 2 }
                        );
                    }
                    assert_eq!(tls.bindings()[0].kind(), TlsRelocationKind::DtpMod64);
                    assert_eq!(tls.bindings()[1].kind(), TlsRelocationKind::DtpOff64);
                    assert_eq!(tls.bindings()[2].kind(), TlsRelocationKind::TpOff64);
                    assert_eq!(tls.bindings()[2].source().table, TableKind::PltRela);
                    assert_eq!(consumer, original_consumer);
                    assert_eq!(provider, original_provider);
                    assert_eq!(base.deferred_count(), 3);
                }
            }
        }
    }
}

#[test]
fn bindings_sort_by_target_without_losing_original_deferred_positions() {
    let (mut consumer, provider) = pair();
    consumer.rela.swap(0, 1);
    consumer.rela.push((TARGET + 40, 0, 37, -77));
    consumer.rela.push((TARGET + 32, 0, 8, 19));
    let consumer = image(&consumer, None);
    let provider = image(&provider, Some((16, 64)));
    let base = base(&[&consumer, &provider]).unwrap();
    let tls = bind(&base).unwrap();
    assert_eq!(
        tls.bindings()
            .iter()
            .map(TlsBinding::deferred_index)
            .collect::<Vec<_>>(),
        [1, 0, 3]
    );
    assert_eq!(tls.remaining_deferred(), &base.deferred()[2..3]);
    assert_eq!(base.patch_count(), 1);
    assert_eq!(base.deferred_count(), 4);
}

#[test]
fn self_bindings_require_tls_and_nonpreemptible_policy() {
    for (binding, visibility) in [(0, 0), (0, 2), (1, 2), (1, 3)] {
        let (mut module, _) = pair();
        module.rela = vec![(TARGET, 1, 17, 0)];
        module.plt.clear();
        module.symbols[0].section = 1;
        module.symbols[0].value = 0;
        module.symbols[0].binding = binding;
        module.symbols[0].other = visibility;
        let bytes = image(&module, Some((16, 64)));
        let base = base(&[&bytes]).unwrap();
        let tls = bind(&base).unwrap();
        assert_eq!(tls.bindings()[0].definition().offset().get(), 0);
        assert_eq!(tls.bindings()[0].definition().module().index(), 0);
    }
    let (mut module, _) = pair();
    module.rela.truncate(1);
    module.plt.clear();
    module.symbols[0].section = 1;
    let bytes = image(&module, Some((16, 64)));
    let base = base(&[&bytes]).unwrap();
    assert!(matches!(
        resolution_error(bind(&base)),
        SymbolicError::UnsupportedSymbol {
            reason: UnsupportedSymbol::PreemptibleDefinition,
            ..
        }
    ));
}

#[test]
fn unsupported_symbol_zero_offsets_and_nonzero_module_addends_fail() {
    for kind in [17, 18] {
        let (mut consumer, provider) = pair();
        consumer.rela = vec![(TARGET, 0, kind, 0)];
        consumer.plt.clear();
        let consumer = image(&consumer, None);
        let provider = image(&provider, Some((16, 64)));
        let base = base(&[&consumer, &provider]).unwrap();
        assert!(
            matches!(bind(&base), Err(TlsBindingError::UnsupportedSymbolZero { consumer, source })
            if consumer.index() == 0 && source == base.deferred()[0].source())
        );
    }
    for addend in [-1, 1, i64::MIN, i64::MAX] {
        let (mut consumer, provider) = pair();
        consumer.rela = vec![(TARGET, 1, 16, addend)];
        consumer.plt.clear();
        let consumer = image(&consumer, None);
        let provider = image(&provider, Some((16, 64)));
        let base = base(&[&consumer, &provider]).unwrap();
        assert!(
            matches!(bind(&base), Err(TlsBindingError::UnsupportedModuleAddend { source, .. })
            if source.record.addend == addend)
        );
    }
}

#[test]
fn tls_symbol_ranges_use_full_template_not_image_virtual_addresses() {
    for (offset, size, initialized, memory, accepted) in [
        (0, 8, 16, 64, true),
        (32, 8, 16, 64, true),
        (63, 0, 0, 64, true),
        (64, 0, 16, 64, false),
        (63, 2, 16, 64, false),
        (0, 0, 0, 0, false),
        (u64::MAX - 1, 8, 16, 64, false),
        (BASE + 0xa00, 8, 16, 64, false),
    ] {
        let (consumer, mut provider) = pair();
        provider.symbols[0].value = offset;
        provider.symbols[0].size = size;
        let consumer = image(&consumer, None);
        let provider = image(&provider, Some((initialized, memory)));
        let base = base(&[&consumer, &provider]).unwrap();
        let result = bind(&base);
        assert_eq!(result.is_ok(), accepted, "offset {offset} size {size}");
        if !accepted {
            assert!(
                matches!(result, Err(TlsBindingError::TlsSymbolRange { definition, .. }) if definition.module.index() == 1)
            );
        }
    }
    let (consumer, provider) = pair();
    let consumer = image(&consumer, None);
    let provider = image(&provider, None);
    let base = base(&[&consumer, &provider]).unwrap();
    assert!(matches!(
        bind(&base),
        Err(TlsBindingError::MissingTlsTemplate { .. })
    ));
}

#[test]
fn tls_type_section_binding_and_visibility_errors_stay_typed() {
    for (kind, section, binding, other, reason) in [
        (1, 1, 1, 2, UnsupportedSymbol::Type),
        (2, 1, 1, 2, UnsupportedSymbol::Type),
        (0, 1, 0, 0, UnsupportedSymbol::Type),
        (10, 1, 0, 0, UnsupportedSymbol::Type),
        (6, 0xfff1, 0, 0, UnsupportedSymbol::Section),
        (6, 0xfff2, 0, 0, UnsupportedSymbol::Section),
        (6, 0xff00, 0, 0, UnsupportedSymbol::Section),
        (6, 1, 2, 2, UnsupportedSymbol::Binding),
        (6, 1, 0, 1, UnsupportedSymbol::Visibility),
        (6, 1, 0, 8, UnsupportedSymbol::OtherBits),
    ] {
        let (mut module, _) = pair();
        module.rela.truncate(1);
        module.plt.clear();
        let symbol = &mut module.symbols[0];
        symbol.kind = kind;
        symbol.section = section;
        symbol.binding = binding;
        symbol.other = other;
        let bytes = image(&module, Some((16, 64)));
        let base = base(&[&bytes]).unwrap();
        assert!(
            matches!(resolution_error(bind(&base)),
                SymbolicError::UnsupportedSymbol { reason: actual, .. } if actual == reason),
            "kind {kind} section {section} binding {binding} other {other}"
        );
    }
}

#[test]
fn exact_foreign_resolution_preserves_missing_version_type_and_weak_errors() {
    for case in 0..6 {
        let (mut consumer, mut provider) = pair();
        match case {
            0 => provider.module_name.push(b'x'),
            1 => provider.minor += 1,
            2 => provider.symbols[0].kind = 1,
            3 => provider.symbols[0].binding = 2,
            4 => {
                consumer.symbols[0].binding = 2;
                provider.library_name.push(b'x');
            }
            5 => provider.symbols[0].other = 3,
            _ => unreachable!(),
        }
        let consumer = image(&consumer, None);
        let provider = image(&provider, Some((16, 64)));
        let base = base(&[&consumer, &provider]).unwrap();
        let result = bind(&base);
        if case == 5 {
            assert!(result.is_ok());
            continue;
        }
        let Err(TlsBindingError::Resolution { error, .. }) = result else {
            panic!("case {case}");
        };
        let error = *error;
        assert!(
            match case {
                0 | 4 => matches!(error, SymbolicError::MissingProvider { .. }),
                1 => matches!(error, SymbolicError::VersionMismatch { .. }),
                2 => matches!(
                    error,
                    SymbolicError::UnsupportedSymbol {
                        reason: UnsupportedSymbol::TypeMismatch,
                        ..
                    }
                ),
                3 => matches!(
                    error,
                    SymbolicError::UnsupportedSymbol {
                        reason: UnsupportedSymbol::WeakProvider,
                        ..
                    }
                ),
                _ => false,
            },
            "case {case}: {error:?}"
        );
    }
    let (consumer, provider) = pair();
    let consumer = image(&consumer, None);
    let provider = image(&provider, Some((16, 64)));
    let base = base(&[&consumer, &provider, &provider]).unwrap();
    assert!(matches!(
        resolution_error(bind(&base)),
        SymbolicError::AmbiguousProvider { .. }
    ));
}

#[test]
fn tls_targets_and_compositions_keep_full_original_record_context() {
    for address in [BASE - 1, BASE + 0x13fc, BASE + 0x1400, BASE + 0x2000] {
        let (mut consumer, provider) = pair();
        consumer.rela = vec![(address, 1, 17, 0)];
        consumer.plt.clear();
        let consumer = image(&consumer, None);
        let provider = image(&provider, Some((16, 64)));
        let base = base(&[&consumer, &provider]).unwrap();
        assert!(
            matches!(bind(&base), Err(TlsBindingError::Relocation { consumer, source, .. })
            if consumer.index() == 0 && source == base.deferred()[0].source())
        );
    }
    for (first_kind, second_kind) in [(16, 17), (17, 37), (37, 18)] {
        let (mut consumer, provider) = pair();
        consumer.rela = vec![(TARGET, 1, first_kind, 0), (TARGET, 1, second_kind, 0)];
        consumer.plt.clear();
        let consumer = image(&consumer, None);
        let provider = image(&provider, Some((16, 64)));
        let base = base(&[&consumer, &provider]).unwrap();
        assert!(
            matches!(bind(&base), Err(TlsBindingError::Symbolic(SymbolicError::UnsupportedComposition { first, second, .. }))
            if first.index == 0 && second.index == 1)
        );
    }
}

#[test]
fn unaligned_bss_targets_retain_the_checked_consumer_bias() {
    for address in [BASE + 0x1001, BASE + 0x13f8] {
        let (mut consumer, provider) = pair();
        consumer.rela = vec![(address, 1, 17, 0)];
        consumer.plt.clear();
        let consumer = image(&consumer, None);
        let provider = image(&provider, Some((16, 64)));
        let base = base(&[&consumer, &provider]).unwrap();
        let tls = bind(&base).unwrap();
        assert_eq!(tls.bindings()[0].address(), A_BIAS + address);
    }
}

#[test]
fn overlaps_are_rejected_across_tables_and_ordinary_tls_classes() {
    for ordinary in [false, true] {
        for delta in [0, 1, 7] {
            let (mut consumer, provider) = pair();
            consumer.rela = vec![(TARGET, 1, 17, 0)];
            consumer.plt = vec![(
                TARGET + delta,
                if ordinary { 0 } else { 1 },
                if ordinary { 8 } else { 18 },
                0,
            )];
            let consumer = image(&consumer, None);
            let provider = image(&provider, Some((16, 64)));
            let base = base(&[&consumer, &provider]).unwrap();
            assert!(
                matches!(bind(&base), Err(TlsBindingError::Symbolic(SymbolicError::OverlappingPatches { first, second, .. }))
                if first.table != second.table)
            );
        }
    }
}

#[test]
fn ordinary_address_relocations_against_tls_are_still_unsupported() {
    for kind in [1, 6, 7] {
        let (mut consumer, provider) = pair();
        consumer.rela = vec![(TARGET, 1, kind, 0)];
        consumer.plt.clear();
        let consumer = image(&consumer, None);
        let provider = image(&provider, Some((16, 64)));
        assert!(matches!(
            base(&[&consumer, &provider]),
            Err(SymbolicError::UnsupportedSymbol {
                reason: UnsupportedSymbol::Type,
                ..
            })
        ));
    }
}

#[test]
fn aggregate_budgets_charge_before_retention_lookup_and_target_work() {
    let (consumer, provider) = pair();
    let consumer = image(&consumer, None);
    let provider = image(&provider, Some((16, 64)));
    let base = base(&[&consumer, &provider]).unwrap();
    for (kind, limits) in [
        (
            TlsBindingBudget::Modules,
            TlsBindingLimits {
                max_modules: 1,
                ..Default::default()
            },
        ),
        (
            TlsBindingBudget::Records,
            TlsBindingLimits {
                max_records: 2,
                ..Default::default()
            },
        ),
        (
            TlsBindingBudget::Bindings,
            TlsBindingLimits {
                max_bindings: 2,
                ..Default::default()
            },
        ),
        (
            TlsBindingBudget::WorkSteps,
            TlsBindingLimits {
                max_work_steps: 0,
                ..Default::default()
            },
        ),
        (
            TlsBindingBudget::WorkSteps,
            TlsBindingLimits {
                max_work_steps: 20,
                ..Default::default()
            },
        ),
    ] {
        assert!(
            matches!(plan_tls_bindings(&base, &limits), Err(TlsBindingError::Budget { kind: actual, .. }) if actual == kind)
        );
    }
    for (budget, limits) in [
        (
            SymbolicBudget::Exports,
            TlsBindingLimits {
                max_exports: 0,
                ..Default::default()
            },
        ),
        (
            SymbolicBudget::KeyBytes,
            TlsBindingLimits {
                max_key_bytes: 0,
                ..Default::default()
            },
        ),
        (
            SymbolicBudget::LookupSteps,
            TlsBindingLimits {
                max_lookup_steps: 0,
                ..Default::default()
            },
        ),
    ] {
        assert!(
            matches!(plan_tls_bindings(&base, &limits), Err(TlsBindingError::Symbolic(SymbolicError::LimitExceeded { budget: actual, .. })) if actual == budget)
        );
    }
    assert!(bind(&base).is_ok());
    for limits in [
        TlsBindingLimits {
            max_modules: 0,
            ..Default::default()
        },
        TlsBindingLimits {
            max_records: 0,
            ..Default::default()
        },
        TlsBindingLimits {
            max_bindings: 0,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            plan_tls_bindings(&base, &limits),
            Err(TlsBindingError::Budget { .. })
        ));
    }
    assert!(
        plan_tls_bindings(
            &base,
            &TlsBindingLimits {
                max_modules: 2,
                max_records: 3,
                max_bindings: 3,
                max_exports: 1,
                ..Default::default()
            }
        )
        .is_ok()
    );
}

#[test]
fn no_tls_records_preserve_unknowns_without_building_a_provider_index() {
    let (mut consumer, provider) = pair();
    consumer.rela = vec![(TARGET, 0, 37, 0)];
    consumer.plt.clear();
    let consumer = image(&consumer, None);
    let provider = image(&provider, Some((16, 64)));
    let base = base(&[&consumer, &provider]).unwrap();
    let tls = plan_tls_bindings(
        &base,
        &TlsBindingLimits {
            max_bindings: 0,
            max_exports: 0,
            max_key_bytes: 0,
            max_lookup_steps: 0,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(tls.bindings().is_empty());
    assert_eq!(tls.remaining_deferred(), base.deferred());
}

#[test]
fn imported_declaration_pair_never_becomes_a_tls_provider() {
    let (consumer, mut provider) = pair();
    provider.provider = false;
    // A separate export scope enables the bounded provider census, while the
    // definition's own selectors occur wholly in import roles.
    provider.declarations.push(Declaration {
        role: DescriptorRole::ExportLibrary,
        encoding: DescriptorEncoding::Classic,
        name: b"unrelated-export".to_vec(),
        id: 99,
        version: 1,
    });
    let consumer = image(&consumer, None);
    let provider = image(&provider, Some((16, 64)));
    let base = base(&[&consumer, &provider]).unwrap();
    assert_eq!(base.provider_exclusion_count(), 1);
    assert!(matches!(
        resolution_error(bind(&base)),
        SymbolicError::MissingProvider { .. }
    ));
}

#[test]
fn symbol_zero_dtpmod_retains_its_own_template_without_a_symbol_provider() {
    for dynlib in [false, true] {
        let (mut consumer, mut provider) = pair();
        consumer.dynlib = dynlib;
        provider.dynlib = dynlib;
        consumer.rela = vec![(TARGET, 0, 16, 0)];
        consumer.plt.clear();
        let original = image(&consumer, Some((16, 64)));
        let foreign = image(&provider, Some((16, 128)));
        let base = base(&[&original, &foreign]).unwrap();
        let tls = bind(&base).unwrap();
        let binding = tls.bindings()[0];
        assert_eq!(binding.consumer().index(), 0);
        assert_eq!(binding.definition().module(), binding.consumer());
        assert_eq!(binding.definition().symbol(), None);
        assert_eq!(binding.definition().template_mem_size(), 64);
        assert_eq!(
            binding.definition().program_header_index(),
            if dynlib { 3 } else { 2 }
        );
        assert_eq!(binding.source(), base.deferred()[0].source());
        assert_eq!(binding.address(), A_BIAS + TARGET);
        assert_eq!(base.patch_count(), 0);
        assert!(tls.remaining_deferred().is_empty());
        assert_eq!(original, image(&consumer, Some((16, 64))));
    }
}

#[test]
fn symbol_zero_dtpmod_refuses_missing_empty_templates_addends_and_bad_targets() {
    for template in [None, Some((0, 0))] {
        let (mut consumer, provider) = pair();
        consumer.rela = vec![(TARGET, 0, 16, 0)];
        consumer.plt.clear();
        let bytes = image(&consumer, template);
        let foreign = image(&provider, Some((16, 64)));
        let base = base(&[&bytes, &foreign]).unwrap();
        assert!(
            matches!(bind(&base), Err(TlsBindingError::MissingLocalTlsTemplate { consumer, .. }) if consumer.index() == 0)
        );
    }
    for (target, addend) in [(TARGET, 1), (TARGET, -1), (u64::MAX, 0)] {
        let (mut module, _) = pair();
        module.rela = vec![(target, 0, 16, addend)];
        module.plt.clear();
        let bytes = image(&module, Some((16, 64)));
        let base = base(&[&bytes]).unwrap();
        if addend == 0 {
            assert!(matches!(
                bind(&base),
                Err(TlsBindingError::Relocation { .. })
            ));
        } else {
            assert!(matches!(
                bind(&base),
                Err(TlsBindingError::UnsupportedModuleAddend { .. })
            ));
        }
    }
}
