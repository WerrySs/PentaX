// SPDX-License-Identifier: GPL-2.0-or-later

//! Whole-session LP64 AMD64 relocation planning under the explicit
//! ExactDeclarationV1 policy: byte-exact module/library names and exact versions,
//! image-local role-scoped selectors, canonical NIDs and supported symbol types.
//! This policy is a bounded PentaX restriction, not a claim about undocumented
//! SCE version compatibility or general ELF interposition. Ordinary address
//! resolution admits a unique weak definition behind one strong definition;
//! multiple weak-only candidates remain ambiguous. Missing weak address values
//! require the explicit NullAddressV1 policy; TLS retains its separate policy.
//! Each role may use its classic or observed `_1` encoding. Resolution compares
//! decoded declarations across encodings; same-role conflicts are rejected.
//! Opaque filename records count toward retained records without reading names.
//! GLOBAL/WEAK DEFAULT/PROTECTED definitions use only owned export declarations.
//! Definitions whose selectors occur wholly in import roles are not offered;
//! bounded image/symbol provenance records this provider-only exclusion.

mod error;
mod extension;
mod missing;
mod model;
mod module_lookup;
mod name_nid;
mod preflight;
mod resolve;
mod tls;

pub use error::{MissingFunctionReason, SymbolicBudget, SymbolicError, UnsupportedSymbol};
pub use extension::{
    FallbackPolicy, SymbolicExtensionError, SymbolicExtensionLimits, append_symbolic_module,
    append_symbolic_module_with_fallbacks, append_symbolic_modules_with_fallbacks,
};
pub use missing::{
    MissingFunctionImport, MissingFunctionPlan, plan_missing_functions,
    plan_missing_functions_for_extension,
};
pub use model::{
    DescriptorVersions, FallbackTransition, MissingWeakPolicy, ModuleId, ModuleInput,
    PlannedModule, ProviderExclusion, ProviderExclusionReason, Resolution,
    SessionDeferredRelocation, SourceIdentity, SymbolRef, SymbolicLimits, SymbolicLinkPlan,
    SymbolicPatch,
};
pub use module_lookup::{
    ModuleExport, ModuleExportIdentity, ModuleExportIdentityAttempt, ModuleExportQuery,
    ModuleLookupAttempt, ModuleLookupError, ModuleLookupLimits, lookup_module_export,
    lookup_qualified_module_export,
};
pub use tls::{
    TlsBinding, TlsBindingBudget, TlsBindingError, TlsBindingLimits, TlsBindingPlan,
    TlsBindingPolicy, TlsDefinition, TlsOffset, TlsRelocationKind, plan_tls_bindings,
};

use crate::relative::{relative_patch, target_address};
use crate::{Relocation, RelocationSource, TableKind};
use error::charge;

fn supported(kind: u32) -> bool {
    matches!(kind, 1 | 6 | 7 | 8)
}

/// Inspect all images and return an immutable complete session plan.
///
/// Input zero is primary; its entry must start in its own executable logical
/// mapping. Other images may have entry zero. All biased mappings must fit and
/// different modules must own disjoint 16 KiB pages. Original RELRO metadata is
/// retained; a staging consumer may impose a narrower protection policy.
///
/// R_X86_64_64 writes low64(S+A); GLOB_DAT and JUMP_SLOT write S, ignoring A;
/// RELATIVE writes low64(B+A). Target/bias arithmetic is checked, data arithmetic
/// wraps. A nonzero undefined symbol must resolve uniquely under the exact
/// policy. Missing weak remains an explicit error unless NullAddressV1 is
/// selected; this produces a typed S=0 value and never a callable substitute.
/// GLOBAL DEFAULT/PROTECTED providers require their image's own module-info
/// and export-library pair. Only the exact absence of both owned roles plus
/// presence of both import roles excludes a scanned definition; partial pairs
/// remain errors. Exclusions do not change defined-symbol resolution.
/// Index zero supplies S=0 directly. All other relocation kinds are retained
/// as deferred records. No partial result, memory write or execution occurs.
///
/// # Errors
/// Returns typed inspection, budget, scope, provider, unsupported symbol,
/// mapping, composition or overlapping-write diagnostics.
pub fn plan_symbolic<'a>(
    inputs: &[ModuleInput<'a>],
    limits: &SymbolicLimits,
) -> Result<SymbolicLinkPlan<'a>, SymbolicError> {
    plan_symbolic_in(inputs, limits, &[], &[], None)
}

fn plan_symbolic_in<'a>(
    inputs: &[ModuleInput<'a>],
    limits: &SymbolicLimits,
    retired: &[SymbolRef],
    approved: &[SymbolRef],
    appended_guest: Option<ModuleId>,
) -> Result<SymbolicLinkPlan<'a>, SymbolicError> {
    let modules = preflight::prepare(inputs, limits)?;
    let primary_entry = preflight::primary_entry(&modules[0])?;
    let (mut resolver, retired_fallbacks) = resolution_state(
        &modules,
        limits,
        retired,
        approved,
        appended_guest.map_or(modules.len() - 1, ModuleId::index),
    )?;
    let mut patches = Vec::new();
    let mut deferred = Vec::new();
    let mut patch_count = 0;
    for module in &modules {
        let Some(metadata) = &module.metadata else {
            continue;
        };
        for (table, records) in [
            (TableKind::Rela, metadata.link().rela()),
            (TableKind::PltRela, metadata.link().plt_rela()),
        ] {
            reject_composition(module.id, table, records)?;
            for (index, &record) in records.iter().enumerate() {
                let source = RelocationSource {
                    table,
                    index,
                    record,
                };
                if record.kind() == 0 {
                    continue;
                }
                if !supported(record.kind()) {
                    deferred.push(SessionDeferredRelocation {
                        module: module.id,
                        source,
                    });
                    continue;
                }
                charge(
                    &mut patch_count,
                    1,
                    limits.max_patches,
                    SymbolicBudget::Patches,
                )?;
                let (address, value, resolution) = if record.kind() == 8 {
                    let patch = relative_patch(source, &module.ranges, module.load_bias).map_err(
                        |error| SymbolicError::Relocation {
                            module: module.id,
                            error,
                        },
                    )?;
                    (patch.address, patch.value, Resolution::Relative)
                } else {
                    let address = target_address(source, &module.ranges, module.load_bias)
                        .map_err(|error| SymbolicError::Relocation {
                            module: module.id,
                            error,
                        })?;
                    let (symbol, resolution) = if record.symbol_index() == 0 {
                        (0, Resolution::ZeroSymbolIndex)
                    } else {
                        resolver.resolve(
                            SymbolRef {
                                module: module.id,
                                symbol: record.symbol_index() as usize,
                            },
                            record.kind() == 7,
                        )?
                    };
                    let value = if record.kind() == 1 {
                        symbol.wrapping_add_signed(record.addend)
                    } else {
                        symbol
                    };
                    (address, value, resolution)
                };
                patches.push(SymbolicPatch {
                    consumer: module.id,
                    address,
                    value,
                    source,
                    resolution,
                });
            }
        }
    }
    patches.sort_unstable_by_key(SymbolicPatch::address);
    for pair in patches.windows(2) {
        if pair[1].address < pair[0].address + 8 {
            return Err(SymbolicError::OverlappingPatches {
                first_module: pair[0].consumer,
                first: pair[0].source,
                second_module: pair[1].consumer,
                second: pair[1].source,
                address: pair[1].address,
            });
        }
    }
    let provider_exclusions = resolver.into_exclusions();
    Ok(SymbolicLinkPlan {
        identity: SourceIdentity::next()?,
        parent: None,
        appended_guest,
        modules,
        patches,
        deferred,
        provider_exclusions,
        primary_entry,
        retired_fallbacks,
        fallback_transitions: Vec::new(),
    })
}

/// Shared resolver construction for final planning and missing-call discovery.
/// The explicit guest can precede a generated delta; it is never inferred from
/// the last input. The caller supplies bounded, source-checked policy records.
fn resolution_state<'m, 'a>(
    modules: &'m [PlannedModule<'a>],
    limits: &SymbolicLimits,
    retired: &[SymbolRef],
    approved: &[SymbolRef],
    guest: usize,
) -> Result<(resolve::Resolver<'m, 'a>, Vec<SymbolRef>), SymbolicError> {
    let capacity =
        retired
            .len()
            .checked_add(approved.len())
            .ok_or(SymbolicError::SizeOverflow {
                budget: SymbolicBudget::Exports,
            })?;
    let mut count = 0;
    charge(
        &mut count,
        capacity as u64,
        limits.max_exports,
        SymbolicBudget::Exports,
    )?;
    let mut retired_fallbacks = Vec::new();
    retired_fallbacks
        .try_reserve_exact(capacity)
        .map_err(|_| SymbolicError::Allocation)?;
    let mut resolver = resolve::Resolver::with_retired(modules, limits.into(), retired)?;
    resolver.steps(retired.len() as u64)?;
    retired_fallbacks.extend_from_slice(retired);
    for &fallback in approved {
        if resolver.retire_for_added(fallback, guest)? {
            retired_fallbacks.push(fallback);
        }
    }
    if !approved.is_empty() {
        let sort_work = (capacity as u64)
            .checked_mul(u64::from(usize::BITS - capacity.leading_zeros()) + 1)
            .ok_or(SymbolicError::SizeOverflow {
                budget: SymbolicBudget::LookupSteps,
            })?;
        resolver.steps(sort_work)?;
        retired_fallbacks.sort_unstable_by_key(|symbol| (symbol.module.index(), symbol.symbol));
        retired_fallbacks.dedup();
    }
    Ok((resolver, retired_fallbacks))
}

fn reject_composition(
    module: ModuleId,
    table: TableKind,
    records: &[Relocation],
) -> Result<(), SymbolicError> {
    for (index, pair) in records.windows(2).enumerate() {
        if pair[0].offset == pair[1].offset
            && (supported(pair[0].kind()) || supported(pair[1].kind()))
        {
            return Err(SymbolicError::UnsupportedComposition {
                module,
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
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "symbolic/tests/fixture.rs"]
mod fixture;
