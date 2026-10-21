// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owner-bound NetCtl callback admission and actual
// return evidence before publishing the event and selecting the next slot.
// Adapted from pinned NetCtl::NetCtlCheckCallback; see upstream provenance.

use super::{
    KernelStartupError, NativeServiceAction, PreparedNativeKernelStartup, PreparedNativeSource,
    charge,
};
use crate::process::hle::kernel_adapter::{KernelMemory, error};
use pentax_kernel::{AdmittedNetCtlCallback, Failure, NetCtlCallbackAction};
use pentax_linker::SourceIdentity;
use pentax_platform::native_memory::{NativeMemoryError, NativeOwnerId, NativeRange};
use std::fmt;

/// One callback admitted for an exact pending NetCtlCheckCallback syscall.
/// This token never proves execution; only an owner-attested actual return can
/// complete its event and admit the next callback in the fixed eight-slot scan.
///
/// ```compile_fail
/// use pentax_core::process::hle::PreparedNetCtlCallback;
/// fn retarget(value: PreparedNetCtlCallback) -> PreparedNetCtlCallback {
///     PreparedNetCtlCallback { serial: 1, ..value }
/// }
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PreparedNetCtlCallback {
    owner: NativeOwnerId,
    source: SourceIdentity,
    serial: u64,
    request: AdmittedNetCtlCallback,
    pc: u64,
    id: u64,
    args: [u64; 6],
    thread_pointer: u64,
}
impl PreparedNetCtlCallback {
    pub const fn entry(&self) -> u64 {
        self.request.entry()
    }
    pub const fn args(&self) -> [u64; 3] {
        self.request.args()
    }
}
impl fmt::Debug for PreparedNetCtlCallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedNetCtlCallback")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy)]
struct CallStamp {
    executor: u64,
    call: u64,
    stack: NativeRange,
    guards: Option<[NativeRange; 2]>,
    slot: u64,
    marker: u64,
}
#[derive(Clone, Copy)]
enum Phase {
    Admitted,
    Running(CallStamp),
    Failed,
}
#[derive(Clone, Copy)]
struct Pending {
    start: PreparedNetCtlCallback,
    phase: Phase,
}
pub(super) struct NetCtlCallbackState {
    // The kernel admits at most one live iteration. It scans a fixed eight
    // slots, so chaining cannot grow this owner state or its native call stack.
    pending: Option<Pending>,
    next_serial: u64,
}
fn invalid(name: &'static str) -> KernelStartupError {
    KernelStartupError::InvalidProvider(name)
}
impl NetCtlCallbackState {
    pub(super) const fn new() -> Self {
        Self {
            pending: None,
            next_serial: 1,
        }
    }
    pub(super) fn prepare(
        &mut self,
        memory: &mut KernelMemory<'_, '_, '_>,
        action: NetCtlCallbackAction,
        pc: u64,
        id: u64,
        args: [u64; 6],
    ) -> Result<NativeServiceAction, KernelStartupError> {
        if self.pending.is_some() {
            return Err(invalid("active NetCtl callback"));
        }
        let request = match action {
            NetCtlCallbackAction::Returned(value) => {
                return Ok(NativeServiceAction::Returned(value));
            }
            NetCtlCallbackAction::Call(request) => request,
        };
        let source = memory.source.ok_or(invalid("NetCtl callback source"))?;
        let maximum = memory
            .lookup_limits
            .ok_or(invalid("NetCtl callback work"))?
            .max_work;
        let work = memory
            .lookup_work
            .as_deref_mut()
            .ok_or(invalid("NetCtl callback work"))?;
        charge(work, 16, maximum)?;
        let mut found = false;
        for module in source.modules() {
            charge(work, 1, maximum)?;
            for segment in &module.runtime_plan().segments {
                charge(work, 1, maximum)?;
                if !segment.flags.executable() || segment.flags.writable() {
                    continue;
                }
                let start = module
                    .load_bias()
                    .checked_add(segment.vaddr)
                    .ok_or(invalid("NetCtl callback target"))?;
                if start <= request.entry()
                    && request.entry() - start < segment.file_bytes.len() as u64
                {
                    if found {
                        return Err(invalid("ambiguous NetCtl callback target"));
                    }
                    found = true;
                }
            }
        }
        if !found {
            return Err(invalid("NetCtl callback target"));
        }
        memory
            .paused
            .preflight_fetch_source(request.entry(), 1)
            .map_err(|e| KernelStartupError::Kernel(Failure::Guest(error(e, false))))?;
        if !matches!(
            memory.paused.preflight_write(request.entry(), 1),
            Err(NativeMemoryError::PermissionDenied(_))
        ) {
            return Err(invalid("writable NetCtl callback target"));
        }
        let serial = self.next_serial;
        let next = serial
            .checked_add(1)
            .ok_or(KernelStartupError::Budget("NetCtl callback serials"))?;
        let start = PreparedNetCtlCallback {
            owner: memory.paused.owner_id(),
            source: source.identity(),
            serial,
            request,
            pc,
            id,
            args,
            thread_pointer: memory
                .thread_tls
                .ok_or(invalid("NetCtl callback thread context"))?
                .thread_pointer,
        };
        self.pending = Some(Pending {
            start,
            phase: Phase::Admitted,
        });
        self.next_serial = next;
        Ok(NativeServiceAction::NetCtl(start))
    }
    fn pending(&self, start: PreparedNetCtlCallback) -> Result<Pending, KernelStartupError> {
        self.pending
            .filter(|pending| pending.start == start)
            .ok_or(invalid("NetCtl callback ticket"))
    }
}

impl PreparedNativeKernelStartup<'_, '_> {
    /// Bind the exact prepared native frame to the admitted callback without
    /// marking its event or claiming that any callback instruction executed.
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub fn observe_netctl_callback_call(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        start: PreparedNetCtlCallback,
        call: &pentax_platform::native_dispatch::NativeNestedCall,
    ) -> Result<(), KernelStartupError> {
        let source = current.materialized_source().source_plan();
        if self
            .tls()
            .instances()
            .get(self.active_tls)
            .is_none_or(|thread| thread.thread_pointer != start.thread_pointer)
        {
            return Err(invalid("NetCtl callback thread context"));
        }
        let parent = call.parent_syscall();
        if start.owner != self.owner_id()
            || current.owner_id() != start.owner
            || call.owner_id() != start.owner
            || source.identity() != self.memory().active_source_identity()
            || source.identity() != start.source
            || call.source_identity() != start.source
            || call.entry() != start.entry()
            || call.args() != start.args()
            || parent.syscall_pc() != start.pc
            || parent.service_id() != start.id
            || parent.args() != start.args
        {
            return Err(invalid("NetCtl callback call identity"));
        }
        charge(
            &mut self.provider_work,
            24,
            self.plan.limits.max_provider_work,
        )?;
        if !matches!(self.netctl_callbacks.pending(start)?.phase, Phase::Admitted) {
            return Err(invalid("NetCtl callback call replay"));
        }
        let stack = call.stack();
        self.netctl_callbacks.pending = Some(Pending {
            start,
            phase: Phase::Running(CallStamp {
                executor: call.executor_id(),
                call: call.call_sequence(),
                stack: stack.stack,
                guards: stack.guards,
                slot: stack.return_slot,
                marker: stack.return_marker,
            }),
        });
        Ok(())
    }

    /// Mark this slot's event only after the exact actual callback return;
    /// then inspect later live slots. A void callback's RAX is ignored. A
    /// source append during the callback is allowed only with the same owner
    /// and the currently active materialization, as for module initializers.
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub fn complete_netctl_callback_call(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        start: PreparedNetCtlCallback,
        returned: &pentax_platform::native_dispatch::NativeCallReturn<'_, '_>,
    ) -> Result<NativeServiceAction, KernelStartupError> {
        let source = current.materialized_source().source_plan();
        if self
            .tls()
            .instances()
            .get(self.active_tls)
            .is_none_or(|thread| thread.thread_pointer != start.thread_pointer)
        {
            return Err(invalid("NetCtl callback thread context"));
        }
        if start.owner != self.owner_id()
            || current.owner_id() != start.owner
            || returned.owner_id() != start.owner
            || source.identity() != self.memory().active_source_identity()
            || returned.source_plan().identity() != source.identity()
        {
            return Err(invalid("NetCtl callback return source"));
        }
        charge(
            &mut self.provider_work,
            24,
            self.plan.limits.max_provider_work,
        )?;
        let pending = self.netctl_callbacks.pending(start)?;
        let Phase::Running(call) = pending.phase else {
            return Err(invalid("NetCtl callback return replay"));
        };
        let stack = returned.stack();
        let initial = returned.initial_state();
        if returned.executor_id() != call.executor
            || returned.call_sequence() != call.call
            || returned.entry() != start.entry()
            || stack.stack != call.stack
            || stack.guards != call.guards
            || stack.return_slot != call.slot
            || stack.return_marker != call.marker
            || [initial.gpr[7], initial.gpr[6], initial.gpr[2]] != start.args()
        {
            return Err(invalid("NetCtl callback return identity"));
        }
        // A kernel/target failure cannot leave a reusable completed ticket.
        self.netctl_callbacks.pending = Some(Pending {
            phase: Phase::Failed,
            ..pending
        });
        let maximum = self.plan.limits.provider_call_maximum(self.provider_work)?;
        let action = self
            .kernel
            .complete_netctl_callback(&start.request)
            .map_err(KernelStartupError::Kernel)?;
        let (mut paused, threads) = self.hle.pause_with_tls().map_err(KernelStartupError::Hle)?;
        let mut memory = KernelMemory {
            paused: &mut paused,
            source: Some(source),
            supplied_modules: self.plan.kernel_provider_index(),
            lookup_limits: Some(pentax_linker::ModuleLookupLimits {
                symbolic: self.plan.limits.startup.hle.linking,
                max_name_bytes: self.plan.limits.kernel.max_name_bytes,
                max_work: maximum,
            }),
            lookup_work: Some(&mut self.provider_work),
            loaded_modules: None,
            thread_tls: threads.get(self.active_tls),
            initial_entry: None,
        };
        self.netctl_callbacks.pending = None;
        let prepared =
            self.netctl_callbacks
                .prepare(&mut memory, action, start.pc, start.id, start.args);
        if prepared.is_err() {
            self.netctl_callbacks.pending = Some(Pending {
                phase: Phase::Failed,
                ..pending
            });
        }
        prepared
    }
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests;
