// SPDX-License-Identifier: GPL-2.0-only

//! Append immutable source while retaining all existing bindings and indices.
use super::{
    FallbackTransition, ModuleId, ModuleInput, Resolution, SourceIdentity, SymbolRef,
    SymbolicError, SymbolicLimits, SymbolicLinkPlan, SymbolicPatch, plan_symbolic_in,
};
use pentax_loader::open_executable;
use std::fmt;

#[derive(Debug, Clone, Copy)]
pub struct SymbolicExtensionLimits {
    pub symbolic: SymbolicLimits,
    /// Conservative input/record bytes admitted before predecessor inspection.
    /// Planning remains independently bounded by the complete symbolic policy.
    pub max_preservation_work: u64,
}
impl Default for SymbolicExtensionLimits {
    fn default() -> Self {
        Self {
            symbolic: Default::default(),
            max_preservation_work: 1024 * 1024 * 1024,
        }
    }
}
#[derive(Debug)]
pub enum SymbolicExtensionError {
    Symbolic(SymbolicError),
    Input,
    ChangedPredecessor,
    FallbackSource,
    InvalidFallback,
    Budget,
    Allocation,
}
impl fmt::Display for SymbolicExtensionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("symbolic source extension refused")
    }
}
impl std::error::Error for SymbolicExtensionError {}

/// Source-bound metadata supplied by a caller which authenticates generated
/// provenance separately. This type does not prove a provider was generated,
/// authorize native writes or establish live memory/source identity.
#[derive(Debug, Clone)]
pub struct FallbackPolicy {
    source: SourceIdentity,
    provider: ModuleId,
    symbols: Vec<SymbolRef>,
}

impl FallbackPolicy {
    pub fn new(
        base: &SymbolicLinkPlan<'_>,
        provider: ModuleId,
        provider_source: &[u8],
        allowed: &[SymbolRef],
        limits: &SymbolicExtensionLimits,
    ) -> Result<Self, SymbolicExtensionError> {
        let module = base
            .modules()
            .get(provider.index())
            .ok_or(SymbolicExtensionError::FallbackSource)?;
        if !std::ptr::eq(module.source_bytes(), provider_source) {
            return Err(SymbolicExtensionError::FallbackSource);
        }
        let work = (allowed.len() as u64)
            .checked_mul(128)
            .filter(|work| *work <= limits.max_preservation_work)
            .ok_or(SymbolicExtensionError::Budget)?;
        if allowed.len() as u64 > limits.symbolic.max_exports || work > limits.max_preservation_work
        {
            return Err(SymbolicExtensionError::Budget);
        }
        let mut resolver =
            super::resolve::Resolver::empty(base.modules(), (&limits.symbolic).into(), true);
        let mut symbols = Vec::new();
        symbols
            .try_reserve_exact(allowed.len())
            .map_err(|_| SymbolicExtensionError::Allocation)?;
        for &reference in allowed {
            if reference.module != provider
                || reference.symbol == 0
                || module
                    .metadata
                    .as_ref()
                    .is_none_or(|metadata| reference.symbol >= metadata.link().symbols().len())
                || contains(base.retired_fallbacks(), reference)
            {
                return Err(SymbolicExtensionError::InvalidFallback);
            }
            resolver
                .validate_fallback(reference)
                .map_err(SymbolicExtensionError::Symbolic)?;
            symbols.push(reference);
        }
        symbols.sort_unstable_by_key(|reference| reference.symbol);
        if symbols.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(SymbolicExtensionError::InvalidFallback);
        }
        Ok(Self {
            source: base.identity(),
            provider,
            symbols,
        })
    }
    #[must_use]
    pub const fn source_identity(&self) -> SourceIdentity {
        self.source
    }
    #[must_use]
    pub const fn provider(&self) -> ModuleId {
        self.provider
    }
    #[must_use]
    pub fn symbols(&self) -> &[SymbolRef] {
        &self.symbols
    }
}

/// Plan one additional input after the retained module set. Every predecessor
/// input/bias, owned descriptor analysis, import patch, deferred record and
/// provider exclusion must remain identical. A newly ambiguous or changed old
/// binding refuses rather than silently rebinding existing live memory.
///
/// This allocates metadata only: no mapping, file I/O, relocation publication,
/// constructor call or lifecycle transition occurs. Its private parent identity
/// names the immutable plan it extends, not an applied native epoch.
pub fn append_symbolic_module<'image>(
    base: &SymbolicLinkPlan<'image>,
    added: ModuleInput<'image>,
    limits: &SymbolicExtensionLimits,
) -> Result<SymbolicLinkPlan<'image>, SymbolicExtensionError> {
    append_symbolic_modules_with_fallbacks(base, &[added], &[], limits)
}

/// Permit only exact approved function fallback transitions. The policy is
/// metadata, not generated-origin or native-publication evidence. All other
/// predecessor invariants remain the strict append contract.
///
/// Only an authenticated GLOBAL FUNC placeholder may be retired. Real GLOBAL
/// definitions outrank WEAK definitions; a unique WEAK FUNC may supply the
/// replacement without changing its declared binding. The winning real
/// definition must belong to the added module, and winning-tier duplicates
/// remain ambiguous. Retirement never authorizes rebinding a real provider.
pub fn append_symbolic_module_with_fallbacks<'image>(
    base: &SymbolicLinkPlan<'image>,
    added: ModuleInput<'image>,
    policy: &FallbackPolicy,
    limits: &SymbolicExtensionLimits,
) -> Result<SymbolicLinkPlan<'image>, SymbolicExtensionError> {
    append_symbolic_modules_with_fallbacks(base, &[added], std::slice::from_ref(policy), limits)
}

/// Atomically plan one guest source and an optional following delta provider.
/// The first added source is the only real replacement target. Every policy
/// must name this exact base and a distinct retained provider. Generated origin
/// and the delta's lack of execution/lifecycle authority remain caller checks.
/// No intermediate plan or parent epoch is published.
pub fn append_symbolic_modules_with_fallbacks<'image>(
    base: &SymbolicLinkPlan<'image>,
    added: &[ModuleInput<'image>],
    policies: &[FallbackPolicy],
    limits: &SymbolicExtensionLimits,
) -> Result<SymbolicLinkPlan<'image>, SymbolicExtensionError> {
    let prepared = prepare_inputs(base, added, policies, limits)?;
    let mut next = plan_symbolic_in(
        &prepared.inputs,
        &limits.symbolic,
        base.retired_fallbacks(),
        &prepared.approved,
        Some(ModuleId(base.modules().len())),
    )
    .map_err(SymbolicExtensionError::Symbolic)?;
    if !preserves_structure(base, &next) {
        return Err(SymbolicExtensionError::ChangedPredecessor);
    }
    if prepared.approved.is_empty() {
        if !preserves(base, &next) {
            return Err(SymbolicExtensionError::ChangedPredecessor);
        }
    } else {
        next.fallback_transitions = transitions(base, &next, &prepared.approved)?;
    }
    next.parent = Some(base.identity());
    Ok(next)
}

pub(super) struct ExtensionInputs<'image> {
    pub(super) inputs: Vec<ModuleInput<'image>>,
    pub(super) approved: Vec<SymbolRef>,
}

/// Shared admission and bounded source/policy retention before either query.
pub(super) fn prepare_inputs<'image>(
    base: &SymbolicLinkPlan<'image>,
    added: &[ModuleInput<'image>],
    policies: &[FallbackPolicy],
    limits: &SymbolicExtensionLimits,
) -> Result<ExtensionInputs<'image>, SymbolicExtensionError> {
    if !(1..=2).contains(&added.len()) {
        return Err(SymbolicExtensionError::Input);
    }
    let count = base
        .modules()
        .len()
        .checked_add(added.len())
        .ok_or(SymbolicExtensionError::Budget)?;
    if count as u64 > limits.symbolic.max_modules || policies.len() > base.modules().len() {
        return Err(SymbolicExtensionError::Budget);
    }
    let mut work = base
        .modules()
        .iter()
        .try_fold(count as u64, |sum, module| {
            sum.checked_add(module.source_bytes().len() as u64)
        })
        .and_then(|sum| sum.checked_add((base.patch_count() as u64).checked_mul(32)?))
        .and_then(|sum| sum.checked_add((base.deferred_count() as u64).checked_mul(32)?))
        .and_then(|sum| sum.checked_add((base.provider_exclusion_count() as u64).checked_mul(32)?))
        .ok_or(SymbolicExtensionError::Budget)?;
    let policy_work = (policies.len() as u64)
        .checked_mul(128)
        .ok_or(SymbolicExtensionError::Budget)?;
    work = work
        .checked_add(policy_work)
        .filter(|value| *value <= limits.max_preservation_work)
        .ok_or(SymbolicExtensionError::Budget)?;
    let mut approved_count = 0_usize;
    for policy in policies {
        if policy.source != base.identity() {
            return Err(SymbolicExtensionError::FallbackSource);
        }
        approved_count = approved_count
            .checked_add(policy.symbols.len())
            .ok_or(SymbolicExtensionError::Budget)?;
    }
    let retirement_capacity = base
        .retired_fallbacks()
        .len()
        .checked_add(approved_count)
        .ok_or(SymbolicExtensionError::Budget)?;
    if retirement_capacity as u64 > limits.symbolic.max_exports {
        return Err(SymbolicExtensionError::Budget);
    }
    if !policies.is_empty() {
        // Each changed patch checks approved, newly retired and previously
        // retired sets independently. Charge all three bounded searches before
        // any transition scan, plus fixed record/type/provenance work.
        let searches = search_work(approved_count)
            .checked_add(search_work(retirement_capacity))
            .and_then(|value| value.checked_add(search_work(base.retired_fallbacks().len())))
            .and_then(|value| value.checked_add(64))
            .ok_or(SymbolicExtensionError::Budget)?;
        let transitions = (base.patch_count() as u64)
            .checked_mul(std::mem::size_of::<FallbackTransition>() as u64 + searches)
            .and_then(|value| value.checked_add((approved_count as u64).checked_mul(128)?))
            .ok_or(SymbolicExtensionError::Budget)?;
        work = work
            .checked_add(transitions)
            .ok_or(SymbolicExtensionError::Budget)?;
    }
    if work > limits.max_preservation_work {
        return Err(SymbolicExtensionError::Budget);
    }
    for module in base.modules() {
        if let Some(metadata) = &module.metadata {
            let link = metadata.link();
            let visits = (link.symbols().len() as u64)
                .checked_mul(2)
                .and_then(|value| value.checked_add(link.rela().len() as u64))
                .and_then(|value| value.checked_add(link.plt_rela().len() as u64))
                .ok_or(SymbolicExtensionError::Budget)?;
            work = work
                .checked_add(visits)
                .filter(|value| *value <= limits.max_preservation_work)
                .ok_or(SymbolicExtensionError::Budget)?;
            let names = link
                .counts()
                .name_bytes
                .checked_add(metadata.metadata_name_bytes())
                .and_then(|value| value.checked_add(metadata.table_bytes()))
                .ok_or(SymbolicExtensionError::Budget)?;
            work = work
                .checked_add(names)
                .filter(|value| *value <= limits.max_preservation_work)
                .ok_or(SymbolicExtensionError::Budget)?;
        }
    }
    for input in added {
        work = work
            .checked_add(input.view.source_bytes().len() as u64)
            .filter(|value| *value <= limits.max_preservation_work)
            .ok_or(SymbolicExtensionError::Budget)?;
    }
    let mut providers = Vec::new();
    providers
        .try_reserve_exact(policies.len())
        .map_err(|_| SymbolicExtensionError::Allocation)?;
    let mut approved = Vec::new();
    approved
        .try_reserve_exact(approved_count)
        .map_err(|_| SymbolicExtensionError::Allocation)?;
    for policy in policies {
        providers.push(policy.provider);
        approved.extend_from_slice(&policy.symbols);
    }
    providers.sort_unstable();
    approved.sort_unstable_by_key(|symbol| (symbol.module.index(), symbol.symbol));
    if providers.windows(2).any(|pair| pair[0] == pair[1])
        || approved.windows(2).any(|pair| pair[0] == pair[1])
    {
        return Err(SymbolicExtensionError::InvalidFallback);
    }
    let mut inputs = Vec::new();
    inputs
        .try_reserve_exact(count)
        .map_err(|_| SymbolicExtensionError::Allocation)?;
    for module in base.modules() {
        let view =
            open_executable(module.source_bytes()).map_err(|_| SymbolicExtensionError::Input)?;
        inputs.push(ModuleInput::new(view, module.load_bias()));
    }
    for input in added {
        let view = open_executable(input.view.source_bytes())
            .map_err(|_| SymbolicExtensionError::Input)?;
        inputs.push(ModuleInput::new(view, input.load_bias));
    }
    Ok(ExtensionInputs { inputs, approved })
}

/// Immutable history only; callers still need actual publication/liveness.
pub(super) fn preserves(base: &SymbolicLinkPlan<'_>, next: &SymbolicLinkPlan<'_>) -> bool {
    preserves_structure(base, next)
        && base.retired_fallbacks == next.retired_fallbacks
        && base.patches.iter().eq(next
            .patches
            .iter()
            .filter(|patch| patch.consumer.index() < base.modules.len()))
}

fn preserves_structure(base: &SymbolicLinkPlan<'_>, next: &SymbolicLinkPlan<'_>) -> bool {
    if !(1..=2).contains(&next.modules.len().saturating_sub(base.modules.len()))
        || next.appended_guest != Some(ModuleId(base.modules.len()))
        || base.primary_entry != next.primary_entry
    {
        return false;
    }
    // Equal original slices are pointer/extent identities, avoiding a second
    // complete file comparison. Both plans derive runtime mappings from those
    // same immutable inputs; owned descriptor interpretation is checked too.
    for (old, retained) in base.modules.iter().zip(&next.modules) {
        if old.id != retained.id
            || old.load_bias != retained.load_bias
            || !std::ptr::eq(old.source_bytes(), retained.source_bytes())
            || old.runtime.metadata != retained.runtime.metadata
            || old.metadata != retained.metadata
        {
            return false;
        }
    }
    let old_count = base.modules.len();
    base.deferred.iter().eq(next
        .deferred
        .iter()
        .filter(|record| record.module.index() < old_count))
        && base.provider_exclusions.iter().eq(next
            .provider_exclusions
            .iter()
            .filter(|record| record.symbol().module.index() < old_count))
}

fn transitions(
    base: &SymbolicLinkPlan<'_>,
    next: &SymbolicLinkPlan<'_>,
    approved: &[SymbolRef],
) -> Result<Vec<FallbackTransition>, SymbolicExtensionError> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(base.patch_count())
        .map_err(|_| SymbolicExtensionError::Allocation)?;
    let mut retained = next
        .patches()
        .iter()
        .filter(|patch| patch.consumer().index() < base.modules().len());
    for &old in base.patches() {
        let &new = retained
            .next()
            .ok_or(SymbolicExtensionError::ChangedPredecessor)?;
        if old == new {
            continue;
        }
        let (old_provider, new_provider) =
            transition_providers(base, old, new, approved, next.retired_fallbacks())?;
        result.push(FallbackTransition {
            parent: base.identity(),
            next: next.identity(),
            old,
            new,
            old_provider,
            new_provider,
        });
    }
    if retained.next().is_some() {
        return Err(SymbolicExtensionError::ChangedPredecessor);
    }
    Ok(result)
}

fn transition_providers(
    base: &SymbolicLinkPlan<'_>,
    old: SymbolicPatch,
    new: SymbolicPatch,
    approved: &[SymbolRef],
    retired: &[SymbolRef],
) -> Result<(SymbolRef, SymbolRef), SymbolicExtensionError> {
    if old.consumer() != new.consumer()
        || old.address() != new.address()
        || old.source() != new.source()
    {
        return Err(SymbolicExtensionError::ChangedPredecessor);
    }
    let (Resolution::Definition(old_provider), Resolution::Definition(new_provider)) =
        (old.resolution(), new.resolution())
    else {
        return Err(SymbolicExtensionError::ChangedPredecessor);
    };
    if !contains(approved, old_provider)
        || new_provider.module.index() != base.modules().len()
        || !contains(retired, old_provider)
        || contains(base.retired_fallbacks(), old_provider)
    {
        return Err(SymbolicExtensionError::ChangedPredecessor);
    }
    let record = old.source().record;
    if !matches!(record.kind(), 1 | 6 | 7) || (record.kind() == 1 && record.addend != 0) {
        return Err(SymbolicExtensionError::ChangedPredecessor);
    }
    let symbol = base.modules()[old.consumer().index()]
        .metadata
        .as_ref()
        .and_then(|metadata| {
            metadata
                .link()
                .symbols()
                .get(record.symbol_index() as usize)
        })
        .ok_or(SymbolicExtensionError::ChangedPredecessor)?;
    if !symbol.is_undefined()
        || symbol.binding() != crate::SymbolBinding::GLOBAL
        || symbol.symbol_type() != crate::SymbolType::FUNC
    {
        return Err(SymbolicExtensionError::ChangedPredecessor);
    }
    Ok((old_provider, new_provider))
}

/// Reject unrelated retained changes before a missing-call query can propose
/// a delta. The complete final append still validates all new relocations.
pub(super) fn validate_retained_bindings(
    base: &SymbolicLinkPlan<'_>,
    resolver: &mut super::resolve::Resolver<'_, '_>,
    approved: &[SymbolRef],
    retired: &[SymbolRef],
) -> Result<(), SymbolicExtensionError> {
    resolver
        .steps(base.patch_count() as u64)
        .map_err(SymbolicExtensionError::Symbolic)?;
    for &old in base.patches() {
        let record = old.source().record;
        if record.kind() == 8 {
            continue;
        }
        let (value, resolution) = if record.symbol_index() == 0 {
            (0, Resolution::ZeroSymbolIndex)
        } else {
            resolver
                .resolve(
                    SymbolRef {
                        module: old.consumer(),
                        symbol: record.symbol_index() as usize,
                    },
                    record.kind() == 7,
                )
                .map_err(SymbolicExtensionError::Symbolic)?
        };
        let new = SymbolicPatch {
            value: if record.kind() == 1 {
                value.wrapping_add_signed(record.addend)
            } else {
                value
            },
            resolution,
            ..old
        };
        if old != new {
            transition_providers(base, old, new, approved, retired)?;
        }
    }
    Ok(())
}

fn contains(symbols: &[SymbolRef], reference: SymbolRef) -> bool {
    symbols
        .binary_search_by_key(&(reference.module.index(), reference.symbol), |symbol| {
            (symbol.module.index(), symbol.symbol)
        })
        .is_ok()
}

fn search_work(count: usize) -> u64 {
    u64::from(usize::BITS - count.leading_zeros()) + 1
}
