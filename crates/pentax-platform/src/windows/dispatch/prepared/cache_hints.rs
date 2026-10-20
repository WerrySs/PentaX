// SPDX-License-Identifier: GPL-2.0-only

//! Intel SDM Vol.2 PREFETCHh permits ignoring these locality hints entirely.
//! Never compute the suggested address, read guest bytes or prefetch host memory.
//! This does not admit cache flush/writeback or other prefetch instruction sets.
use super::*;
use pentax_codegen::{Code, Instruction, OpKind};

pub(super) fn supported(instruction: &Instruction) -> bool {
    matches!(
        instruction.code(),
        Code::Prefetchnta_m8 | Code::Prefetcht0_m8 | Code::Prefetcht1_m8 | Code::Prefetcht2_m8
    )
}

impl NativeBlockStepper<'_, '_, '_> {
    pub(super) fn emulate_cache_hint(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, '_, '_>,
    ) -> Result<NativeBlockStop, NativeDispatchError> {
        let Some(instruction) = block.exit().instruction().filter(|i| supported(i)) else {
            return Err(NativeDispatchError::InvalidState);
        };
        // LOCK is invalid architecturally. Repeat/HLE prefixes and a missing
        // declared SSE family remain unsupported PentaX policy, not invented
        // guest exceptions. FS/GS and address-size overrides need no address
        // calculation when the hint is ignored, including invalid targets.
        if instruction.op_count() != 1
            || instruction.op0_kind() != OpKind::Memory
            || instruction.has_lock_prefix()
            || instruction.has_rep_prefix()
            || instruction.has_repne_prefix()
            || instruction.has_xacquire_prefix()
            || instruction.has_xrelease_prefix()
            || instruction
                .cpuid_features()
                .iter()
                .any(|feature| !block.policy().features().contains(*feature))
        {
            return Ok(NativeBlockStop::Unsupported {
                guest_pc: block.exit().guest_pc(),
            });
        }
        charge(
            &mut self.machine.usage.work_units,
            self.machine
                .target_visits
                .checked_add(32)
                .ok_or(NativeDispatchError::Budget("cache hint work"))?,
            self.machine.limits.max_work_units,
            "cache hint work",
        )?;
        let next_pc = block.exit().next_guest_pc();
        check_target(self.source, memory, next_pc)?;
        self.machine.state.guest_pc = next_pc;
        self.machine.phase = Phase::Ready;
        Ok(NativeBlockStop::Resume { guest_pc: next_pc })
    }
}
