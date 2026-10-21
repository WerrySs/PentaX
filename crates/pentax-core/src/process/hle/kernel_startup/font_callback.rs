// SPDX-License-Identifier: GPL-2.0-only
// Modified 2026 for PentaX: owner-bound Font text-parser calls and actual-return
// evidence. Source contract: pinned libFont.cpp; see upstream provenance.

use super::{
    KernelStartupError, NativeServiceAction, PreparedNativeKernelStartup, PreparedNativeSource,
    charge,
};
use crate::process::hle::kernel_adapter::{KernelMemory, error};
use pentax_kernel::{AdmittedFontParser, Failure, FontParserAction};
use pentax_linker::SourceIdentity;
use pentax_platform::native_memory::{NativeMemoryError, NativeOwnerId, NativeRange};
use std::fmt;

const MAX_PENDING: usize = 32;

/// Admission for one parser invocation, never evidence that its body ran.
/// Every iteration receives a new kernel ticket and a new prepared serial.
///
/// ```compile_fail
/// use pentax_core::process::hle::PreparedFontParserCallback;
/// fn retarget(value: PreparedFontParserCallback) -> PreparedFontParserCallback {
///     PreparedFontParserCallback { serial: 1, ..value }
/// }
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PreparedFontParserCallback {
    owner: NativeOwnerId,
    source: SourceIdentity,
    serial: u64,
    request: AdmittedFontParser,
    pc: u64,
    id: u64,
    args: [u64; 6],
    thread_pointer: u64,
}
impl PreparedFontParserCallback {
    pub const fn entry(&self) -> u64 {
        self.request.entry()
    }
    pub const fn args(&self) -> [u64; 3] {
        self.request.args()
    }
}
impl fmt::Debug for PreparedFontParserCallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedFontParserCallback")
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
    start: PreparedFontParserCallback,
    phase: Phase,
}

pub(super) struct FontParserCallbackState {
    pending: Vec<Pending>,
    next_serial: u64,
}
fn invalid(name: &'static str) -> KernelStartupError {
    KernelStartupError::InvalidProvider(name)
}
fn scratch(request: AdmittedFontParser) -> Result<u64, KernelStartupError> {
    let start = request
        .stack_pointer()
        .checked_sub(64)
        .ok_or(invalid("font parser scratch range"))?;
    if request.args()[1] != start || request.args()[2] != start + 8 {
        return Err(invalid("font parser scratch identity"));
    }
    Ok(start)
}
fn stack_identity(request: AdmittedFontParser, call: CallStamp) -> Result<(), KernelStartupError> {
    let start = scratch(request)?;
    let end = start
        .checked_add(40)
        .ok_or(invalid("font parser scratch range"))?;
    let stack_end = call
        .stack
        .address
        .checked_add(call.stack.size)
        .ok_or(invalid("font parser stack range"))?;
    let after_slot = call
        .slot
        .checked_add(8)
        .ok_or(invalid("font parser return slot"))?;
    let parent_end = call
        .parent_stack_pointer
        .checked_add(8)
        .ok_or(invalid("font parser parent stack range"))?;
    if request.stack_pointer() != call.parent_stack_pointer
        || call.stack.address > call.slot
        || after_slot > start
        || call.stack.address > start
        || end > call.parent_stack_pointer
        || parent_end > stack_end
    {
        return Err(invalid("font parser caller red zone"));
    }
    Ok(())
}

impl FontParserCallbackState {
    pub(super) fn new() -> Result<Self, KernelStartupError> {
        let mut pending = Vec::new();
        pending
            .try_reserve_exact(MAX_PENDING)
            .map_err(|_| KernelStartupError::Allocation)?;
        if pending.capacity() > MAX_PENDING {
            return Err(KernelStartupError::Budget("font parser callback capacity"));
        }
        Ok(Self {
            pending,
            next_serial: 1,
        })
    }

    pub(super) fn prepare(
        &mut self,
        memory: &mut KernelMemory<'_, '_, '_>,
        action: FontParserAction,
        pc: u64,
        id: u64,
        args: [u64; 6],
    ) -> Result<NativeServiceAction, KernelStartupError> {
        let request = match action {
            FontParserAction::Returned(value) => return Ok(NativeServiceAction::Returned(value)),
            FontParserAction::Call(request) => request,
        };
        let source = memory.source.ok_or(invalid("font parser source"))?;
        let maximum = memory
            .lookup_limits
            .ok_or(invalid("font parser work"))?
            .max_work;
        let work = memory
            .lookup_work
            .as_deref_mut()
            .ok_or(invalid("font parser work"))?;
        charge(work, self.pending.len() as u64 + 32, maximum)?;
        if self.pending.len() >= MAX_PENDING {
            return Err(KernelStartupError::Budget("font parser callbacks"));
        }
        if self
            .pending
            .iter()
            .any(|pending| pending.start.request == request)
        {
            return Err(invalid("active font parser ticket"));
        }
        let scratch = scratch(request)?;
        memory
            .paused
            .preflight_read(scratch, 40)
            .map_err(|e| KernelStartupError::Kernel(Failure::Guest(error(e, false))))?;
        memory
            .paused
            .preflight_write(scratch, 40)
            .map_err(|e| KernelStartupError::Kernel(Failure::Guest(error(e, true))))?;
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
                    .ok_or(invalid("font parser target"))?;
                if start <= request.entry()
                    && request.entry() - start < segment.file_bytes.len() as u64
                {
                    if found {
                        return Err(invalid("ambiguous font parser target"));
                    }
                    found = true;
                }
            }
        }
        if !found {
            return Err(invalid("font parser target"));
        }
        memory
            .paused
            .preflight_fetch_source(request.entry(), 1)
            .map_err(|e| KernelStartupError::Kernel(Failure::Guest(error(e, false))))?;
        if !matches!(
            memory.paused.preflight_write(request.entry(), 1),
            Err(NativeMemoryError::PermissionDenied(_))
        ) {
            return Err(invalid("writable font parser target"));
        }
        let serial = self.next_serial;
        let next = serial
            .checked_add(1)
            .ok_or(KernelStartupError::Budget("font parser serials"))?;
        let start = PreparedFontParserCallback {
            owner: memory.paused.owner_id(),
            source: source.identity(),
            serial,
            request,
            pc,
            id,
            args,
            thread_pointer: memory
                .thread_tls
                .ok_or(invalid("font parser thread context"))?
                .thread_pointer,
        };
        self.pending.push(Pending {
            start,
            phase: Phase::Admitted,
        });
        self.next_serial = next;
        Ok(NativeServiceAction::FontParser(start))
    }

    fn index(&self, start: PreparedFontParserCallback) -> Result<usize, KernelStartupError> {
        self.pending
            .iter()
            .position(|pending| pending.start == start)
            .ok_or(invalid("font parser ticket"))
    }
}

impl PreparedNativeKernelStartup<'_, '_> {
    /// Attest one actual native executor's pending call frame and the caller's
    /// red-zone cells. No parser execution or numeric return is inferred here.
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub fn observe_font_parser_call(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        start: PreparedFontParserCallback,
        call: &pentax_platform::native_dispatch::NativeNestedCall,
    ) -> Result<(), KernelStartupError> {
        let source = current.materialized_source().source_plan();
        if self
            .tls()
            .instances()
            .get(self.active_tls)
            .is_none_or(|thread| thread.thread_pointer != start.thread_pointer)
        {
            return Err(invalid("font parser thread context"));
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
            || parent.stack_pointer() != start.request.stack_pointer()
            || call.parent_fs_base() != expected_fs
        {
            return Err(invalid("font parser call identity"));
        }
        charge(
            &mut self.provider_work,
            self.font_parsers.pending.len() as u64 + 32,
            self.plan.limits.max_provider_work,
        )?;
        let index = self.font_parsers.index(start)?;
        if !matches!(self.font_parsers.pending[index].phase, Phase::Admitted) {
            return Err(invalid("font parser call replay"));
        }
        let stack = call.stack();
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
        stack_identity(start.request, stamp)?;
        self.font_parsers.pending[index].phase = Phase::Running(stamp);
        Ok(())
    }

    /// Accept parsed cells only after the exact admitted callback actually
    /// returns. The next iteration is a new ticket on the same parent syscall.
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub fn complete_font_parser_call(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        start: PreparedFontParserCallback,
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
            return Err(invalid("font parser thread context"));
        }
        if start.owner != self.owner_id()
            || current.owner_id() != start.owner
            || returned.owner_id() != start.owner
            || source.identity() != self.memory().active_source_identity()
            || returned.source_plan().identity() != source.identity()
        {
            return Err(invalid("font parser return source"));
        }
        charge(
            &mut self.provider_work,
            self.font_parsers.pending.len() as u64 + 2048,
            self.plan.limits.max_provider_work,
        )?;
        engine
            .validate_syscall_call_return(nested, returned)
            .map_err(|_| invalid("font parser executor return continuation"))?;
        let index = self.font_parsers.index(start)?;
        let pending = self.font_parsers.pending[index];
        let Phase::Running(call) = pending.phase else {
            return Err(invalid("font parser return replay"));
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
            return Err(invalid("font parser return identity"));
        }
        stack_identity(start.request, call)?;
        // Any fallible kernel completion consumes this execution receipt.
        self.font_parsers.pending[index].phase = Phase::Failed;
        let maximum = self.plan.limits.provider_call_maximum(self.provider_work)?;
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
        let action = self
            .kernel
            .complete_font_parser(&start.request, returned.value() as u32 as u64, &mut memory)
            .map_err(KernelStartupError::Kernel)?;
        self.font_parsers.pending.swap_remove(index);
        let prepared =
            self.font_parsers
                .prepare(&mut memory, action, start.pc, start.id, start.args);
        if prepared.is_err() {
            self.font_parsers.pending.push(Pending {
                phase: Phase::Failed,
                ..pending
            });
        }
        prepared
    }
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests;
