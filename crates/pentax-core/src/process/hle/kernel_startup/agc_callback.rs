// SPDX-License-Identifier: GPL-2.0-only
//! Real source-bound AGC allocator callbacks. A ticket is never execution proof.

use super::{
    KernelStartupError, NativeServiceAction, NativeServiceRequest, PreparedNativeKernelStartup,
    PreparedNativeSource, charge,
};
use crate::gpu::driver::{CommandBuilderAction, PendingCommandBuilder};
use pentax_linker::{SourceIdentity, SymbolicLinkPlan};
use pentax_platform::native_memory::{
    NativeMemoryError, NativeOwnerId, NativeRange, PausedNativeMemory,
};
use std::fmt;

const MAX_PENDING: usize = 32;
// Preserve the original aggregate payload envelope of thirty-two maximum
// WriteData continuations. New range/group bodies share this same closed cap.
// Fixed Pending metadata remains separately bounded by MAX_PENDING.
const MAX_RETAINED_PAYLOAD_BYTES: usize = MAX_PENDING * 65_524;

/// One admitted allocator callback. Only its actual nested native return can
/// complete the suspended builder; copying this token cannot replay a body.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PreparedAgcCommandBufferCallback {
    owner: NativeOwnerId,
    source: SourceIdentity,
    serial: u64,
    entry: u64,
    callback_args: [u64; 3],
    pc: u64,
    id: u64,
    args: [u64; 6],
    parent_rsp: u64,
    thread_pointer: u64,
    thread_index: usize,
}
impl PreparedAgcCommandBufferCallback {
    pub const fn entry(&self) -> u64 {
        self.entry
    }
    pub const fn args(&self) -> [u64; 3] {
        self.callback_args
    }
}
impl fmt::Debug for PreparedAgcCommandBufferCallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedAgcCommandBufferCallback")
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
    fs: Option<u64>,
    gs: Option<u64>,
}
#[derive(Clone, Copy)]
enum Phase {
    Admitted,
    Running(CallStamp),
    Failed,
}
struct Pending {
    start: PreparedAgcCommandBufferCallback,
    phase: Phase,
    body: Option<PendingCommandBuilder>,
}
#[derive(Clone, Copy)]
struct Origin {
    owner: NativeOwnerId,
    pc: u64,
    id: u64,
    args: [u64; 6],
    parent_rsp: Option<u64>,
    thread_pointer: Option<u64>,
    thread_index: usize,
}
pub(super) struct AgcCallbackState {
    pending: Vec<Pending>,
    next_serial: u64,
}
fn invalid(label: &'static str) -> KernelStartupError {
    KernelStartupError::InvalidProvider(label)
}
impl AgcCallbackState {
    pub(super) fn new() -> Result<Self, KernelStartupError> {
        let mut pending = Vec::new();
        pending
            .try_reserve_exact(MAX_PENDING)
            .map_err(|_| KernelStartupError::Allocation)?;
        if pending.capacity() > MAX_PENDING {
            return Err(KernelStartupError::Budget("AGC callback capacity"));
        }
        Ok(Self {
            pending,
            next_serial: 1,
        })
    }
    pub(super) fn prepare(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        source: &SymbolicLinkPlan<'_>,
        action: CommandBuilderAction,
        request: NativeServiceRequest,
        thread: (Option<u64>, usize),
        budget: (&mut u64, u64),
    ) -> Result<NativeServiceAction, KernelStartupError> {
        let body = match action {
            CommandBuilderAction::Returned(value) => {
                return Ok(NativeServiceAction::Returned(value));
            }
            CommandBuilderAction::Grow(body) => body,
        };
        self.admit_body(
            memory,
            source,
            body,
            Origin {
                owner: request.owner_id,
                pc: request.syscall_pc,
                id: request.service_id,
                args: request.args,
                parent_rsp: request.stack_pointer,
                thread_pointer: thread.0,
                thread_index: thread.1,
            },
            budget,
            None,
        )
    }
    fn admit_body(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        source: &SymbolicLinkPlan<'_>,
        body: PendingCommandBuilder,
        origin: Origin,
        budget: (&mut u64, u64),
        reuse: Option<usize>,
    ) -> Result<NativeServiceAction, KernelStartupError> {
        let (work, maximum) = budget;
        charge(
            work,
            MAX_PENDING as u64 * std::mem::size_of::<Pending>() as u64 + 32,
            maximum,
        )?;
        if reuse.is_none() && self.pending.len() == MAX_PENDING {
            return Err(KernelStartupError::Budget("AGC pending callbacks"));
        }
        if let Some(index) = reuse
            && self.pending.get(index).is_none_or(|pending| {
                !matches!(pending.phase, Phase::Failed) || pending.body.is_some()
            })
        {
            return Err(invalid("AGC callback continuation slot"));
        }
        if body.owner() != memory.owner_id() || body.owner() != origin.owner {
            return Err(KernelStartupError::NativeOwnerMismatch);
        }
        // Sequential grows reuse a consumed slot whose previous body was
        // already taken. Count actual capacities of every remaining body,
        // including nested admissions, before retaining this new continuation.
        let mut payload_bytes = body
            .retained_payload_bytes()
            .ok_or(KernelStartupError::Budget("AGC retained callback payload"))?;
        for pending in &self.pending {
            if let Some(body) = &pending.body {
                payload_bytes = payload_bytes
                    .checked_add(
                        body.retained_payload_bytes()
                            .ok_or(KernelStartupError::Budget("AGC retained callback payload"))?,
                    )
                    .ok_or(KernelStartupError::Budget("AGC retained callback payload"))?;
            }
        }
        if payload_bytes > MAX_RETAINED_PAYLOAD_BYTES {
            return Err(KernelStartupError::Budget("AGC retained callback payload"));
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
                    .ok_or(invalid("AGC callback target"))?;
                if start <= body.entry() && body.entry() - start < segment.file_bytes.len() as u64 {
                    if found {
                        return Err(invalid("ambiguous AGC callback target"));
                    }
                    found = true;
                }
            }
        }
        if !found {
            return Err(invalid("AGC callback target"));
        }
        memory
            .preflight_fetch_source(body.entry(), 1)
            .map_err(|_| invalid("AGC callback fetch"))?;
        if !matches!(
            memory.preflight_write(body.entry(), 1),
            Err(NativeMemoryError::PermissionDenied(_))
        ) {
            return Err(invalid("writable AGC callback target"));
        }
        let next = self
            .next_serial
            .checked_add(1)
            .ok_or(KernelStartupError::Budget("AGC callback serials"))?;
        let start = PreparedAgcCommandBufferCallback {
            owner: body.owner(),
            source: source.identity(),
            serial: self.next_serial,
            entry: body.entry(),
            callback_args: body.args(),
            pc: origin.pc,
            id: origin.id,
            args: origin.args,
            parent_rsp: origin
                .parent_rsp
                .ok_or(invalid("AGC callback parent stack"))?,
            thread_pointer: origin
                .thread_pointer
                .ok_or(invalid("AGC callback thread context"))?,
            thread_index: origin.thread_index,
        };
        let pending = Pending {
            start,
            phase: Phase::Admitted,
            body: Some(body),
        };
        if let Some(index) = reuse {
            self.pending[index] = pending;
        } else {
            self.pending.push(pending);
        }
        self.next_serial = next;
        Ok(NativeServiceAction::AgcCommandGrow(start))
    }
    fn index(&self, start: PreparedAgcCommandBufferCallback) -> Result<usize, KernelStartupError> {
        self.pending
            .iter()
            .position(|p| p.start == start)
            .ok_or(invalid("AGC callback ticket"))
    }
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
impl PreparedNativeKernelStartup<'_, '_> {
    pub fn observe_agc_command_buffer_call(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        start: PreparedAgcCommandBufferCallback,
        call: &pentax_platform::native_dispatch::NativeNestedCall,
    ) -> Result<(), KernelStartupError> {
        let source = current.materialized_source().source_plan();
        let expected_fs = if self.tls().thread_control_policy()
            == crate::process::tls::ThreadControlBlockPolicy::None
        {
            None
        } else {
            Some(start.thread_pointer)
        };
        let parent = call.parent_syscall();
        if start.owner != self.owner_id()
            || current.owner_id() != start.owner
            || call.owner_id() != start.owner
            || source.identity() != self.memory().active_source_identity()
            || source.identity() != start.source
            || call.source_identity() != start.source
            || call.entry() != start.entry
            || call.args() != start.args()
            || parent.syscall_pc() != start.pc
            || parent.service_id() != start.id
            || parent.args() != start.args
            || parent.stack_pointer() != start.parent_rsp
            || call.parent_stack_pointer() != start.parent_rsp
            || call.parent_fs_base() != expected_fs
            || self.active_tls != start.thread_index
            || self
                .tls()
                .instances()
                .get(self.active_tls)
                .is_none_or(|tls| tls.thread_pointer != start.thread_pointer)
        {
            return Err(invalid("AGC callback call identity"));
        }
        charge(
            &mut self.provider_work,
            MAX_PENDING as u64 + 32,
            self.plan.limits.max_provider_work,
        )?;
        let index = self.agc_callbacks.index(start)?;
        if !matches!(self.agc_callbacks.pending[index].phase, Phase::Admitted) {
            return Err(invalid("AGC callback call replay"));
        }
        let stack = call.stack();
        if stack
            .return_slot
            .checked_add(8)
            .is_none_or(|end| end > start.parent_rsp)
            || stack.stack.address > stack.return_slot
            || stack
                .stack
                .address
                .checked_add(stack.stack.size)
                .is_none_or(|end| {
                    start
                        .parent_rsp
                        .checked_add(8)
                        .is_none_or(|parent| parent > end)
                })
        {
            return Err(invalid("AGC callback stack identity"));
        }
        self.agc_callbacks.pending[index].phase = Phase::Running(CallStamp {
            executor: call.executor_id(),
            call: call.call_sequence(),
            stack: stack.stack,
            guards: stack.guards,
            slot: stack.return_slot,
            marker: stack.return_marker,
            fs: call.parent_fs_base(),
            gs: call.parent_gs_base(),
        });
        Ok(())
    }

    pub fn complete_agc_command_buffer_call(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        start: PreparedAgcCommandBufferCallback,
        returned: &pentax_platform::native_dispatch::NativeCallReturn<'_, '_>,
        nested: pentax_platform::native_dispatch::NativeNestedCall,
        engine: &pentax_platform::native_dispatch::NativeBlockStepper<'_, '_, '_>,
    ) -> Result<NativeServiceAction, KernelStartupError> {
        engine
            .validate_syscall_call_return(nested, returned)
            .map_err(|_| invalid("AGC callback executor return"))?;
        let index = self.agc_callbacks.index(start)?;
        let Phase::Running(call) = self.agc_callbacks.pending[index].phase else {
            return Err(invalid("AGC callback return replay"));
        };
        let stack = returned.stack();
        let initial = returned.initial_state();
        if returned.owner_id() != start.owner
            || returned.source_plan().identity() != start.source
            || returned.executor_id() != call.executor
            || returned.call_sequence() != call.call
            || returned.entry() != start.entry
            || stack.stack != call.stack
            || stack.guards != call.guards
            || stack.return_slot != call.slot
            || stack.return_marker != call.marker
            || initial.gpr[4] != call.slot
            || initial.fs_base != call.fs
            || initial.gs_base != call.gs
            || [initial.gpr[7], initial.gpr[6], initial.gpr[2]] != start.args()
        {
            return Err(invalid("AGC callback return identity"));
        }
        // The genuine receipt is consumed before all completion-time fallible
        // source, TLS, quota, descriptor, backing or payload checks. No replay.
        self.agc_callbacks.pending[index].phase = Phase::Failed;
        let body = self.agc_callbacks.pending[index]
            .body
            .take()
            .ok_or(invalid("AGC callback body"))?;
        let source = current.materialized_source().source_plan();
        if current.owner_id() != start.owner
            || self.owner_id() != start.owner
            || source.identity() != start.source
            || source.identity() != self.memory().active_source_identity()
            || self.active_tls != start.thread_index
            || self
                .tls()
                .instances()
                .get(self.active_tls)
                .is_none_or(|tls| tls.thread_pointer != start.thread_pointer)
        {
            return Err(invalid("AGC callback completion source"));
        }
        charge(
            &mut self.provider_work,
            MAX_PENDING as u64 + 32,
            self.plan.limits.max_provider_work,
        )?;
        let mut paused = self.hle.pause().map_err(KernelStartupError::Hle)?;
        let action = self
            .graphics
            .as_mut()
            .ok_or(invalid("missing graphics state"))?
            .complete_command_builder(&mut paused, body, returned.value())
            .map_err(KernelStartupError::Graphics)?;
        match action {
            CommandBuilderAction::Returned(value) => {
                self.agc_callbacks.pending.swap_remove(index);
                Ok(NativeServiceAction::Returned(value))
            }
            CommandBuilderAction::Grow(body) => self.agc_callbacks.admit_body(
                &mut paused,
                source,
                body,
                Origin {
                    owner: start.owner,
                    pc: start.pc,
                    id: start.id,
                    args: start.args,
                    parent_rsp: Some(start.parent_rsp),
                    thread_pointer: Some(start.thread_pointer),
                    thread_index: start.thread_index,
                },
                (&mut self.provider_work, self.plan.limits.max_provider_work),
                Some(index),
            ),
        }
    }
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests;
