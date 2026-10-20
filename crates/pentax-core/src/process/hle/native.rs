// SPDX-License-Identifier: GPL-2.0-only

//! Source-bound HLE over the actual owned paused native pages. Service stops
//! remain crate-private until the native dispatcher/context boundary exists.

use super::dispatch::{AbiArgs, ServiceBudget, VariadicContext, invoke, invoke_with_variadic};
use super::session::attest;
use super::{HleError, HleSessionPlan};
use crate::memory::native::{NativeGuestMemory, NativeLoadLimits, prepare_in};
use pentax_platform::native_memory::{NativeAddressSpace, PausedNativeMemory};

#[derive(Debug)]
pub struct PreparedNativeHleSession<'plan, 'image> {
    plan: &'plan HleSessionPlan<'image>,
    memory: NativeGuestMemory<'plan, 'image>,
    // Kept even before a native dispatcher is integrated. No public call can
    // reset the retained per-session service budget or replace provider sites.
    budget: ServiceBudget,
}
impl<'plan, 'image> PreparedNativeHleSession<'plan, 'image> {
    pub fn memory(&self) -> &NativeGuestMemory<'plan, 'image> {
        &self.memory
    }
    pub fn pause(&mut self) -> Result<PausedNativeMemory<'_>, HleError> {
        self.memory.pause().map_err(HleError::Memory)
    }
    pub const fn plan(&self) -> &'plan HleSessionPlan<'image> {
        self.plan
    }
    pub fn service_attempts(&self) -> u64 {
        self.budget.attempts()
    }
    pub fn completed_services(&self) -> u64 {
        self.budget.completed()
    }
    #[allow(dead_code)]
    pub(crate) fn service_stop(&mut self, stop: NativeServiceStop) -> Result<u64, HleError> {
        service_stop(self.plan, &mut self.memory, &mut self.budget, stop)
    }
}

pub(super) fn service_stop(
    plan: &HleSessionPlan<'_>,
    memory: &mut NativeGuestMemory<'_, '_>,
    budget: &mut ServiceBudget,
    stop: NativeServiceStop,
) -> Result<u64, HleError> {
    let attempt = budget.begin_attempt().map_err(HleError::Service)?;
    let function = super::gateway::attested_function(plan.sites(), stop.rip, stop.rcx, stop.rax)?;
    let mut paused = memory.pause().map_err(HleError::Memory)?;
    invoke(
        attempt,
        function,
        AbiArgs {
            rdi: stop.rdi,
            rsi: stop.rsi,
            rdx: stop.rdx,
        },
        &mut paused,
        budget,
    )
    .map(|value| value.rax)
    .map_err(HleError::Service)
}

/// The native block runner stops BEFORE SYSCALL. Its original argument words
/// are already captured; no hardware clobber or synthetic RCX is consulted.
#[derive(Clone, Copy)]
pub(super) struct NativeLibcCall {
    pub args: [u64; 6],
    pub vector_args: Option<[[u8; 16]; 8]>,
    pub stack_pointer: Option<u64>,
}

pub(super) fn pre_service_stop(
    plan: &HleSessionPlan<'_>,
    memory: &mut NativeGuestMemory<'_, '_>,
    budget: &mut ServiceBudget,
    syscall_pc: u64,
    next_pc: u64,
    service_id: u64,
    call: NativeLibcCall,
) -> Result<u64, HleError> {
    let attempt = budget.begin_attempt().map_err(HleError::Service)?;
    let site = plan
        .sites()
        .iter()
        .find(|site| site.trap == syscall_pc)
        .ok_or(HleError::SiteMismatch { rip: syscall_pc })?;
    if next_pc != site.resume {
        return Err(HleError::ContinuationMismatch {
            rip: site.resume,
            rcx: next_pc,
        });
    }
    let expected_id = site.function.service_id();
    if service_id != u64::from(expected_id) {
        return Err(HleError::ServiceIdMismatch {
            expected: expected_id,
            actual: service_id,
        });
    }
    budget
        .charge_native_attestation()
        .map_err(HleError::Service)?;
    let mut paused = memory.pause().map_err(HleError::Memory)?;
    let mut actual = [0; super::provider::STUB_SIZE as usize];
    paused
        .fetch_source(site.entry, &mut actual)
        .map_err(|_| HleError::InvalidProvider("native libc marker is not executable"))?;
    if actual != super::provider::stub(site.function) {
        return Err(HleError::InvalidProvider("native libc marker bytes"));
    }
    if paused.read(plan.provider_bias(), &mut [0]).is_err()
        || !matches!(
            paused.preflight_write(plan.provider_bias(), 1),
            Err(pentax_platform::native_memory::NativeMemoryError::PermissionDenied(_))
        )
    {
        return Err(HleError::InvalidProvider(
            "native libc provider lost immutable permissions",
        ));
    }
    invoke_with_variadic(
        attempt,
        site.function,
        AbiArgs {
            rdi: call.args[0],
            rsi: call.args[1],
            rdx: call.args[2],
        },
        &mut paused,
        budget,
        Some(VariadicContext {
            gp: [call.args[3], call.args[4], call.args[5]],
            vectors: call.vector_args,
            stack_pointer: call.stack_pointer,
        }),
    )
    .map(|result| result.rax)
    .map_err(HleError::Service)
}

#[derive(Clone, Copy)]
pub(crate) struct NativeServiceStop {
    pub rip: u64,
    pub rcx: u64,
    pub rax: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
}

/// Prepare exact ordinary HLE sources directly into native pages and attest the
/// generated provider there. All pages remain OS NX; no executor is returned.
pub fn prepare_native_hle_session<'plan, 'image>(
    plan: &'plan HleSessionPlan<'image>,
    limits: &NativeLoadLimits,
) -> Result<PreparedNativeHleSession<'plan, 'image>, HleError> {
    prepare_native_in(plan, limits, None)
}
fn prepare_native_in<'plan, 'image>(
    plan: &'plan HleSessionPlan<'image>,
    limits: &NativeLoadLimits,
    space: Option<NativeAddressSpace>,
) -> Result<PreparedNativeHleSession<'plan, 'image>, HleError> {
    let mut memory = prepare_in(plan.link_plan(), limits, space).map_err(HleError::Staging)?;
    attest(&memory.pause().map_err(HleError::Memory)?, plan)?;
    Ok(PreparedNativeHleSession {
        plan,
        memory,
        budget: ServiceBudget::new(plan.service_limits()).with_provider(plan.provider_bias()),
    })
}

#[cfg(test)]
mod tests;
