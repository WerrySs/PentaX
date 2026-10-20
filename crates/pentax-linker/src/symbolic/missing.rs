// SPDX-License-Identifier: GPL-2.0-or-later

//! Pure discovery of absent callable imports using the existing exact resolver.
//! This returns no address, patch, complete relocation plan or execution right.

use super::error::charge;
use super::resolve::{Key, Resolver};
use super::{
    DescriptorVersions, FallbackPolicy, MissingFunctionReason, ModuleInput, PlannedModule,
    SymbolRef, SymbolicBudget, SymbolicError, SymbolicExtensionError, SymbolicExtensionLimits,
    SymbolicLimits, SymbolicLinkPlan, preflight,
};
use crate::{NidToken, TableKind};
use std::collections::{BTreeMap, btree_map::Entry};
use std::fmt;

/// One byte-exact module/library/version/NID identity, absent from the supplied
/// providers and used by a supported ordinary function-address relocation.
/// Only undefined GLOBAL FUNC imports qualify. No implementation is claimed.
pub struct MissingFunctionImport {
    key: Key,
    first: SymbolRef,
    references: u64,
}
impl fmt::Debug for MissingFunctionImport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never dump input identifiers or raw source bytes implicitly.
        f.debug_struct("MissingFunctionImport")
            .field("references", &self.references)
            .finish_non_exhaustive()
    }
}
impl MissingFunctionImport {
    pub fn module_name(&self) -> &[u8] {
        &self.key.module
    }
    pub fn library_name(&self) -> &[u8] {
        &self.key.library
    }
    pub const fn nid(&self) -> NidToken {
        self.key.nid
    }
    pub const fn versions(&self) -> DescriptorVersions {
        DescriptorVersions {
            module_major: self.key.major,
            module_minor: self.key.minor,
            library: self.key.version,
        }
    }
    pub const fn first_consumer(&self) -> SymbolRef {
        self.first
    }
    pub const fn reference_count(&self) -> u64 {
        self.references
    }
}

/// Source-retaining classification data, not a symbolic relocation plan. TLS,
/// unknown relocation kinds and other startup requirements remain independent.
/// A consumer must still run normal strict linking/preparation after supplying
/// any explicitly unavailable-function provider it constructs from these data.
pub struct MissingFunctionPlan<'image> {
    modules: Vec<PlannedModule<'image>>,
    functions: Vec<MissingFunctionImport>,
}
impl fmt::Debug for MissingFunctionPlan<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MissingFunctionPlan")
            .field("modules", &self.modules.len())
            .field("functions", &self.functions.len())
            .finish_non_exhaustive()
    }
}
impl<'image> MissingFunctionPlan<'image> {
    pub fn source_modules(&self) -> &[PlannedModule<'image>] {
        &self.modules
    }
    /// Unique semantic identities in canonical resolver-key order.
    pub fn functions(&self) -> &[MissingFunctionImport] {
        &self.functions
    }
    /// Move classification data out without copying source names. This conveys
    /// no complete-link or materialization authority.
    pub fn into_functions(self) -> Vec<MissingFunctionImport> {
        self.functions
    }
}

/// Classify absent function imports without relaxing `plan_symbolic`.
///
/// Existing providers, version conflicts, ambiguity, visibility/type policy and
/// role-scoped selector decoding all pass through the same resolver as strict
/// linking. Missing weak/object imports and nonzero R_X86_64_64 function addends
/// refuse; they are not interchangeable with callable unavailable functions.
/// GLOB_DAT/JUMP_SLOT retain their existing ignored-addend rule. Deferred TLS
/// and unknown relocations are not classified or declared implemented here.
///
/// Aggregate source limits and resolver key/lookup bounds apply before scans
/// and copies, with an additional inclusive unique-function bound. Reused
/// standard-library ordered maps retain the resolver's allocator boundary.
pub fn plan_missing_functions<'image>(
    inputs: &[ModuleInput<'image>],
    limits: &SymbolicLimits,
    max_functions: u64,
) -> Result<MissingFunctionPlan<'image>, SymbolicError> {
    let modules = preflight::prepare(inputs, limits)?;
    preflight::primary_entry(&modules[0])?;
    let mut resolver = Resolver::new(&modules, limits.into())?;
    let functions = discover(&modules, &mut resolver, max_functions)?;
    drop(resolver);
    Ok(MissingFunctionPlan { modules, functions })
}

/// Discover only newly absent callables after appending one actual guest image.
/// Retained unavailable providers stay present; authenticated policies retire
/// only exact markers which that guest genuinely supplies. Classification and
/// final atomic linking share the same resolver/history and explicit guest.
/// Unrelated old binding changes refuse before discovery. No delta, relocation
/// plan, native publication or constructor execution is created here.
pub fn plan_missing_functions_for_extension<'image>(
    base: &SymbolicLinkPlan<'image>,
    guest: ModuleInput<'image>,
    policies: &[FallbackPolicy],
    limits: &SymbolicExtensionLimits,
    max_functions: u64,
) -> Result<MissingFunctionPlan<'image>, SymbolicExtensionError> {
    let prepared = super::extension::prepare_inputs(base, &[guest], policies, limits)?;
    let modules = preflight::prepare(&prepared.inputs, &limits.symbolic)
        .map_err(SymbolicExtensionError::Symbolic)?;
    preflight::primary_entry(&modules[0]).map_err(SymbolicExtensionError::Symbolic)?;
    let (mut resolver, retired) = super::resolution_state(
        &modules,
        &limits.symbolic,
        base.retired_fallbacks(),
        &prepared.approved,
        base.modules().len(),
    )
    .map_err(SymbolicExtensionError::Symbolic)?;
    super::extension::validate_retained_bindings(
        base,
        &mut resolver,
        &prepared.approved,
        &retired,
    )?;
    let functions = discover(&modules, &mut resolver, max_functions)
        .map_err(SymbolicExtensionError::Symbolic)?;
    drop(resolver);
    Ok(MissingFunctionPlan { modules, functions })
}

fn discover(
    modules: &[PlannedModule<'_>],
    resolver: &mut Resolver<'_, '_>,
    max_functions: u64,
) -> Result<Vec<MissingFunctionImport>, SymbolicError> {
    let mut absent = BTreeMap::<Key, (SymbolRef, u64)>::new();
    let mut count = 0;
    for module in modules {
        let Some(metadata) = &module.metadata else {
            continue;
        };
        for (table, records) in [
            (TableKind::Rela, metadata.link().rela()),
            (TableKind::PltRela, metadata.link().plt_rela()),
        ] {
            super::reject_composition(module.id, table, records)?;
            for record in records {
                if !matches!(record.kind(), 1 | 6 | 7) || record.symbol_index() == 0 {
                    continue;
                }
                let consumer = SymbolRef {
                    module: module.id,
                    symbol: record.symbol_index() as usize,
                };
                match resolver.resolve(consumer, record.kind() == 7) {
                    Ok(_) => continue,
                    Err(SymbolicError::MissingProvider { consumer: missing })
                        if missing == consumer => {}
                    Err(error) => return Err(error),
                }
                let key = resolver.missing_function_key(consumer)?;
                if record.kind() == 1 && record.addend != 0 {
                    return Err(SymbolicError::UnsupportedMissingFunction {
                        consumer,
                        reason: MissingFunctionReason::AddressAddend,
                    });
                }
                resolver.step()?;
                match absent.entry(key) {
                    Entry::Vacant(entry) => {
                        charge(
                            &mut count,
                            1,
                            max_functions,
                            SymbolicBudget::MissingFunctions,
                        )?;
                        entry.insert((consumer, 1));
                    }
                    Entry::Occupied(mut entry) => {
                        let (_, references) = entry.get_mut();
                        *references =
                            references
                                .checked_add(1)
                                .ok_or(SymbolicError::SizeOverflow {
                                    budget: SymbolicBudget::MissingFunctions,
                                })?;
                    }
                }
            }
        }
    }
    let mut functions = Vec::new();
    functions
        .try_reserve_exact(absent.len())
        .map_err(|_| SymbolicError::Allocation)?;
    functions.extend(
        absent
            .into_iter()
            .map(|(key, (first, references))| MissingFunctionImport {
                key,
                first,
                references,
            }),
    );
    Ok(functions)
}
