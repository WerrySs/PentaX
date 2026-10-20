// SPDX-License-Identifier: GPL-2.0-only

//! A deliberately bounded software CPU profile, never host identification.
//! CPUID register semantics and serialization follow Intel SDM Vol. 2A, CPUID.
//! The advertised subset is PentaX policy, not an AMD/Intel processor model or
//! a promise that every form in an instruction family has been implemented.

use super::*;
use pentax_codegen::{Code, Instruction};
use std::sync::atomic::compiler_fence;

const RBX: usize = 3;

pub(super) fn supported(instruction: &Instruction) -> bool {
    instruction.code() == Code::Cpuid
}

// Register order is EAX, EBX, ECX, EDX. Keep the caller's subleaf when an
// out-of-range leaf falls back to basic leaf 7; that leaf currently exposes no
// structured features, including for nonzero subleaves.
fn profile(leaf: u32, subleaf: u32, counter_available: bool) -> [u32; 4] {
    let leaf = match leaf {
        0..=7 | 0x8000_0000..=0x8000_0001 => leaf,
        _ => 7,
    };
    match (leaf, subleaf) {
        (0, _) => [7, 0x746e_6550, 0x5550_4375, 0x6d45_5861], // PentaXEmuCPU
        (1, _) => [
            0,       // No hardware signature.
            0,       // No topology, APIC ID or cache-line enumeration.
            1 << 23, // POPCNT.
            (1 << 8)
                | (1 << 15)
                | (1 << 25)
                | (1 << 26)
                | if counter_available { 1 << 4 } else { 0 }, // CX8, CMOV, SSE, SSE2, TSC.
        ],
        (0x8000_0000, _) => [0x8000_0001, 0, 0, 0],
        (0x8000_0001, _) => [
            0,
            0,
            1 << 5,                                                  // LZCNT.
            (1 << 29) | if counter_available { 1 << 27 } else { 0 }, // Long mode, RDTSCP.
        ],
        _ => [0; 4],
    }
}

fn serialize() {
    // Compiler barriers surround a real serializing instruction. This fixed
    // host leaf accepts no guest input, and none of its results become guest
    // data. CPUID is available in the x86-64 execution environment.
    compiler_fence(Ordering::SeqCst);
    let _ = std::arch::x86_64::__cpuid(0);
    compiler_fence(Ordering::SeqCst);
}

impl NativeBlockStepper<'_, '_, '_> {
    pub(super) fn emulate_cpu_identification(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, '_, '_>,
    ) -> Result<NativeBlockStop, NativeDispatchError> {
        self.emulate_cpu_identification_with(
            memory,
            block,
            || crate::time_counter::sample().is_ok(),
            serialize,
        )
    }

    pub(super) fn emulate_cpu_identification_with(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, '_, '_>,
        counter_available: impl FnOnce() -> bool,
        serialize: impl FnOnce(),
    ) -> Result<NativeBlockStop, NativeDispatchError> {
        let instruction = block
            .exit()
            .instruction()
            .ok_or(NativeDispatchError::UnsupportedCpuInstruction)?;
        if !supported(instruction)
            || instruction.op_count() != 0
            || instruction.len() != 2
            || instruction.has_segment_prefix()
            || instruction.has_lock_prefix()
            || instruction.has_rep_prefix()
            || instruction.has_repne_prefix()
            || instruction.has_xacquire_prefix()
            || instruction.has_xrelease_prefix()
        {
            return Err(NativeDispatchError::UnsupportedCpuInstruction);
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
        let leaf = self.machine.state.gpr[RAX] as u32;
        let subleaf = self.machine.state.gpr[RCX] as u32;
        // Reuse the exact same cached frequency admission as the timestamp and
        // kernel services. A first feature query can calibrate that backend;
        // failure hides both TSC and RDTSCP instead of inventing a clock domain.
        let available = matches!(leaf, 1 | 0x8000_0001) && counter_available();
        let words = profile(leaf, subleaf, available);
        let mut next = self.machine.state;
        for (register, value) in [RAX, RBX, RCX, RDX].into_iter().zip(words) {
            next.gpr[register] = u64::from(value);
        }
        next.guest_pc = next_pc;
        // No fallible work follows the actual serialization boundary.
        serialize();
        self.machine.state = next;
        self.machine.usage.integer_operations = operations;
        self.machine.phase = Phase::Ready;
        Ok(NativeBlockStop::Resume { guest_pc: next_pc })
    }
}
