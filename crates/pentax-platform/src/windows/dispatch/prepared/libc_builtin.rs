// SPDX-License-Identifier: GPL-2.0-only
//! Explicit supplied-library ABI substitution, not an observed SYSCALL.

use super::*;

/// One pending entry and already admitted normal RET. This token proves only
/// executor phase and source/context identity, never authority to implement an
/// arbitrary function. The core must separately authenticate the exact export
/// and opt-in ABI profile, and keep the engine and memory exclusively borrowed
/// through the helper and completion. No native body has executed here.
///
/// The token cannot be cloned. Cancellation is valid only before attempting
/// the helper; after any helper error the caller must terminalize this route.
pub struct NativeLibcBuiltin<'view, 'plan, 'image> {
    source: &'view MaterializedSource<'plan, 'image>,
    owner: NativeOwnerId,
    executor: u64,
    sequence: u64,
    call: u64,
    thread: usize,
    entry: u64,
    stack_pointer: u64,
    args: [u64; 6],
    target: u64,
    next_rsp: u64,
    outer: bool,
}

impl std::fmt::Debug for NativeLibcBuiltin<'_, '_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeLibcBuiltin")
            .field("sequence", &self.sequence)
            .finish_non_exhaustive()
    }
}

impl NativeLibcBuiltin<'_, '_, '_> {
    pub const fn owner_id(&self) -> NativeOwnerId {
        self.owner
    }
    pub const fn executor_id(&self) -> u64 {
        self.executor
    }
    pub const fn call_sequence(&self) -> u64 {
        self.call
    }
    pub fn source_identity(&self) -> pentax_linker::SourceIdentity {
        self.source.source_plan().identity()
    }
    pub const fn entry(&self) -> u64 {
        self.entry
    }
    pub const fn stack_pointer(&self) -> u64 {
        self.stack_pointer
    }
    pub const fn args(&self) -> [u64; 6] {
        self.args
    }
    /// The actual owned eight-byte return slot, not a caller-supplied address.
    pub const fn return_slot(&self) -> u64 {
        self.stack_pointer
    }
    /// None denotes the validated outer return marker, never an unknown target.
    pub const fn return_target(&self) -> Option<u64> {
        if self.outer { None } else { Some(self.target) }
    }
}

impl<'view, 'plan, 'image> NativeBlockStepper<'view, 'plan, 'image> {
    /// A defensive phase check, not proof of a fresh ABI call. In particular,
    /// MOVS/STOS may resume at the same entry without tracked comparison state;
    /// the core must also exclude entry REP strings on every retry.
    pub fn libc_builtin_entry_ready(&self) -> bool {
        self.machine.phase == Phase::Ready && self.machine.string_repeat.is_none()
    }

    /// Admit an authenticated builtin at the current function entry. The plan
    /// attests only its consumed entry window, not the complete function body.
    /// The core supplies export/ABI authority separately; these checks cannot
    /// identify a libc function by its bytes or make a public symbol trusted.
    ///
    /// One logical dispatch block and the existing conservative source/emission
    /// admission counters are charged. No publication, native entry or native
    /// instruction is counted. All completion work is reserved before effects.
    pub fn begin_libc_builtin(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, 'plan, 'image>,
    ) -> Result<NativeLibcBuiltin<'view, 'plan, 'image>, NativeDispatchError> {
        if !self.libc_builtin_entry_ready() {
            return Err(NativeDispatchError::WrongPhase);
        }
        self.preflight_block(memory, block)?;
        if block.instructions().first().is_some_and(|record| {
            let instruction = record.instruction();
            record.guest_pc() == block.guest_pc()
                && (instruction.has_rep_prefix() || instruction.has_repne_prefix())
                && strings::supported(instruction)
        }) {
            return Err(NativeDispatchError::WrongPhase);
        }
        let end_pc = if block.exit().instruction().is_some() {
            block.exit().next_guest_pc()
        } else {
            block.exit().guest_pc()
        };
        if end_pc <= block.guest_pc() {
            return Err(NativeDispatchError::SourceMismatch);
        }
        // Fixed token/return observations plus the second full stack and target
        // check at completion. Completion has no new quota/allocation decision.
        let guards = self.machine.stack.guards.map_or(0, |[lower, upper]| {
            lower.size / NATIVE_GUEST_PAGE + upper.size / NATIVE_GUEST_PAGE
        });
        let work = self
            .machine
            .stack
            .stack
            .size
            .div_ceil(NATIVE_GUEST_PAGE)
            .checked_add(1)
            .and_then(|value| value.checked_add(guards))
            .and_then(|value| value.checked_add(self.machine.target_visits))
            .and_then(|value| value.checked_add(128))
            .ok_or(NativeDispatchError::Budget("builtin return work"))?;
        charge(
            &mut self.machine.usage.work_units,
            work,
            self.machine.limits.max_work_units,
            "builtin return work",
        )?;
        attest_live_source(memory, block)?;
        let rsp = self.machine.state.gpr[RSP];
        let mut bytes = [0; 8];
        memory.read(rsp, &mut bytes)?;
        let target = u64::from_le_bytes(bytes);
        let next_rsp = rsp
            .checked_add(8)
            .ok_or(NativeDispatchError::InvalidStack)?;
        check_rsp(&self.machine.stack, next_rsp)?;
        let outer = target == self.machine.stack.return_marker;
        if outer {
            if rsp != self.machine.stack.return_slot {
                return Err(NativeDispatchError::InvalidTarget(target));
            }
        } else {
            self.check_target(memory, target)?;
        }
        let token = NativeLibcBuiltin {
            source: self.source,
            owner: self.machine.owner,
            executor: self.machine.executor,
            // Every successful admission consumed a new checked logical block.
            // Cancellation retains that charge, so identical retries differ.
            sequence: self.machine.usage.blocks,
            call: self.machine.call_sequence,
            thread: self.machine.thread_index,
            entry: self.machine.state.guest_pc,
            stack_pointer: rsp,
            args: builtin_args(&self.machine.state),
            target,
            next_rsp,
            outer,
        };
        self.machine.phase = Phase::LibcBuiltin(token.sequence);
        Ok(token)
    }

    /// Validate the current pending route before the separately authorized HLE
    /// effect. This is phase identity, not service authorization or a guarantee
    /// that memory will retain its permissions after the caller mutates it.
    pub fn validate_libc_builtin(
        &self,
        memory: &PausedNativeMemory<'_>,
        token: &NativeLibcBuiltin<'_, '_, '_>,
    ) -> Result<(), NativeDispatchError> {
        if memory.owner_id() != self.machine.owner || token.owner != self.machine.owner {
            return Err(NativeDispatchError::WrongOwner);
        }
        self.check_libc_builtin(token)
    }

    /// Cancel only before attempting the helper. All attempted admission charges
    /// remain. After any helper attempt, use fail or complete instead; retrying
    /// the native function after a possible write is not supported.
    pub fn cancel_libc_builtin(
        &mut self,
        token: NativeLibcBuiltin<'_, '_, '_>,
    ) -> Result<(), NativeDispatchError> {
        self.check_libc_builtin(&token)?;
        self.machine.phase = Phase::Ready;
        Ok(())
    }

    /// Consume an errored helper route without permitting a native fallback.
    pub fn fail_libc_builtin(
        &mut self,
        token: NativeLibcBuiltin<'_, '_, '_>,
    ) -> Result<(), NativeDispatchError> {
        self.check_libc_builtin(&token)?;
        self.machine.phase = Phase::Terminal;
        Ok(())
    }

    /// Publish a completed helper followed by normal RET semantics. The caller
    /// must reject physical destination aliases of the return slot/source before
    /// invoking the helper. Safe callers can still change memory between calls:
    /// the genuine token is consumed BEFORE late stack/target checks. A fault
    /// retains helper writes and RAX, leaves RSP/PC unadvanced, and is terminal.
    pub fn complete_libc_builtin(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        token: NativeLibcBuiltin<'_, '_, '_>,
        value: u64,
    ) -> Result<NativeBlockStop, NativeDispatchError> {
        self.validate_libc_builtin(memory, &token)?;
        self.machine.phase = Phase::Terminal;
        self.machine.state.gpr[RAX] = value;
        check_builtin_stack_backing(memory, &self.machine.stack)?;
        let mut bytes = [0; 8];
        memory.read(token.stack_pointer, &mut bytes)?;
        let target = u64::from_le_bytes(bytes);
        if target != token.target {
            return Err(NativeDispatchError::InvalidTarget(target));
        }
        if !token.outer {
            self.check_target(memory, target)?;
        }
        self.machine.state.gpr[RSP] = token.next_rsp;
        self.machine.state.guest_pc = target;
        if token.outer {
            self.machine.phase = Phase::Returned;
            self.machine.observed_return = Some(CallObservation {
                owner: self.machine.owner,
                executor: self.machine.executor,
                call: self.machine.call_sequence,
                initial: self.machine.call_initial,
                stack: self.machine.stack,
                value,
            });
            Ok(NativeBlockStop::Returned { value })
        } else {
            self.machine.phase = Phase::Ready;
            Ok(NativeBlockStop::Resume { guest_pc: target })
        }
    }

    fn check_libc_builtin(
        &self,
        token: &NativeLibcBuiltin<'_, '_, '_>,
    ) -> Result<(), NativeDispatchError> {
        if token.owner != self.machine.owner {
            return Err(NativeDispatchError::WrongOwner);
        }
        if self.machine.phase != Phase::LibcBuiltin(token.sequence)
            || token.executor != self.machine.executor
            || token.call != self.machine.call_sequence
            || token.thread != self.machine.thread_index
            || !std::ptr::eq(token.source, self.source)
            || token.source_identity() != self.machine.source_identity
            || token.entry != self.machine.state.guest_pc
            || token.stack_pointer != self.machine.state.gpr[RSP]
            || token.args != builtin_args(&self.machine.state)
        {
            return Err(NativeDispatchError::WrongPhase);
        }
        Ok(())
    }
}

fn builtin_args(state: &NativeCpuState) -> [u64; 6] {
    [
        state.gpr[RDI],
        state.gpr[RSI],
        state.gpr[RDX],
        state.gpr[RCX],
        state.gpr[8],
        state.gpr[9],
    ]
}

// Stack geometry and validation work were admitted at begin and cannot change
// while pending. Recheck only live backing, in the same guard/read/write/NX
// order as validate_stack, without another quota decision after helper effects.
fn check_builtin_stack_backing(
    memory: &PausedNativeMemory<'_>,
    stack: &NativeStackFrame,
) -> Result<(), NativeDispatchError> {
    if let Some([lower, upper]) = stack.guards {
        memory.preflight_reserved_unmapped(lower.address, lower.size)?;
        memory.preflight_reserved_unmapped(upper.address, upper.size)?;
    }
    if memory
        .preflight_read_write_non_executable(stack.stack.address, stack.stack.size)
        .is_err()
    {
        memory.preflight_read(stack.stack.address, stack.stack.size)?;
        memory.preflight_write(stack.stack.address, stack.stack.size)?;
        if memory
            .preflight_non_executable(stack.stack.address, stack.stack.size)
            .is_err()
        {
            return Err(NativeDispatchError::InvalidStack);
        }
    }
    Ok(())
}
