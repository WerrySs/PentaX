// SPDX-License-Identifier: GPL-2.0-or-later

//! Opt-in composition of typed static TLS and source-bound HLE providers.
//!
//! Binding and layout plans borrow one retained ordinary link plan. They are
//! derived locally in each phase, so no self-reference, copied authority table
//! or caller-selected patch values enter the preparation API. Recomputations
//! each enforce the retained limits independently. No PS5 TCB/DTV is defined.

use super::session::{HleSessionPlan, attest, plan_session};
use super::{GeneratedProviders, HleError, HleLimits, MemoryServiceLimits, ProviderPlacement};
use crate::memory::GuestMemory;
use crate::process::SessionLoadLimits;
use crate::process::tls::{
    PreparedTlsSession, StaticTlsLimits, StaticTlsStorage, load_relocated_static_tls,
    plan_relocated_static_tls,
};
use pentax_linker::{ModuleInput, SymbolicLinkPlan, TlsBindingLimits, plan_tls_bindings};

/// Explicit admission of exact generic AMD64 TLS bindings and PentaX's static
/// Variant II input-order layout. This is not a firmware startup ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HleStartupPolicy {
    GenericAmd64StaticTlsV1,
}

/// Retained, independent bounds for all three planning/preparation components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HleStartupLimits {
    pub hle: HleLimits,
    pub tls_bindings: TlsBindingLimits,
    pub static_tls: StaticTlsLimits,
}

/// Exact generated-provider plan with an explicit, retained TLS admission.
/// There is no constructor from detached link, layout, patch or site metadata.
#[derive(Debug)]
pub struct HleStartupPlan<'image> {
    pub(super) hle: HleSessionPlan<'image>,
    limits: HleStartupLimits,
    policy: HleStartupPolicy,
    tls_bindings: usize,
}

impl<'image> HleStartupPlan<'image> {
    pub const fn policy(&self) -> HleStartupPolicy {
        self.policy
    }
    pub const fn limits(&self) -> HleStartupLimits {
        self.limits
    }
    pub const fn link_plan(&self) -> &SymbolicLinkPlan<'image> {
        self.hle.link_plan()
    }
    pub const fn provider_index(&self) -> usize {
        self.hle.provider_index()
    }
    pub const fn provider_bias(&self) -> u64 {
        self.hle.provider_bias()
    }
    pub const fn provider_bindings(&self) -> usize {
        self.hle.provider_bindings()
    }
    pub const fn tls_bindings(&self) -> usize {
        self.tls_bindings
    }
    pub const fn service_limits(&self) -> MemoryServiceLimits {
        self.hle.service_limits()
    }
}

/// Read-only preparation retaining the actual owned TLS session, templates,
/// independent instances and guarded storage alongside HLE attestation.
/// No public mutable memory, interpreter or native conversion is provided.
#[derive(Debug)]
pub struct PreparedHleStartup {
    pub(super) tls: PreparedTlsSession,
    pub(super) sites: [super::session::AttestedSite; super::KnownFunction::ALL.len()],
    services: MemoryServiceLimits,
    policy: HleStartupPolicy,
    provider: usize,
    bias: u64,
    bindings: usize,
    tls_bindings: usize,
}

impl PreparedHleStartup {
    pub fn memory(&self) -> &GuestMemory {
        self.tls.memory()
    }
    pub fn primary_entry(&self) -> u64 {
        self.tls.primary_entry()
    }
    pub fn module_count(&self) -> usize {
        self.tls.module_count()
    }
    pub fn applied_relocations(&self) -> usize {
        self.tls.applied_relocations()
    }
    pub const fn tls(&self) -> &PreparedTlsSession {
        &self.tls
    }
    pub const fn policy(&self) -> HleStartupPolicy {
        self.policy
    }
    pub const fn provider_index(&self) -> usize {
        self.provider
    }
    pub const fn provider_bias(&self) -> u64 {
        self.bias
    }
    pub const fn provider_bindings(&self) -> usize {
        self.bindings
    }
    pub const fn tls_bindings(&self) -> usize {
        self.tls_bindings
    }
    pub const fn service_limits(&self) -> MemoryServiceLimits {
        self.services
    }
}

/// Plan exact supplied images and the factory's provider, then validate every
/// deferred record through the typed TLS binder and static layout policy.
/// Unknown remaining records, unsupported TLS writes and bounds fail before a
/// plan is published. Ordinary [`super::plan_hle_session`] still refuses TLS.
///
/// # Errors
/// Returns ordinary HLE planning, typed TLS binding or layout failures.
pub fn plan_hle_startup<'image>(
    inputs: &[ModuleInput<'image>],
    providers: &'image GeneratedProviders,
    placement: ProviderPlacement,
    limits: &HleStartupLimits,
) -> Result<HleStartupPlan<'image>, HleError> {
    let hle = plan_session(inputs, providers, placement, &limits.hle, true)?;
    let count = {
        let bindings = plan_tls_bindings(hle.link_plan(), &limits.tls_bindings)
            .map_err(HleError::TlsBinding)?;
        let _layout =
            plan_relocated_static_tls(&bindings, &limits.static_tls).map_err(HleError::Tls)?;
        bindings.bindings().len()
    };
    Ok(HleStartupPlan {
        hle,
        limits: *limits,
        policy: HleStartupPolicy::GenericAmd64StaticTlsV1,
        tls_bindings: count,
    })
}

/// Materialize this exact plan with its retained policies and budgets, apply
/// ordinary and typed TLS writes before copying initialized TLS templates into
/// independent guarded blocks, then attest the generated RX provider in place.
/// Temporary borrowing plans never outlive this call; the returned owner keeps
/// the actual [`PreparedTlsSession`]. Any failure drops unpublished memory.
///
/// # Errors
/// Returns typed binding, TLS storage/staging or provider attestation failures.
pub fn prepare_hle_startup(
    plan: &HleStartupPlan<'_>,
    storage: &StaticTlsStorage<'_>,
    limits: &SessionLoadLimits,
) -> Result<PreparedHleStartup, HleError> {
    let bindings = plan_tls_bindings(plan.link_plan(), &plan.limits.tls_bindings)
        .map_err(HleError::TlsBinding)?;
    let layout =
        plan_relocated_static_tls(&bindings, &plan.limits.static_tls).map_err(HleError::Tls)?;
    let tls = load_relocated_static_tls(&layout, storage, limits).map_err(HleError::Tls)?;
    attest(tls.memory(), &plan.hle)?;
    Ok(PreparedHleStartup {
        tls,
        sites: *plan.hle.sites(),
        services: plan.service_limits(),
        policy: plan.policy,
        provider: plan.provider_index(),
        bias: plan.provider_bias(),
        bindings: plan.provider_bindings(),
        tls_bindings: bindings.bindings().len(),
    })
}
