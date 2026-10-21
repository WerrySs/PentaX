// SPDX-License-Identifier: GPL-2.0-or-later

use super::error::charge;
use super::{
    DescriptorVersions, MissingWeakPolicy, PlannedModule, ProviderExclusion,
    ProviderExclusionReason, Resolution, SymbolRef, SymbolicBudget as B, SymbolicError as E,
    SymbolicLimits, UnsupportedSymbol as U,
};
use crate::{
    DecodedDescriptors, DescriptorRole, LibraryDescriptor, ModuleDescriptor, NidToken,
    QualifiedNid, SceMetadata, SelectorId, Symbol, SymbolBinding as Binding,
    SymbolSection as Section, SymbolType as Type, SymbolVisibility as Visibility,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub(super) struct ResolutionLimits {
    missing_weak: MissingWeakPolicy,
    max_exports: u64,
    max_key_bytes: u64,
    max_lookup_steps: u64,
}

impl From<&SymbolicLimits> for ResolutionLimits {
    fn from(limits: &SymbolicLimits) -> Self {
        Self {
            missing_weak: limits.missing_weak,
            max_exports: limits.max_exports,
            max_key_bytes: limits.max_key_bytes,
            max_lookup_steps: limits.max_lookup_steps,
        }
    }
}

impl From<&super::tls::TlsBindingLimits> for ResolutionLimits {
    fn from(limits: &super::tls::TlsBindingLimits) -> Self {
        Self {
            missing_weak: MissingWeakPolicy::Reject,
            max_exports: limits.max_exports,
            max_key_bytes: limits.max_key_bytes,
            max_lookup_steps: limits.max_lookup_steps,
        }
    }
}

#[derive(Clone, Copy)]
enum Domain {
    Address { jump: bool },
    Tls,
}

impl Domain {
    fn is_jump(self) -> bool {
        matches!(self, Self::Address { jump: true })
    }
}

/// Keep at most two identities per tier; never choose a weak-only duplicate
/// through input order. The caller validates type, binding and domain first.
#[derive(Default)]
struct DefinitionTiers {
    strong: Option<SymbolRef>,
    weak: Option<SymbolRef>,
    second_weak: Option<SymbolRef>,
}

impl DefinitionTiers {
    fn add(
        &mut self,
        consumer: SymbolRef,
        candidate: SymbolRef,
        binding: Binding,
    ) -> Result<(), E> {
        if binding == Binding::WEAK {
            if self.weak.is_none() {
                self.weak = Some(candidate);
            } else {
                self.second_weak.get_or_insert(candidate);
            }
        } else if let Some(first) = self.strong {
            return Err(E::AmbiguousProvider {
                consumer,
                first,
                second: candidate,
            });
        } else {
            self.strong = Some(candidate);
        }
        Ok(())
    }

    fn winner(self, consumer: SymbolRef) -> Result<Option<SymbolRef>, E> {
        if self.strong.is_none()
            && let (Some(first), Some(second)) = (self.weak, self.second_weak)
        {
            return Err(E::AmbiguousProvider {
                consumer,
                first,
                second,
            });
        }
        Ok(self.strong.or(self.weak))
    }
}

/// Selector IDs have already been interpreted in their image/role scope.
/// Names remain byte-exact; versions are exact under ExactDeclarationV1 policy.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Key {
    pub(super) module: Vec<u8>,
    pub(super) library: Vec<u8>,
    pub(super) nid: NidToken,
    // Keep identity first so alternate versions occupy one contiguous range.
    pub(super) major: u8,
    pub(super) minor: u8,
    pub(super) version: u16,
}

impl Key {
    fn versions(&self) -> DescriptorVersions {
        DescriptorVersions {
            module_major: self.major,
            module_minor: self.minor,
            library: self.version,
        }
    }

    fn same_identity(&self, other: &Self) -> bool {
        self.module == other.module && self.library == other.library && self.nid == other.nid
    }
}

pub(super) struct Resolver<'m, 'a> {
    modules: &'m [PlannedModule<'a>],
    providers: BTreeMap<Key, Vec<SymbolRef>>,
    cache: BTreeMap<(usize, usize, bool), (u64, Resolution)>,
    tls_cache: BTreeMap<(usize, usize), SymbolRef>,
    exclusions: Vec<ProviderExclusion>,
    keys: u64,
    steps: u64,
    limits: ResolutionLimits,
    query_work: bool,
}

impl<'m, 'a> Resolver<'m, 'a> {
    pub(super) fn new(
        modules: &'m [PlannedModule<'a>],
        limits: ResolutionLimits,
    ) -> Result<Self, E> {
        let mut result = Self::empty(modules, limits, false);
        result.populate(None, &[])?;
        Ok(result)
    }

    pub(super) fn with_retired(
        modules: &'m [PlannedModule<'a>],
        limits: ResolutionLimits,
        retired: &[SymbolRef],
    ) -> Result<Self, E> {
        let mut result = Self::empty(modules, limits, false);
        result.populate(None, retired)?;
        Ok(result)
    }

    pub(super) fn empty(
        modules: &'m [PlannedModule<'a>],
        limits: ResolutionLimits,
        query_work: bool,
    ) -> Self {
        Self {
            modules,
            providers: BTreeMap::new(),
            cache: BTreeMap::new(),
            tls_cache: BTreeMap::new(),
            exclusions: Vec::new(),
            keys: 0,
            steps: 0,
            limits,
            query_work,
        }
    }

    pub(super) fn populate(
        &mut self,
        selected: Option<usize>,
        retired: &[SymbolRef],
    ) -> Result<(), E> {
        let mut exports = 0;
        if self.query_work {
            self.steps(self.modules.len() as u64)?;
        }
        for module in self.modules {
            if selected.is_some_and(|index| index != module.id.index()) {
                continue;
            }
            let Some(metadata) = &module.metadata else {
                continue;
            };
            if self.query_work {
                self.steps(metadata.link().symbols().len() as u64)?;
            }
            // An image only offers decoded exports when it declares an
            // export-library scope. Its unrelated defined symbols are not
            // silently interpreted through import-library declarations.
            if metadata
                .descriptors()
                .decoded()
                .is_none_or(|decoded| decoded.export_libraries().is_empty())
            {
                continue;
            }
            for (index, symbol) in metadata.link().symbols().iter().enumerate().skip(1) {
                if symbol.is_undefined()
                    || symbol.binding() == Binding::LOCAL
                    || matches!(
                        symbol.visibility(),
                        Visibility::HIDDEN | Visibility::INTERNAL
                    )
                {
                    continue;
                }
                let Some(name) = &symbol.name else {
                    continue;
                };
                if !name.as_bytes().contains(&b'#') {
                    continue;
                }
                let reference = SymbolRef {
                    module: module.id,
                    symbol: index,
                };
                charge(&mut exports, 1, self.limits.max_exports, B::Exports)?;
                let Some(key) = self.provider_key(metadata, reference)? else {
                    continue;
                };
                if !retired.is_empty() {
                    self.steps(u64::from(usize::BITS - retired.len().leading_zeros()) + 1)?;
                    if retired
                        .binary_search_by_key(
                            &(reference.module.index(), reference.symbol),
                            |symbol| (symbol.module.index(), symbol.symbol),
                        )
                        .is_ok()
                    {
                        continue;
                    }
                }
                self.step()?;
                self.providers.entry(key).or_default().push(reference);
            }
        }
        Ok(())
    }

    pub(super) fn into_exclusions(self) -> Vec<ProviderExclusion> {
        self.exclusions
    }

    pub(super) fn validate_fallback(&mut self, reference: SymbolRef) -> Result<(), E> {
        let module = &self.modules[reference.module.index()];
        let metadata = module
            .metadata
            .as_ref()
            .ok_or(E::MissingDescriptors { symbol: reference })?;
        let symbol = &metadata.link().symbols()[reference.symbol];
        if symbol.is_undefined() || symbol.symbol_type() != Type::FUNC {
            return Err(E::UnsupportedSymbol {
                symbol: reference,
                reason: U::Type,
            });
        }
        if symbol.binding() != Binding::GLOBAL {
            return Err(E::UnsupportedSymbol {
                symbol: reference,
                reason: U::Binding,
            });
        }
        check_provider(symbol, reference, Domain::Address { jump: true })?;
        if self.provider_key(metadata, reference)?.is_none() {
            return Err(E::MissingProvider {
                consumer: reference,
            });
        }
        definition_value(module, reference, true)?;
        Ok(())
    }

    /// Retire only an approved exact function when the added image owns the
    /// unique winning real definition. GLOBAL outranks WEAK without changing
    /// either definition's binding. The caller authenticates generated origin.
    pub(super) fn retire_for_added(
        &mut self,
        fallback: SymbolRef,
        added: usize,
    ) -> Result<bool, E> {
        self.validate_fallback(fallback)?;
        let metadata = self.modules[fallback.module.index()]
            .metadata
            .as_ref()
            .expect("validated fallback");
        let mut key = self
            .provider_key(metadata, fallback)?
            .expect("offered fallback");
        let mut tiers = DefinitionTiers::default();
        let mut added_candidate = false;
        if let Some(candidates) = self.providers.get(&key) {
            let count = candidates.len() as u64;
            let visits = count.checked_mul(2).ok_or(E::SizeOverflow {
                budget: B::LookupSteps,
            })?;
            charge(
                &mut self.steps,
                visits,
                self.limits.max_lookup_steps,
                B::LookupSteps,
            )?;
            for &candidate in candidates {
                if candidate == fallback {
                    continue;
                }
                let module = &self.modules[candidate.module.index()];
                let symbol = &module
                    .metadata
                    .as_ref()
                    .expect("indexed provider")
                    .link()
                    .symbols()[candidate.symbol];
                if symbol.symbol_type() != Type::FUNC {
                    if candidate.module.index() == added {
                        return Err(E::UnsupportedSymbol {
                            symbol: candidate,
                            reason: U::TypeMismatch,
                        });
                    }
                    continue;
                }
                check_provider(symbol, candidate, Domain::Address { jump: true })?;
                if candidate.module.index() == added {
                    // Validate every offered added function, including a weak
                    // candidate that loses to a strong definition in its image.
                    definition_value(module, candidate, true)?;
                    added_candidate = true;
                }
                tiers.add(fallback, candidate, symbol.binding())?;
            }
        }
        if !added_candidate {
            // A same-identity declaration at another version cannot silently
            // stand in for the requested exact fallback identity.
            let requested = key.versions();
            key.major = 0;
            key.minor = 0;
            key.version = 0;
            self.step()?;
            for (available, candidates) in self.providers.range(&key..) {
                charge(
                    &mut self.steps,
                    candidates.len() as u64 + 1,
                    self.limits.max_lookup_steps,
                    B::LookupSteps,
                )?;
                if !available.same_identity(&key) {
                    break;
                }
                if available.versions() != requested
                    && let Some(&provider) = candidates
                        .iter()
                        .find(|candidate| candidate.module.index() == added)
                {
                    return Err(E::VersionMismatch {
                        consumer: fallback,
                        requested,
                        available: available.versions(),
                        provider,
                    });
                }
            }
            return Ok(false);
        }
        if tiers
            .winner(fallback)?
            .is_none_or(|winner| winner.module.index() != added)
        {
            // An existing real provider cannot authorize an added-image
            // transition. Ordinary planning and predecessor checks still run.
            return Ok(false);
        }
        // No consumer lookup has run yet, so caches cannot contain an old value.
        self.step()?;
        let candidates = self.providers.get_mut(&key).expect("replacement key");
        candidates.retain(|candidate| *candidate != fallback);
        Ok(true)
    }

    pub(super) fn step(&mut self) -> Result<(), E> {
        self.steps(1)
    }
    pub(super) fn work_used(&self) -> u64 {
        self.steps
    }
    pub(super) fn steps(&mut self, count: u64) -> Result<(), E> {
        charge(
            &mut self.steps,
            count,
            self.limits.max_lookup_steps,
            B::LookupSteps,
        )
    }

    pub(super) fn filter_module_exports(
        &mut self,
        query: &super::module_lookup::ModuleExportQuery<'_>,
    ) -> Result<(), E> {
        // This resolver belongs only to the separate qualified lookup. Charge
        // all comparisons before filtering its temporary index; no allocation
        // or change to ordinary resolution/dlsym selection is involved.
        // keys already bounds every copied module/library name, including
        // duplicates. Its scalar census is a conservative comparison envelope.
        let per_key = (query.module.len() as u64)
            .checked_add(query.library.len() as u64)
            .and_then(|n| n.checked_add(1))
            .ok_or(E::SizeOverflow {
                budget: B::LookupSteps,
            })?;
        let work = (self.providers.len() as u64)
            .checked_mul(per_key)
            .and_then(|n| n.checked_add(self.keys))
            .ok_or(E::SizeOverflow {
                budget: B::LookupSteps,
            })?;
        self.steps(work)?;
        self.providers.retain(|key, _| {
            key.nid == query.nid
                && key.module == query.module
                && key.library == query.library
                && key.versions() == query.versions
        });
        Ok(())
    }

    pub(super) fn module_export(
        &mut self,
        nid: NidToken,
    ) -> Result<Option<(SymbolRef, u64)>, super::ModuleLookupError> {
        let mut first = [None; 3];
        let mut second = [None; 3];
        // No cross-image or name/version fallback: populate selected exactly
        // one retained module and interpreted its own export declarations.
        for (key, references) in &self.providers {
            charge(
                &mut self.steps,
                references.len() as u64 + 1,
                self.limits.max_lookup_steps,
                B::LookupSteps,
            )?;
            if key.nid != nid {
                continue;
            }
            for &reference in references {
                let symbol = &self.modules[reference.module.index()]
                    .metadata
                    .as_ref()
                    .expect("indexed export")
                    .link()
                    .symbols()[reference.symbol];
                let rank = if symbol.symbol_type() == Type::FUNC {
                    0
                } else if symbol.symbol_type() == Type::OBJECT {
                    1
                } else {
                    2
                };
                if first[rank].is_some() {
                    second[rank].get_or_insert(reference);
                } else {
                    first[rank] = Some(reference);
                }
            }
        }
        let Some(rank) = first.iter().position(Option::is_some) else {
            return Ok(None);
        };
        let reference = first[rank].expect("selected rank");
        if let Some(other) = second[rank] {
            return Err(super::ModuleLookupError::Ambiguous {
                first: reference,
                second: other,
            });
        }
        let module = &self.modules[reference.module.index()];
        let symbol = &module
            .metadata
            .as_ref()
            .expect("indexed export")
            .link()
            .symbols()[reference.symbol];
        // This separate module-local query retains its pinned GLOBAL-only
        // policy; ordinary relocation precedence does not broaden dlsym.
        if symbol.binding() == Binding::WEAK {
            return Err(E::UnsupportedSymbol {
                symbol: reference,
                reason: U::WeakProvider,
            }
            .into());
        }
        check_provider(symbol, reference, Domain::Address { jump: false })?;
        charge(
            &mut self.steps,
            module.ranges.len() as u64 + module.runtime.segments.len() as u64 + 4,
            self.limits.max_lookup_steps,
            B::LookupSteps,
        )?;
        let (address, resolution) = definition_value(module, reference, false)?;
        if matches!(resolution, Resolution::Absolute(_)) {
            return Err(super::ModuleLookupError::Absolute);
        }
        Ok(Some((reference, address)))
    }

    fn missing_provider(&mut self, mut key: Key, consumer: SymbolRef) -> Result<E, E> {
        let requested = key.versions();
        key.major = 0;
        key.minor = 0;
        key.version = 0;
        // One seek and one identity comparison; no identity names are copied.
        self.step()?;
        self.step()?;
        let next = self
            .providers
            .range((std::ops::Bound::Included(&key), std::ops::Bound::Unbounded))
            .next();
        let Some((available, candidates)) =
            next.filter(|(available, _)| available.same_identity(&key))
        else {
            return Ok(E::MissingProvider { consumer });
        };
        charge(
            &mut self.steps,
            1,
            self.limits.max_lookup_steps,
            B::LookupSteps,
        )?;
        // Every provider entry is created together with its first identity.
        Ok(E::VersionMismatch {
            consumer,
            requested,
            available: available.versions(),
            provider: candidates[0],
        })
    }

    fn key(
        &mut self,
        metadata: &SceMetadata,
        reference: SymbolRef,
        import: bool,
    ) -> Result<Key, E> {
        let qualified = qualified_name(metadata, reference)?;
        let descriptors = metadata
            .descriptors()
            .decoded()
            .ok_or(E::MissingDescriptors { symbol: reference })?;
        let (module_role, modules, library_role, libraries) = roles(descriptors, import);
        let module =
            self.find_module(modules, qualified.module())?
                .ok_or(E::MissingDeclaration {
                    symbol: reference,
                    role: module_role,
                    selector: qualified.module().value(),
                })?;
        let library =
            self.find_library(libraries, qualified.library())?
                .ok_or(E::MissingDeclaration {
                    symbol: reference,
                    role: library_role,
                    selector: qualified.library().value(),
                })?;
        self.owned_key(qualified, module, library)
    }

    fn provider_key(
        &mut self,
        metadata: &SceMetadata,
        reference: SymbolRef,
    ) -> Result<Option<Key>, E> {
        // Canonical parsing precedes classification. The preflight inspection
        // has already rejected role conflicts and duplicate descriptor IDs.
        let qualified = qualified_name(metadata, reference)?;
        let descriptors = metadata
            .descriptors()
            .decoded()
            .ok_or(E::MissingDescriptors { symbol: reference })?;
        // Every role is image-local and independently searched/charged. Import
        // declarations never construct a provider key, even if both pairs exist.
        let module = self.find_module(descriptors.module_info(), qualified.module())?;
        let library = self.find_library(descriptors.export_libraries(), qualified.library())?;
        let needed = self.find_module(descriptors.needed_modules(), qualified.module())?;
        let imported = self.find_library(descriptors.import_libraries(), qualified.library())?;
        if module.is_none() && library.is_none() && needed.is_some() && imported.is_some() {
            // max_exports was charged before parsing this candidate. Charge
            // diagnostic retention before allocation; no key/name is copied.
            self.step()?;
            self.exclusions.push(ProviderExclusion {
                symbol: reference,
                reason: ProviderExclusionReason::NotOfferedImportScope,
            });
            return Ok(None);
        }
        let module = module.ok_or(E::MissingDeclaration {
            symbol: reference,
            role: DescriptorRole::ModuleInfo,
            selector: qualified.module().value(),
        })?;
        let library = library.ok_or(E::MissingDeclaration {
            symbol: reference,
            role: DescriptorRole::ExportLibrary,
            selector: qualified.library().value(),
        })?;
        self.owned_key(qualified, module, library).map(Some)
    }

    fn find_module<'d>(
        &mut self,
        descriptors: &'d [ModuleDescriptor],
        selector: SelectorId,
    ) -> Result<Option<&'d ModuleDescriptor>, E> {
        for descriptor in descriptors {
            self.step()?;
            if descriptor.id() == selector {
                return Ok(Some(descriptor));
            }
        }
        Ok(None)
    }

    fn find_library<'d>(
        &mut self,
        descriptors: &'d [LibraryDescriptor],
        selector: SelectorId,
    ) -> Result<Option<&'d LibraryDescriptor>, E> {
        for descriptor in descriptors {
            self.step()?;
            if descriptor.id() == selector {
                return Ok(Some(descriptor));
            }
        }
        Ok(None)
    }

    fn owned_key(
        &mut self,
        qualified: QualifiedNid,
        module: &ModuleDescriptor,
        library: &LibraryDescriptor,
    ) -> Result<Key, E> {
        let bytes = module.name().as_bytes().len() as u64 + library.name().as_bytes().len() as u64;
        charge(
            &mut self.keys,
            bytes,
            self.limits.max_key_bytes,
            B::KeyBytes,
        )?;
        if self.query_work {
            self.steps(bytes)?;
        }
        Ok(Key {
            module: module.name().as_bytes().to_vec(),
            major: module.version_major(),
            minor: module.version_minor(),
            library: library.name().as_bytes().to_vec(),
            version: library.version(),
            nid: qualified.nid(),
        })
    }

    pub(super) fn resolve(
        &mut self,
        reference: SymbolRef,
        jump: bool,
    ) -> Result<(u64, Resolution), E> {
        let cache_key = (reference.module.index(), reference.symbol, jump);
        self.step()?;
        if let Some(&value) = self.cache.get(&cache_key) {
            return Ok(value);
        }
        let value = match self.select_definition(reference, Domain::Address { jump }) {
            Ok(definition) => {
                definition_value(&self.modules[definition.module.index()], definition, jump)?
            }
            Err(E::MissingProvider { consumer })
                if consumer == reference
                    && self.limits.missing_weak == MissingWeakPolicy::NullAddressV1 =>
            {
                // select_definition already checked type, visibility, binding,
                // exact declared scope and the absence of a version conflict.
                let symbol = &self.modules[reference.module.index()]
                    .metadata
                    .as_ref()
                    .expect("inspected import")
                    .link()
                    .symbols()[reference.symbol];
                if !symbol.is_undefined() || symbol.binding() != Binding::WEAK {
                    return Err(E::MissingProvider { consumer });
                }
                (0, Resolution::UnresolvedWeak(reference))
            }
            Err(error) => return Err(error),
        };
        self.step()?;
        self.cache.insert(cache_key, value);
        Ok(value)
    }

    /// Reuse the exact role/version decoder after a MissingProvider outcome.
    /// This data is not a provider address or execution authority.
    pub(super) fn missing_function_key(&mut self, reference: SymbolRef) -> Result<Key, E> {
        let metadata = self.modules[reference.module.index()]
            .metadata
            .as_ref()
            .expect("missing references were inspected");
        let symbol = &metadata.link().symbols()[reference.symbol];
        check_import(symbol, reference, Domain::Address { jump: false })?;
        let reason = if symbol.symbol_type() != Type::FUNC {
            Some(super::MissingFunctionReason::SymbolType)
        } else if symbol.binding() != Binding::GLOBAL {
            Some(super::MissingFunctionReason::WeakBinding)
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(E::UnsupportedMissingFunction {
                consumer: reference,
                reason,
            });
        }
        self.key(metadata, reference, true)
    }

    /// Select a TLS identity without invoking ordinary address arithmetic.
    pub(super) fn resolve_tls(&mut self, reference: SymbolRef) -> Result<SymbolRef, E> {
        let cache_key = (reference.module.index(), reference.symbol);
        self.step()?;
        if let Some(&definition) = self.tls_cache.get(&cache_key) {
            return Ok(definition);
        }
        let definition = self.select_definition(reference, Domain::Tls)?;
        self.step()?;
        self.tls_cache.insert(cache_key, definition);
        Ok(definition)
    }

    fn select_definition(&mut self, reference: SymbolRef, domain: Domain) -> Result<SymbolRef, E> {
        let module = &self.modules[reference.module.index()];
        // Records are private inspected metadata; every nonzero relocation
        // index was checked by inspect_sce before the plan reaches this point.
        let symbol = &module
            .metadata
            .as_ref()
            .expect("symbol references require inspected metadata")
            .link()
            .symbols()[reference.symbol];
        // TLS type/section errors cannot be disguised as preemptible ordinary
        // addresses. Address mode retains its existing diagnostic ordering.
        if matches!(domain, Domain::Tls) {
            check_tls_common(symbol, reference)?;
        }
        if symbol.is_undefined() {
            check_import(symbol, reference, domain)?;
            let key = self.key(
                module.metadata.as_ref().expect("checked above"),
                reference,
                true,
            )?;
            self.step()?;
            let Some(candidates) = self.providers.get(&key) else {
                return Err(self.missing_provider(key, reference)?);
            };
            let mut tiers = DefinitionTiers::default();
            let mut wrong_type = None;
            // Charge before candidate traversal; diagnostics retain at most
            // two identities, never an attacker-sized candidate list.
            charge(
                &mut self.steps,
                candidates.len() as u64,
                self.limits.max_lookup_steps,
                B::LookupSteps,
            )?;
            for &candidate in candidates {
                let provider = &self.modules[candidate.module.index()];
                let definition = &provider
                    .metadata
                    .as_ref()
                    .expect("indexed metadata")
                    .link()
                    .symbols()[candidate.symbol];
                if definition.symbol_type() != symbol.symbol_type() {
                    wrong_type.get_or_insert(candidate);
                    continue;
                }
                check_provider(definition, candidate, domain)?;
                tiers.add(reference, candidate, definition.binding())?;
            }
            // A unique strong definition outranks weak definitions, independent
            // of module order. Multiple weak-only definitions still require an
            // explicit scope/precedence policy; never choose one by accident.
            let candidate = tiers.winner(reference)?.ok_or(match wrong_type {
                Some(symbol) => E::UnsupportedSymbol {
                    symbol,
                    reason: U::TypeMismatch,
                },
                None => E::MissingProvider {
                    consumer: reference,
                },
            })?;
            Ok(candidate)
        } else {
            if symbol.binding() != Binding::LOCAL
                && !matches!(
                    symbol.visibility(),
                    Visibility::HIDDEN | Visibility::PROTECTED
                )
            {
                return Err(E::UnsupportedSymbol {
                    symbol: reference,
                    reason: U::PreemptibleDefinition,
                });
            }
            if !matches!(symbol.binding(), Binding::LOCAL | Binding::GLOBAL)
                && !(matches!(domain, Domain::Address { .. }) && symbol.binding() == Binding::WEAK)
            {
                return Err(E::UnsupportedSymbol {
                    symbol: reference,
                    reason: U::Binding,
                });
            }
            check_domain(symbol, reference, domain)?;
            Ok(reference)
        }
    }
}

fn qualified_name(metadata: &SceMetadata, reference: SymbolRef) -> Result<QualifiedNid, E> {
    let symbol = &metadata.link().symbols()[reference.symbol];
    QualifiedNid::parse(symbol.name.as_ref().map_or(&[], |name| name.as_bytes())).map_err(|error| {
        E::QualifiedName {
            symbol: reference,
            error,
        }
    })
}

fn roles(
    descriptors: &DecodedDescriptors,
    import: bool,
) -> (
    DescriptorRole,
    &[ModuleDescriptor],
    DescriptorRole,
    &[LibraryDescriptor],
) {
    if import {
        (
            DescriptorRole::NeededModule,
            descriptors.needed_modules(),
            DescriptorRole::ImportLibrary,
            descriptors.import_libraries(),
        )
    } else {
        (
            DescriptorRole::ModuleInfo,
            descriptors.module_info(),
            DescriptorRole::ExportLibrary,
            descriptors.export_libraries(),
        )
    }
}

fn check_common(symbol: &Symbol, reference: SymbolRef) -> Result<(), E> {
    let reason = if symbol.other & !7 != 0 {
        Some(U::OtherBits)
    } else if !matches!(
        symbol.visibility(),
        Visibility::DEFAULT | Visibility::HIDDEN | Visibility::PROTECTED
    ) {
        Some(U::Visibility)
    } else if !matches!(
        Section::of(symbol.section_index),
        Section::Undefined | Section::Section(_) | Section::Absolute
    ) {
        Some(U::Section)
    } else if !matches!(symbol.symbol_type(), Type::OBJECT | Type::FUNC)
        && !(symbol.binding() == Binding::LOCAL
            && symbol.symbol_type() == Type::NOTYPE
            && Section::of(symbol.section_index) == Section::Absolute)
    {
        Some(U::Type)
    } else {
        None
    };
    match reason {
        Some(reason) => Err(E::UnsupportedSymbol {
            symbol: reference,
            reason,
        }),
        None => Ok(()),
    }
}

fn check_domain(symbol: &Symbol, reference: SymbolRef, domain: Domain) -> Result<(), E> {
    match domain {
        Domain::Address { .. } => check_common(symbol, reference),
        Domain::Tls => check_tls_common(symbol, reference),
    }
}

fn check_tls_common(symbol: &Symbol, reference: SymbolRef) -> Result<(), E> {
    let reason = if symbol.other & !7 != 0 {
        Some(U::OtherBits)
    } else if !matches!(
        symbol.visibility(),
        Visibility::DEFAULT | Visibility::HIDDEN | Visibility::PROTECTED
    ) {
        Some(U::Visibility)
    } else if symbol.symbol_type() != Type::TLS {
        Some(U::Type)
    } else if !matches!(
        Section::of(symbol.section_index),
        Section::Undefined | Section::Section(_)
    ) {
        Some(U::Section)
    } else {
        None
    };
    match reason {
        Some(reason) => Err(E::UnsupportedSymbol {
            symbol: reference,
            reason,
        }),
        None => Ok(()),
    }
}

fn check_import(symbol: &Symbol, reference: SymbolRef, domain: Domain) -> Result<(), E> {
    check_domain(symbol, reference, domain)?;
    if !matches!(symbol.binding(), Binding::GLOBAL | Binding::WEAK) {
        return Err(E::UnsupportedSymbol {
            symbol: reference,
            reason: U::Binding,
        });
    }
    if symbol.visibility() != Visibility::DEFAULT {
        return Err(E::UnsupportedSymbol {
            symbol: reference,
            reason: U::Visibility,
        });
    }
    if domain.is_jump() && symbol.symbol_type() != Type::FUNC {
        return Err(E::UnsupportedSymbol {
            symbol: reference,
            reason: U::Type,
        });
    }
    Ok(())
}

fn check_provider(symbol: &Symbol, reference: SymbolRef, domain: Domain) -> Result<(), E> {
    check_domain(symbol, reference, domain)?;
    if symbol.binding() != Binding::GLOBAL
        && !(matches!(domain, Domain::Address { .. }) && symbol.binding() == Binding::WEAK)
    {
        return Err(E::UnsupportedSymbol {
            symbol: reference,
            reason: if symbol.binding() == Binding::WEAK {
                U::WeakProvider
            } else {
                U::Binding
            },
        });
    }
    if !matches!(
        symbol.visibility(),
        Visibility::DEFAULT | Visibility::PROTECTED
    ) {
        return Err(E::UnsupportedSymbol {
            symbol: reference,
            reason: U::Visibility,
        });
    }
    Ok(())
}

fn definition_value(
    module: &PlannedModule<'_>,
    reference: SymbolRef,
    jump: bool,
) -> Result<(u64, Resolution), E> {
    let symbol = &module
        .metadata
        .as_ref()
        .expect("inspected symbol")
        .link()
        .symbols()[reference.symbol];
    check_common(symbol, reference)?;
    if jump && symbol.symbol_type() != Type::FUNC {
        return Err(E::UnsupportedSymbol {
            symbol: reference,
            reason: U::Type,
        });
    }
    if Section::of(symbol.section_index) == Section::Absolute {
        if jump || symbol.symbol_type() == Type::FUNC {
            return Err(E::UnsupportedSymbol {
                symbol: reference,
                reason: U::AbsoluteFunction,
            });
        }
        return Ok((symbol.value, Resolution::Absolute(reference)));
    }
    let end = symbol
        .value
        .checked_add(symbol.size)
        .ok_or(E::SymbolRange { symbol: reference })?;
    let first = module
        .ranges
        .partition_point(|range| range.end <= symbol.value);
    let mut cursor = symbol.value;
    let mut starts_inside = false;
    for range in &module.ranges[first..] {
        if range.start > cursor {
            break;
        }
        starts_inside = true;
        cursor = range.end.min(end);
        if cursor == end {
            break;
        }
    }
    if !starts_inside || cursor != end {
        return Err(E::SymbolRange { symbol: reference });
    }
    if jump
        && !module.runtime.segments.iter().any(|segment| {
            segment.flags.executable()
                && segment.vaddr <= symbol.value
                && symbol.value - segment.vaddr < segment.mem_size
        })
    {
        return Err(E::FunctionNotExecutable { symbol: reference });
    }
    let value = symbol
        .value
        .checked_add(module.load_bias)
        .ok_or(E::SymbolAddressOverflow { symbol: reference })?;
    Ok((value, Resolution::Definition(reference)))
}
