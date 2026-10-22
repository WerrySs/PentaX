// SPDX-License-Identifier: GPL-2.0-only

//! Executor-owned fiber continuations; switching never executes guest code.

use super::*;
use std::sync::{Arc, atomic::AtomicUsize};

/// Identity of a real, prepared entry call. Guest context bytes cannot mint it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeFiberOrigin {
    owner: NativeOwnerId,
    executor: u64,
    call: u64,
    entry: u64,
    stack: NativeStackFrame,
}
impl NativeFiberOrigin {
    pub const fn owner_id(&self) -> NativeOwnerId {
        self.owner
    }
    pub const fn executor_id(&self) -> u64 {
        self.executor
    }
    pub const fn call_sequence(&self) -> u64 {
        self.call
    }
    pub const fn entry(&self) -> u64 {
        self.entry
    }
    pub const fn stack(&self) -> &NativeStackFrame {
        &self.stack
    }
    pub fn matches_return(&self, observed: &NativeCallReturn<'_, '_>) -> bool {
        self.owner == observed.owner_id()
            && self.executor == observed.executor_id()
            && self.call == observed.call_sequence()
            && self.entry == observed.entry()
            && self.stack == *observed.stack()
    }
}

/// Non-cloneable context belonging to one exact memory owner and executor.
/// Selected pthread identity and its modeled TLS bases do not move with it.
pub struct NativeFiberContext {
    owner: NativeOwnerId,
    executor: u64,
    state: NativeCpuState,
    stack: NativeStackFrame,
    phase: Phase,
    call_initial: NativeCpuState,
    call_sequence: u64,
    suspended: Vec<SuspendedCall>,
    observed_return: Option<CallObservation>,
    string_repeat: Option<strings::RepeatState>,
    origin: Option<NativeFiberOrigin>,
    slots: Arc<AtomicUsize>,
    thread: usize,
}
impl std::fmt::Debug for NativeFiberContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeFiberContext")
            .field("call", &self.call_sequence)
            .finish_non_exhaustive()
    }
}
impl Drop for NativeFiberContext {
    fn drop(&mut self) {
        self.slots.fetch_sub(1, Ordering::Relaxed);
    }
}
impl NativeFiberContext {
    pub const fn owner_id(&self) -> NativeOwnerId {
        self.owner
    }
    pub const fn executor_id(&self) -> u64 {
        self.executor
    }
    pub const fn state(&self) -> &NativeCpuState {
        &self.state
    }
    pub const fn stack(&self) -> &NativeStackFrame {
        &self.stack
    }
    pub const fn origin(&self) -> Option<NativeFiberOrigin> {
        self.origin
    }
    pub const fn pending_syscall(&self) -> Option<PreSyscallToken> {
        match self.phase {
            Phase::Syscall(token) => Some(token),
            _ => None,
        }
    }
    pub const fn returned(&self) -> bool {
        matches!(self.phase, Phase::Returned)
    }
}
fn separate(
    memory: &PausedNativeMemory<'_>,
    left: NativeRange,
    right: NativeRange,
    usage: &mut NativeDispatchUsage,
    limits: &NativeDispatchLimits,
) -> Result<(), NativeDispatchError> {
    let pages = |r: NativeRange| {
        (r.address % NATIVE_GUEST_PAGE)
            .checked_add(r.size)
            .map(|v| v.div_ceil(NATIVE_GUEST_PAGE))
            .ok_or(NativeDispatchError::InvalidStack)
    };
    let l = pages(left)?;
    let r = pages(right)?;
    let work = l
        .checked_mul(r)
        .and_then(|v| v.checked_add(l.checked_add(r)?.checked_mul(3)?))
        .and_then(|v| v.checked_add(1))
        .ok_or(NativeDispatchError::Budget("fiber stack separation"))?;
    charge(
        &mut usage.work_units,
        work,
        limits.max_work_units,
        "fiber stack separation",
    )?;
    memory.preflight_disjoint_backing(left, right, work)?;
    Ok(())
}
impl NativeBlockStepper<'_, '_, '_> {
    /// Prepare an initial fiber with the caller's admitted CPU/control policy.
    /// The complete stack, actual RX entry and return marker are checked here.
    pub fn prepare_fiber(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        state: NativeCpuState,
        stack: NativeStackFrame,
    ) -> Result<NativeFiberContext, NativeDispatchError> {
        if memory.owner_id() != self.machine.owner {
            return Err(NativeDispatchError::WrongOwner);
        }
        if matches!(self.machine.phase, Phase::Terminal | Phase::LibcBuiltin(_)) {
            return Err(NativeDispatchError::WrongPhase);
        }
        if state.gpr[RSP] != stack.return_slot {
            return Err(NativeDispatchError::InvalidStack);
        }
        self.validate_thread_state(memory, &state, &stack, Phase::Ready, true)?;
        // A new fiber stack must not overwrite a live pthread's call chain,
        // including a different virtual alias of that backing.
        for other in &self.machine.thread_stacks {
            separate(
                memory,
                stack.stack,
                other.stack,
                &mut self.machine.usage,
                &self.machine.limits,
            )?;
        }

        charge(
            &mut self.machine.usage.work_units,
            std::mem::size_of::<NativeFiberContext>() as u64 + 16,
            self.machine.limits.max_work_units,
            "fiber registration",
        )?;
        let call = self
            .machine
            .next_call_sequence
            .checked_add(1)
            .ok_or(NativeDispatchError::Budget("call sequence"))?;
        self.machine
            .fiber_slots
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < self.machine.limits.max_thread_contexts).then_some(n + 1)
            })
            .map_err(|_| NativeDispatchError::Budget("fiber contexts"))?;
        self.machine.next_call_sequence = call;
        Ok(NativeFiberContext {
            owner: self.machine.owner,
            executor: self.machine.executor,
            state,
            stack,
            phase: Phase::Ready,
            call_initial: state,
            call_sequence: call,
            suspended: Vec::new(),
            observed_return: None,
            string_repeat: None,
            origin: Some(NativeFiberOrigin {
                owner: self.machine.owner,
                executor: self.machine.executor,
                call,
                entry: state.guest_pc,
                stack,
            }),
            slots: Arc::clone(&self.machine.fiber_slots),
            thread: self.machine.thread_index,
        })
    }
    /// Exchange continuations under the existing exclusive pause, retaining the
    /// selected pthread/TLS. Failure leaves both contexts selected as before.
    /// A later runtime commit failure must stop execution terminally; it cannot
    /// be reported as a successful service return or resumed destination.
    pub fn exchange_fiber(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        parked: &mut NativeFiberContext,
    ) -> Result<(), NativeDispatchError> {
        if memory.owner_id() != self.machine.owner
            || parked.owner != self.machine.owner
            || parked.executor != self.machine.executor
        {
            return Err(NativeDispatchError::WrongOwner);
        }
        if matches!(self.machine.phase, Phase::Terminal | Phase::LibcBuiltin(_))
            || matches!(parked.phase, Phase::Terminal | Phase::LibcBuiltin(_))
        {
            return Err(NativeDispatchError::WrongPhase);
        }
        let fs = self.machine.state.fs_base;
        let gs = self.machine.state.gs_base;
        // Nested HLE tokens retain exact parent TLS bindings. Ordinary idle
        // fibers may migrate; callbacks resume only on the same pthread and TLS.
        if (parked.thread != self.machine.thread_index
            || parked.state.fs_base != fs
            || parked.state.gs_base != gs)
            && !parked.suspended.is_empty()
        {
            return Err(NativeDispatchError::Budget(
                "fiber callback thread migration",
            ));
        }
        let mut state = parked.state;
        state.fs_base = fs;
        state.gs_base = gs;
        self.validate_thread_state(memory, &state, &parked.stack, parked.phase, true)?;
        for (index, other) in self.machine.thread_stacks.iter().enumerate() {
            if parked.origin.is_none() && index == self.machine.thread_index {
                continue;
            }
            separate(
                memory,
                parked.stack.stack,
                other.stack,
                &mut self.machine.usage,
                &self.machine.limits,
            )?;
        }

        use std::mem::swap;
        swap(&mut self.machine.state, &mut parked.state);
        self.machine.state.fs_base = fs;
        self.machine.state.gs_base = gs;
        swap(&mut self.machine.stack, &mut parked.stack);
        swap(&mut self.machine.phase, &mut parked.phase);
        swap(&mut self.machine.call_initial, &mut parked.call_initial);
        swap(&mut self.machine.call_sequence, &mut parked.call_sequence);
        swap(&mut self.machine.suspended, &mut parked.suspended);
        swap(
            &mut self.machine.observed_return,
            &mut parked.observed_return,
        );
        swap(&mut self.machine.string_repeat, &mut parked.string_repeat);
        swap(&mut self.machine.fiber_origin, &mut parked.origin);
        parked.thread = self.machine.thread_index;

        Ok(())
    }
}
