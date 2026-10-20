// SPDX-License-Identifier: GPL-2.0-or-later

//! Typed generic AMD64 TLS identities, independent of runtime TLS layout.

mod error;
mod model;
#[cfg(test)]
mod tests;

pub use error::{TlsBindingBudget, TlsBindingError};
pub use model::{
    TlsBinding, TlsBindingLimits, TlsBindingPlan, TlsBindingPolicy, TlsDefinition, TlsOffset,
    TlsRelocationKind,
};

use super::resolve::{ResolutionLimits, Resolver};
use super::{ModuleId, SymbolRef, SymbolicError, SymbolicLinkPlan};
use crate::relative::target_address;
use crate::{RelocationSource, TableKind};
use error::charge;
use std::collections::{BTreeMap, btree_map::Entry};

/// Bind DTPMOD64/DTPOFF64/TPOFF64 records retained by the
/// exact base. All other deferred records remain explicit and block complete
/// downstream preparation. Ordinary linking behavior is not widened.
///
/// Defined STT_TLS values are offsets, including zero; no image bias is added
/// to a definition. DTPMOD64 with symbol zero selects the consumer's nonempty
/// PT_TLS template (local dynamic model), without resolving a symbol. Other
/// symbol-zero forms and nonzero DTPMOD addends remain unsupported. Runtime IDs
/// and TP arithmetic belong to a
/// later image-bound layout. No memory is written or executable capability created.
pub fn plan_tls_bindings<'plan, 'image>(
    base: &'plan SymbolicLinkPlan<'image>,
    limits: &TlsBindingLimits,
) -> Result<TlsBindingPlan<'plan, 'image>, TlsBindingError> {
    let mut work = 0;
    let mut modules = 0;
    charge(
        &mut modules,
        base.modules().len() as u64,
        limits.max_modules,
        TlsBindingBudget::Modules,
    )?;
    step(&mut work, modules, limits)?;
    let mut records = 0;
    let mut symbols = 0_u64;
    for module in base.modules() {
        if let Some(metadata) = &module.metadata {
            for table in [metadata.link().rela(), metadata.link().plt_rela()] {
                charge(
                    &mut records,
                    table.len() as u64,
                    limits.max_records,
                    TlsBindingBudget::Records,
                )?;
            }
            symbols = symbols
                .checked_add(metadata.link().symbols().len() as u64)
                .ok_or(TlsBindingError::Budget {
                    kind: TlsBindingBudget::WorkSteps,
                    limit: limits.max_work_steps,
                })?;
        }
    }
    // Composition visits and deferred partition visits are separate work.
    step(&mut work, records, limits)?;
    step(&mut work, base.deferred().len() as u64, limits)?;
    let mut requested = 0;
    for deferred in base.deferred() {
        if TlsRelocationKind::from_code(deferred.source().record.kind()).is_some() {
            charge(
                &mut requested,
                1,
                limits.max_bindings,
                TlsBindingBudget::Bindings,
            )?;
        }
    }
    // Charge the second partition traversal before any output retention.
    step(&mut work, base.deferred().len() as u64, limits)?;
    step(&mut work, requested, limits)?;
    if requested == 0 {
        return Ok(TlsBindingPlan {
            base,
            bindings: Vec::new(),
            remaining: base.deferred().to_vec(),
            _limits: *limits,
        });
    }
    step(&mut work, modules, limits)?;
    reject_composition(base)?;
    // The shared index scans retained symbols; its key/candidate operations
    // separately use the existing bounded resolver accounting.
    step(&mut work, modules, limits)?;
    step(&mut work, symbols, limits)?;
    let mut resolver = Resolver::with_retired(
        &base.modules,
        ResolutionLimits::from(limits),
        base.retired_fallbacks(),
    )
    .map_err(TlsBindingError::Symbolic)?;
    let mut ordered = BTreeMap::new();
    let mut remaining = Vec::new();
    for (deferred_index, deferred) in base.deferred().iter().enumerate() {
        let source = deferred.source();
        let consumer = deferred.module();
        let Some(kind) = TlsRelocationKind::from_code(source.record.kind()) else {
            remaining.push(*deferred);
            continue;
        };
        if source.record.symbol_index() == 0 && kind != TlsRelocationKind::DtpMod64 {
            return Err(TlsBindingError::UnsupportedSymbolZero { consumer, source });
        }
        if kind == TlsRelocationKind::DtpMod64 && source.record.addend != 0 {
            return Err(TlsBindingError::UnsupportedModuleAddend { consumer, source });
        }
        let module = &base.modules[consumer.index()];
        step(&mut work, module.ranges.len() as u64, limits)?;
        let address =
            target_address(source, &module.ranges, module.load_bias).map_err(|error| {
                TlsBindingError::Relocation {
                    consumer,
                    source,
                    error: Box::new(error),
                }
            })?;
        let definition = if source.record.symbol_index() == 0 {
            // Local dynamic TLS identifies this module, not the null symbol entry.
            let tls = module
                .runtime
                .tls
                .as_ref()
                .filter(|tls| tls.mem_size != 0)
                .ok_or(TlsBindingError::MissingLocalTlsTemplate { consumer, source })?;
            step(&mut work, 1, limits)?;
            TlsDefinition {
                module: consumer,
                symbol: None,
                offset: TlsOffset(0),
                size: 0,
                program_header_index: tls.initialized.program_header_index,
                template_mem_size: tls.mem_size,
            }
        } else {
            let definition = resolver
                .resolve_tls(SymbolRef {
                    module: consumer,
                    symbol: source.record.symbol_index() as usize,
                })
                .map_err(|error| TlsBindingError::Resolution {
                    consumer,
                    source,
                    error: Box::new(error),
                })?;
            step(&mut work, 1, limits)?;
            let provider = &base.modules[definition.module.index()];
            let symbol = &provider
                .metadata
                .as_ref()
                .expect("resolved retained metadata")
                .link()
                .symbols()[definition.symbol];
            let tls = provider
                .runtime
                .tls
                .as_ref()
                .ok_or(TlsBindingError::MissingTlsTemplate {
                    consumer,
                    source,
                    definition,
                })?;
            if symbol.value >= tls.mem_size
                || symbol
                    .value
                    .checked_add(symbol.size)
                    .is_none_or(|end| end > tls.mem_size)
            {
                return Err(TlsBindingError::TlsSymbolRange {
                    consumer,
                    source,
                    definition,
                });
            }
            TlsDefinition {
                module: definition.module,
                symbol: Some(definition),
                offset: TlsOffset(symbol.value),
                size: symbol.size,
                program_header_index: tls.initialized.program_header_index,
                template_mem_size: tls.mem_size,
            }
        };
        let binding = TlsBinding {
            deferred_index,
            consumer,
            source,
            address,
            kind,
            definition,
        };
        step(&mut work, 1, limits)?;
        match ordered.entry(address) {
            Entry::Vacant(entry) => {
                entry.insert(binding);
            }
            Entry::Occupied(entry) => return Err(overlap(*entry.get(), binding)),
        }
    }
    // One collection visit and one combined merge visit per retained binding.
    step(&mut work, requested, limits)?;
    step(&mut work, requested, limits)?;
    step(&mut work, base.patch_count() as u64, limits)?;
    let bindings: Vec<_> = ordered.into_values().collect();
    reject_overlaps(base, &bindings)?;
    Ok(TlsBindingPlan {
        base,
        bindings,
        remaining,
        _limits: *limits,
    })
}

fn step(work: &mut u64, amount: u64, limits: &TlsBindingLimits) -> Result<(), TlsBindingError> {
    charge(
        work,
        amount,
        limits.max_work_steps,
        TlsBindingBudget::WorkSteps,
    )
}

fn reject_composition(base: &SymbolicLinkPlan<'_>) -> Result<(), TlsBindingError> {
    for module in base.modules() {
        let Some(metadata) = &module.metadata else {
            continue;
        };
        for (table, records) in [
            (TableKind::Rela, metadata.link().rela()),
            (TableKind::PltRela, metadata.link().plt_rela()),
        ] {
            for (index, pair) in records.windows(2).enumerate() {
                if pair[0].offset == pair[1].offset
                    && (TlsRelocationKind::from_code(pair[0].kind()).is_some()
                        || TlsRelocationKind::from_code(pair[1].kind()).is_some())
                {
                    return Err(TlsBindingError::Symbolic(
                        SymbolicError::UnsupportedComposition {
                            module: module.id,
                            first: RelocationSource {
                                table,
                                index,
                                record: pair[0],
                            },
                            second: RelocationSource {
                                table,
                                index: index + 1,
                                record: pair[1],
                            },
                        },
                    ));
                }
            }
        }
    }
    Ok(())
}

fn overlap(first: TlsBinding, second: TlsBinding) -> TlsBindingError {
    TlsBindingError::Symbolic(SymbolicError::OverlappingPatches {
        first_module: first.consumer,
        first: first.source,
        second_module: second.consumer,
        second: second.source,
        address: second.address,
    })
}

fn reject_overlaps(
    base: &SymbolicLinkPlan<'_>,
    bindings: &[TlsBinding],
) -> Result<(), TlsBindingError> {
    let mut ordinary = base.patches().iter().peekable();
    let mut tls = bindings.iter().peekable();
    let mut previous: Option<(ModuleId, RelocationSource, u64)> = None;
    while ordinary.peek().is_some() || tls.peek().is_some() {
        let take_ordinary = match (ordinary.peek(), tls.peek()) {
            (Some(a), Some(b)) => a.address() <= b.address,
            (Some(_), None) => true,
            _ => false,
        };
        let current = if take_ordinary {
            let patch = ordinary.next().expect("peeked ordinary patch");
            (patch.consumer(), patch.source(), patch.address())
        } else {
            let binding = tls.next().expect("peeked TLS binding");
            (binding.consumer, binding.source, binding.address)
        };
        if let Some(first) = previous {
            // Both planners already proved complete target ends fit u64.
            if current.2 < first.2 + 8 {
                return Err(TlsBindingError::Symbolic(
                    SymbolicError::OverlappingPatches {
                        first_module: first.0,
                        first: first.1,
                        second_module: current.0,
                        second: current.1,
                        address: current.2,
                    },
                ));
            }
        }
        previous = Some(current);
    }
    Ok(())
}
