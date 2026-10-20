// SPDX-License-Identifier: GPL-2.0-only

//! Complete guest context exchange while retaining one code cache and budget.

use super::*;

/// A parked context of one exact executor. It cannot be constructed or cloned
/// by the caller. It contains no host registers or executable cache pointers.
/// Creating this value executes nothing and does not implement pthread_create.
pub struct NativeThreadContext {
    owner: NativeOwnerId,
    executor: u64,
    index: usize,
    state: NativeCpuState,
    stack: NativeStackFrame,
    phase: Phase,
    call_initial: NativeCpuState,
    call_sequence: u64,
    suspended: Vec<SuspendedCall>,
    observed_return: Option<CallObservation>,
    string_repeat: Option<strings::RepeatState>,
}

impl std::fmt::Debug for NativeThreadContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeThreadContext")
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

impl NativeThreadContext {
    pub const fn executor_id(&self) -> u64 {
        self.executor
    }
    pub const fn call_sequence(&self) -> u64 {
        self.call_sequence
    }
    pub const fn owner_id(&self) -> NativeOwnerId {
        self.owner
    }
    pub const fn index(&self) -> usize {
        self.index
    }
    pub const fn state(&self) -> &NativeCpuState {
        &self.state
    }
}

impl NativeBlockStepper<'_, '_, '_> {
    pub const fn thread_index(&self) -> usize {
        self.machine.thread_index
    }

    /// Register an independent live stack and initial guest state. All contexts
    /// retain the same source lineage, cache, executor and cumulative budgets.
    /// Stack ranges, including their guards, must not overlap another context.
    /// Core TLS and guest thread identity must be admitted separately by the
    /// dedicated runtime before this context is selected for unsafe execution.
    /// The outer return slot may still be uninitialized: the runtime publishes
    /// it with the thread handle after admission. Selection verifies its bytes.
    pub fn prepare_thread(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        state: NativeCpuState,
        stack: NativeStackFrame,
    ) -> Result<NativeThreadContext, NativeDispatchError> {
        if self.machine.phase == Phase::Terminal {
            return Err(NativeDispatchError::WrongPhase);
        }
        if memory.owner_id() != self.machine.owner {
            return Err(NativeDispatchError::WrongOwner);
        }
        let index = self.machine.thread_stacks.len();
        if index >= self.machine.limits.max_thread_contexts {
            return Err(NativeDispatchError::Budget("thread contexts"));
        }
        if state.gpr[RSP] != stack.return_slot {
            return Err(NativeDispatchError::InvalidStack);
        }
        self.validate_thread_state(memory, &state, &stack, Phase::Ready, false)?;
        charge(
            &mut self.machine.usage.work_units,
            index as u64 + std::mem::size_of::<NativeThreadContext>() as u64,
            self.machine.limits.max_work_units,
            "thread registration",
        )?;
        let extent = |frame: &NativeStackFrame| -> Result<(u64, u64), NativeDispatchError> {
            match frame.guards {
                Some([lower, upper]) => Ok((lower.address, end(upper)?)),
                None => Ok((frame.stack.address, end(frame.stack)?)),
            }
        };
        let (lower, upper) = extent(&stack)?;
        for other in &self.machine.thread_stacks {
            let (other_lower, other_upper) = extent(other)?;
            if lower < other_upper && other_lower < upper {
                return Err(NativeDispatchError::InvalidStack);
            }
        }
        let call_sequence = self
            .machine
            .next_call_sequence
            .checked_add(1)
            .ok_or(NativeDispatchError::Budget("call sequence"))?;
        self.machine
            .thread_stacks
            .try_reserve_exact(1)
            .map_err(|_| NativeDispatchError::Allocation)?;
        self.machine.thread_stacks.push(stack);
        self.machine.next_call_sequence = call_sequence;
        Ok(NativeThreadContext {
            owner: self.machine.owner,
            executor: self.machine.executor,
            index,
            state,
            stack,
            phase: Phase::Ready,
            call_initial: state,
            call_sequence,
            suspended: Vec::new(),
            observed_return: None,
            string_repeat: None,
        })
    }

    /// Exchange the selected and parked guest contexts at a block boundary.
    /// On success `parked` holds the old context; on refusal both remain intact
    /// except for charged validation work. Pending syscalls, nested callbacks,
    /// REP continuation, vector state and modeled segment bases move together.
    /// No code runs here. Before stepping, the runtime must select matching core
    /// TLS/kernel state and retain the existing source/operand safety proofs.
    pub fn exchange_thread(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        parked: &mut NativeThreadContext,
    ) -> Result<(), NativeDispatchError> {
        if memory.owner_id() != self.machine.owner
            || parked.owner != self.machine.owner
            || parked.executor != self.machine.executor
        {
            return Err(NativeDispatchError::WrongOwner);
        }
        if self.machine.phase == Phase::Terminal
            || parked.phase == Phase::Terminal
            || parked.index == self.machine.thread_index
        {
            return Err(NativeDispatchError::WrongPhase);
        }
        self.validate_thread_state(memory, &parked.state, &parked.stack, parked.phase, true)?;
        use std::mem::swap;
        swap(&mut self.machine.thread_index, &mut parked.index);
        swap(&mut self.machine.state, &mut parked.state);
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
        Ok(())
    }

    fn validate_thread_state(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        state: &NativeCpuState,
        stack: &NativeStackFrame,
        phase: Phase,
        require_marker: bool,
    ) -> Result<(), NativeDispatchError> {
        validate_state(state)?;
        charge(
            &mut self.machine.usage.work_units,
            self.machine.target_visits + 16,
            self.machine.limits.max_work_units,
            "thread selection",
        )?;
        validate_stack(
            memory,
            stack,
            state.gpr[RSP],
            &mut self.machine.usage,
            &self.machine.limits,
        )?;
        if require_marker {
            let mut marker = [0; 8];
            memory.read(stack.return_slot, &mut marker)?;
            if u64::from_le_bytes(marker) != stack.return_marker {
                return Err(NativeDispatchError::InvalidStack);
            }
        }
        if phase != Phase::Returned {
            check_target(self.source, memory, state.guest_pc)?;
            if !matches!(
                memory.preflight_write(state.guest_pc, 1),
                Err(NativeMemoryError::PermissionDenied(_))
            ) {
                return Err(NativeDispatchError::WritableSource);
            }
        }
        Ok(())
    }
}
