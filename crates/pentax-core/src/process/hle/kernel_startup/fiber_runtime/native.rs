// SPDX-License-Identifier: GPL-2.0-only
//! Actual paused-owner/source binding for fiber transfer and entry completion.
use super::super::*;
use pentax_kernel::{AdmittedFiberTransfer, FiberAction, FiberKey, FiberTransferKind};
use pentax_linker::SourceIdentity;
use pentax_platform::native_dispatch::{
    NativeCallReturn, NativeFiberContext, NativeFiberOrigin, NativeStackFrame, PreSyscallToken,
};
use pentax_platform::native_memory::{
    NativeMemoryEffect, NativeMemoryTransaction, NativeMemoryWrite, NativeOwnerId,
    NativePermissions, NativeRange,
};

#[derive(Clone, Copy, Debug)]
pub struct FiberEntryLayout {
    pub entry: u64,
    pub args: [u64; 2],
    pub frame: NativeStackFrame,
}
#[derive(Clone, Copy, Debug)]
pub struct PreparedFiberAction {
    request: AdmittedFiberTransfer,
    owner: NativeOwnerId,
    source: SourceIdentity,
    pc: u64,
    id: u64,
    args: [u64; 6],
    rsp: Option<u64>,
    thread: usize,
    layout: Option<FiberEntryLayout>,
    actual_return: Option<NativeFiberOrigin>,
}
impl PreparedFiberAction {
    pub const fn kind(&self) -> FiberTransferKind {
        self.request.kind()
    }
    pub const fn from(&self) -> Option<FiberKey> {
        self.request.from()
    }
    pub const fn target(&self) -> Option<FiberKey> {
        self.request.target()
    }
    pub const fn target_layout(&self) -> Option<FiberEntryLayout> {
        self.layout
    }
}
struct Record {
    key: FiberKey,
    layout: FiberEntryLayout,
    origin: Option<NativeFiberOrigin>,
    owned: Option<NativeStackFrame>,
}
struct Root {
    thread: usize,
    token: PreSyscallToken,
}
pub(in super::super) struct FiberRuntime {
    records: Vec<Record>,
    roots: Vec<Root>,
    maximum: usize,
    maximum_roots: usize,
    pending: Option<PreparedFiberAction>,
    observed: Option<(u64, Option<NativeFiberOrigin>)>,
}
fn invalid(label: &'static str) -> KernelStartupError {
    KernelStartupError::InvalidProvider(label)
}
fn mem(error: pentax_platform::native_memory::NativeMemoryError) -> KernelStartupError {
    KernelStartupError::Hle(HleError::Memory(crate::memory::MemoryError::Native(error)))
}
fn add(a: u64, b: u64) -> Result<u64, KernelStartupError> {
    a.checked_add(b)
        .filter(|v| *v < ADDRESS_LIMIT)
        .ok_or(invalid("fiber extent"))
}
fn separation_work(left: NativeRange, right: NativeRange) -> Result<u64, KernelStartupError> {
    let pages = |r: NativeRange| {
        (r.address % PAGE_SIZE)
            .checked_add(r.size)
            .map(|v| v.div_ceil(PAGE_SIZE))
            .ok_or(invalid("fiber separation extent"))
    };
    let l = pages(left)?;
    let r = pages(right)?;
    l.checked_mul(r)
        .and_then(|v| v.checked_add(l.checked_add(r)?.checked_mul(3)?))
        .and_then(|v| v.checked_add(1))
        .ok_or(KernelStartupError::Budget("fiber separation"))
}
fn separate(
    memory: &mut KernelMemory<'_, '_, '_>,
    left: NativeRange,
    right: NativeRange,
) -> Result<(), KernelStartupError> {
    let cost = separation_work(left, right)?;
    account(memory, cost)?;
    memory
        .paused
        .preflight_disjoint_backing(left, right, cost)
        .map_err(mem)?;
    Ok(())
}
impl FiberRuntime {
    pub(in super::super) fn preflight_thread_stack(
        &self,
        paused: &PausedNativeMemory<'_>,
        stack: NativeRange,
        kernel: &Kernel,
        work: &mut u64,
        maximum: u64,
    ) -> Result<(), KernelStartupError> {
        for row in &self.records {
            if !kernel.fiber_may_resume(row.key) {
                continue;
            }
            let object = NativeRange {
                address: row.key.address(),
                size: 208,
            };
            let cost = separation_work(stack, row.layout.frame.stack)?
                .checked_add(separation_work(stack, object)?)
                .ok_or(KernelStartupError::Budget("thread/fiber separation"))?;
            charge(work, cost, maximum)?;
            paused
                .preflight_disjoint_backing(stack, object, cost)
                .map_err(mem)?;
            if paused
                .preflight_read_write_non_executable(
                    row.layout.frame.stack.address,
                    row.layout.frame.stack.size,
                )
                .is_ok()
            {
                paused
                    .preflight_disjoint_backing(stack, row.layout.frame.stack, cost)
                    .map_err(mem)?;
            }
        }
        Ok(())
    }
    pub(in super::super) fn new(
        maximum: usize,
        threads: usize,
    ) -> Result<Self, KernelStartupError> {
        let mut records = Vec::new();
        records
            .try_reserve_exact(maximum)
            .map_err(|_| KernelStartupError::Allocation)?;
        let mut roots = Vec::new();
        roots
            .try_reserve_exact(threads)
            .map_err(|_| KernelStartupError::Allocation)?;
        Ok(Self {
            records,
            roots,
            maximum,
            maximum_roots: threads,
            pending: None,
            observed: None,
        })
    }
}
fn account(memory: &mut KernelMemory<'_, '_, '_>, amount: u64) -> Result<(), KernelStartupError> {
    let max = memory
        .lookup_limits
        .ok_or(invalid("fiber work admission"))?
        .max_work;
    charge(
        memory
            .lookup_work
            .as_deref_mut()
            .ok_or(invalid("fiber work ledger"))?,
        amount,
        max,
    )
}
#[allow(clippy::too_many_arguments)]
pub(in super::super) fn prepare(
    memory: &mut KernelMemory<'_, '_, '_>,
    kernel: &mut Kernel,
    runtime: &mut FiberRuntime,
    storage: &mut pthread_create::ThreadRuntime,
    action: FiberAction,
    pc: u64,
    id: u64,
    args: [u64; 6],
    rsp: Option<u64>,
    thread: usize,
    main: Option<NativeEntryLayout>,
) -> Result<NativeServiceAction, KernelStartupError> {
    let request = match action {
        FiberAction::Returned(v) => return Ok(NativeServiceAction::Returned(v)),
        FiberAction::Transfer(r) => r,
    };
    if runtime.pending.is_some() {
        return Err(invalid("pending native fiber action"));
    }
    account(
        memory,
        runtime.records.len() as u64 + runtime.roots.len() as u64 + 32,
    )?;
    let source = memory.source.ok_or(invalid("fiber source"))?;
    let mut layout = None;
    if let Some(key) = request.target() {
        let exact = runtime.records.iter().position(|r| r.key == key);
        if exact.is_none() {
            let old = runtime
                .records
                .iter()
                .position(|r| r.key.address() == key.address());
            if old.is_none() && runtime.records.len() >= runtime.maximum {
                return Err(KernelStartupError::Budget("fiber records"));
            }
            let (entry, argument, context, size) = kernel
                .fiber_entry(key)
                .map_err(KernelStartupError::Kernel)?;
            let mut found = false;
            for module in source.modules() {
                for segment in &module.runtime_plan().segments {
                    account(memory, 1)?;
                    let begin = add(module.load_bias(), segment.vaddr)?;
                    if segment.flags.executable()
                        && !segment.flags.writable()
                        && entry >= begin
                        && entry - begin < segment.file_bytes.len() as u64
                    {
                        if found {
                            return Err(invalid("ambiguous fiber entry"));
                        }
                        found = true;
                    }
                }
            }
            if !found {
                return Err(invalid("fiber entry outside initialized source"));
            }
            memory
                .paused
                .preflight_fetch_source(entry, 1)
                .map_err(mem)?;
            if !matches!(
                memory.paused.preflight_write(entry, 1),
                Err(pentax_platform::native_memory::NativeMemoryError::PermissionDenied(_))
            ) {
                return Err(invalid("writable fiber entry"));
            }
            let marker = main
                .ok_or(invalid("fiber before prepared process entry"))?
                .return_marker;
            let cached = old.and_then(|i| runtime.records[i].owned);
            let (frame, owned) = if context == 0 {
                if let Some(frame) = cached {
                    (frame, Some(frame))
                } else {
                    // A bounded slot in the explicitly reserved thread-storage
                    // capability. Reinitializing this object reuses its slot.
                    const BYTES: u64 = 256 * 1024;
                    let window = storage
                        .window
                        .ok_or(invalid("fiber stack storage unavailable"))?;
                    let start = storage.next;
                    let stack = add(start, PAGE_SIZE)?;
                    let end = add(stack, BYTES)?;
                    let next = add(end, PAGE_SIZE)?;
                    if next > add(window.address, window.size)? {
                        return Err(KernelStartupError::Budget("fiber stack storage"));
                    }
                    account(memory, BYTES + 2 * PAGE_SIZE)?;
                    memory
                        .paused
                        .preflight_reserved_unmapped(start, next - start)
                        .map_err(mem)?;
                    let frame = NativeStackFrame {
                        stack: NativeRange {
                            address: stack,
                            size: BYTES,
                        },
                        guards: Some([
                            NativeRange {
                                address: start,
                                size: PAGE_SIZE,
                            },
                            NativeRange {
                                address: end,
                                size: PAGE_SIZE,
                            },
                        ]),
                        return_slot: (end & !15) - 8,
                        return_marker: marker,
                    };
                    memory
                        .paused
                        .apply_transaction(
                            NativeMemoryTransaction {
                                effect: NativeMemoryEffect::MapZeroed {
                                    address: stack,
                                    size: BYTES,
                                    permissions: NativePermissions::READ_WRITE,
                                },
                                writes: &[],
                            },
                            &Default::default(),
                        )
                        .map_err(mem)?;
                    storage.next = next;
                    (frame, Some(frame))
                }
            } else {
                let end = add(context, size)?;
                if size < 512 || context & 15 != 0 || size & 15 != 0 {
                    return Err(invalid("fiber stack geometry"));
                }
                (
                    NativeStackFrame {
                        stack: NativeRange {
                            address: context,
                            size,
                        },
                        guards: None,
                        return_slot: (end & !15) - 8,
                        return_marker: marker,
                    },
                    cached,
                )
            };
            account(memory, frame.stack.size.div_ceil(PAGE_SIZE) + 8)?;
            memory
                .paused
                .preflight_read_write_non_executable(frame.stack.address, frame.stack.size)
                .map_err(mem)?;
            // A refused caller-provided stack must not first overwrite an
            // active pthread's saved return slot. Admit all live backing
            // separation before publishing the new fiber marker.
            for other in main
                .into_iter()
                .map(|entry| entry.stack)
                .chain(storage.stack_ranges())
            {
                separate(memory, frame.stack, other)?;
            }
            let object = NativeRange {
                address: key.address(),
                size: 208,
            };
            separate(memory, frame.stack, object)?;
            for row in &runtime.records {
                if row.key.address() == key.address() || !kernel.fiber_may_resume(row.key) {
                    continue;
                }
                let cost = separation_work(frame.stack, row.layout.frame.stack)?;
                account(memory, cost)?;
                separate(
                    memory,
                    frame.stack,
                    NativeRange {
                        address: row.key.address(),
                        size: 208,
                    },
                )?;
                if memory
                    .paused
                    .preflight_read_write_non_executable(
                        row.layout.frame.stack.address,
                        row.layout.frame.stack.size,
                    )
                    .is_err()
                {
                    continue;
                }
                memory
                    .paused
                    .preflight_disjoint_backing(frame.stack, row.layout.frame.stack, cost)
                    .map_err(mem)?;
            }
            memory
                .paused
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::None,
                        writes: &[NativeMemoryWrite {
                            address: frame.return_slot,
                            bytes: &marker.to_le_bytes(),
                        }],
                    },
                    &Default::default(),
                )
                .map_err(mem)?;
            kernel
                .bind_fiber_stack(key, frame.stack.address, frame.stack.size)
                .map_err(KernelStartupError::Kernel)?;
            let initial = FiberEntryLayout {
                entry,
                args: [argument, request.argument()],
                frame,
            };
            let record = Record {
                key,
                layout: initial,
                origin: None,
                owned,
            };
            if let Some(i) = old {
                runtime.records[i] = record;
            } else {
                runtime.records.push(record);
            }
            layout = Some(initial);
        } else if exact.is_some_and(|index| runtime.records[index].origin.is_none()) {
            return Err(invalid("unobserved prior fiber preparation"));
        }
    }
    let pending = PreparedFiberAction {
        request,
        owner: memory.paused.owner_id(),
        source: source.identity(),
        pc,
        id,
        args,
        rsp,
        thread,
        layout,
        actual_return: None,
    };
    runtime.pending = Some(pending);
    runtime.observed = None;
    Ok(NativeServiceAction::Fiber(pending))
}
impl PreparedNativeKernelStartup<'_, '_> {
    pub fn fiber_context_limit(&self) -> usize {
        self.plan
            .limits
            .kernel
            .max_handles
            .min(self.plan.limits.kernel.max_thread_contexts)
    }
    fn check_fiber_action(
        &mut self,
        proof: &PreparedNativeSource<'_, '_>,
        pending: PreparedFiberAction,
    ) -> Result<(), KernelStartupError> {
        charge(
            &mut self.provider_work,
            self.fiber_runtime.records.len() as u64 + self.fiber_runtime.roots.len() as u64 + 32,
            self.plan.limits.max_provider_work,
        )?;
        if proof.owner_id() != self.owner_id()
            || pending.owner != self.owner_id()
            || proof.materialized_source().source_plan().identity()
                != self.memory().active_source_identity()
            || pending.source != self.memory().active_source_identity()
            || pending.thread != self.active_tls
            || self.fiber_runtime.pending.as_ref().map(|r| r.request) != Some(pending.request)
        {
            return Err(invalid("fiber action identity"));
        }
        Ok(())
    }
    /// Bind the actual opaque selected target before a CPU exchange. A root
    /// continuation must be the exact Run token of the currently selected thread.
    pub fn observe_fiber_target(
        &mut self,
        proof: &PreparedNativeSource<'_, '_>,
        pending: &PreparedFiberAction,
        target: &NativeFiberContext,
    ) -> Result<(), KernelStartupError> {
        self.check_fiber_action(proof, *pending)?;
        if target.owner_id() != pending.owner {
            return Err(invalid("fiber target owner"));
        }
        if self.fiber_runtime.observed.is_some() {
            return Err(invalid("fiber target replay"));
        }
        if let Some(key) = pending.target() {
            let stack = target.stack().stack;
            let mut cost = separation_work(
                stack,
                NativeRange {
                    address: key.address(),
                    size: 208,
                },
            )?;
            for row in &self.fiber_runtime.records {
                if row.key != key && self.kernel.fiber_may_resume(row.key) {
                    cost = cost
                        .checked_add(separation_work(stack, row.layout.frame.stack)?)
                        .and_then(|v| {
                            v.checked_add(
                                separation_work(
                                    stack,
                                    NativeRange {
                                        address: row.key.address(),
                                        size: 208,
                                    },
                                )
                                .ok()?,
                            )
                        })
                        .ok_or(KernelStartupError::Budget("fiber live separation"))?;
                }
            }
            charge(
                &mut self.provider_work,
                cost,
                self.plan.limits.max_provider_work,
            )?;
            let paused = self.hle.pause().map_err(KernelStartupError::Hle)?;
            paused
                .preflight_disjoint_backing(
                    stack,
                    NativeRange {
                        address: key.address(),
                        size: 208,
                    },
                    cost,
                )
                .map_err(mem)?;
            for row in &self.fiber_runtime.records {
                if row.key == key || !self.kernel.fiber_may_resume(row.key) {
                    continue;
                }
                paused
                    .preflight_disjoint_backing(
                        stack,
                        NativeRange {
                            address: row.key.address(),
                            size: 208,
                        },
                        cost,
                    )
                    .map_err(mem)?;
                // An unmapped idle stack has no selected execution authority;
                // its own later selection must revalidate the complete span.
                if paused
                    .preflight_read_write_non_executable(
                        row.layout.frame.stack.address,
                        row.layout.frame.stack.size,
                    )
                    .is_err()
                {
                    continue;
                }
                paused
                    .preflight_disjoint_backing(stack, row.layout.frame.stack, cost)
                    .map_err(mem)?;
            }
        }

        if let Some(key) = pending.target() {
            let record = self
                .fiber_runtime
                .records
                .iter_mut()
                .find(|r| r.key == key)
                .ok_or(invalid("fiber target record"))?;
            let origin = target.origin().ok_or(invalid("fiber target origin"))?;
            if origin.owner_id() != pending.owner
                || origin.entry() != record.layout.entry
                || *origin.stack() != record.layout.frame
                || origin.executor_id() != target.executor_id()
            {
                return Err(invalid("fiber entry provenance"));
            }
            match record.origin {
                Some(expected) if expected != origin => {
                    return Err(invalid("fiber parked origin changed"));
                }
                None if pending.layout.is_none() => {
                    return Err(invalid("missing fresh fiber origin"));
                }
                _ => {}
            }
            if pending.layout.is_some()
                && (target.pending_syscall().is_some()
                    || target.returned()
                    || target.state().guest_pc != record.layout.entry
                    || target.state().gpr[4] != record.layout.frame.return_slot
                    || target.state().gpr[7] != record.layout.args[0]
                    || target.state().gpr[6] != record.layout.args[1])
            {
                return Err(invalid("fiber initial call mismatch"));
            }
            record.origin = Some(origin);
        } else {
            let root = self
                .fiber_runtime
                .roots
                .iter()
                .find(|r| r.thread == pending.thread)
                .ok_or(invalid("fiber root missing"))?;
            if target.pending_syscall() != Some(root.token) || target.origin().is_some() {
                return Err(invalid("fiber root identity"));
            }
        }
        self.fiber_runtime.observed = Some((target.executor_id(), target.origin()));
        Ok(())
    }
    /// Publish after actual exchange and before any destination entry. Failure
    /// is terminal for this worker; the exchanged context must never be resumed.
    pub fn commit_fiber_action(
        &mut self,
        proof: &PreparedNativeSource<'_, '_>,
        pending: PreparedFiberAction,
        departed: &NativeFiberContext,
    ) -> Result<u64, KernelStartupError> {
        self.check_fiber_action(proof, pending)?;
        let (executor, _) = self
            .fiber_runtime
            .observed
            .ok_or(invalid("unobserved fiber target"))?;
        if departed.owner_id() != pending.owner || departed.executor_id() != executor {
            return Err(invalid("departing fiber owner"));
        }
        if let Some(origin) = pending.actual_return {
            if !departed.returned() || departed.origin() != Some(origin) {
                return Err(invalid("fiber entry return phase"));
            }
        } else {
            let token = departed
                .pending_syscall()
                .ok_or(invalid("departing fiber syscall"))?;
            if token.syscall_pc() != pending.pc
                || token.service_id() != pending.id
                || token.args() != pending.args
                || Some(token.stack_pointer()) != pending.rsp
                || token.owner_id() != pending.owner
                || token.executor_id() != executor
            {
                return Err(invalid("departing fiber call identity"));
            }
        }
        if let Some(key) = pending.from() {
            let origin = self
                .fiber_runtime
                .records
                .iter()
                .find(|r| r.key == key)
                .and_then(|r| r.origin)
                .ok_or(invalid("departing fiber origin"))?;
            if departed.origin() != Some(origin) {
                return Err(invalid("departing fiber generation"));
            }
        } else if departed.origin().is_some() {
            return Err(invalid("Run caller is fiber"));
        }
        let root_index = self
            .fiber_runtime
            .roots
            .iter()
            .position(|r| r.thread == pending.thread);
        if pending.kind() == FiberTransferKind::Run
            && root_index.is_none()
            && self.fiber_runtime.roots.len() >= self.fiber_runtime.maximum_roots
        {
            return Err(KernelStartupError::Budget("fiber roots"));
        }
        let state = departed.state();
        let saved = [
            state.gpr[3],
            state.gpr[5],
            state.gpr[7],
            state.gpr[6],
            state.gpr[12],
            state.gpr[13],
            state.gpr[14],
            state.gpr[15],
            state.gpr[4],
            departed
                .pending_syscall()
                .map_or(state.guest_pc, |t| t.next_pc()),
        ];
        let (mut paused, instances) = self.hle.pause_with_tls().map_err(KernelStartupError::Hle)?;
        let mut guest = KernelMemory {
            paused: &mut paused,
            source: Some(proof.materialized_source().source_plan()),
            supplied_modules: self.plan.kernel_provider_index(),
            lookup_limits: None,
            lookup_work: None,
            loaded_modules: None,
            thread_tls: instances.get(self.active_tls),
            initial_entry: None,
        };
        let result = self
            .kernel
            .commit_fiber_transfer(pending.request, saved, &mut guest)
            .map_err(KernelStartupError::Kernel)?;
        if pending.kind() == FiberTransferKind::Run {
            let root = Root {
                thread: pending.thread,
                token: departed
                    .pending_syscall()
                    .ok_or(invalid("Run root token"))?,
            };
            if let Some(i) = root_index {
                self.fiber_runtime.roots[i] = root;
            } else {
                self.fiber_runtime.roots.push(root);
            }
        }
        self.fiber_runtime.pending = None;
        self.fiber_runtime.observed = None;
        Ok(result)
    }
    /// Only a real return of the admitted fiber entry can terminate it. Nested
    /// HLE callback returns deliberately fall through to their existing handler.
    pub fn fiber_entry_return(
        &mut self,
        proof: &PreparedNativeSource<'_, '_>,
        observed: &NativeCallReturn<'_, '_>,
    ) -> Result<Option<PreparedFiberAction>, KernelStartupError> {
        let Some(key) = self.kernel.active_fiber() else {
            return Ok(None);
        };
        charge(
            &mut self.provider_work,
            self.fiber_runtime.records.len() as u64 + 16,
            self.plan.limits.max_provider_work,
        )?;
        let origin = self
            .fiber_runtime
            .records
            .iter()
            .find(|r| r.key == key)
            .and_then(|r| r.origin)
            .ok_or(invalid("active fiber origin"))?;
        if !origin.matches_return(observed) {
            return Ok(None);
        }
        if observed.source_plan().identity() != proof.materialized_source().source_plan().identity()
            || proof.owner_id() != self.owner_id()
            || observed.owner_id() != self.owner_id()
            || observed.source_plan().identity() != self.memory().active_source_identity()
            || self.fiber_runtime.pending.is_some()
        {
            return Err(invalid("fiber entry return source"));
        }
        let request = self
            .kernel
            .admit_fiber_entry_return()
            .map_err(KernelStartupError::Kernel)?;
        let pending = PreparedFiberAction {
            request,
            owner: self.owner_id(),
            source: self.memory().active_source_identity(),
            pc: 0,
            id: 0,
            args: [0; 6],
            rsp: None,
            thread: self.active_tls,
            layout: None,
            actual_return: Some(origin),
        };
        self.fiber_runtime.pending = Some(pending);
        self.fiber_runtime.observed = None;
        Ok(Some(pending))
    }
}

#[cfg(test)]
mod tests;
