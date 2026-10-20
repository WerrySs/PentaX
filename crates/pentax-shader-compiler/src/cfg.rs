// SPDX-License-Identifier: GPL-2.0-only
// Direct Rust adaptation of KytyPS5-Performance at
// 209dffe9032d72749257ce18680d506b891284de, frontend/cfg/ShaderCFG.{h,cpp}:
// BuildGraph labels/blocks/terminators, PruneUnreachableBlocks, dominators,
// post-dominators, back edges, natural loops, Tarjan components, and the
// Structurize pipeline (CanonicalizeNaturalLoops, SplitSharedMergeBlocks,
// IsolateSemanticLoopHeaders, merge assignment, StructureBreakRegions).
// PentaX modifications: flat bit-set analyses with charged work, an iterative
// Tarjan walk, typed failures instead of EXIT, and typed refusals for the
// source's S_SETPC_B64 jump tables, dispatcher fallback and goto routing.

use pentax_shader::Opcode;

use crate::CompileError;
use crate::budget::{Budget, LimitKind};
use crate::error::{CfgErrorKind, UnsupportedReason};
use crate::program::{BranchTarget, ShaderProgram, is_conditional_branch};

pub(crate) const NONE: u32 = u32::MAX;

/// Source `CFG::BranchCondition` (indirect/goto forms are not produced).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BranchCondition {
    Always,
    SccZero,
    SccNonZero,
    VccZero,
    VccNonZero,
    ExecZero,
    ExecNonZero,
    ScalarInstruction,
}

/// Source `CFG::TerminatorKind` (indirect branches are refused).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminatorKind {
    Branch,
    ConditionalBranch,
    Return,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Terminator {
    pub kind: TerminatorKind,
    pub condition: BranchCondition,
    pub true_block: u32,
    pub false_block: u32,
    pub merge_block: u32,
    pub continue_block: u32,
    pub loop_header: bool,
}

impl Terminator {
    const RETURN: Self = Self {
        kind: TerminatorKind::Return,
        condition: BranchCondition::Always,
        true_block: NONE,
        false_block: NONE,
        merge_block: NONE,
        continue_block: NONE,
        loop_header: false,
    };
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Block {
    pub start_pc: u32,
    pub end_pc: u32,
    pub inst_begin: u32,
    pub inst_end: u32,
    pub succ: [u32; 2],
    pub succ_len: u8,
    pub pred_start: u32,
    pub pred_len: u32,
    pub term: Terminator,
}

impl Block {
    pub fn successors(&self) -> &[u32] {
        &self.succ[..usize::from(self.succ_len)]
    }

    fn is_empty(&self) -> bool {
        self.inst_begin == self.inst_end
    }
}

#[derive(Clone, Copy, Debug)]
struct Loop {
    header: u32,
    continue_block: u32,
    merge: u32,
    body_count: u32,
}

/// Flat bit-set rows.
#[derive(Debug, Default)]
struct Sets {
    words: usize,
    data: Vec<u64>,
}

impl Sets {
    fn row(&self, row: u32) -> &[u64] {
        let start = row as usize * self.words;
        self.data.get(start..start + self.words).unwrap_or(&[])
    }

    fn contains(&self, row: u32, bit: u32) -> bool {
        if bit == NONE {
            return false;
        }
        self.data
            .get(row as usize * self.words + bit as usize / 64)
            .is_some_and(|word| word >> (bit % 64) & 1 != 0)
    }

    fn count(&self, row: u32) -> u32 {
        self.row(row).iter().map(|word| word.count_ones()).sum()
    }
}

/// The working graph of the ported algorithms. Block ids equal indices.
pub(crate) struct Graph<'a> {
    budget: &'a mut Budget,
    pub blocks: Vec<Block>,
    preds: Vec<u32>,
    pub entry: u32,
    dominators: Sets,
    post_dominators: Sets,
    back_edges: Vec<(u32, u32)>,
    loops: Vec<Loop>,
    loop_bodies: Sets,
    irreducible: Option<u32>,
    scratch: Vec<u32>,
    scratch2: Vec<u32>,
    marks: Vec<u8>,
}

fn failure(graph: &Graph<'_>, block: u32, reason: &'static str) -> CompileError {
    CompileError::cfg(
        graph.blocks.get(block as usize).map(|b| b.start_pc),
        CfgErrorKind::StructuredControlFlow(reason),
    )
}

impl<'a> Graph<'a> {
    fn block(&self, id: u32) -> Option<&Block> {
        self.blocks.get(id as usize)
    }

    pub fn predecessors(&self, id: u32) -> &[u32] {
        match self.block(id) {
            Some(block) => {
                let start = block.pred_start as usize;
                self.preds
                    .get(start..start + block.pred_len as usize)
                    .unwrap_or(&[])
            }
            None => &[],
        }
    }

    fn dominates(&self, dominator: u32, block: u32) -> bool {
        (block as usize) < self.blocks.len() && self.dominators.contains(block, dominator)
    }

    fn post_dominates(&self, post_dominator: u32, block: u32) -> bool {
        (block as usize) < self.blocks.len() && self.post_dominators.contains(block, post_dominator)
    }

    fn charge(&mut self, units: usize) -> Result<(), CompileError> {
        self.budget.charge_usize(units)
    }

    // ---------------------------------------------------------------- build

    pub fn build(program: &'a ShaderProgram, budget: &'a mut Budget) -> Result<Self, CompileError> {
        let words = program.source_words().len();
        let end_pc = program.end_pc();
        // Labels as a bit set over word indices 0..=words.
        let mut labels = budget.table(words + 1, false, usize::MAX, LimitKind::Bytes)?;
        let mut set_label = |pc: u32| {
            if let Some(label) = labels.get_mut(pc as usize / 4) {
                *label = true;
            }
        };
        set_label(0);
        set_label(end_pc);
        for (index, inst) in program.instructions().enumerate() {
            budget.charge(1)?;
            let opcode = inst.opcode();
            let next_pc = inst.byte_offset() + 4 * inst.raw_words().len() as u32;
            match opcode {
                Opcode::SSetpcB64 => {
                    return Err(CompileError::unsupported(
                        inst.byte_offset(),
                        Some(opcode),
                        UnsupportedReason::IndirectBranch,
                    ));
                }
                Opcode::SCbranchCdbgsys | Opcode::SCbranchCdbgsysOrUser => {
                    return Err(CompileError::unsupported(
                        inst.byte_offset(),
                        Some(opcode),
                        UnsupportedReason::ConditionalDebugBranch,
                    ));
                }
                _ => {}
            }
            if crate::program::is_direct_branch(opcode) {
                match program.branch_target(index) {
                    Some(BranchTarget::Instruction(target)) => {
                        let pc = program
                            .instruction(target)
                            .ok_or(CompileError::Internal("branch target index"))?
                            .byte_offset();
                        set_label(pc);
                    }
                    Some(BranchTarget::End) => set_label(end_pc),
                    None => {
                        return Err(CompileError::cfg(
                            Some(inst.byte_offset()),
                            CfgErrorKind::InvalidBranchTarget,
                        ));
                    }
                }
                if next_pc <= end_pc {
                    set_label(next_pc);
                }
            } else if opcode == Opcode::SEndpgm {
                set_label(next_pc);
            }
        }

        let mut blocks: Vec<Block> = Vec::new();
        let mut label_pcs: Vec<u32> = Vec::new();
        for (word, &label) in labels.iter().enumerate() {
            budget.charge(1)?;
            if label {
                budget.push(
                    &mut label_pcs,
                    (word * 4) as u32,
                    usize::MAX,
                    LimitKind::Bytes,
                )?;
            }
        }
        let max_blocks = budget.limits.max_blocks;
        for (i, &start) in label_pcs.iter().enumerate() {
            if start > end_pc {
                continue;
            }
            let end = label_pcs.get(i + 1).copied().unwrap_or(end_pc);
            let inst_begin = program.lower_bound(budget, start)?;
            let inst_end = program.lower_bound(budget, end)?;
            if start != end_pc
                && program
                    .instruction(inst_begin)
                    .is_none_or(|inst| inst.byte_offset() != start)
            {
                return Err(CompileError::cfg(Some(start), CfgErrorKind::InvalidLabel));
            }
            budget.charge(32)?;
            budget.push(
                &mut blocks,
                Block {
                    start_pc: start,
                    end_pc: end,
                    inst_begin: inst_begin as u32,
                    inst_end: inst_end as u32,
                    succ: [NONE; 2],
                    succ_len: 0,
                    pred_start: 0,
                    pred_len: 0,
                    term: Terminator::RETURN,
                },
                max_blocks,
                LimitKind::Blocks,
            )?;
        }
        // Word index -> block id for block starts.
        let mut pc_to_block = budget.table(words + 1, NONE, usize::MAX, LimitKind::Bytes)?;
        for (id, block) in blocks.iter().enumerate() {
            if let Some(slot) = pc_to_block.get_mut(block.start_pc as usize / 4) {
                *slot = id as u32;
            }
        }
        let lookup = |pc: u32| pc_to_block.get(pc as usize / 4).copied().unwrap_or(NONE);
        for entry in &mut blocks {
            budget.charge(1)?;
            let block = *entry;
            let mut term = Terminator::RETURN;
            if !block.is_empty() {
                let last_index = block.inst_end as usize - 1;
                let last = program
                    .instruction(last_index)
                    .ok_or(CompileError::Internal("block instruction range"))?;
                let next_pc = last.byte_offset() + 4 * last.raw_words().len() as u32;
                let target_pc = |program: &ShaderProgram| match program.branch_target(last_index) {
                    Some(BranchTarget::Instruction(target)) => {
                        program.instruction(target).map(|inst| inst.byte_offset())
                    }
                    Some(BranchTarget::End) => Some(program.end_pc()),
                    None => None,
                };
                if last.opcode() == Opcode::SEndpgm {
                    term.kind = TerminatorKind::Return;
                } else if last.opcode() == Opcode::SBranch {
                    let target = target_pc(program).map_or(NONE, lookup);
                    if target == NONE {
                        return Err(CompileError::cfg(
                            Some(last.byte_offset()),
                            CfgErrorKind::InvalidBranchTarget,
                        ));
                    }
                    term.kind = TerminatorKind::Branch;
                    term.true_block = target;
                } else if is_conditional_branch(last.opcode()) {
                    let target = target_pc(program).map_or(NONE, lookup);
                    let fallthrough = lookup(next_pc);
                    if target == NONE {
                        return Err(CompileError::cfg(
                            Some(last.byte_offset()),
                            CfgErrorKind::InvalidBranchTarget,
                        ));
                    }
                    if fallthrough == NONE {
                        return Err(CompileError::cfg(
                            Some(last.byte_offset()),
                            CfgErrorKind::MissingFallthrough,
                        ));
                    }
                    term.kind = TerminatorKind::ConditionalBranch;
                    term.condition = match last.opcode() {
                        Opcode::SCbranchScc0 => BranchCondition::SccZero,
                        Opcode::SCbranchScc1 => BranchCondition::SccNonZero,
                        Opcode::SCbranchVccz => BranchCondition::VccZero,
                        Opcode::SCbranchVccnz => BranchCondition::VccNonZero,
                        Opcode::SCbranchExecz => BranchCondition::ExecZero,
                        Opcode::SCbranchExecnz => BranchCondition::ExecNonZero,
                        _ => BranchCondition::ScalarInstruction,
                    };
                    term.true_block = target;
                    term.false_block = fallthrough;
                } else {
                    let next = lookup(block.end_pc);
                    if next != NONE && block.end_pc != block.start_pc {
                        term.kind = TerminatorKind::Branch;
                        term.true_block = next;
                    }
                }
            }
            entry.term = term;
            set_successors(entry);
        }
        drop(labels);
        let mut graph = Self {
            budget,
            blocks,
            preds: Vec::new(),
            entry: 0,
            dominators: Sets::default(),
            post_dominators: Sets::default(),
            back_edges: Vec::new(),
            loops: Vec::new(),
            loop_bodies: Sets::default(),
            irreducible: None,
            scratch: Vec::new(),
            scratch2: Vec::new(),
            marks: Vec::new(),
        };
        graph.rebuild_predecessors()?;
        graph.prune_unreachable()?;
        graph.recompute()?;
        if let Some(block) = graph.irreducible {
            return Err(CompileError::cfg(
                graph.block(block).map(|b| b.start_pc),
                CfgErrorKind::IrreducibleControlFlow,
            ));
        }
        Ok(graph)
    }

    fn rebuild_predecessors(&mut self) -> Result<(), CompileError> {
        let count = self.blocks.len();
        let max_edges = self.budget.limits.max_edges;
        let mut total = 0usize;
        for block in &mut self.blocks {
            set_successors(block);
            block.pred_len = 0;
        }
        for id in 0..count {
            let succ = self.blocks[id].succ;
            let len = usize::from(self.blocks[id].succ_len);
            for &s in succ.iter().take(len) {
                if let Some(target) = self.blocks.get_mut(s as usize) {
                    target.pred_len += 1;
                    total += 1;
                }
            }
        }
        if total > max_edges {
            return Err(CompileError::limit(LimitKind::Edges));
        }
        self.budget.charge_usize(count + total)?;
        let mut offset = 0u32;
        for block in &mut self.blocks {
            block.pred_start = offset;
            offset += block.pred_len;
            block.pred_len = 0;
        }
        self.preds.clear();
        self.budget
            .reserve(&mut self.preds, total, max_edges, LimitKind::Edges)?;
        self.preds.resize(total, NONE);
        // Predecessors are visited in ascending id order, so each list is sorted.
        for id in 0..count {
            let succ = self.blocks[id].succ;
            let len = usize::from(self.blocks[id].succ_len);
            for &s in succ.iter().take(len) {
                if let Some(target) = self.blocks.get_mut(s as usize) {
                    let slot = (target.pred_start + target.pred_len) as usize;
                    target.pred_len += 1;
                    if let Some(entry) = self.preds.get_mut(slot) {
                        *entry = id as u32;
                    }
                }
            }
        }
        Ok(())
    }

    fn prune_unreachable(&mut self) -> Result<(), CompileError> {
        let count = self.blocks.len();
        self.marks.clear();
        self.budget
            .reserve(&mut self.marks, count, usize::MAX, LimitKind::Bytes)?;
        self.marks.resize(count, 0);
        self.scratch.clear();
        self.budget
            .push(&mut self.scratch, self.entry, usize::MAX, LimitKind::Bytes)?;
        while let Some(id) = self.scratch.pop() {
            self.budget.charge(1)?;
            match self.marks.get_mut(id as usize) {
                Some(mark) if *mark == 0 => *mark = 1,
                _ => continue,
            }
            let block = self.blocks[id as usize];
            for &s in block.successors() {
                self.budget
                    .push(&mut self.scratch, s, usize::MAX, LimitKind::Bytes)?;
            }
        }
        if self.marks.iter().all(|&mark| mark != 0) {
            return Ok(());
        }
        let mut id_map = Vec::new();
        self.budget
            .reserve(&mut id_map, count, usize::MAX, LimitKind::Bytes)?;
        let mut next = 0u32;
        for &mark in &self.marks {
            id_map.push(if mark != 0 {
                next += 1;
                next - 1
            } else {
                NONE
            });
        }
        let mut kept = 0usize;
        for old in 0..count {
            if self.marks[old] != 0 {
                self.blocks[kept] = self.blocks[old];
                kept += 1;
            }
        }
        self.blocks.truncate(kept);
        self.entry = remap(self.entry, &id_map);
        for block in &mut self.blocks {
            remap_terminator(&mut block.term, &id_map);
        }
        self.rebuild_predecessors()
    }

    // ------------------------------------------------------------- analyses

    fn recompute(&mut self) -> Result<(), CompileError> {
        self.compute_dominators()?;
        self.compute_post_dominators()?;
        self.compute_back_edges()?;
        self.compute_natural_loops()?;
        self.compute_components()
    }

    fn alloc_sets(&mut self, rows: usize) -> Result<Sets, CompileError> {
        let words = self.blocks.len().div_ceil(64).max(1);
        let len = rows
            .checked_mul(words)
            .ok_or(CompileError::limit(LimitKind::Bytes))?;
        let data = self.budget.table(len, 0u64, usize::MAX, LimitKind::Bytes)?;
        Ok(Sets { words, data })
    }

    fn compute_dominators(&mut self) -> Result<(), CompileError> {
        let count = self.blocks.len();
        let mut sets = self.alloc_sets(count)?;
        let words = sets.words;
        let full = full_row(self.budget, count, words)?;
        for id in 0..count {
            self.budget.charge_usize(words + 1)?;
            let row = &mut sets.data[id * words..(id + 1) * words];
            if id as u32 == self.entry {
                set_bit(row, id as u32);
            } else {
                for (dst, src) in row.iter_mut().zip(&full) {
                    *dst = *src;
                }
            }
        }
        let mut next = vec_zero(self.budget, words)?;
        loop {
            let mut changed = false;
            for id in 0..count {
                if id as u32 == self.entry {
                    continue;
                }
                let (pstart, plen) = (
                    self.blocks[id].pred_start as usize,
                    self.blocks[id].pred_len as usize,
                );
                self.budget.charge_usize(words * (plen + 4))?;
                let preds = self.preds.get(pstart..pstart + plen).unwrap_or(&[]);
                next.iter_mut().for_each(|word| *word = 0);
                if preds.is_empty() {
                    set_bit(&mut next, id as u32);
                } else {
                    let first = preds[0] as usize;
                    next.copy_from_slice(&sets.data[first * words..(first + 1) * words]);
                    for &p in &preds[1..] {
                        let p = p as usize;
                        for (dst, src) in
                            next.iter_mut().zip(&sets.data[p * words..(p + 1) * words])
                        {
                            *dst &= *src;
                        }
                    }
                    set_bit(&mut next, id as u32);
                }
                let row = &mut sets.data[id * words..(id + 1) * words];
                if row != next.as_slice() {
                    row.copy_from_slice(&next);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        self.dominators = sets;
        Ok(())
    }

    fn compute_post_dominators(&mut self) -> Result<(), CompileError> {
        let count = self.blocks.len();
        let mut sets = self.alloc_sets(count)?;
        let words = sets.words;
        let full = full_row(self.budget, count, words)?;
        for id in 0..count {
            self.budget.charge_usize(words + 1)?;
            let row = &mut sets.data[id * words..(id + 1) * words];
            if self.blocks[id].succ_len == 0 {
                set_bit(row, id as u32);
            } else {
                row.copy_from_slice(&full);
            }
        }
        let mut next = vec_zero(self.budget, words)?;
        loop {
            let mut changed = false;
            for id in 0..count {
                let block = self.blocks[id];
                self.budget.charge_usize(words * 6)?;
                next.iter_mut().for_each(|word| *word = 0);
                if block.succ_len == 0 {
                    set_bit(&mut next, id as u32);
                } else {
                    let first = block.succ[0] as usize;
                    next.copy_from_slice(&sets.data[first * words..(first + 1) * words]);
                    for &s in &block.successors()[1..] {
                        let s = s as usize;
                        for (dst, src) in
                            next.iter_mut().zip(&sets.data[s * words..(s + 1) * words])
                        {
                            *dst &= *src;
                        }
                    }
                    set_bit(&mut next, id as u32);
                }
                let row = &mut sets.data[id * words..(id + 1) * words];
                if row != next.as_slice() {
                    row.copy_from_slice(&next);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        self.post_dominators = sets;
        Ok(())
    }

    fn compute_back_edges(&mut self) -> Result<(), CompileError> {
        self.back_edges.clear();
        for id in 0..self.blocks.len() as u32 {
            let block = self.blocks[id as usize];
            for &s in block.successors() {
                self.budget.charge(1)?;
                if self.dominates(s, id) {
                    let max_edges = self.budget.limits.max_edges;
                    self.budget
                        .push(&mut self.back_edges, (id, s), max_edges, LimitKind::Edges)?;
                }
            }
        }
        Ok(())
    }

    fn compute_natural_loops(&mut self) -> Result<(), CompileError> {
        let count = self.blocks.len();
        let edges = self.back_edges.len();
        let mut bodies = self.alloc_sets(edges)?;
        let words = bodies.words;
        self.loops.clear();
        for edge in 0..edges {
            let (latch, header) = self.back_edges[edge];
            // Source NaturalLoopBody: predecessors from the latch up to the header.
            let row = edge * words;
            set_bit(&mut bodies.data[row..row + words], header);
            set_bit(&mut bodies.data[row..row + words], latch);
            self.scratch.clear();
            if latch != header {
                self.budget
                    .push(&mut self.scratch, latch, usize::MAX, LimitKind::Bytes)?;
            }
            while let Some(id) = self.scratch.pop() {
                let start = self.blocks[id as usize].pred_start as usize;
                let len = self.blocks[id as usize].pred_len as usize;
                for index in start..start + len {
                    self.budget.charge(1)?;
                    let pred = self.preds[index];
                    let body = &mut bodies.data[row..row + words];
                    if !test_bit(body, pred) {
                        set_bit(body, pred);
                        if pred != header {
                            self.budget.push(
                                &mut self.scratch,
                                pred,
                                usize::MAX,
                                LimitKind::Bytes,
                            )?;
                        }
                    }
                }
            }
            // Exits in ascending order, then the source merge fold.
            let mut merge = NONE;
            let mut first = true;
            for id in 0..count as u32 {
                self.budget.charge(1)?;
                if test_bit(&bodies.data[row..row + words], id) {
                    continue;
                }
                self.budget.charge_usize(self.predecessors(id).len())?;
                let exit = self
                    .predecessors(id)
                    .iter()
                    .any(|&p| test_bit(&bodies.data[row..row + words], p));
                if exit {
                    if first {
                        merge = id;
                        first = false;
                    } else {
                        merge = self.nearest_common_post_dominator(merge, id)?;
                    }
                }
            }
            self.budget.charge_usize(words)?;
            let body_count = bodies.data[row..row + words]
                .iter()
                .map(|word| word.count_ones())
                .sum();
            self.budget.push(
                &mut self.loops,
                Loop {
                    header,
                    continue_block: latch,
                    merge,
                    body_count,
                },
                usize::MAX,
                LimitKind::Edges,
            )?;
        }
        self.loop_bodies = bodies;
        Ok(())
    }

    /// Source `Graph::FindNearestCommonPostDominator`.
    fn nearest_common_post_dominator(&mut self, a: u32, b: u32) -> Result<u32, CompileError> {
        if self.block(a).is_none() || self.block(b).is_none() {
            return Ok(NONE);
        }
        let words = self.post_dominators.words;
        self.scratch2.clear();
        for word in 0..words {
            let common = self.post_dominators.row(a).get(word).copied().unwrap_or(0)
                & self.post_dominators.row(b).get(word).copied().unwrap_or(0);
            let mut bits = common;
            while bits != 0 {
                let bit = bits.trailing_zeros();
                bits &= bits - 1;
                self.budget.push(
                    &mut self.scratch2,
                    (word * 64) as u32 + bit,
                    usize::MAX,
                    LimitKind::Bytes,
                )?;
            }
        }
        let common = std::mem::take(&mut self.scratch2);
        let mut result = common.first().copied().unwrap_or(NONE);
        for &candidate in &common {
            self.budget.charge_usize(common.len())?;
            if common
                .iter()
                .all(|&other| other == candidate || self.post_dominates(other, candidate))
            {
                result = candidate;
                break;
            }
        }
        self.scratch2 = common;
        Ok(result)
    }

    fn compute_components(&mut self) -> Result<(), CompileError> {
        // Iterative Tarjan over blocks in id order (source TarjanVisit).
        let count = self.blocks.len();
        self.irreducible = None;
        let mut index = self
            .budget
            .table(count, NONE, usize::MAX, LimitKind::Bytes)?;
        let mut low = self
            .budget
            .table(count, NONE, usize::MAX, LimitKind::Bytes)?;
        let mut on_stack = self
            .budget
            .table(count, false, usize::MAX, LimitKind::Bytes)?;
        let mut component = self
            .budget
            .table(count, NONE, usize::MAX, LimitKind::Bytes)?;
        let mut stack: Vec<u32> = Vec::new();
        let mut call: Vec<(u32, u8)> = Vec::new();
        let mut members: Vec<u32> = Vec::new();
        let mut next_index = 0u32;
        let mut components = 0u32;
        for root in 0..count as u32 {
            if index[root as usize] != NONE {
                continue;
            }
            self.budget
                .push(&mut call, (root, 0), usize::MAX, LimitKind::Bytes)?;
            index[root as usize] = next_index;
            low[root as usize] = next_index;
            next_index += 1;
            self.budget
                .push(&mut stack, root, usize::MAX, LimitKind::Bytes)?;
            on_stack[root as usize] = true;
            while let Some(&(node, child)) = call.last() {
                self.budget.charge(1)?;
                let block = self.blocks[node as usize];
                if usize::from(child) < usize::from(block.succ_len) {
                    if let Some(top) = call.last_mut() {
                        top.1 += 1;
                    }
                    let succ = block.succ[usize::from(child)];
                    let s = succ as usize;
                    if index[s] == NONE {
                        index[s] = next_index;
                        low[s] = next_index;
                        next_index += 1;
                        self.budget
                            .push(&mut stack, succ, usize::MAX, LimitKind::Bytes)?;
                        on_stack[s] = true;
                        self.budget
                            .push(&mut call, (succ, 0), usize::MAX, LimitKind::Bytes)?;
                    } else if on_stack[s] {
                        low[node as usize] = low[node as usize].min(index[s]);
                    }
                    continue;
                }
                call.pop();
                if let Some(&(parent, _)) = call.last() {
                    low[parent as usize] = low[parent as usize].min(low[node as usize]);
                }
                if low[node as usize] != index[node as usize] {
                    continue;
                }
                let mut first_member = NONE;
                members.clear();
                while let Some(member) = stack.pop() {
                    self.budget.charge(1)?;
                    on_stack[member as usize] = false;
                    component[member as usize] = components;
                    first_member = first_member.min(member);
                    self.budget
                        .push(&mut members, member, usize::MAX, LimitKind::Bytes)?;
                    if member == node {
                        break;
                    }
                }
                let id = components;
                components += 1;
                let mut cyclic = members.len() > 1;
                let mut entries = 0u32;
                let mut first_entry = NONE;
                for &member in &members {
                    let block = self.blocks[member as usize];
                    if block.successors().contains(&member) {
                        cyclic = true;
                    }
                    let outside = self
                        .predecessors(member)
                        .iter()
                        .any(|&p| component[p as usize] != id);
                    self.budget
                        .charge_usize(1 + self.predecessors(member).len())?;
                    if outside {
                        entries += 1;
                        first_entry = first_entry.min(member);
                    }
                }
                if cyclic && entries > 1 && self.irreducible.is_none() {
                    self.irreducible = Some(if first_entry != NONE {
                        first_entry
                    } else {
                        first_member
                    });
                }
            }
        }
        Ok(())
    }

    // -------------------------------------------------------- loop queries

    fn in_body(&self, loop_index: usize, block: u32) -> bool {
        self.loop_bodies.contains(loop_index as u32, block)
    }

    fn innermost_loop(&self, block: u32) -> Result<Option<usize>, CompileError> {
        self.budget.charge_usize(self.loops.len())?;
        let mut best: Option<usize> = None;
        for (index, l) in self.loops.iter().enumerate() {
            if self.in_body(index, block)
                && best.is_none_or(|b| l.body_count < self.loops[b].body_count)
            {
                best = Some(index);
            }
        }
        Ok(best)
    }

    fn inside_loop_construct(&self, l: &Loop, block: u32) -> bool {
        block != NONE
            && block != l.merge
            && block != l.continue_block
            && self.dominates(l.header, block)
            && !self.dominates(l.merge, block)
    }

    fn is_loop_control_gateway(&self, l: &Loop, block: u32) -> bool {
        let Some(b) = self.block(block) else {
            return false;
        };
        if b.term.kind != TerminatorKind::ConditionalBranch {
            return false;
        }
        let control = |t: u32| t == l.merge || t == l.continue_block;
        control(b.term.true_block) && control(b.term.false_block)
    }

    fn has_linear_path_to_terminal(&mut self, start: u32) -> Result<bool, CompileError> {
        let count = self.blocks.len();
        let mut visited = std::mem::take(&mut self.marks);
        visited.clear();
        self.budget
            .reserve(&mut visited, count, usize::MAX, LimitKind::Bytes)?;
        visited.resize(count, 0);
        let mut id = start;
        let result = loop {
            self.budget.charge(1)?;
            let Some(block) = self.block(id) else {
                break false;
            };
            if visited.get(id as usize).copied().unwrap_or(1) != 0 {
                break false;
            }
            if block.succ_len == 0 {
                break true;
            }
            if block.succ_len != 1 {
                break false;
            }
            visited[id as usize] = 1;
            id = block.succ[0];
        };
        self.marks = visited;
        Ok(result)
    }

    fn is_enclosing_linear_exit(
        &mut self,
        header: u32,
        block_id: u32,
    ) -> Result<bool, CompileError> {
        if !self.has_linear_path_to_terminal(block_id)? {
            return Ok(false);
        }
        let mut current = self.block(block_id).copied();
        let mut current_id = block_id;
        let mut steps = 0usize;
        while let Some(block) = current {
            steps += 1;
            if steps > self.blocks.len() {
                break;
            }
            self.budget.charge(1)?;
            if !(self.dominates(header, current_id) && block.succ_len == 1) {
                break;
            }
            current_id = block.succ[0];
            current = self.block(current_id).copied();
        }
        let Some(_) = current else {
            return Ok(false);
        };
        let pred_count = self.predecessors(current_id).len();
        if self.dominates(header, current_id) || pred_count == 0 {
            return Ok(false);
        }
        self.budget.charge_usize(pred_count)?;
        let preds = self.predecessors(current_id);
        Ok(preds
            .iter()
            .all(|&p| self.dominates(header, p) || self.dominates(p, header)))
    }

    fn can_reach_before(
        &mut self,
        start: u32,
        target: u32,
        stop: u32,
    ) -> Result<bool, CompileError> {
        let count = self.blocks.len();
        let mut visited = std::mem::take(&mut self.marks);
        visited.clear();
        self.budget
            .reserve(&mut visited, count, usize::MAX, LimitKind::Bytes)?;
        visited.resize(count, 0);
        self.scratch.clear();
        self.budget
            .push(&mut self.scratch, start, usize::MAX, LimitKind::Bytes)?;
        let mut found = false;
        while let Some(id) = self.scratch.pop() {
            self.budget.charge(1)?;
            if id == target {
                found = true;
                break;
            }
            if id == stop || visited.get(id as usize).copied().unwrap_or(1) != 0 {
                continue;
            }
            visited[id as usize] = 1;
            if let Some(block) = self.block(id).copied() {
                for &s in block.successors() {
                    self.budget
                        .push(&mut self.scratch, s, usize::MAX, LimitKind::Bytes)?;
                }
            }
        }
        self.marks = visited;
        Ok(found)
    }

    /// Source `FindSelectionMerge`.
    fn find_selection_merge(&mut self, block_id: u32) -> Result<u32, CompileError> {
        let block = self.blocks[block_id as usize];
        let true_target = block.term.true_block;
        let false_target = block.term.false_block;
        let global_merge = self.nearest_common_post_dominator(true_target, false_target)?;
        let Some(loop_index) = self.innermost_loop(block_id)? else {
            let global_terminal = self.block(global_merge).is_none_or(|b| b.succ_len == 0);
            if global_terminal {
                let false_reaches_true =
                    self.can_reach_before(false_target, true_target, global_merge)?;
                let true_reaches_false =
                    self.can_reach_before(true_target, false_target, global_merge)?;
                if false_reaches_true != true_reaches_false {
                    return Ok(if false_reaches_true {
                        true_target
                    } else {
                        false_target
                    });
                }
                if global_merge == NONE {
                    if self.is_enclosing_linear_exit(block_id, true_target)? {
                        return Ok(true_target);
                    }
                    if self.is_enclosing_linear_exit(block_id, false_target)? {
                        return Ok(false_target);
                    }
                    if self.has_linear_path_to_terminal(true_target)? {
                        return Ok(false_target);
                    }
                    if self.has_linear_path_to_terminal(false_target)? {
                        return Ok(true_target);
                    }
                }
            }
            return Ok(global_merge);
        };
        let l = self.loops[loop_index];
        if self.is_loop_control_gateway(&l, true_target)
            && self.dominates(block_id, true_target)
            && self.inside_loop_construct(&l, false_target)
        {
            return Ok(true_target);
        }
        if self.is_loop_control_gateway(&l, false_target)
            && self.dominates(block_id, false_target)
            && self.inside_loop_construct(&l, true_target)
        {
            return Ok(false_target);
        }
        Ok(global_merge)
    }

    /// Source `IsInnermostLoopControlConditional`.
    fn is_innermost_loop_control_conditional(&self, block_id: u32) -> Result<bool, CompileError> {
        let block = self.blocks[block_id as usize];
        if block.term.kind != TerminatorKind::ConditionalBranch {
            return Ok(false);
        }
        let Some(loop_index) = self.innermost_loop(block_id)? else {
            return Ok(false);
        };
        let l = self.loops[loop_index];
        if l.merge == NONE || l.continue_block == NONE {
            return Ok(false);
        }
        let t = block.term.true_block;
        let f = block.term.false_block;
        if block_id == l.continue_block {
            let repeat = |x: u32| x == l.header || x == l.merge;
            return Ok(repeat(t) && repeat(f));
        }
        let t_in = self.in_body(loop_index, t);
        let f_in = self.in_body(loop_index, f);
        if t_in != f_in {
            return Ok(true);
        }
        let control = |x: u32| x == l.merge || x == l.continue_block;
        Ok(
            (control(t) && (control(f) || self.inside_loop_construct(&l, f)))
                || (control(f) && self.inside_loop_construct(&l, t)),
        )
    }

    fn merge_leaves_containing_loop(&self, header: u32, merge: u32) -> Result<bool, CompileError> {
        self.budget.charge_usize(self.loops.len())?;
        Ok(self.loops.iter().any(|l| {
            l.header != header
                && self.inside_loop_construct(l, header)
                && !self.inside_loop_construct(l, merge)
        }))
    }

    // ---------------------------------------------------------- rewriting

    fn append_synthetic_branch_block(&mut self, target: u32) -> Result<u32, CompileError> {
        let (start_pc, inst_begin) = self
            .block(target)
            .map_or((0, 0), |b| (b.start_pc, b.inst_begin));
        let id = self.blocks.len() as u32;
        let max_blocks = self.budget.limits.max_blocks;
        self.budget.push(
            &mut self.blocks,
            Block {
                start_pc,
                end_pc: start_pc,
                inst_begin,
                inst_end: inst_begin,
                succ: [target, NONE],
                succ_len: 1,
                pred_start: 0,
                pred_len: 0,
                term: Terminator {
                    kind: TerminatorKind::Branch,
                    true_block: target,
                    ..Terminator::RETURN
                },
            },
            max_blocks,
            LimitKind::Blocks,
        )?;
        Ok(id)
    }

    fn replace_target(&mut self, block: u32, old: u32, new: u32) {
        if let Some(b) = self.blocks.get_mut(block as usize) {
            if b.term.true_block == old {
                b.term.true_block = new;
            }
            if b.term.false_block == old {
                b.term.false_block = new;
            }
            set_successors(b);
        }
    }

    /// Source `ApplyBlockOrder` through `MoveBlockBefore`.
    fn move_block_before(&mut self, block_id: u32, before_id: u32) -> Result<u32, CompileError> {
        let count = self.blocks.len();
        if block_id == before_id || block_id as usize >= count || before_id as usize >= count {
            return Ok(block_id);
        }
        self.charge(count * 4)?;
        let mut order: Vec<u32> = Vec::new();
        self.budget
            .reserve(&mut order, count, usize::MAX, LimitKind::Bytes)?;
        for i in 0..count as u32 {
            if i == before_id {
                order.push(block_id);
            }
            if i != block_id {
                order.push(i);
            }
        }
        let mut id_map = self
            .budget
            .table(count, NONE, usize::MAX, LimitKind::Bytes)?;
        for (new, &old) in order.iter().enumerate() {
            id_map[old as usize] = new as u32;
        }
        let mut blocks = Vec::new();
        self.budget
            .reserve(&mut blocks, count, usize::MAX, LimitKind::Bytes)?;
        for &old in &order {
            let mut block = self.blocks[old as usize];
            remap_terminator(&mut block.term, &id_map);
            set_successors(&mut block);
            blocks.push(block);
        }
        self.blocks = blocks;
        self.entry = remap(self.entry, &id_map);
        Ok(remap(block_id, &id_map))
    }

    fn rebuild_and_recompute(&mut self) -> Result<(), CompileError> {
        self.rebuild_predecessors()?;
        self.recompute()
    }

    fn isolate_semantic_loop_header(&mut self, old_header: u32) -> Result<bool, CompileError> {
        let Some(header) = self.block(old_header).copied() else {
            return Ok(false);
        };
        if header.is_empty() {
            return Ok(false);
        }
        let preds: Vec<u32> = self.copy_predecessors(old_header)?;
        let new_header = self.append_synthetic_branch_block(old_header)?;
        for &p in &preds {
            self.replace_target(p, old_header, new_header);
        }
        if self.entry == old_header {
            self.entry = new_header;
        }
        self.move_block_before(new_header, old_header)?;
        self.rebuild_and_recompute()?;
        Ok(true)
    }

    fn copy_predecessors(&mut self, id: u32) -> Result<Vec<u32>, CompileError> {
        let source = self.predecessors(id);
        let len = source.len();
        let mut out = Vec::new();
        self.budget
            .reserve(&mut out, len, usize::MAX, LimitKind::Bytes)?;
        out.extend_from_slice(self.predecessors(id));
        self.budget.charge_usize(len)?;
        Ok(out)
    }

    fn canonicalize_natural_loops(&mut self) -> Result<(), CompileError> {
        let rewrite_budget = self.blocks.len() * 2 + 16;
        for _ in 0..rewrite_budget {
            let mut changed = false;
            for index in 0..self.loops.len() {
                let header = self.loops[index].header;
                let mut latches: Vec<u32> = Vec::new();
                for &(from, to) in &self.back_edges {
                    self.budget.charge(1)?;
                    if to == header {
                        self.budget.charge_usize(latches.len())?;
                    }
                    if to == header && !latches.contains(&from) {
                        self.budget
                            .push(&mut latches, from, usize::MAX, LimitKind::Bytes)?;
                    }
                }
                if latches.len() <= 1 {
                    continue;
                }
                let continue_block = self.append_synthetic_branch_block(header)?;
                for &latch in &latches {
                    self.replace_target(latch, header, continue_block);
                }
                self.rebuild_and_recompute()?;
                changed = true;
                break;
            }
            if changed {
                continue;
            }
            for index in 0..self.loops.len() {
                let l = self.loops[index];
                let Some(header) = self.block(l.header).copied() else {
                    continue;
                };
                let control = |t: u32| t == l.merge || t == l.continue_block;
                if header.term.kind != TerminatorKind::ConditionalBranch
                    || control(header.term.true_block)
                    || control(header.term.false_block)
                    || !self.in_body(index, header.term.true_block)
                    || !self.in_body(index, header.term.false_block)
                {
                    continue;
                }
                self.isolate_semantic_loop_header(l.header)?;
                changed = true;
                break;
            }
            if !changed {
                return Ok(());
            }
        }
        Err(failure(
            self,
            self.entry,
            "CFG loop canonicalization exceeded rewrite budget",
        ))
    }

    fn isolate_semantic_loop_headers(&mut self) -> Result<(), CompileError> {
        let isolation_budget = self.loops.len() + 1;
        for _ in 0..isolation_budget {
            self.budget.charge_usize(self.loops.len())?;
            let found = self
                .loops
                .iter()
                .map(|l| l.header)
                .find(|&h| self.block(h).is_some_and(|b| !b.is_empty()));
            let Some(header) = found else {
                return Ok(());
            };
            if !self.isolate_semantic_loop_header(header)? {
                return Err(failure(
                    self,
                    header,
                    "failed to isolate semantic loop header",
                ));
            }
        }
        Err(failure(
            self,
            self.entry,
            "CFG semantic loop-header isolation exceeded rewrite budget",
        ))
    }

    /// Source `DominatedBlocks(header, stop)` into `self.marks` (1 = member).
    fn mark_dominated_blocks(&mut self, header: u32, stop: u32) -> Result<(), CompileError> {
        let count = self.blocks.len();
        self.marks.clear();
        self.budget
            .reserve(&mut self.marks, count, usize::MAX, LimitKind::Bytes)?;
        self.marks.resize(count, 0);
        self.scratch.clear();
        self.budget
            .push(&mut self.scratch, header, usize::MAX, LimitKind::Bytes)?;
        while let Some(id) = self.scratch.pop() {
            self.budget.charge(1)?;
            if id == stop
                || self.marks.get(id as usize).copied().unwrap_or(1) != 0
                || !self.dominates(header, id)
            {
                continue;
            }
            self.marks[id as usize] = 1;
            let block = self.blocks[id as usize];
            for &s in block.successors() {
                if s != stop && self.dominates(header, s) {
                    self.budget
                        .push(&mut self.scratch, s, usize::MAX, LimitKind::Bytes)?;
                }
            }
        }
        Ok(())
    }

    /// Source `SplitSharedMergeBlock`, with the construct in `self.marks`.
    fn split_shared_merge_block(
        &mut self,
        merge: u32,
        force_split: bool,
    ) -> Result<bool, CompileError> {
        if merge == NONE
            || merge as usize >= self.blocks.len()
            || self.marks.get(merge as usize).copied().unwrap_or(0) != 0
        {
            return Ok(false);
        }
        let preds = self.copy_predecessors(merge)?;
        let mut construct: Vec<u32> = Vec::new();
        let mut external = false;
        for &p in &preds {
            if self.marks.get(p as usize).copied().unwrap_or(0) != 0 {
                self.budget
                    .push(&mut construct, p, usize::MAX, LimitKind::Bytes)?;
            } else {
                external = true;
            }
        }
        if construct.is_empty() || (!force_split && !external) {
            return Ok(false);
        }
        if construct.len() == 1 {
            let merge_start = self.blocks[merge as usize].start_pc;
            if let Some(p) = self.block(construct[0])
                && p.is_empty()
                && p.term.kind == TerminatorKind::Branch
                && p.term.true_block == merge
                && p.start_pc == merge_start
            {
                return Ok(false);
            }
        }
        let synthetic = self.append_synthetic_branch_block(merge)?;
        for &p in &construct {
            self.replace_target(p, merge, synthetic);
        }
        self.move_block_before(synthetic, merge)?;
        Ok(true)
    }

    fn split_one_loop_merge(&mut self) -> Result<bool, CompileError> {
        for index in 0..self.loops.len() {
            let l = self.loops[index];
            self.mark_dominated_blocks(l.header, l.merge)?;
            let force = self.merge_leaves_containing_loop(l.header, l.merge)?;
            if self.split_shared_merge_block(l.merge, force)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Source `SelectionRegion` into `self.marks` (2 = member).
    fn mark_selection_region(&mut self, header: u32, merge: u32) -> Result<(), CompileError> {
        let count = self.blocks.len();
        self.marks.clear();
        self.budget
            .reserve(&mut self.marks, count, usize::MAX, LimitKind::Bytes)?;
        self.marks.resize(count, 0);
        let term = self.blocks[header as usize].term;
        let l = self.innermost_loop(header)?.map(|i| self.loops[i]);
        self.scratch.clear();
        self.budget.push(
            &mut self.scratch,
            term.true_block,
            usize::MAX,
            LimitKind::Bytes,
        )?;
        self.budget.push(
            &mut self.scratch,
            term.false_block,
            usize::MAX,
            LimitKind::Bytes,
        )?;
        while let Some(id) = self.scratch.pop() {
            self.budget.charge(1)?;
            if id == merge
                || self.marks.get(id as usize).copied().unwrap_or(1) != 0
                || l.is_some_and(|l| id == l.merge || id == l.continue_block)
            {
                continue;
            }
            self.marks[id as usize] = 2;
            let block = self.blocks[id as usize];
            for &s in block.successors() {
                self.budget
                    .push(&mut self.scratch, s, usize::MAX, LimitKind::Bytes)?;
            }
        }
        Ok(())
    }

    fn split_one_selection_merge(&mut self) -> Result<bool, CompileError> {
        let mut headers: Vec<(u32, u32)> = Vec::new();
        for id in 0..self.blocks.len() as u32 {
            self.budget.charge(1)?;
            let block = self.blocks[id as usize];
            self.budget.charge_usize(self.loops.len())?;
            if block.term.kind == TerminatorKind::ConditionalBranch
                && !self.loops.iter().any(|l| l.header == id)
            {
                self.budget.charge_usize(self.dominators.words)?;
                let depth = self.dominators.count(id);
                self.budget
                    .push(&mut headers, (depth, id), usize::MAX, LimitKind::Bytes)?;
            }
        }
        // Deepest dominator chains first, then ascending id (source sort).
        // Fallible, in-place insertion sort: stable slice sorting may allocate
        // outside the arena budget, and a comparator cannot propagate failure.
        for index in 1..headers.len() {
            let mut at = index;
            while at > 0 {
                self.budget.charge(1)?;
                let before = headers[at - 1];
                let item = headers[at];
                if before.0 > item.0 || (before.0 == item.0 && before.1 <= item.1) {
                    break;
                }
                headers.swap(at - 1, at);
                at -= 1;
            }
        }
        for &(_, id) in &headers {
            self.budget.charge(1)?;
            if self.is_innermost_loop_control_conditional(id)? {
                continue;
            }
            let merge = self.find_selection_merge(id)?;
            if merge == NONE || self.block(merge).is_none() {
                continue;
            }
            self.mark_selection_region(id, merge)?;
            for member in 0..self.blocks.len() as u32 {
                self.budget.charge(1)?;
                if self.marks[member as usize] != 2 {
                    continue;
                }
                let pred_count = self.predecessors(member).len();
                self.budget.charge_usize(pred_count + 1)?;
                let preds = self.predecessors(member);
                let entered = preds
                    .iter()
                    .any(|&p| p != id && self.marks.get(p as usize).copied().unwrap_or(0) != 2);
                if entered {
                    return Err(failure(
                        self,
                        id,
                        "selection header has an externally entered region block; semantic block cloning is disabled",
                    ));
                }
            }
            self.mark_dominated_blocks(id, merge)?;
            let force = self.merge_leaves_containing_loop(id, merge)?;
            if self.split_shared_merge_block(merge, force)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn split_shared_merge_blocks(&mut self) -> Result<(), CompileError> {
        let split_budget = (self.blocks.len() * 4).max(16);
        for _ in 0..split_budget {
            if !self.split_one_loop_merge()? && !self.split_one_selection_merge()? {
                return Ok(());
            }
            self.rebuild_and_recompute()?;
        }
        Err(failure(
            self,
            self.entry,
            "CFG shared merge splitting exceeded budget",
        ))
    }

    /// Source `StructureBreakRegions`.
    fn structure_break_regions(&mut self) -> Result<(), CompileError> {
        let pass_budget = self.blocks.len();
        for _ in 0..=pass_budget {
            let mut found: Option<(u32, u32, u32)> = None;
            for id in 0..self.blocks.len() as u32 {
                self.budget.charge(1)?;
                let block = self.blocks[id as usize];
                if block.term.kind != TerminatorKind::ConditionalBranch
                    || block.term.loop_header
                    || block.term.merge_block != NONE
                {
                    continue;
                }
                let Some(loop_index) = self.innermost_loop(id)? else {
                    continue;
                };
                let l = self.loops[loop_index];
                if l.merge == NONE || l.continue_block == NONE || id == l.continue_block {
                    continue;
                }
                let t = block.term.true_block;
                let f = block.term.false_block;
                let t_in = self.in_body(loop_index, t);
                let f_in = self.in_body(loop_index, f);
                let outside = if t_in { f } else { t };
                if t_in == f_in || outside == l.merge || outside == l.continue_block {
                    continue;
                }
                found = Some((id, outside, l.merge));
                break;
            }
            let Some((header_id, region, loop_merge)) = found else {
                return Ok(());
            };
            let mut exit_block = NONE;
            self.scratch.clear();
            let mut current = region;
            loop {
                self.budget.charge(1)?;
                let Some(block) = self.block(current).copied() else {
                    break;
                };
                self.budget.charge_usize(self.scratch.len())?;
                if self.scratch.contains(&current) || !self.dominates(header_id, current) {
                    break;
                }
                self.budget
                    .push(&mut self.scratch, current, usize::MAX, LimitKind::Bytes)?;
                let term = block.term;
                let next = if term.loop_header || term.kind == TerminatorKind::ConditionalBranch {
                    term.merge_block
                } else if term.kind == TerminatorKind::Branch {
                    term.true_block
                } else {
                    NONE
                };
                if next == loop_merge {
                    if term.kind == TerminatorKind::Branch && !term.loop_header {
                        exit_block = current;
                    }
                    break;
                }
                if next == NONE {
                    break;
                }
                current = next;
            }
            if exit_block == NONE {
                return Err(failure(
                    self,
                    header_id,
                    "conditional block enters a break region of the loop, and the region has no structured exit to that merge",
                ));
            }
            let gateway = self.append_synthetic_branch_block(loop_merge)?;
            self.replace_target(exit_block, loop_merge, gateway);
            if let Some(header) = self.blocks.get_mut(header_id as usize) {
                header.term.merge_block = gateway;
            }
            self.move_block_before(gateway, loop_merge)?;
            self.rebuild_and_recompute()?;
        }
        Err(failure(
            self,
            self.entry,
            "CFG break-region structuring exceeded its budget",
        ))
    }

    /// Source `StructurizeImpl`. The source retries failures through goto
    /// routing; this port reports the first failure (`UnsupportedReason::GotoRouting`
    /// is documented in the README).
    pub fn structurize(&mut self) -> Result<(), CompileError> {
        self.canonicalize_natural_loops()?;
        self.split_shared_merge_blocks()?;
        self.isolate_semantic_loop_headers()?;
        for block in &mut self.blocks {
            block.term.merge_block = NONE;
            block.term.continue_block = NONE;
            block.term.loop_header = false;
        }
        // merge block -> header
        let mut merge_headers =
            self.budget
                .table(self.blocks.len(), NONE, usize::MAX, LimitKind::Bytes)?;
        let mut reserve =
            |graph: &Graph<'_>, header: u32, merge: u32| -> Result<(), CompileError> {
                match merge_headers.get_mut(merge as usize) {
                    Some(slot) if *slot == NONE || *slot == header => {
                        *slot = header;
                        Ok(())
                    }
                    Some(_) => Err(failure(graph, header, "duplicate structured merge block")),
                    None => Err(failure(
                        graph,
                        header,
                        "structured merge block is out of range",
                    )),
                }
            };
        for index in 0..self.loops.len() {
            let l = self.loops[index];
            let Some(header) = self.block(l.header).copied() else {
                return Err(failure(
                    self,
                    l.header,
                    "loop has no structured merge/continue",
                ));
            };
            if l.merge == NONE || l.continue_block == NONE {
                return Err(failure(
                    self,
                    l.header,
                    "loop has no structured merge/continue",
                ));
            }
            if !header.is_empty() || header.term.kind != TerminatorKind::Branch {
                return Err(failure(
                    self,
                    l.header,
                    "loop header is not a dedicated empty control block",
                ));
            }
            reserve(self, l.header, l.merge)?;
            let entry = &mut self.blocks[l.header as usize];
            entry.term.loop_header = true;
            entry.term.merge_block = l.merge;
            entry.term.continue_block = l.continue_block;
        }
        for id in 0..self.blocks.len() as u32 {
            self.budget.charge(1)?;
            let block = self.blocks[id as usize];
            if block.term.kind != TerminatorKind::ConditionalBranch || block.term.loop_header {
                continue;
            }
            if self.is_innermost_loop_control_conditional(id)? {
                continue;
            }
            let merge = self.find_selection_merge(id)?;
            if merge == NONE {
                return Err(failure(
                    self,
                    id,
                    "conditional block has no structured merge",
                ));
            }
            reserve(self, id, merge)?;
            self.blocks[id as usize].term.merge_block = merge;
        }
        self.structure_break_regions()
    }
}

fn set_successors(block: &mut Block) {
    let mut succ = [NONE; 2];
    let mut len = 0u8;
    let mut add = |value: u32| {
        if value != NONE && !succ[..usize::from(len)].contains(&value) {
            succ[usize::from(len)] = value;
            len += 1;
        }
    };
    match block.term.kind {
        TerminatorKind::Branch => add(block.term.true_block),
        TerminatorKind::ConditionalBranch => {
            add(block.term.true_block);
            add(block.term.false_block);
        }
        TerminatorKind::Return => {}
    }
    // Source SortUnique on successors.
    if len == 2 && succ[0] > succ[1] {
        succ.swap(0, 1);
    }
    block.succ = succ;
    block.succ_len = len;
}

fn remap(id: u32, map: &[u32]) -> u32 {
    if id == NONE {
        return id;
    }
    map.get(id as usize).copied().unwrap_or(id)
}

fn remap_terminator(term: &mut Terminator, map: &[u32]) {
    term.true_block = remap(term.true_block, map);
    term.false_block = remap(term.false_block, map);
    term.merge_block = remap(term.merge_block, map);
    term.continue_block = remap(term.continue_block, map);
}

fn set_bit(row: &mut [u64], bit: u32) {
    if let Some(word) = row.get_mut(bit as usize / 64) {
        *word |= 1 << (bit % 64);
    }
}

fn test_bit(row: &[u64], bit: u32) -> bool {
    bit != NONE
        && row
            .get(bit as usize / 64)
            .is_some_and(|word| word >> (bit % 64) & 1 != 0)
}

fn full_row(budget: &mut Budget, count: usize, words: usize) -> Result<Vec<u64>, CompileError> {
    let mut row = budget.table(words, u64::MAX, usize::MAX, LimitKind::Bytes)?;
    if !count.is_multiple_of(64)
        && let Some(last) = row.last_mut()
    {
        *last = (1u64 << (count % 64)) - 1;
    }
    Ok(row)
}

fn vec_zero(budget: &mut Budget, words: usize) -> Result<Vec<u64>, CompileError> {
    budget.table(words, 0u64, usize::MAX, LimitKind::Bytes)
}

// --------------------------------------------------------------- public view

/// One block of a structured CFG.
#[derive(Clone, Copy, Debug)]
pub struct CfgBlock<'a> {
    cfg: &'a Cfg<'a>,
    id: u32,
}

impl<'a> CfgBlock<'a> {
    fn raw(&self) -> &'a Block {
        &self.cfg.blocks[self.id as usize]
    }
    pub fn id(&self) -> u32 {
        self.id
    }
    pub fn start_pc(&self) -> u32 {
        self.raw().start_pc
    }
    pub fn end_pc(&self) -> u32 {
        self.raw().end_pc
    }
    /// Instruction index range in the owning `ShaderProgram`.
    pub fn instructions(&self) -> std::ops::Range<usize> {
        self.raw().inst_begin as usize..self.raw().inst_end as usize
    }
    /// True for blocks added by structurization (no instructions, same PC as
    /// their target).
    pub fn is_empty(&self) -> bool {
        self.raw().is_empty()
    }
    pub fn terminator(&self) -> TerminatorKind {
        self.raw().term.kind
    }
    pub fn condition(&self) -> BranchCondition {
        self.raw().term.condition
    }
    pub fn true_target(&self) -> Option<u32> {
        some(self.raw().term.true_block)
    }
    pub fn false_target(&self) -> Option<u32> {
        some(self.raw().term.false_block)
    }
    pub fn merge(&self) -> Option<u32> {
        some(self.raw().term.merge_block)
    }
    pub fn continue_target(&self) -> Option<u32> {
        some(self.raw().term.continue_block)
    }
    pub fn is_loop_header(&self) -> bool {
        self.raw().term.loop_header
    }
    pub fn successors(&self) -> &'a [u32] {
        self.raw().successors()
    }
    pub fn predecessors(&self) -> &'a [u32] {
        let block = self.raw();
        let start = block.pred_start as usize;
        self.cfg
            .preds
            .get(start..start + block.pred_len as usize)
            .unwrap_or(&[])
    }
}

fn some(value: u32) -> Option<u32> {
    (value != NONE).then_some(value)
}

/// A reachable, reducible, structured CFG of one `ShaderProgram`. It borrows
/// the program, so block instruction ranges cannot be applied to another one.
#[derive(Debug)]
pub struct Cfg<'a> {
    program: &'a ShaderProgram,
    pub(crate) blocks: Vec<Block>,
    pub(crate) preds: Vec<u32>,
    pub(crate) entry: u32,
    work: u64,
}

impl<'a> Cfg<'a> {
    /// Port of `CFG::BuildGraph` followed by `CFG::Structurize`.
    pub fn build(program: &'a ShaderProgram, limits: &crate::Limits) -> Result<Self, CompileError> {
        let mut budget = Budget::new(limits);
        Self::build_in(program, &mut budget)
    }

    pub(crate) fn build_in(
        program: &'a ShaderProgram,
        budget: &mut Budget,
    ) -> Result<Self, CompileError> {
        let mut graph = Graph::build(program, budget)?;
        graph.structurize()?;
        graph.rebuild_predecessors()?;
        let work = graph.budget.work();
        Ok(Self {
            program,
            blocks: std::mem::take(&mut graph.blocks),
            preds: std::mem::take(&mut graph.preds),
            entry: graph.entry,
            work,
        })
    }

    pub fn program(&self) -> &'a ShaderProgram {
        self.program
    }
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }
    pub fn entry(&self) -> u32 {
        self.entry
    }
    pub fn block(&'a self, id: u32) -> Option<CfgBlock<'a>> {
        ((id as usize) < self.blocks.len()).then_some(CfgBlock { cfg: self, id })
    }
    pub fn blocks(&'a self) -> impl Iterator<Item = CfgBlock<'a>> + 'a {
        (0..self.blocks.len() as u32).map(move |id| CfgBlock { cfg: self, id })
    }
    pub fn work_units(&self) -> u64 {
        self.work
    }
}
