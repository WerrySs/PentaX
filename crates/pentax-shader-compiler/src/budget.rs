// SPDX-License-Identifier: GPL-2.0-only
// PentaX bounded-resource accounting for the KytyPS5-Performance compiler port
// (209dffe9032d72749257ce18680d506b891284de). The pinned source has no such
// bounds; every arena of this port is grown through `Budget`.

use std::{cell::Cell, mem::size_of};

use crate::CompileError;

/// Independent, finite limits for every compiler stage. Counts bound the
/// number of records; `max_bytes` bounds the summed Rust-reported capacity of
/// every arena grown by the compiler (allocator bookkeeping is excluded).
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Limits passed to the read-only `pentax-shader` linear decoder.
    pub decode: pentax_shader::Limits,
    /// CFG blocks, including synthetic structurization blocks.
    pub max_blocks: usize,
    /// CFG successor/predecessor edges.
    pub max_edges: usize,
    /// IR instructions (including phis and forwarded identities).
    pub max_ir_instructions: usize,
    /// IR operand slots.
    pub max_ir_operands: usize,
    /// SSA definition-table slots (blocks times tracked register variables).
    pub max_ssa_slots: usize,
    /// Emitted SPIR-V words (all sections together).
    pub max_spirv_words: usize,
    /// SPIR-V result ids (the module bound).
    pub max_spirv_ids: u32,
    /// Summed arena capacity bytes.
    pub max_bytes: usize,
    /// Work units: one per scanned record, table entry, graph edge, bit-set
    /// word, copied element and emitted word.
    pub max_work: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            decode: pentax_shader::Limits::default(),
            max_blocks: 4096,
            max_edges: 16_384,
            max_ir_instructions: 262_144,
            max_ir_operands: 1_048_576,
            max_ssa_slots: 4_194_304,
            max_spirv_words: 4_194_304,
            max_spirv_ids: 1 << 22,
            max_bytes: 256 * 1024 * 1024,
            max_work: 1 << 30,
        }
    }
}

/// The bound that a compilation exceeded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LimitKind {
    Blocks,
    Edges,
    IrInstructions,
    IrOperands,
    SsaSlots,
    SpirvWords,
    SpirvIds,
    Bytes,
    Work,
    /// A fixed source table (64 buffers, 32 PS inputs, 32 push dwords, ...).
    SourceTable,
}

/// Monotonic work and capacity accounting shared by all stages of one call.
#[derive(Debug)]
pub(crate) struct Budget {
    pub limits: Limits,
    work: Cell<u64>,
    bytes: usize,
}

impl Budget {
    pub fn new(limits: &Limits) -> Self {
        Self {
            limits: *limits,
            work: Cell::new(0),
            bytes: 0,
        }
    }

    pub fn work(&self) -> u64 {
        self.work.get()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn charge(&self, units: u64) -> Result<(), CompileError> {
        let next = self
            .work
            .get()
            .checked_add(units)
            .ok_or(CompileError::limit(LimitKind::Work))?;
        if next > self.limits.max_work {
            return Err(CompileError::limit(LimitKind::Work));
        }
        self.work.set(next);
        Ok(())
    }

    pub fn charge_usize(&self, units: usize) -> Result<(), CompileError> {
        self.charge(u64::try_from(units).map_err(|_| CompileError::limit(LimitKind::Work))?)
    }

    /// Ensure room for `additional` more elements, growing geometrically and
    /// charging both the capacity delta (bytes) and the elements a
    /// reallocation may move (work). `count_limit` bounds the element count.
    pub fn reserve<T>(
        &mut self,
        vec: &mut Vec<T>,
        additional: usize,
        count_limit: usize,
        kind: LimitKind,
    ) -> Result<(), CompileError> {
        let needed = vec
            .len()
            .checked_add(additional)
            .ok_or(CompileError::limit(kind))?;
        if needed > count_limit {
            return Err(CompileError::limit(kind));
        }
        if needed <= vec.capacity() {
            return Ok(());
        }
        let target = vec
            .capacity()
            .saturating_mul(2)
            .max(needed)
            .max(8)
            .min(count_limit);
        let old_bytes = vec.capacity().saturating_mul(size_of::<T>());
        self.charge_usize(vec.len())?;
        let extra = target - vec.len();
        let new_bytes_estimate = target
            .checked_mul(size_of::<T>())
            .ok_or(CompileError::limit(LimitKind::Bytes))?;
        if self
            .bytes
            .checked_add(new_bytes_estimate.saturating_sub(old_bytes))
            .is_none_or(|total| total > self.limits.max_bytes)
        {
            return Err(CompileError::limit(LimitKind::Bytes));
        }
        vec.try_reserve_exact(extra)
            .map_err(|_| CompileError::Allocation)?;
        let new_bytes = vec.capacity().saturating_mul(size_of::<T>());
        let total = self
            .bytes
            .checked_add(new_bytes.saturating_sub(old_bytes))
            .ok_or(CompileError::limit(LimitKind::Bytes))?;
        if total > self.limits.max_bytes {
            return Err(CompileError::limit(LimitKind::Bytes));
        }
        self.bytes = total;
        Ok(())
    }

    pub fn push<T>(
        &mut self,
        vec: &mut Vec<T>,
        value: T,
        count_limit: usize,
        kind: LimitKind,
    ) -> Result<(), CompileError> {
        self.reserve(vec, 1, count_limit, kind)?;
        self.charge(1)?;
        vec.push(value);
        Ok(())
    }

    /// A zero-initialized table of `len` elements, fully charged.
    pub fn table<T: Clone>(
        &mut self,
        len: usize,
        fill: T,
        count_limit: usize,
        kind: LimitKind,
    ) -> Result<Vec<T>, CompileError> {
        let mut vec = Vec::new();
        self.reserve(&mut vec, len, count_limit, kind)?;
        self.charge_usize(len)?;
        vec.resize(len, fill);
        Ok(vec)
    }
}
