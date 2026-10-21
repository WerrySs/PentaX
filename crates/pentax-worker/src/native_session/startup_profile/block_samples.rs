// SPDX-License-Identifier: GPL-2.0-only
//! Optional bounded observations, never execution or source authority.
//! The exit histogram retains the first 64 distinct catalogue Codes across
//! successful samples. Overflow counts observations of unretained Codes, not
//! distinct unknown Codes. A missing exit instruction has a separate bucket.
//! Top 4 counts are exact only within that retained set; these are sampled
//! planned exits, not whole-run frequencies or proof that an exit instruction
//! ran. Main/child refers to the actual selected thread
//! context, including callbacks on it, and establishes no scheduling fairness.
//! Self-target counters describe only completed periodic samples. Strict native
//! register bodies conservatively exclude unknown templates and are not proof
//! of safe native looping or whole-run instruction frequency.
//! A self-target observation concerns the planned direct Jcc target, whether
//! the branch was actually taken or not; no branch outcome is recorded.

use pentax_codegen::{
    Code, ExitKind, FlowControl, Instruction, MaterializedBlockPlan, MemoryRegisterDependencies,
    Mnemonic, OpKind, Register,
};
use pentax_linker::SourceIdentity;
use std::fmt::{self, Write};

mod private_samples;

const CAPACITY: usize = 64;
const EXIT_CAPACITY: usize = 64;
const MAX_WORK: u64 = 64 * 1024 * 1024;
const FIXED_WORK: u64 = 4 * CAPACITY as u64 + 64 + EXIT_CAPACITY as u64;
// Includes the original FP classification and immutable loop-summary visits.
// This is an observer-only precharge within the unchanged MAX_WORK allowance.
const RECORD_WORK: u64 = 64;

// Exact public forms shared with the separately reviewed native FP candidate.
// Counting an occurrence does not establish host, prefix or MXCSR admission.
const FORMS: [Code; 12] = [
    Code::VEX_Vaddps_xmm_xmm_xmmm128,
    Code::VEX_Vaddps_ymm_ymm_ymmm256,
    Code::VEX_Vaddpd_xmm_xmm_xmmm128,
    Code::VEX_Vaddpd_ymm_ymm_ymmm256,
    Code::VEX_Vsubps_xmm_xmm_xmmm128,
    Code::VEX_Vsubps_ymm_ymm_ymmm256,
    Code::VEX_Vsubpd_xmm_xmm_xmmm128,
    Code::VEX_Vsubpd_ymm_ymm_ymmm256,
    Code::VEX_Vmulps_xmm_xmm_xmmm128,
    Code::VEX_Vmulps_ymm_ymm_ymmm256,
    Code::VEX_Vmulpd_xmm_xmm_xmmm128,
    Code::VEX_Vmulpd_ymm_ymm_ymmm256,
];

#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    epoch: SourceIdentity,
    module: usize,
    pc: u64,
}
#[derive(Clone, Copy)]
struct Entry {
    key: Key,
    is_main: bool,
    planned: Option<Code>,
    frequency: u8,
}

enum ExitUpdate {
    Code {
        index: usize,
        code: Code,
        count: u64,
    },
    Overflow(u64),
    NoInstruction(u64),
}

struct SampleRecord<'a> {
    instruction: &'a Instruction,
    emitted: bool,
    memory: Option<(bool, MemoryRegisterDependencies)>,
}

pub(in crate::native_session) struct PendingSample {
    key: Key,
    is_main: bool,
    planned: Option<Code>,
    completed: u64,
    records: u64,
    counts: [[u64; 2]; 12],
    exit: ExitUpdate,
    main_context: u64,
    child_context: u64,
    self_target_branches: u64,
    self_target_native_register_blocks: u64,
    self_target_native_register_instructions: u64,
}

pub(in crate::native_session) struct BlockSamples {
    epoch: Option<SourceIdentity>,
    entries: [Option<Entry>; CAPACITY],
    cursor: usize,
    recent: u8,
    distinct: u8,
    max_repeat: u8,
    planned: Option<Code>,
    completed: u64,
    records: u64,
    epoch_changes: u64,
    counts: [[u64; 2]; 12],
    exits: [Option<(Code, u64)>; EXIT_CAPACITY],
    exit_codes_retained: u8,
    exit_overflow: u64,
    exit_no_instruction: u64,
    main_context: u64,
    child_context: u64,
    self_target_branches: u64,
    self_target_native_register_blocks: u64,
    self_target_native_register_instructions: u64,
    work: u64,
    maximum_work: u64,
    disabled: bool,
    overflow: bool,
    private_samples_requested: bool,
    private_samples: Option<private_samples::PrivateSamples>,
}

impl Default for BlockSamples {
    fn default() -> Self {
        Self {
            epoch: None,
            entries: [None; CAPACITY],
            cursor: 0,
            recent: 0,
            distinct: 0,
            max_repeat: 0,
            planned: None,
            completed: 0,
            records: 0,
            epoch_changes: 0,
            counts: [[0; 2]; 12],
            exits: [None; EXIT_CAPACITY],
            exit_codes_retained: 0,
            exit_overflow: 0,
            exit_no_instruction: 0,
            main_context: 0,
            child_context: 0,
            self_target_branches: 0,
            self_target_native_register_blocks: 0,
            self_target_native_register_instructions: 0,
            work: 0,
            maximum_work: MAX_WORK,
            disabled: false,
            overflow: false,
            private_samples_requested: false,
            private_samples: None,
        }
    }
}

// A parent derives Debug. Never delegate to private keys or opaque identities.
impl fmt::Debug for BlockSamples {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockSamples")
            .field("completed", &self.completed)
            .field("recent", &self.recent)
            .field("distinct", &self.distinct)
            .field("max_repeat", &self.max_repeat)
            .field("exit_codes_retained", &self.exit_codes_retained)
            .field("main_context", &self.main_context)
            .field("child_context", &self.child_context)
            .field("disabled", &self.disabled)
            .field("overflow", &self.overflow)
            .finish()
    }
}

impl BlockSamples {
    fn admit(&mut self, cost: u64) -> bool {
        if self.disabled {
            return false;
        }
        let Some(next) = self
            .work
            .checked_add(cost)
            .filter(|n| *n <= self.maximum_work)
        else {
            self.disabled = true;
            return false;
        };
        self.work = next;
        true
    }

    pub(super) fn begin_epoch(&mut self, epoch: SourceIdentity) {
        if self.epoch == Some(epoch) {
            return;
        }
        if let Some(samples) = &mut self.private_samples {
            samples.clear();
        }
        if !self.admit(CAPACITY as u64) {
            // Logical invalidation needs no scan when observation is disabled.
            self.recent = 0;
            self.distinct = 0;
            self.max_repeat = 0;
            self.planned = None;
            return;
        }
        let Some(changes) = self
            .epoch_changes
            .checked_add(u64::from(self.epoch.is_some()))
        else {
            self.disabled = true;
            self.overflow = true;
            return;
        };
        self.entries.fill(None);
        self.cursor = 0;
        self.recent = 0;
        self.distinct = 0;
        self.max_repeat = 0;
        self.planned = None;
        self.epoch = Some(epoch);
        self.epoch_changes = changes;
    }

    pub(super) fn prepare(
        &mut self,
        epoch: SourceIdentity,
        block: &MaterializedBlockPlan<'_, '_, '_>,
        maximum_records: u64,
        is_main: bool,
    ) -> Option<PendingSample> {
        let self_target = matches!(block.exit().kind(), ExitKind::Conditional { target, .. }
            if target == block.guest_pc());
        self.prepare_observed_records(
            Key {
                epoch,
                module: block.module().index(),
                pc: block.guest_pc(),
            },
            block.exit().instruction().map(|i| i.code()),
            block.instructions().iter().map(|record| SampleRecord {
                instruction: record.instruction(),
                emitted: record.emitted_len() != 0,
                memory: record.immutable_memory_summary(),
            }),
            maximum_records,
            is_main,
            self_target,
        )
    }

    #[cfg(test)]
    fn prepare_records<'a>(
        &mut self,
        key: Key,
        planned: Option<Code>,
        instructions: impl ExactSizeIterator<Item = &'a Instruction>,
        maximum_records: u64,
        is_main: bool,
    ) -> Option<PendingSample> {
        self.prepare_observed_records(
            key,
            planned,
            instructions.map(|instruction| SampleRecord {
                instruction,
                emitted: false,
                memory: None,
            }),
            maximum_records,
            is_main,
            false,
        )
    }

    fn prepare_observed_records<'a>(
        &mut self,
        key: Key,
        planned: Option<Code>,
        instructions: impl ExactSizeIterator<Item = SampleRecord<'a>>,
        maximum_records: u64,
        is_main: bool,
        self_target: bool,
    ) -> Option<PendingSample> {
        if self.disabled {
            return None;
        }
        let Ok(count) = u64::try_from(instructions.len()) else {
            self.disabled = true;
            self.overflow = true;
            return None;
        };
        if self.epoch != Some(key.epoch) || count > maximum_records {
            self.disabled = true;
            return None;
        }
        let Some(cost) = count
            .checked_mul(RECORD_WORK)
            .and_then(|n| n.checked_add(FIXED_WORK))
        else {
            self.disabled = true;
            self.overflow = true;
            return None;
        };
        if !self.admit(cost) {
            return None;
        }
        let mut counts = self.counts;
        let Some(completed) = self.completed.checked_add(1) else {
            self.disabled = true;
            self.overflow = true;
            return None;
        };
        let Some(records) = self.records.checked_add(count) else {
            self.disabled = true;
            self.overflow = true;
            return None;
        };
        let mut native_register = self_target && count > 1;
        for (index, record) in instructions.enumerate() {
            let instruction = record.instruction;
            if let Some((form, kind)) = candidate(instruction) {
                let Some(next) = counts[form][kind].checked_add(1) else {
                    self.disabled = true;
                    self.overflow = true;
                    return None;
                };
                counts[form][kind] = next;
            }
            if self_target {
                if index as u64 + 1 == count {
                    // Only the known final, nonemitted direct Jcc is excluded
                    // from the body. It must target this exact block start.
                    native_register &= !record.emitted
                        && instruction.flow_control() == FlowControl::ConditionalBranch
                        && instruction.op0_kind() == OpKind::NearBranch64
                        && instruction.near_branch_target() == key.pc;
                } else {
                    native_register &= native_register_record(&record);
                }
            }
        }
        let self_target_branches = self
            .self_target_branches
            .checked_add(u64::from(self_target));
        let self_target_native_register_blocks = self
            .self_target_native_register_blocks
            .checked_add(u64::from(native_register));
        let self_target_native_register_instructions = self
            .self_target_native_register_instructions
            .checked_add(if native_register { count - 1 } else { 0 });
        let (
            Some(self_target_branches),
            Some(self_target_native_register_blocks),
            Some(self_target_native_register_instructions),
        ) = (
            self_target_branches,
            self_target_native_register_blocks,
            self_target_native_register_instructions,
        )
        else {
            self.disabled = true;
            self.overflow = true;
            return None;
        };
        let main_context = self.main_context.checked_add(u64::from(is_main));
        let child_context = self.child_context.checked_add(u64::from(!is_main));
        let (Some(main_context), Some(child_context)) = (main_context, child_context) else {
            self.disabled = true;
            self.overflow = true;
            return None;
        };
        let exit = self.prepare_exit(planned)?;
        Some(PendingSample {
            key,
            is_main,
            planned,
            completed,
            records,
            counts,
            exit,
            main_context,
            child_context,
            self_target_branches,
            self_target_native_register_blocks,
            self_target_native_register_instructions,
        })
    }

    fn prepare_exit(&mut self, planned: Option<Code>) -> Option<ExitUpdate> {
        // FIXED_WORK reserves all 64 visits before this bounded lookup. The
        // first 64 distinct public Codes remain retained across source epochs.
        if let Some(code) = planned {
            for (index, entry) in self.exits.iter().enumerate() {
                match entry {
                    Some((found, count)) if *found == code => {
                        if let Some(count) = count.checked_add(1) {
                            return Some(ExitUpdate::Code { index, code, count });
                        }
                        self.disabled = true;
                        self.overflow = true;
                        return None;
                    }
                    None => {
                        return Some(ExitUpdate::Code {
                            index,
                            code,
                            count: 1,
                        });
                    }
                    _ => {}
                }
            }
            if let Some(next) = self.exit_overflow.checked_add(1) {
                return Some(ExitUpdate::Overflow(next));
            }
        } else if let Some(next) = self.exit_no_instruction.checked_add(1) {
            return Some(ExitUpdate::NoInstruction(next));
        }
        self.disabled = true;
        self.overflow = true;
        None
    }

    pub(super) fn complete(&mut self, pending: PendingSample) {
        // All visits/counter additions were admitted before guest dispatch.
        if let Some(samples) = &mut self.private_samples {
            samples.clear();
        }
        let old = self.entries[self.cursor].take();
        if old.is_some_and(|entry| entry.frequency == 1) {
            self.distinct -= 1;
        }
        let mut frequency = 1;
        let mut maximum = 0;
        let mut planned = None;
        for entry in self.entries.iter_mut().flatten() {
            if old.is_some_and(|previous| previous.key == entry.key) {
                entry.frequency -= 1;
            }
            if entry.key == pending.key {
                entry.frequency += 1;
                frequency += 1;
            }
            if entry.frequency > maximum {
                maximum = entry.frequency;
                planned = entry.planned;
            }
        }
        if frequency == 1 {
            self.distinct += 1;
        }
        if frequency > maximum {
            maximum = frequency;
            planned = pending.planned;
        }
        self.entries[self.cursor] = Some(Entry {
            key: pending.key,
            is_main: pending.is_main,
            planned: pending.planned,
            frequency,
        });
        self.cursor = (self.cursor + 1) % CAPACITY;
        self.recent = self.recent.saturating_add(1).min(CAPACITY as u8);
        self.max_repeat = maximum;
        self.planned = planned;
        self.completed = pending.completed;
        self.records = pending.records;
        self.counts = pending.counts;
        match pending.exit {
            ExitUpdate::Code { index, code, count } => {
                if self.exits[index].is_none() {
                    self.exit_codes_retained += 1;
                }
                self.exits[index] = Some((code, count));
            }
            ExitUpdate::Overflow(next) => self.exit_overflow = next,
            ExitUpdate::NoInstruction(next) => self.exit_no_instruction = next,
        }
        self.main_context = pending.main_context;
        self.child_context = pending.child_context;
        self.self_target_branches = pending.self_target_branches;
        self.self_target_native_register_blocks = pending.self_target_native_register_blocks;
        self.self_target_native_register_instructions =
            pending.self_target_native_register_instructions;
    }

    pub(super) fn protocol_fields(&self) -> String {
        // One final RESULT only. All labels are closed ISA names; keys stay private.
        let mut fields = format!(
            "\tsample_observer_disabled={}\tsample_observer_overflow={}\tsample_observer_work={}\tsample_observer_work_limit={}\tsample_capacity={}\tsample_completed={}\tsample_records={}\tsample_recent={}\tsample_distinct={}\tsample_max_repeat={}\tsample_epoch_changes={}",
            u8::from(self.disabled),
            u8::from(self.overflow),
            self.work,
            self.maximum_work,
            CAPACITY,
            self.completed,
            self.records,
            self.recent,
            self.distinct,
            self.max_repeat,
            self.epoch_changes
        );
        match self.planned {
            Some(code) => {
                let _ = write!(fields, "\tsample_planned_exit={code:?}");
            }
            None => fields.push_str("\tsample_planned_exit=none"),
        }
        fields.push_str("\tsample_fp_forms=");
        for (index, code) in FORMS.iter().enumerate() {
            if index != 0 {
                fields.push(',');
            }
            let _ = write!(
                fields,
                "{code:?}:{}:{}",
                self.counts[index][0], self.counts[index][1]
            );
        }
        let _ = write!(
            fields,
            "\tsample_exit_capacity={EXIT_CAPACITY}\tsample_exit_codes_retained={}\tsample_exit_overflow={}\tsample_exit_no_instruction={}\tsample_main_context={}\tsample_child_context={}\tsample_self_target_branches={}\tsample_self_target_native_register_blocks={}\tsample_self_target_native_register_instructions={}\tsample_exit_top=",
            self.exit_codes_retained,
            self.exit_overflow,
            self.exit_no_instruction,
            self.main_context,
            self.child_context,
            self.self_target_branches,
            self.self_target_native_register_blocks,
            self.self_target_native_register_instructions
        );
        // Sorting copies only 64 inline catalogue-code/count pairs, once when
        // reporting. Counts descend, with static Code order breaking ties.
        let mut sorted = self.exits;
        sorted.sort_unstable_by_key(|entry| match entry {
            Some((code, count)) => (std::cmp::Reverse(*count), *code as u32),
            None => (std::cmp::Reverse(0), u32::MAX),
        });
        let mut any = false;
        for (code, count) in sorted.iter().flatten().take(4) {
            if any {
                fields.push(',');
            }
            let _ = write!(fields, "{code:?}:{count}");
            any = true;
        }
        if !any {
            fields.push_str("none");
        }
        fields
    }
}

fn native_register_record(record: &SampleRecord<'_>) -> bool {
    record.emitted
        && record.memory.is_some_and(|(has_memory, dependencies)| {
            !has_memory && dependencies.written & (1 << 4) == 0
        })
        && record.instruction.flow_control() == FlowControl::Next
        && !record
            .instruction
            .op_kinds()
            .any(|kind| kind == OpKind::Memory)
        && !record.instruction.is_stack_instruction()
        && !record.instruction.is_save_restore_instruction()
        && !record.instruction.is_privileged()
        && !matches!(
            record.instruction.mnemonic(),
            Mnemonic::Div | Mnemonic::Idiv | Mnemonic::Lfence | Mnemonic::Mfence | Mnemonic::Sfence
        )
}

fn candidate(instruction: &Instruction) -> Option<(usize, usize)> {
    let form = FORMS.iter().position(|code| *code == instruction.code())?;
    let width = if form % 2 == 0 { 16 } else { 32 };
    if instruction.op_count() != 3
        || instruction.has_segment_prefix()
        || instruction.has_lock_prefix()
        || instruction.has_rep_prefix()
        || instruction.has_repne_prefix()
        || instruction.has_xacquire_prefix()
        || instruction.has_xrelease_prefix()
    {
        return None;
    }
    let vector = |operand| {
        if instruction.op_kind(operand) != OpKind::Register {
            return false;
        }
        let register = instruction.op_register(operand);
        register.size() == width
            && if width == 16 {
                register.is_xmm() && register <= Register::XMM15
            } else {
                register.is_ymm() && register <= Register::YMM15
            }
    };
    if !vector(0) || !vector(1) {
        return None;
    }
    if vector(2) {
        Some((form, 0))
    } else if instruction.op2_kind() == OpKind::Memory && instruction.memory_size().size() == width
    {
        Some((form, 1))
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
