// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owner-bound offline NP snapshots, nested calls
// and current native return receipts before continuing a snapshot.
// Adapted from pinned NpManager::NpCheckCallback; see upstream provenance.

use super::{
    KernelStartupError, NativeServiceAction, PreparedNativeKernelStartup, PreparedNativeSource,
    charge,
};
use crate::process::hle::kernel_adapter::{KernelMemory, error};
use pentax_kernel::{AdmittedNpCallback, Failure, NpCallbackAction};
use pentax_linker::SourceIdentity;
use pentax_platform::native_memory::{NativeMemoryError, NativeOwnerId, NativeRange};
use std::fmt;

/// One callback admitted for an exact pending NpCheckCallback syscall.
/// This token never proves execution; only an owner-attested actual return can
/// continue the immutable snapshot, without rescanning new registrations.
///
/// ```compile_fail
/// use pentax_core::process::hle::PreparedNpCallback;
/// fn retarget(value: PreparedNpCallback) -> PreparedNpCallback {
///     PreparedNpCallback { serial: 1, ..value }
/// }
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PreparedNpCallback {
    owner: NativeOwnerId,
    source: SourceIdentity,
    serial: u64,
    request: AdmittedNpCallback,
    pc: u64,
    id: u64,
    args: [u64; 6],
    thread_pointer: u64,
    parent_stack_pointer: u64,
}
impl PreparedNpCallback {
    pub const fn entry(&self) -> u64 {
        self.request.entry()
    }
    pub const fn args(&self) -> [u64; 3] {
        self.request.args()
    }
}
impl fmt::Debug for PreparedNpCallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedNpCallback").finish_non_exhaustive()
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
    parent_stack_pointer: u64,
    parent_fs_base: Option<u64>,
    parent_gs_base: Option<u64>,
}
#[derive(Clone, Copy)]
enum Phase {
    Admitted,
    Running(CallStamp),
    Failed,
}
#[derive(Clone, Copy)]
struct Pending {
    start: PreparedNpCallback,
    phase: Phase,
}
pub(super) struct NpCallbackState {
    // Existing native callback-frame bound; independent rows preserve nested
    // and cooperative-thread checks without replacing an outer ticket.
    pending: [Option<Pending>; 32],
    next_serial: u64,
}
fn invalid(name: &'static str) -> KernelStartupError {
    KernelStartupError::InvalidProvider(name)
}
fn stack_identity(start: PreparedNpCallback, call: CallStamp) -> Result<(), KernelStartupError> {
    let stack_end = call
        .stack
        .address
        .checked_add(call.stack.size)
        .ok_or(invalid("NP stack range"))?;
    let after_slot = call.slot.checked_add(8).ok_or(invalid("NP return slot"))?;
    let parent_end = call
        .parent_stack_pointer
        .checked_add(8)
        .ok_or(invalid("NP parent stack range"))?;
    if start.parent_stack_pointer != call.parent_stack_pointer
        || call.stack.address > call.slot
        || after_slot > call.parent_stack_pointer
        || parent_end > stack_end
    {
        return Err(invalid("NP caller stack identity"));
    }
    Ok(())
}

impl NpCallbackState {
    pub(super) const fn new() -> Self {
        Self {
            pending: [None; 32],
            next_serial: 1,
        }
    }
    pub(super) fn prepare(
        &mut self,
        memory: &mut KernelMemory<'_, '_, '_>,
        action: NpCallbackAction,
        pc: u64,
        id: u64,
        args: [u64; 6],
        stack_pointer: Option<u64>,
    ) -> Result<NativeServiceAction, KernelStartupError> {
        let request = match action {
            NpCallbackAction::Returned(value) => {
                return Ok(NativeServiceAction::Returned(value));
            }
            NpCallbackAction::Call(request) => request,
        };
        let parent_stack_pointer = stack_pointer.ok_or(invalid("NP callback parent stack"))?;
        let source = memory.source.ok_or(invalid("Np callback source"))?;
        let maximum = memory
            .lookup_limits
            .ok_or(invalid("Np callback work"))?
            .max_work;
        let work = memory
            .lookup_work
            .as_deref_mut()
            .ok_or(invalid("Np callback work"))?;
        charge(work, 48, maximum)?;
        let slot = self
            .pending
            .iter()
            .position(Option::is_none)
            .ok_or(KernelStartupError::Budget("NP callback frames"))?;
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
                    .ok_or(invalid("Np callback target"))?;
                if start <= request.entry()
                    && request.entry() - start < segment.file_bytes.len() as u64
                {
                    if found {
                        return Err(invalid("ambiguous Np callback target"));
                    }
                    found = true;
                }
            }
        }
        if !found {
            return Err(invalid("Np callback target"));
        }
        memory
            .paused
            .preflight_fetch_source(request.entry(), 1)
            .map_err(|e| KernelStartupError::Kernel(Failure::Guest(error(e, false))))?;
        if !matches!(
            memory.paused.preflight_write(request.entry(), 1),
            Err(NativeMemoryError::PermissionDenied(_))
        ) {
            return Err(invalid("writable Np callback target"));
        }
        let serial = self.next_serial;
        let next = serial
            .checked_add(1)
            .ok_or(KernelStartupError::Budget("Np callback serials"))?;
        let start = PreparedNpCallback {
            owner: memory.paused.owner_id(),
            source: source.identity(),
            serial,
            request,
            pc,
            id,
            args,
            parent_stack_pointer,
            thread_pointer: memory
                .thread_tls
                .ok_or(invalid("Np callback thread context"))?
                .thread_pointer,
        };
        self.pending[slot] = Some(Pending {
            start,
            phase: Phase::Admitted,
        });
        self.next_serial = next;
        Ok(NativeServiceAction::Np(start))
    }
    fn pending(&self, start: PreparedNpCallback) -> Result<Pending, KernelStartupError> {
        self.pending
            .iter()
            .flatten()
            .find(|pending| pending.start == start)
            .copied()
            .ok_or(invalid("NP callback ticket"))
    }
}

impl PreparedNativeKernelStartup<'_, '_> {
    /// Cancel only the exact still-retained ticket. This revocation proves no
    /// execution and deliberately requires no additional exhausted work quota.
    pub fn abort_np_callback_call(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        start: PreparedNpCallback,
    ) -> Result<(), KernelStartupError> {
        if start.owner != self.owner_id()
            || current.owner_id() != start.owner
            || current.materialized_source().source_plan().identity()
                != self.memory().active_source_identity()
        {
            return Err(invalid("NP abort owner"));
        }
        let slot = self
            .np_callbacks
            .pending
            .iter()
            .position(|row| row.is_some_and(|pending| pending.start == start))
            .ok_or(invalid("NP abort ticket"))?;
        let pending = self.np_callbacks.pending[slot].ok_or(invalid("NP abort ticket"))?;
        self.kernel.fail_np_callback(&start.request);
        self.np_callbacks.pending[slot] = Some(Pending {
            phase: Phase::Failed,
            ..pending
        });
        Ok(())
    }
    /// Terminal worker cleanup of this owner's private bank, including parked
    /// thread/fiber frames. Fixed32 revocations allocate nothing and cannot fail
    /// when an admission budget has just been exhausted. No foreign row exists.
    pub fn abort_np_callbacks(&mut self) {
        for row in self.np_callbacks.pending.iter_mut().flatten() {
            self.kernel.fail_np_callback(&row.start.request);
            row.phase = Phase::Failed;
        }
    }

    /// Bind the exact prepared frame without completing this snapshot step
    /// or claiming that any callback instruction executed.
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub fn observe_np_callback_call(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        start: PreparedNpCallback,
        call: &pentax_platform::native_dispatch::NativeNestedCall,
    ) -> Result<(), KernelStartupError> {
        let source = current.materialized_source().source_plan();
        if self
            .tls()
            .instances()
            .get(self.active_tls)
            .is_none_or(|thread| thread.thread_pointer != start.thread_pointer)
        {
            return Err(invalid("Np callback thread context"));
        }
        let parent = call.parent_syscall();
        let expected_fs = if self.tls().thread_control_policy()
            == crate::process::tls::ThreadControlBlockPolicy::None
        {
            None
        } else {
            Some(start.thread_pointer)
        };
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
            || parent.stack_pointer() != start.parent_stack_pointer
            || call.parent_fs_base() != expected_fs
        {
            return Err(invalid("Np callback call identity"));
        }
        charge(
            &mut self.provider_work,
            88,
            self.plan.limits.max_provider_work,
        )?;
        if !matches!(self.np_callbacks.pending(start)?.phase, Phase::Admitted) {
            return Err(invalid("Np callback call replay"));
        }
        let stack = call.stack();
        let slot = self
            .np_callbacks
            .pending
            .iter()
            .position(|row| row.is_some_and(|value| value.start == start))
            .ok_or(invalid("NP callback ticket"))?;
        let stamp = CallStamp {
            executor: call.executor_id(),
            call: call.call_sequence(),
            stack: stack.stack,
            guards: stack.guards,
            slot: stack.return_slot,
            marker: stack.return_marker,
            parent_stack_pointer: call.parent_stack_pointer(),
            parent_fs_base: call.parent_fs_base(),
            parent_gs_base: call.parent_gs_base(),
        };
        stack_identity(start, stamp)?;
        self.np_callbacks.pending[slot] = Some(Pending {
            start,
            phase: Phase::Running(stamp),
        });
        Ok(())
    }

    /// Continue only this sealed snapshot after an actual callback return.
    /// New registrations stay outside its captured end. Void RAX is ignored. A
    /// source append during the callback is allowed only with the same owner
    /// and the currently active materialization, as for module initializers.
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub fn complete_np_callback_call(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        start: PreparedNpCallback,
        returned: &pentax_platform::native_dispatch::NativeCallReturn<'_, '_>,
        nested: pentax_platform::native_dispatch::NativeNestedCall,
        engine: &pentax_platform::native_dispatch::NativeBlockStepper<'_, '_, '_>,
    ) -> Result<NativeServiceAction, KernelStartupError> {
        let source = current.materialized_source().source_plan();
        if self
            .tls()
            .instances()
            .get(self.active_tls)
            .is_none_or(|thread| thread.thread_pointer != start.thread_pointer)
        {
            return Err(invalid("Np callback thread context"));
        }
        if start.owner != self.owner_id()
            || current.owner_id() != start.owner
            || returned.owner_id() != start.owner
            || source.identity() != self.memory().active_source_identity()
            || returned.source_plan().identity() != source.identity()
        {
            return Err(invalid("Np callback return source"));
        }
        charge(
            &mut self.provider_work,
            88,
            self.plan.limits.max_provider_work,
        )?;
        engine
            .validate_syscall_call_return(nested, returned)
            .map_err(|_| invalid("NP executor return continuation"))?;
        let pending = self.np_callbacks.pending(start)?;
        let Phase::Running(call) = pending.phase else {
            return Err(invalid("Np callback return replay"));
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
            || initial.gpr[4] != call.slot
            || initial.fs_base != call.parent_fs_base
            || initial.gs_base != call.parent_gs_base
            || [initial.gpr[7], initial.gpr[6], initial.gpr[2]] != start.args()
        {
            return Err(invalid("Np callback return identity"));
        }
        let slot = self
            .np_callbacks
            .pending
            .iter()
            .position(|row| row.is_some_and(|value| value.start == start))
            .ok_or(invalid("NP callback ticket"))?;
        stack_identity(start, call)?;
        // A kernel/target failure cannot leave a reusable completed ticket.
        self.np_callbacks.pending[slot] = Some(Pending {
            phase: Phase::Failed,
            ..pending
        });
        let outcome = (|| {
            let maximum = self.plan.limits.provider_call_maximum(self.provider_work)?;
            let (mut paused, threads) =
                self.hle.pause_with_tls().map_err(KernelStartupError::Hle)?;
            let action = match self.kernel.complete_np_callback(&start.request) {
                Ok(action) => action,
                Err(error) => {
                    self.kernel.fail_np_callback(&start.request);
                    return Err(KernelStartupError::Kernel(error));
                }
            };
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
            self.np_callbacks.pending[slot] = None;
            let prepared = self.np_callbacks.prepare(
                &mut memory,
                action,
                start.pc,
                start.id,
                start.args,
                Some(start.parent_stack_pointer),
            );
            if prepared.is_err() {
                if let NpCallbackAction::Call(request) = action {
                    self.kernel.fail_np_callback(&request);
                }
                self.np_callbacks.pending[slot] = Some(Pending {
                    phase: Phase::Failed,
                    ..pending
                });
            }
            prepared
        })();
        if outcome.is_err() {
            self.kernel.fail_np_callback(&start.request);
        }
        outcome
    }
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests;
