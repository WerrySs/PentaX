// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: source/owner-bound once callback admission,
// prepared-call identity and actual-return evidence before DONE publication.
// Adapted from pinned Posix::pthread_once; see docs/upstream-provenance.md.

use super::{
    KernelStartupError, NativeServiceAction, PreparedNativeKernelStartup, PreparedNativeSource,
    charge,
};
use crate::process::hle::kernel_adapter::{KernelMemory, error};
use pentax_kernel::{AdmittedPthreadOnce, Failure, PthreadOnceAction};
use pentax_linker::{SourceIdentity, SymbolicLinkPlan};
use pentax_platform::native_memory::{NativeMemoryError, NativeOwnerId, NativeRange};
use std::fmt;

const MAX_PENDING: usize = 32;
/// Admitted once initializer, never evidence of invocation or completion.
/// Only this prepared owner can mint it from an attested service request.
///
/// ```compile_fail
/// use pentax_core::process::hle::PreparedPthreadOnce;
/// fn retarget(value: PreparedPthreadOnce) -> PreparedPthreadOnce {
///     PreparedPthreadOnce { serial: 1, ..value }
/// }
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PreparedPthreadOnce {
    owner: NativeOwnerId,
    source: SourceIdentity,
    serial: u64,
    request: AdmittedPthreadOnce,
    pc: u64,
    id: u64,
    args: [u64; 6],
    thread_pointer: u64,
}
impl PreparedPthreadOnce {
    pub const fn entry(&self) -> u64 {
        self.request.entry()
    }
}
impl fmt::Debug for PreparedPthreadOnce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedPthreadOnce")
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
struct Pending {
    start: PreparedPthreadOnce,
    phase: Phase,
}
pub(super) struct OnceState {
    pending: Vec<Pending>,
    next_serial: u64,
}
fn invalid(name: &'static str) -> KernelStartupError {
    KernelStartupError::InvalidProvider(name)
}
impl OnceState {
    pub(super) fn new() -> Result<Self, KernelStartupError> {
        let mut pending = Vec::new();
        pending
            .try_reserve_exact(MAX_PENDING)
            .map_err(|_| KernelStartupError::Allocation)?;
        Ok(Self {
            pending,
            next_serial: 1,
        })
    }
    pub(super) fn prepare(
        &mut self,
        memory: &mut KernelMemory<'_, '_, '_>,
        action: PthreadOnceAction,
        pc: u64,
        id: u64,
        args: [u64; 6],
    ) -> Result<NativeServiceAction, KernelStartupError> {
        let request = match action {
            PthreadOnceAction::Returned(value) => return Ok(NativeServiceAction::Returned(value)),
            PthreadOnceAction::Initialize(request) => request,
        };
        let source = memory.source.ok_or(invalid("pthread once source"))?;
        let maximum = memory
            .lookup_limits
            .ok_or(invalid("pthread once work"))?
            .max_work;
        let work = memory
            .lookup_work
            .as_deref_mut()
            .ok_or(invalid("pthread once work"))?;
        charge(work, self.pending.len() as u64 + 16, maximum)?;
        if self.pending.len() >= MAX_PENDING {
            return Err(KernelStartupError::Budget("pthread once callbacks"));
        }
        if self
            .pending
            .iter()
            .any(|pending| pending.start.request.control() == request.control())
        {
            return Err(invalid("active pthread once control"));
        }
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
                    .ok_or(invalid("pthread once target"))?;
                if start <= request.entry()
                    && request.entry() - start < segment.file_bytes.len() as u64
                {
                    if found {
                        return Err(invalid("ambiguous pthread once target"));
                    }
                    found = true;
                }
            }
        }
        if !found {
            return Err(invalid("pthread once target"));
        }
        memory
            .paused
            .preflight_fetch_source(request.entry(), 1)
            .map_err(|e| KernelStartupError::Kernel(Failure::Guest(error(e, false))))?;
        if !matches!(
            memory.paused.preflight_write(request.entry(), 1),
            Err(NativeMemoryError::PermissionDenied(_))
        ) {
            return Err(invalid("writable pthread once target"));
        }
        let serial = self.next_serial;
        let next = serial
            .checked_add(1)
            .ok_or(KernelStartupError::Budget("pthread once serials"))?;
        let start = PreparedPthreadOnce {
            owner: memory.paused.owner_id(),
            source: source.identity(),
            serial,
            request,
            pc,
            id,
            args,
            thread_pointer: memory
                .thread_tls
                .ok_or(invalid("pthread once thread context"))?
                .thread_pointer,
        };
        self.pending.push(Pending {
            start,
            phase: Phase::Admitted,
        });
        self.next_serial = next;
        Ok(NativeServiceAction::Once(start))
    }
    fn index(&self, start: PreparedPthreadOnce) -> Result<usize, KernelStartupError> {
        self.pending
            .iter()
            .position(|pending| pending.start == start)
            .ok_or(invalid("pthread once ticket"))
    }
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests;

impl PreparedNativeKernelStartup<'_, '_> {
    fn once_state_write(
        &mut self,
        source: &SymbolicLinkPlan<'_>,
        request: &AdmittedPthreadOnce,
        complete: bool,
    ) -> Result<(), KernelStartupError> {
        let (mut paused, threads) = self.hle.pause_with_tls().map_err(KernelStartupError::Hle)?;
        let mut memory = KernelMemory {
            paused: &mut paused,
            source: Some(source),
            supplied_modules: self.plan.kernel_provider_index(),
            lookup_limits: None,
            lookup_work: None,
            loaded_modules: None,
            thread_tls: threads.get(self.active_tls),
            initial_entry: if self.active_tls == 0 {
                self.initial_entry
            } else {
                None
            },
        };
        if complete {
            self.kernel.complete_pthread_once(request, &mut memory)
        } else {
            self.kernel.begin_pthread_once(request, &mut memory)
        }
        .map_err(KernelStartupError::Kernel)
    }

    /// Bind the actual prepared nested frame before writing IN_PROGRESS.
    /// This admits a call but never claims its execution or return.
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub fn observe_pthread_once_call(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        start: PreparedPthreadOnce,
        call: &pentax_platform::native_dispatch::NativeNestedCall,
    ) -> Result<(), KernelStartupError> {
        let source = current.materialized_source().source_plan();
        if self
            .tls()
            .instances()
            .get(self.active_tls)
            .is_none_or(|thread| thread.thread_pointer != start.thread_pointer)
        {
            return Err(invalid("pthread once thread context"));
        }
        let parent = call.parent_syscall();
        if start.owner != self.owner_id()
            || current.owner_id() != start.owner
            || call.owner_id() != start.owner
            || source.identity() != self.memory().active_source_identity()
            || source.identity() != start.source
            || call.source_identity() != start.source
            || call.entry() != start.entry()
            || call.args() != [0; 3]
            || parent.syscall_pc() != start.pc
            || parent.service_id() != start.id
            || parent.args() != start.args
        {
            return Err(invalid("pthread once call identity"));
        }
        charge(
            &mut self.provider_work,
            self.once.pending.len() as u64 + 24,
            self.plan.limits.max_provider_work,
        )?;
        let index = self.once.index(start)?;
        if !matches!(self.once.pending[index].phase, Phase::Admitted) {
            return Err(invalid("pthread once call replay"));
        }
        if let Err(error) = self.once_state_write(source, &start.request, false) {
            self.once.pending[index].phase = Phase::Failed;
            return Err(error);
        }
        let stack = call.stack();
        self.once.pending[index].phase = Phase::Running(CallStamp {
            executor: call.executor_id(),
            call: call.call_sequence(),
            stack: stack.stack,
            guards: stack.guards,
            slot: stack.return_slot,
            marker: stack.return_marker,
        });
        Ok(())
    }
    /// Publish DONE only after an opaque actual return from this exact call.
    /// A void initializer's numeric result is deliberately ignored. Completion
    /// may follow admitted source append while retaining the same execution.
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub fn complete_pthread_once_call(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        start: PreparedPthreadOnce,
        returned: &pentax_platform::native_dispatch::NativeCallReturn<'_, '_>,
    ) -> Result<u64, KernelStartupError> {
        let source = current.materialized_source().source_plan();
        if self
            .tls()
            .instances()
            .get(self.active_tls)
            .is_none_or(|thread| thread.thread_pointer != start.thread_pointer)
        {
            return Err(invalid("pthread once thread context"));
        }
        if start.owner != self.owner_id()
            || current.owner_id() != start.owner
            || returned.owner_id() != start.owner
            || source.identity() != self.memory().active_source_identity()
            || returned.source_plan().identity() != source.identity()
        {
            return Err(invalid("pthread once return source"));
        }
        charge(
            &mut self.provider_work,
            self.once.pending.len() as u64 + 24,
            self.plan.limits.max_provider_work,
        )?;
        let index = self.once.index(start)?;
        let Phase::Running(call) = self.once.pending[index].phase else {
            return Err(invalid("pthread once return replay"));
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
            || [initial.gpr[7], initial.gpr[6], initial.gpr[2]] != [0; 3]
        {
            return Err(invalid("pthread once return identity"));
        }
        if let Err(error) = self.once_state_write(source, &start.request, true) {
            self.once.pending[index].phase = Phase::Failed;
            return Err(error);
        }
        self.once.pending.swap_remove(index);
        Ok(0)
    }
}
