// SPDX-License-Identifier: GPL-2.0-only
// Original conservative proof for values undefined only on inactive compute
// lanes. No value is initialized, replaced with zero or exposed by this pass.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Guard {
    yes: [Value; 4],
    no: [Value; 4],
    yes_len: usize,
    no_len: usize,
}
impl Default for Guard {
    fn default() -> Self {
        Self {
            yes: [Value::U1(false); 4],
            no: [Value::U1(false); 4],
            yes_len: 0,
            no_len: 0,
        }
    }
}

fn implies(ir: &Ir, budget: &Budget, a: Value, b: Value, depth: u8) -> Result<bool, CompileError> {
    budget.charge(1)?;
    let a = ir.resolve(budget, a)?;
    let b = ir.resolve(budget, b)?;
    if a == b || b == Value::U1(true) || a == Value::U1(false) {
        return Ok(true);
    }
    if depth == 0 {
        return Ok(false);
    }
    if let Value::Inst(id) = a
        && id.index() < ir.insts.len()
        && ir.inst(id).op == Op::LogicalAnd
    {
        return Ok(implies(ir, budget, ir.arg(id, 0), b, depth - 1)?
            || implies(ir, budget, ir.arg(id, 1), b, depth - 1)?);
    }
    Ok(false)
}

enum Added {
    Impossible,
    Proven(Guard),
    Unknown,
}
fn invariant(ir: &Ir, budget: &Budget, value: Value, depth: u8) -> Result<bool, CompileError> {
    budget.charge(1)?;
    let Value::Inst(id) = ir.resolve(budget, value)? else {
        return Ok(true);
    };
    if id.index() >= ir.insts.len() {
        return Ok(false);
    }
    let inst = ir.inst(id);
    // Synthetic entry0 executes once, outside every guest loop. Restrict all
    // facts to its defined pure values; a Phi or later load is not invariant.
    if depth == 0
        || inst.block != 0
        || inst.op == Op::Phi
        || inst.op == Op::Undef
        || inst.op.has_side_effects()
        || inst.op.is_buffer_load()
        || matches!(inst.op, Op::LoadSharedU32 | Op::Ballot | Op::AnyLane)
    {
        return Ok(false);
    }
    for index in 0..ir.nargs(id) {
        if !invariant(ir, budget, ir.arg(id, index), depth - 1)? {
            return Ok(false);
        }
    }
    Ok(true)
}
fn add(
    ir: &Ir,
    budget: &Budget,
    mut guard: Guard,
    mut v: Value,
    mut yes: bool,
    looped: bool,
) -> Result<Added, CompileError> {
    v = ir.resolve(budget, v)?;
    if let Value::Inst(id) = v
        && id.index() < ir.insts.len()
        && ir.inst(id).op == Op::LogicalNot
    {
        v = ir.resolve(budget, ir.arg(id, 0))?;
        yes = !yes;
    }
    if let Value::U1(value) = v {
        return Ok(if value == yes {
            Added::Proven(guard)
        } else {
            Added::Impossible
        });
    }
    if looped && !invariant(ir, budget, v, 128)? {
        return Ok(Added::Unknown);
    }
    if yes {
        for &other in &guard.no[..guard.no_len] {
            if implies(ir, budget, v, other, 32)? {
                return Ok(Added::Impossible);
            }
        }
        if guard.yes[..guard.yes_len].contains(&v) {
            return Ok(Added::Proven(guard));
        }
        if guard.yes_len == guard.yes.len() {
            return Ok(Added::Unknown);
        }
        guard.yes[guard.yes_len] = v;
        guard.yes_len += 1;
    } else {
        for &other in &guard.yes[..guard.yes_len] {
            if implies(ir, budget, other, v, 32)? {
                return Ok(Added::Impossible);
            }
        }
        if guard.no[..guard.no_len].contains(&v) {
            return Ok(Added::Proven(guard));
        }
        if guard.no_len == guard.no.len() {
            return Ok(Added::Unknown);
        }
        guard.no[guard.no_len] = v;
        guard.no_len += 1;
    }
    Ok(Added::Proven(guard))
}

/// Trace only actual live uses of this undefined VGPR value. Select arms and
/// predicated buffer/LDS operands restrict when it can reach an observable use.
/// Branches, descriptors, other side effects and unproved combinations
/// remain conservative. The fixed guard depth is a proof limit, not permission.
pub(super) fn inactive_only(
    ir: &Ir,
    budget: &mut Budget,
    target: InstId,
) -> Result<bool, CompileError> {
    // In a loop the same condition id can change on a later iteration. Only
    // synthetic-entry invariants may then supply guard facts. Forward use
    // propagation reaches a fixed point over (value,invariant-guard), without
    // treating changing Phi ids as contradictory values across iterations.
    let mut looped = false;
    for (index, block) in ir.blocks.iter().enumerate() {
        budget.charge(1)?;
        if block.term.loop_header
            || block.term.continue_block != crate::ir::NIL
            || [block.term.true_block, block.term.false_block]
                .into_iter()
                .any(|next| next != crate::ir::NIL && next as usize <= index)
        {
            looped = true;
        }
    }
    if looped && !ir.predecessors(0).is_empty() {
        return Ok(false);
    }
    let mut pending = Vec::new();
    let mut seen = Vec::new();
    let limit = budget.limits.max_ir_instructions;
    budget.push(
        &mut pending,
        (target, Guard::default()),
        limit,
        LimitKind::IrInstructions,
    )?;
    while let Some((value, guard)) = pending.pop() {
        budget.charge_usize(seen.len())?;
        if seen.contains(&(value, guard)) {
            continue;
        }
        budget.push(&mut seen, (value, guard), limit, LimitKind::IrInstructions)?;
        for block in 0..ir.blocks.len() as u32 {
            budget.charge(1)?;
            if ir.blocks[block as usize].condition == Some(Value::Inst(value)) {
                return Ok(false);
            }
            for id in ir.block_insts(block) {
                budget.charge(1)?;
                let inst = ir.inst(id);
                let count = ir.nargs(id);
                for index in 0..count {
                    budget.charge(1)?;
                    if ir.arg(id, index) != Value::Inst(value) {
                        continue;
                    }
                    let condition =
                        if matches!(inst.op, Op::SelectU1 | Op::SelectU32 | Op::SelectF32)
                            && index != 0
                        {
                            Some((ir.arg(id, 0), index == 1))
                        } else if ((inst.op.is_buffer_load() || inst.op.is_buffer_store())
                            && inst.op != Op::ReadConstBuffer
                            && index > 0
                            && index + 1 < count)
                            || (matches!(inst.op, Op::LoadSharedU32 | Op::StoreSharedU32)
                                && index + 1 < count)
                        {
                            Some((ir.arg(id, count - 1), true))
                        } else {
                            None
                        };
                    let next = if let Some((condition, truth)) = condition {
                        match add(ir, budget, guard, condition, truth, looped)? {
                            Added::Impossible => continue,
                            Added::Unknown => return Ok(false),
                            Added::Proven(guard) => guard,
                        }
                    } else {
                        guard
                    };
                    // A live memory read must itself have a defined address
                    // and predicate; a later unused result is not permission.
                    // Lane communication also cannot inherit a per-lane guard.
                    if inst.op.has_side_effects()
                        || inst.op.is_buffer_load()
                        || matches!(inst.op, Op::LoadSharedU32 | Op::Ballot | Op::AnyLane)
                    {
                        return Ok(false);
                    }
                    budget.push(&mut pending, (id, next), limit, LimitKind::IrInstructions)?;
                }
            }
        }
    }
    Ok(true)
}
