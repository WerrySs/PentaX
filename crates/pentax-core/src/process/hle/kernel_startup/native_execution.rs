// SPDX-License-Identifier: GPL-2.0-only

//! Prepared-owner provenance and checked PRE-SYSCALL service requests.
//! No native executor, raw pointer or independently supplied TLS value is used.

use super::{
    KernelServiceStop, KernelStartupError, KernelStartupPlan, NativeServiceAction,
    PreparedNativeKernelStartup,
};
use crate::process::hle::HleError;
use crate::process::tls::plan_relocated_static_tls;
use pentax_codegen::{
    BlockLimits, DeclaredAppliedWrite, MaterializedSource, declare_materialized_source,
};
use pentax_kernel::CallArgs;
use pentax_linker::{SymbolicLinkPlan, plan_tls_bindings};
use pentax_platform::native_memory::{NativeOwnerId, PausedNativeMemory};

/// History of complete typed materialization for one exact prepared owner.
/// This can only be built from that owner's retained startup plan. It borrows
/// the immutable plan rather than the mutable session, so service dispatch and
/// exclusive pauses remain possible while the provenance is retained.
///
/// It proves preparation history, not that writable guest bytes stay unchanged,
/// that every future memory operand stays inside the owner, or that an owner is
/// still live. A native consumer must compare the actual owner, attest current
/// source bytes on every publication, and honor its unsafe execution contract.
/// Caller-declared values cannot be promoted into this provenance type:
///
/// ```compile_fail
/// use pentax_codegen::MaterializedSource;
/// use pentax_core::process::hle::PreparedNativeSource;
/// use pentax_platform::native_memory::NativeOwnerId;
/// fn forge<'plan, 'image>(owner_id: NativeOwnerId,
///     materialized: MaterializedSource<'plan, 'image>) -> PreparedNativeSource<'plan, 'image> {
///     PreparedNativeSource { owner_id, materialized }
/// }
/// ```
#[derive(Debug)]
pub struct PreparedNativeSource<'plan, 'image> {
    owner_id: NativeOwnerId,
    materialized: MaterializedSource<'plan, 'image>,
}

impl<'plan, 'image> PreparedNativeSource<'plan, 'image> {
    pub const fn owner_id(&self) -> NativeOwnerId {
        self.owner_id
    }
    pub const fn materialized_source(&self) -> &MaterializedSource<'plan, 'image> {
        &self.materialized
    }
}

/// Request data captured before a private library-call marker's SYSCALL.
/// Public fields are not evidence that an instruction ran. Every request is
/// checked against the prepared owner, exact site, full ID, live bytes and
/// retained budgets. Arguments are RDI, RSI, RDX, original RCX, R8 and R9;
/// neither a synthetic continuation nor R10 replaces argument four.
#[derive(Debug, Clone, Copy)]
pub struct NativeServiceRequest {
    pub owner_id: NativeOwnerId,
    pub syscall_pc: u64,
    pub next_pc: u64,
    pub service_id: u64,
    pub args: [u64; 6],
    /// Raw XMM0..7 captured at the same PRE-SYSCALL stop. Legacy direct
    /// requests explicitly omit these registers; omission is not a zero value.
    pub vector_args: Option<[[u8; 16]; 8]>,
    /// Callee RSP captured at the unchanged library marker; absent for legacy requests.
    pub stack_pointer: Option<u64>,
}

impl<'plan, 'image> PreparedNativeKernelStartup<'plan, 'image> {
    pub fn owner_id(&self) -> NativeOwnerId {
        self.memory().owner_id()
    }

    /// Exclusive access to the actual prepared native backing. No running view
    /// or host pointer is exposed; a poisoned owner cannot be paused again.
    pub fn pause(&mut self) -> Result<PausedNativeMemory<'_>, KernelStartupError> {
        self.hle.pause().map_err(KernelStartupError::Hle)
    }

    /// Derive materialized-source provenance without accepting caller TLS
    /// values. Binding/layout recomputation each obeys the original retained
    /// TLS limits. `limits` independently bounds declaration construction and
    /// the pure codegen declaration phase, including their arena capacities.
    /// Nothing returned here is an execution or continuous-liveness capability.
    pub fn prepare_native_source(
        &mut self,
        limits: &BlockLimits,
    ) -> Result<PreparedNativeSource<'plan, 'image>, KernelStartupError> {
        let source: &'plan SymbolicLinkPlan<'image> = self.plan.link_plan();
        self.prepare_epoch_native_source(source, limits)
    }

    /// Derive the current immutable epoch's declarations from this actual owner
    /// and its retained TLS, after successful append publication. A private
    /// source identity must match the backing's active history, and complete
    /// applied counts/templates must match; a caller-supplied numeric ID or
    /// detached materialization cannot substitute. No guest code executes here.
    pub fn prepare_epoch_native_source<'current, 'input>(
        &mut self,
        source: &'current SymbolicLinkPlan<'input>,
        limits: &BlockLimits,
    ) -> Result<PreparedNativeSource<'current, 'input>, KernelStartupError> {
        let owner_id = self.pause()?.owner_id();
        let retained: &'plan KernelStartupPlan<'image> = self.plan;
        if self.memory().active_source_identity() != source.identity() {
            return Err(KernelStartupError::InvalidProvider(
                "active source epoch required",
            ));
        }
        if !std::ptr::eq(retained.link_plan(), self.memory().source_plan())
            || source.modules().len() != self.memory().module_count()
            || !retained
                .link_plan()
                .deferred()
                .iter()
                .eq(source.deferred().iter().filter(|record| {
                    record.module().index() < retained.link_plan().modules().len()
                }))
        {
            return Err(KernelStartupError::InvalidProvider(
                "native source identity",
            ));
        }
        let count = source.deferred_count();
        let patches = source
            .patch_count()
            .checked_add(count)
            .ok_or(KernelStartupError::Budget("source declarations"))?;
        if source.modules().len() as u64 > limits.max_modules
            || patches as u64 > limits.max_patches
            || count as u64 > limits.max_work_units
        {
            return Err(KernelStartupError::Budget("source declarations"));
        }
        super::unavailable_runtime::published::validate_published(self, source)?;
        if let Some(binding) = retained.json {
            super::json_runtime::validate(
                source,
                &binding,
                &retained.limits,
                &mut self.provider_work,
            )?;
            let paused = self.hle.pause().map_err(KernelStartupError::Hle)?;
            super::json_runtime::attest_all(
                &paused,
                &binding,
                &mut self.provider_work,
                retained.limits.max_provider_work,
            )?;
        }
        let bytes = count
            .checked_mul(std::mem::size_of::<DeclaredAppliedWrite>())
            .ok_or(KernelStartupError::Budget("source declaration bytes"))?;
        if bytes as u64 > limits.max_allocation_bytes {
            return Err(KernelStartupError::Budget("source declaration bytes"));
        }
        let admitted = retained.hle.limits();
        let bindings = plan_tls_bindings(source, &admitted.tls_bindings)
            .map_err(|e| KernelStartupError::Hle(HleError::TlsBinding(e)))?;
        let layout = plan_relocated_static_tls(&bindings, &admitted.static_tls)
            .map_err(|e| KernelStartupError::Hle(HleError::Tls(e)))?;
        if (count as u64)
            .checked_add(layout.templates().len() as u64)
            .is_none_or(|work| work > limits.max_work_units)
        {
            return Err(KernelStartupError::Budget("source declaration work"));
        }
        if layout.tls_patches().len() != count
            || self.memory().applied_relocations() != patches
            || self.tls().templates() != layout.templates()
        {
            return Err(KernelStartupError::InvalidProvider(
                "native materialization history",
            ));
        }
        let mut writes = Vec::new();
        writes
            .try_reserve_exact(count)
            .map_err(|_| KernelStartupError::Allocation)?;
        if writes
            .capacity()
            .checked_mul(std::mem::size_of::<DeclaredAppliedWrite>())
            .is_none_or(|bytes| bytes as u64 > limits.max_allocation_bytes)
        {
            return Err(KernelStartupError::Budget("source declaration bytes"));
        }
        for patch in layout.tls_patches() {
            let binding = bindings.bindings().get(patch.binding_index()).ok_or(
                KernelStartupError::InvalidProvider("native TLS binding index"),
            )?;
            writes.push(DeclaredAppliedWrite {
                deferred_index: binding.deferred_index(),
                consumer: patch.consumer(),
                source: patch.source(),
                address: patch.address(),
                value: patch.value(),
            });
        }
        let materialized = declare_materialized_source(source, &writes, limits)
            .map_err(KernelStartupError::Codegen)?;
        Ok(PreparedNativeSource {
            owner_id,
            materialized,
        })
    }

    pub const fn native_service_requests(&self) -> u64 {
        self.native_service_requests
    }

    /// Service one PRE-SYSCALL observation. Only an admitted library result may
    /// complete a pending token, including explicitly supported source errno
    /// return domains. Other failures retain their types and cannot become
    /// generic success or guessed errno. The caller synthesizes marker RCX/R11
    /// only after this admission.
    pub fn service_request(
        &mut self,
        request: NativeServiceRequest,
    ) -> Result<u64, KernelStartupError> {
        self.last_admitted_kernel_service = None;
        let source = self.plan.link_plan();
        if source.identity() != self.memory().active_source_identity() {
            return Err(KernelStartupError::InvalidProvider(
                "stale initial service source",
            ));
        }
        match self.service_for(source, request, false)? {
            NativeServiceAction::Returned(value) => Ok(value),
            NativeServiceAction::Load(_)
            | NativeServiceAction::Once(_)
            | NativeServiceAction::NetCtl(_)
            | NativeServiceAction::FontParser(_)
            | NativeServiceAction::ImeCallback(_)
            | NativeServiceAction::CreateThread(_)
            | NativeServiceAction::WaitThread(_) => {
                Err(KernelStartupError::InvalidProvider("pending module load"))
            }
        }
    }

    /// Admit one request against the actual active materialized epoch. A new
    /// file request suspends execution; it does not synthesize a guest return.
    pub fn service_action(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        request: NativeServiceRequest,
    ) -> Result<NativeServiceAction, KernelStartupError> {
        self.last_admitted_kernel_service = None;
        if current.owner_id() != self.owner_id()
            || current.materialized_source().source_plan().identity()
                != self.memory().active_source_identity()
        {
            return Err(KernelStartupError::InvalidProvider(
                "stale native service source",
            ));
        }
        self.service_for(current.materialized_source().source_plan(), request, true)
    }

    fn service_for(
        &mut self,
        source: &SymbolicLinkPlan<'_>,
        request: NativeServiceRequest,
        actions: bool,
    ) -> Result<NativeServiceAction, KernelStartupError> {
        let maximum = self
            .plan
            .limits
            .kernel
            .max_attempts
            .checked_add(self.plan.hle.service_limits().max_attempts)
            .ok_or(KernelStartupError::Budget("native request attempts"))?;
        self.native_service_requests = self
            .native_service_requests
            .checked_add(1)
            .filter(|n| *n <= maximum)
            .ok_or(KernelStartupError::Budget("native request attempts"))?;
        if request.owner_id != self.owner_id() {
            return Err(KernelStartupError::NativeOwnerMismatch);
        }
        if request.syscall_pc.checked_add(2) != Some(request.next_pc) {
            return Err(KernelStartupError::ContinuationMismatch {
                syscall_pc: request.syscall_pc,
                next_pc: request.next_pc,
            });
        }
        if let Some(value) =
            super::unavailable_runtime::published::route_stop(self, source, request)?
        {
            return Ok(NativeServiceAction::Returned(value));
        }
        if self
            .plan
            .json
            .as_ref()
            .is_some_and(|binding| binding.contains(request.syscall_pc))
        {
            return super::json_runtime::stop(self, source, request)
                .map(NativeServiceAction::Returned);
        }
        if self
            .plan
            .graphics
            .as_ref()
            .is_some_and(|binding| binding.contains(request.syscall_pc))
        {
            return super::graphics_runtime::stop(self, request).map(NativeServiceAction::Returned);
        }
        if self
            .plan
            .unavailable
            .as_ref()
            .is_some_and(|binding| binding.contains(request.syscall_pc))
        {
            return super::unavailable_runtime::stop(self, request)
                .map(NativeServiceAction::Returned);
        }
        let libc = self.plan.hle.provider_bias();
        if libc <= request.syscall_pc && request.syscall_pc < libc + crate::memory::PAGE_SIZE {
            self.hle
                .pre_service_stop(
                    request.syscall_pc,
                    request.next_pc,
                    request.service_id,
                    super::super::native::NativeLibcCall {
                        args: request.args,
                        vector_args: request.vector_args,
                        stack_pointer: request.stack_pointer,
                    },
                )
                .map(NativeServiceAction::Returned)
                .map_err(KernelStartupError::Hle)
        } else {
            self.kernel_stop_for_with_vectors(
                source,
                KernelServiceStop {
                    syscall_pc: request.syscall_pc,
                    service_id: request.service_id,
                    args: CallArgs(request.args),
                },
                actions,
                request.stack_pointer,
                request.vector_args,
            )
        }
    }
}
