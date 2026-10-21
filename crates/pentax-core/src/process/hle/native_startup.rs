// SPDX-License-Identifier: GPL-2.0-only

//! Explicit typed TLS/HLE composition over one owned physically NX address space.

use super::dispatch::ServiceBudget;
use super::native::{NativeLibcCall, NativeServiceStop, service_stop};
use super::session::attest;
use super::{HleError, HleStartupPlan};
use crate::memory::native::{NativeGuestMemory, NativeLoadLimits};
use crate::process::tls::{PreparedNativeTlsSession, StaticTlsStorage, plan_relocated_static_tls};
use pentax_linker::plan_tls_bindings;
use pentax_platform::native_memory::{NativeAddressSpace, PausedNativeMemory};

/// Retains the exact admitted startup source, actual TLS/native backing owner
/// and cumulative service budget. Temporary binding plans are never detached
/// into authoritative metadata, and no self-reference or executor is required.
#[derive(Debug)]
pub struct PreparedNativeHleStartup<'plan, 'image> {
    plan: &'plan HleStartupPlan<'image>,
    tls: PreparedNativeTlsSession<'plan, 'image>,
    budget: ServiceBudget,
}

impl<'plan, 'image> PreparedNativeHleStartup<'plan, 'image> {
    pub const fn plan(&self) -> &'plan HleStartupPlan<'image> {
        self.plan
    }
    pub fn tls(&self) -> &PreparedNativeTlsSession<'plan, 'image> {
        &self.tls
    }
    pub fn memory(&self) -> &NativeGuestMemory<'plan, 'image> {
        self.tls.memory()
    }
    pub(crate) fn tls_mut(&mut self) -> &mut PreparedNativeTlsSession<'plan, 'image> {
        &mut self.tls
    }
    pub fn pause(&mut self) -> Result<PausedNativeMemory<'_>, HleError> {
        self.tls.memory.pause().map_err(HleError::Memory)
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub(super) fn pause_with_service_budget(
        &mut self,
    ) -> Result<(PausedNativeMemory<'_>, &mut ServiceBudget), HleError> {
        let memory = self.tls.memory.pause().map_err(HleError::Memory)?;
        Ok((memory, &mut self.budget))
    }
    pub(crate) fn pause_with_tls(
        &mut self,
    ) -> Result<
        (
            PausedNativeMemory<'_>,
            &[crate::process::tls::StaticTlsInstance],
        ),
        HleError,
    > {
        self.tls.pause_with_instances().map_err(HleError::Tls)
    }
    pub fn service_attempts(&self) -> u64 {
        self.budget.attempts()
    }
    pub fn completed_services(&self) -> u64 {
        self.budget.completed()
    }

    #[allow(dead_code)]
    pub(crate) fn service_stop(&mut self, stop: NativeServiceStop) -> Result<u64, HleError> {
        service_stop(&self.plan.hle, &mut self.tls.memory, &mut self.budget, stop)
    }

    pub(super) fn pre_service_stop(
        &mut self,
        syscall_pc: u64,
        next_pc: u64,
        service_id: u64,
        call: NativeLibcCall,
    ) -> Result<u64, HleError> {
        super::native::pre_service_stop(
            &self.plan.hle,
            &mut self.tls.memory,
            &mut self.budget,
            syscall_pc,
            next_pc,
            service_id,
            call,
        )
    }
}

/// Opt in using the same admitted plan/limits as sparse combined preparation.
/// Exact typed writes precede final permissions and TLS template copying;
/// generated provider bytes are attested on these same paused native pages.
/// No native execution, host FS, firmware TCB/DTV or console startup ABI follows.
/// The source plan must remain alive for the prepared owner:
///
/// ```compile_fail
/// use pentax_core::memory::native::NativeLoadLimits;
/// use pentax_core::process::hle::{HleStartupPlan, PreparedNativeHleStartup, prepare_native_hle_startup};
/// use pentax_core::process::tls::StaticTlsStorage;
/// fn detach(plan: HleStartupPlan<'static>) -> PreparedNativeHleStartup<'static, 'static> {
///     prepare_native_hle_startup(&plan, &StaticTlsStorage { thread_pointers: &[] },
///         &NativeLoadLimits::default()).unwrap()
/// }
/// ```
pub fn prepare_native_hle_startup<'plan, 'image>(
    plan: &'plan HleStartupPlan<'image>,
    storage: &StaticTlsStorage<'_>,
    limits: &NativeLoadLimits,
) -> Result<PreparedNativeHleStartup<'plan, 'image>, HleError> {
    prepare_native_startup_in(plan, storage, limits, None)
}

pub(super) fn prepare_native_startup_in<'plan, 'image>(
    plan: &'plan HleStartupPlan<'image>,
    storage: &StaticTlsStorage<'_>,
    limits: &NativeLoadLimits,
    space: Option<NativeAddressSpace>,
) -> Result<PreparedNativeHleStartup<'plan, 'image>, HleError> {
    let retained = plan.limits();
    let bindings = plan_tls_bindings(plan.link_plan(), &retained.tls_bindings)
        .map_err(HleError::TlsBinding)?;
    let layout =
        plan_relocated_static_tls(&bindings, &retained.static_tls).map_err(HleError::Tls)?;
    let mut tls = crate::process::tls::prepare_native_tls_in(&layout, storage, limits, space)
        .map_err(HleError::Tls)?;
    attest(&tls.memory.pause().map_err(HleError::Memory)?, &plan.hle)?;
    Ok(PreparedNativeHleStartup {
        plan,
        tls,
        budget: ServiceBudget::new(plan.service_limits()).with_provider(plan.provider_bias()),
    })
}
