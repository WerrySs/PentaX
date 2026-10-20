// SPDX-License-Identifier: GPL-2.0-only

//! Admission of additional stop providers. Factory identity is authenticated
//! here against an independent classification, never inferred from ELF names.

use super::*;
use pentax_linker::{ModuleInput, plan_missing_functions_for_extension};

impl PreparedNativeKernelStartup<'_, '_> {
    /// Aggregate remaining provider bounds, including retained original markers.
    /// Retired markers still occupy memory and do not refund these budgets.
    pub fn dynamic_unavailable_limits(
        &self,
    ) -> Result<UnavailableProviderLimits, KernelStartupError> {
        let dynamic = &self.loaded_modules.dynamic;
        let limits = dynamic
            .unavailable_limits
            .ok_or(invalid("dynamic unavailable policy"))?;
        Ok(UnavailableProviderLimits {
            max_functions: limits
                .max_functions
                .checked_sub(dynamic.unavailable_functions)
                .ok_or(KernelStartupError::Budget("unavailable functions"))?,
            max_image_bytes: limits
                .max_image_bytes
                .checked_sub(dynamic.unavailable_bytes)
                .ok_or(KernelStartupError::Budget("unavailable image bytes"))?,
            max_work_steps: limits
                .max_work_steps
                .checked_sub(dynamic.unavailable_work)
                .ok_or(KernelStartupError::Budget("unavailable generation work"))?,
        })
    }

    /// Place the optional pure-RX provider after the selected guest's full
    /// aligned extent. The native batch still admits every actual mapping.
    pub fn dynamic_unavailable_bias(
        &self,
        input: &DynamicModuleInput,
    ) -> Result<u64, KernelStartupError> {
        let bias = self.dynamic_load_bias(input)?;
        let runtime = open_executable(input.bytes())
            .and_then(|view| view.runtime_plan())
            .map_err(|_| invalid("module input"))?;
        let (_, high, _) = shape(&runtime)?;
        aligned(
            bias.checked_add(high).ok_or(invalid("module placement"))?,
            GRANULE,
        )
    }

    pub(super) fn authenticate_dynamic_delta(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        input: &DynamicModuleInput,
        source: &SymbolicLinkPlan<'_>,
        delta: Option<&GeneratedUnavailableProvider>,
    ) -> Result<Option<PublishedUnavailable>, KernelStartupError> {
        let Some(factory) = delta else {
            return Ok(None);
        };
        let remaining = self.dynamic_unavailable_limits()?;
        if factory.function_count() == 0
            || factory.function_count() > remaining.max_functions
            || factory.image().len() as u64 > remaining.max_image_bytes
            || factory.work_steps() > remaining.max_work_steps
        {
            return Err(KernelStartupError::Budget("dynamic unavailable provider"));
        }
        let previous = current.materialized_source().source_plan();
        let index = previous.modules().len() + 1;
        let module = source
            .modules()
            .get(index)
            .ok_or(invalid("dynamic unavailable module"))?;
        let bias = self.dynamic_unavailable_bias(input)?;
        if !std::ptr::eq(module.source_bytes(), factory.image()) || module.load_bias() != bias {
            return Err(invalid("dynamic unavailable factory identity"));
        }
        // Re-derive the missing set from the actual owner, current source and
        // exact admitted input. A caller-created factory alone grants nothing.
        let policies = self.dynamic_fallback_policies(current)?;
        let view = open_executable(input.bytes()).map_err(|_| invalid("module input"))?;
        let expected = plan_missing_functions_for_extension(
            previous,
            ModuleInput::new(view, self.dynamic_load_bias(input)?),
            &policies,
            &self.dynamic_extension_limits(),
            remaining.max_functions as u64,
        )
        .map_err(fallbacks::extension_error)?;
        if expected.functions().len() != factory.function_count() {
            return Err(invalid("dynamic unavailable classification count"));
        }
        for (ordinal, expected) in expected.functions().iter().enumerate() {
            let actual = factory
                .function(ordinal)
                .ok_or(invalid("dynamic unavailable ordinal"))?;
            let work = (expected.module_name().len() as u64)
                .checked_add(expected.library_name().len() as u64)
                .and_then(|value| value.checked_mul(2))
                .and_then(|value| value.checked_add(64))
                .ok_or(KernelStartupError::Budget(
                    "dynamic unavailable classification",
                ))?;
            charge(
                &mut self.provider_work,
                work,
                self.plan.limits.max_provider_work,
            )?;
            if expected.module_name() != actual.module_name()
                || expected.library_name() != actual.library_name()
                || expected.versions() != actual.versions()
                || expected.nid() != actual.nid()
                || expected.first_consumer() != actual.first_consumer()
                || expected.reference_count() != actual.reference_count()
            {
                return Err(invalid("dynamic unavailable classification identity"));
            }
        }
        let origin = PublishedUnavailable::prepare(
            factory,
            index,
            bias,
            &self.plan.limits,
            &mut self.provider_work,
        )?;
        origin.validate_source(
            source,
            &mut self.provider_work,
            self.plan.limits.max_provider_work,
        )?;
        Ok(Some(origin))
    }
}
