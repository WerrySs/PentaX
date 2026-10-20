// SPDX-License-Identifier: GPL-2.0-only

//! Timestamp reads share the kernel adapter's admitted invariant TSC domain.
//! The cooperative executor models one virtual processor, whose TSC_AUX is zero.

use super::*;
use crate::time_counter::{CounterError, CounterSample};
use pentax_codegen::{Code, Instruction};

pub(super) fn supported(instruction: &Instruction) -> bool {
    matches!(instruction.code(), Code::Rdtsc | Code::Rdtscp)
}

impl NativeBlockStepper<'_, '_, '_> {
    pub(super) fn emulate_timestamp(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, '_, '_>,
    ) -> Result<NativeBlockStop, NativeDispatchError> {
        self.emulate_timestamp_with(memory, block, crate::time_counter::sample)
    }

    pub(super) fn emulate_timestamp_with(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, '_, '_>,
        sample: impl FnOnce() -> Result<CounterSample, CounterError>,
    ) -> Result<NativeBlockStop, NativeDispatchError> {
        let instruction = block
            .exit()
            .instruction()
            .ok_or(NativeDispatchError::UnsupportedIntegerInstruction)?;
        if !supported(instruction)
            || instruction.op_count() != 0
            || instruction.has_segment_prefix()
            || instruction.has_lock_prefix()
            || instruction.has_rep_prefix()
            || instruction.has_repne_prefix()
            || instruction.has_xacquire_prefix()
            || instruction.has_xrelease_prefix()
        {
            return Err(NativeDispatchError::UnsupportedIntegerInstruction);
        }
        charge(
            &mut self.machine.usage.work_units,
            self.machine
                .target_visits
                .checked_add(128)
                .ok_or(NativeDispatchError::Budget("integer work"))?,
            self.machine.limits.max_work_units,
            "integer work",
        )?;
        let next_pc = block.exit().next_guest_pc();
        check_target(self.source, memory, next_pc)?;
        let operations = self
            .machine
            .usage
            .integer_operations
            .checked_add(1)
            .ok_or(NativeDispatchError::Budget("integer operations"))?;
        // No counter observation or one-time frequency calibration is attempted
        // until the complete instruction and its continuation are admitted.
        let counter = sample().map_err(NativeDispatchError::TimeCounter)?;
        if counter.frequency == 0 {
            return Err(NativeDispatchError::TimeCounter(CounterError::Calibration));
        }
        let mut next = self.machine.state;
        next.gpr[RAX] = u64::from(counter.ticks as u32);
        next.gpr[RDX] = counter.ticks >> 32;
        if instruction.code() == Code::Rdtscp {
            // Guest virtual CPU zero, not the host's privileged IA32_TSC_AUX.
            next.gpr[RCX] = 0;
        }
        next.guest_pc = next_pc;
        self.machine.state = next;
        self.machine.usage.integer_operations = operations;
        self.machine.phase = Phase::Ready;
        Ok(NativeBlockStop::Resume { guest_pc: next_pc })
    }
}
