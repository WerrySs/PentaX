// SPDX-License-Identifier: GPL-2.0-only

//! Prepared TLS context selection over one retained native memory owner.

use super::{KernelStartupError, PreparedNativeKernelStartup};
use crate::process::hle::HleError;
use crate::process::tls::ThreadControlBlockPolicy;
use pentax_linker::SymbolicLinkPlan;
use pentax_platform::native_memory::{NativeMemoryError, NativeOwnerId};

/// Core-created thread context for an exact source and native owner.
/// It records preparation history plus admitted live header state, not ongoing
/// liveness, firmware ABI compatibility or authority to execute instructions.
/// Its provenance cannot be constructed from caller-supplied numeric metadata:
///
/// ```compile_fail
/// use pentax_core::process::hle::PreparedNativeThread;
/// use pentax_linker::SymbolicLinkPlan;
/// use pentax_platform::native_memory::NativeOwnerId;
/// fn forge<'p,'i>(owner:NativeOwnerId,source:&'p SymbolicLinkPlan<'i>) -> PreparedNativeThread<'p,'i> {
///     PreparedNativeThread {owner,source,thread_pointer:0x80000}
/// }
/// ```
#[derive(Debug)]
pub struct PreparedNativeThread<'plan, 'image> {
    owner: NativeOwnerId,
    source: &'plan SymbolicLinkPlan<'image>,
    thread_pointer: u64,
    pub(super) index: usize,
}
impl<'plan, 'image> PreparedNativeThread<'plan, 'image> {
    pub const fn owner_id(&self) -> NativeOwnerId {
        self.owner
    }
    pub const fn source_plan(&self) -> &'plan SymbolicLinkPlan<'image> {
        self.source
    }
    pub const fn thread_pointer(&self) -> u64 {
        self.thread_pointer
    }
}

impl<'plan, 'image> PreparedNativeKernelStartup<'plan, 'image> {
    /// Effective class of the already selected guest TLS context. The native
    /// scheduler uses this allocation-free observation for its next turn; it
    /// neither changes host scheduling nor counts as another guest service.
    pub fn active_thread_priority_class(&self) -> pentax_kernel::ThreadPriorityClass {
        self.kernel.active_thread_priority_class()
    }

    /// Admit the first prepared TLS instance as this session's main-thread
    /// context under the explicitly selected observed self-pointer profile.
    /// No host segment register or scheduler state changes. Other instances
    /// remain independent prepared storage, not invented running threads.
    pub fn prepare_main_thread(
        &mut self,
    ) -> Result<PreparedNativeThread<'plan, 'image>, KernelStartupError> {
        let thread = self.prepare_thread_context(0)?;
        self.activate_thread_context(&thread)?;
        Ok(thread)
    }

    /// Admit one already materialized, initially zero TLS instance. This does
    /// not allocate a stack, create a thread, start code or change CPU registers.
    pub fn prepare_thread_context(
        &mut self,
        index: usize,
    ) -> Result<PreparedNativeThread<'plan, 'image>, KernelStartupError> {
        if self.tls().thread_control_policy() != ThreadControlBlockPolicy::KytyObservedSelfPointerV1
        {
            return Err(KernelStartupError::InvalidProvider(
                "main thread requires explicit TCB profile",
            ));
        }
        // Header read + full comparison, three permission checks and fixed
        // source/geometry visits are admitted before touching live memory.
        if self.plan.hle.limits().static_tls.max_work_steps < 160 {
            return Err(KernelStartupError::Budget("thread context"));
        }
        let source: &'plan SymbolicLinkPlan<'image> = self.plan.link_plan();
        if !std::ptr::eq(source, self.memory().source_plan()) {
            return Err(KernelStartupError::InvalidProvider(
                "thread source identity",
            ));
        }
        let instance =
            self.tls()
                .instances()
                .get(index)
                .ok_or(KernelStartupError::InvalidProvider(
                    "missing main TLS instance",
                ))?;
        let header = instance
            .thread_control_block
            .ok_or(KernelStartupError::InvalidProvider("missing thread header"))?;
        if header.address != instance.thread_pointer || header.size != 64 {
            return Err(KernelStartupError::InvalidProvider(
                "thread header geometry",
            ));
        }
        let mut bytes = [0; 64];
        let paused = self.pause()?;
        paused
            .preflight_write(header.address, header.size)
            .map_err(|e| {
                KernelStartupError::Hle(HleError::Memory(crate::memory::MemoryError::Native(e)))
            })?;
        if !matches!(
            paused.preflight_fetch_source(header.address, 1),
            Err(NativeMemoryError::PermissionDenied(_))
        ) {
            return Err(KernelStartupError::InvalidProvider(
                "thread header must be NX",
            ));
        }
        paused.read(header.address, &mut bytes).map_err(|e| {
            KernelStartupError::Hle(HleError::Memory(crate::memory::MemoryError::Native(e)))
        })?;
        if bytes[..8] != header.address.to_le_bytes() || bytes[8..].iter().any(|value| *value != 0)
        {
            return Err(KernelStartupError::InvalidProvider(
                "initial thread header bytes",
            ));
        }
        Ok(PreparedNativeThread {
            owner: paused.owner_id(),
            source,
            thread_pointer: header.address,
            index,
        })
    }

    /// Select kernel TLS, identity, errno and key values together. The executor
    /// must select the matching saved CPU/stack before running its next block.
    /// This token conveys preparation history, never authority to run code.
    pub fn activate_thread_context(
        &mut self,
        thread: &PreparedNativeThread<'plan, 'image>,
    ) -> Result<(), KernelStartupError> {
        if thread.owner != self.owner_id()
            || !std::ptr::eq(thread.source, self.plan.link_plan())
            || self
                .tls()
                .instances()
                .get(thread.index)
                .is_none_or(|instance| instance.thread_pointer != thread.thread_pointer)
        {
            return Err(KernelStartupError::InvalidProvider(
                "thread selection provenance",
            ));
        }
        {
            let paused = self.pause()?;
            paused
                .preflight_write(thread.thread_pointer, 64)
                .map_err(|error| {
                    KernelStartupError::Kernel(pentax_kernel::Failure::Guest(
                        super::super::kernel_adapter::error(error, true),
                    ))
                })?;
            paused
                .preflight_non_executable(thread.thread_pointer, 64)
                .map_err(|error| {
                    KernelStartupError::Kernel(pentax_kernel::Failure::Guest(
                        super::super::kernel_adapter::error(error, false),
                    ))
                })?;
            let mut pointer = [0; 8];
            paused
                .read(thread.thread_pointer, &mut pointer)
                .map_err(|error| {
                    KernelStartupError::Kernel(pentax_kernel::Failure::Guest(
                        super::super::kernel_adapter::error(error, false),
                    ))
                })?;
            if pointer != thread.thread_pointer.to_le_bytes() {
                return Err(KernelStartupError::InvalidProvider(
                    "thread selection header",
                ));
            }
        }
        self.kernel
            .activate_thread_context(thread.thread_pointer)
            .map_err(KernelStartupError::Kernel)?;
        self.active_tls = thread.index;
        Ok(())
    }
}
