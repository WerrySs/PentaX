// SPDX-License-Identifier: GPL-2.0-only

//! Core-owned authentication of generated stop markers before late binding.
//! Linker policy data alone never proves that an input definition is generated.

use super::*;
use pentax_linker::{FallbackPolicy, Resolution, SymbolRef, SymbolicExtensionError};

pub(super) fn extension_error(error: SymbolicExtensionError) -> KernelStartupError {
    match error {
        SymbolicExtensionError::Allocation => KernelStartupError::Allocation,
        SymbolicExtensionError::Budget => KernelStartupError::Budget("fallback linking work"),
        SymbolicExtensionError::Symbolic(error) => {
            KernelStartupError::Hle(crate::process::hle::HleError::Link(error))
        }
        _ => invalid("fallback linking policy"),
    }
}

impl PreparedNativeKernelStartup<'_, '_> {
    /// Authenticate every retained generated origin in this actual source. A
    /// late-loaded delta has the same retirement policy as the initial factory.
    pub fn dynamic_fallback_policies(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
    ) -> Result<Vec<FallbackPolicy>, KernelStartupError> {
        let mut policies = Vec::new();
        policies
            .try_reserve_exact(self.loaded_modules.dynamic.published.len() + 1)
            .map_err(|_| KernelStartupError::Allocation)?;
        if let Some(policy) = self.dynamic_fallback_policy(current)? {
            policies.push(policy);
        }
        let source = current.materialized_source().source_plan();
        let maximum = self.plan.limits.max_provider_work;
        let extension = self.dynamic_extension_limits();
        let (memory, _) = self.hle.pause_with_tls().map_err(KernelStartupError::Hle)?;
        for origin in &self.loaded_modules.dynamic.published {
            origin.validate_source(source, &mut self.provider_work, maximum)?;
            origin.attest_all(&memory, &mut self.provider_work, maximum)?;
            let work = (origin.function_count() as u64 + 1)
                .checked_mul(source.retired_fallbacks().len() as u64 + 128)
                .ok_or(KernelStartupError::Budget("published fallback work"))?;
            charge(&mut self.provider_work, work, maximum)?;
            policies.push(origin.fallback_policy(source, &extension)?);
        }
        Ok(policies)
    }

    fn fallback_site(
        &self,
        reference: SymbolRef,
    ) -> Result<(u64, crate::process::hle::unavailable::UnavailableSite), KernelStartupError> {
        if let Some(binding) = self.plan.unavailable
            && reference.module.index() == binding.index
        {
            return binding
                .factory
                .symbol_site(reference.symbol)
                .filter(|site| site.ordinal.is_some())
                .map(|site| (binding.bias, site))
                .ok_or(invalid("unowned fallback retirement"));
        }
        let origin = self
            .loaded_modules
            .dynamic
            .published
            .iter()
            .find(|origin| origin.index() == reference.module.index())
            .ok_or(invalid("unowned fallback retirement"))?;
        origin
            .symbol_site(reference.symbol)
            .filter(|site| site.ordinal.is_some())
            .map(|site| (origin.bias(), site))
            .ok_or(invalid("unowned fallback retirement"))
    }

    /// Authenticate only this retained factory's actual missing-function sites.
    /// An ELF name, binding flag or similar-looking instruction grants nothing.
    pub fn dynamic_fallback_policy(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
    ) -> Result<Option<FallbackPolicy>, KernelStartupError> {
        let source = current.materialized_source().source_plan();
        if current.owner_id() != self.owner_id()
            || source.identity() != self.memory().active_source_identity()
        {
            return Err(invalid("fallback source owner"));
        }
        let Some(binding) = self.plan.unavailable else {
            return Ok(None);
        };
        super::super::super::unavailable_runtime::validate_source_and_bindings(
            source,
            &binding,
            &self.plan.limits,
            &mut self.provider_work,
        )?;
        let maximum = self.plan.limits.max_provider_work;
        {
            let (paused, _) = self.hle.pause_with_tls().map_err(KernelStartupError::Hle)?;
            super::super::super::unavailable_runtime::attest_all(
                &paused,
                &binding,
                &mut self.provider_work,
                maximum,
            )?;
        }
        let module = source
            .modules()
            .get(binding.index)
            .ok_or(invalid("fallback module"))?;
        if !std::ptr::eq(module.source_bytes(), binding.factory.image()) {
            return Err(invalid("fallback factory identity"));
        }
        charge(
            &mut self.provider_work,
            (binding.factory.function_count() as u64 + 1)
                * (source.retired_fallbacks().len() as u64 + 1),
            maximum,
        )?;
        let mut allowed = Vec::new();
        allowed
            .try_reserve_exact(binding.factory.function_count())
            .map_err(|_| KernelStartupError::Allocation)?;
        for symbol in 1..=binding.factory.function_count() {
            let site = binding
                .factory
                .symbol_site(symbol)
                .ok_or(invalid("fallback factory site"))?;
            if site.ordinal != Some((symbol - 1) as u32)
                || binding.factory.function(symbol - 1).is_none()
            {
                return Err(invalid("fallback factory ordinal"));
            }
            let reference = SymbolRef {
                module: module.id(),
                symbol,
            };
            if !source.retired_fallbacks().contains(&reference) {
                allowed.push(reference);
            }
        }
        FallbackPolicy::new(
            source,
            module.id(),
            binding.factory.image(),
            &allowed,
            &self.dynamic_extension_limits(),
        )
        .map(Some)
        .map_err(extension_error)
    }

    pub(super) fn validate_dynamic_fallbacks(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        next: &SymbolicLinkPlan<'_>,
    ) -> Result<(), KernelStartupError> {
        let previous = current.materialized_source().source_plan();
        if current.owner_id() != self.owner_id()
            || previous.identity() != self.memory().active_source_identity()
            || next.parent_identity() != Some(previous.identity())
        {
            return Err(invalid("fallback publication owner"));
        }
        let visits =
            (previous.retired_fallbacks().len() as u64 + 1)
                .checked_mul(next.retired_fallbacks().len() as u64 + 1)
                .and_then(|value| {
                    value.checked_add((next.fallback_transitions().len() as u64).checked_mul(
                        previous.patch_count() as u64 + next.patch_count() as u64 + 64,
                    )?)
                })
                .and_then(|value| value.checked_add(next.retired_fallbacks().len() as u64))
                .ok_or(KernelStartupError::Budget("fallback publication work"))?;
        charge(
            &mut self.provider_work,
            visits,
            self.plan.limits.max_provider_work,
        )?;
        if previous
            .retired_fallbacks()
            .iter()
            .any(|reference| !next.retired_fallbacks().contains(reference))
        {
            return Err(invalid("fallback retirement history"));
        }
        if next.fallback_transitions().is_empty()
            && next.retired_fallbacks() == previous.retired_fallbacks()
        {
            return Ok(());
        }
        // Independently authenticate all origins and live immutable marker code;
        // the worker's earlier policy is metadata, not publication authority.
        let _policies = self.dynamic_fallback_policies(current)?;
        if let Some(binding) = self.plan.unavailable {
            super::super::super::unavailable_runtime::validate_source_and_bindings(
                next,
                &binding,
                &self.plan.limits,
                &mut self.provider_work,
            )?;
        }
        for origin in &self.loaded_modules.dynamic.published {
            origin.validate_source(
                next,
                &mut self.provider_work,
                self.plan.limits.max_provider_work,
            )?;
        }
        for reference in next.retired_fallbacks() {
            self.fallback_site(*reference)?;
        }
        for transition in next.fallback_transitions() {
            let old = transition.old_patch();
            let new = transition.new_patch();
            let provider = transition.old_provider();
            let (bias, site) = self.fallback_site(provider)?;
            if transition.parent_identity() != previous.identity()
                || transition.next_identity() != next.identity()
                || transition.new_provider().module.index() != previous.modules().len()
                || old.resolution() != Resolution::Definition(provider)
                || new.resolution() != Resolution::Definition(transition.new_provider())
                || old.value() != bias + site.rva
                || old.consumer() != new.consumer()
                || old.address() != new.address()
                || old.source() != new.source()
                || !previous.patches().contains(&old)
                || !next.patches().contains(&new)
            {
                return Err(invalid("fallback transition provenance"));
            }
        }
        Ok(())
    }
}
