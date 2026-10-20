// SPDX-License-Identifier: GPL-2.0-only
//! Optional bounded observations, never execution or source authority.

use pentax_codegen::{Code, Instruction, MaterializedBlockPlan, OpKind, Register};
use pentax_linker::SourceIdentity;
use std::fmt::{self, Write};

const CAPACITY: usize = 64;
const MAX_WORK: u64 = 64 * 1024 * 1024;
const FIXED_WORK: u64 = 4 * CAPACITY as u64 + 64;
const RECORD_WORK: u64 = 32;

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
    planned: Option<Code>,
    frequency: u8,
}

pub(in crate::native_session) struct PendingSample {
    key: Key,
    planned: Option<Code>,
    completed: u64,
    records: u64,
    counts: [[u64; 2]; 12],
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
    work: u64,
    maximum_work: u64,
    disabled: bool,
    overflow: bool,
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
            work: 0,
            maximum_work: MAX_WORK,
            disabled: false,
            overflow: false,
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
    ) -> Option<PendingSample> {
        self.prepare_records(
            Key {
                epoch,
                module: block.module().index(),
                pc: block.guest_pc(),
            },
            block.exit().instruction().map(|i| i.code()),
            block
                .instructions()
                .iter()
                .map(|record| record.instruction()),
            maximum_records,
        )
    }

    fn prepare_records<'a>(
        &mut self,
        key: Key,
        planned: Option<Code>,
        instructions: impl ExactSizeIterator<Item = &'a Instruction>,
        maximum_records: u64,
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
        for instruction in instructions {
            if let Some((form, kind)) = candidate(instruction) {
                let Some(next) = counts[form][kind].checked_add(1) else {
                    self.disabled = true;
                    self.overflow = true;
                    return None;
                };
                counts[form][kind] = next;
            }
        }
        Some(PendingSample {
            key,
            planned,
            completed,
            records,
            counts,
        })
    }

    pub(super) fn complete(&mut self, pending: PendingSample) {
        // All visits/counter additions were admitted before guest dispatch.
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
        fields
    }
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
