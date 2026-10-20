// SPDX-License-Identifier: GPL-2.0-only
// Direct Rust adaptation of KytyPS5-Performance at
// 209dffe9032d72749257ce18680d506b891284de, ir/passes/SsaRewrite.cpp: the
// explicit-stack Braun et al. reader, incomplete phis for unsealed blocks,
// sealing after every block was visited, and TryRemoveTrivialPhi.
// PentaX modifications: flat (block, variable) tables; a register without a
// definition on a path from entry reads a typed `Undef` carrying its
// variable (the source substitutes InitialValue zero/false); phi users are
// re-simplified by the following fold pass instead of through use lists.

use crate::CompileError;
use crate::budget::{Budget, LimitKind};
use crate::ir::{InstId, Ir, NIL, Op, PENDING, Type, Value};
use crate::translate::var;

/// The SSA variable read or written by a register pseudo operation.
pub(crate) fn variable_of(op: Op, aux: u32) -> Option<(u32, bool)> {
    Some(match op {
        Op::GetScalarRegister => (aux, false),
        Op::SetScalarRegister => (aux, true),
        Op::GetThreadBitScalarRegister => (var::THREAD_BIT + aux, false),
        Op::SetThreadBitScalarRegister => (var::THREAD_BIT + aux, true),
        Op::GetScalarMaskTag => (var::MASK_TAG + aux, false),
        Op::SetScalarMaskTag => (var::MASK_TAG + aux, true),
        Op::GetScc => (var::SCC, false),
        Op::SetScc => (var::SCC, true),
        Op::GetExec => (var::EXEC, false),
        Op::SetExec => (var::EXEC, true),
        Op::GetExecLo => (var::EXEC_LO, false),
        Op::SetExecLo => (var::EXEC_LO, true),
        Op::GetExecHi => (var::EXEC_HI, false),
        Op::SetExecHi => (var::EXEC_HI, true),
        Op::GetVcc => (var::VCC, false),
        Op::SetVcc => (var::VCC, true),
        Op::GetVccLo => (var::VCC_LO, false),
        Op::SetVccLo => (var::VCC_LO, true),
        Op::GetVccHi => (var::VCC_HI, false),
        Op::SetVccHi => (var::VCC_HI, true),
        Op::GetM0 => (var::M0, false),
        Op::SetM0 => (var::M0, true),
        Op::GetVectorRegister => (var::VGPR + aux, false),
        Op::SetVectorRegister => (var::VGPR + aux, true),
        _ => return None,
    })
}

pub(crate) fn variable_type(variable: u32) -> Type {
    if (var::THREAD_BIT..var::SCC).contains(&variable)
        || variable == var::SCC
        || variable == var::EXEC
        || variable == var::VCC
    {
        Type::U1
    } else {
        Type::U32
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Start,
    SetValue,
    PushPhiArgument,
}

#[derive(Clone, Copy)]
struct ReadState {
    block: u32,
    result: Value,
    phi: Option<InstId>,
    pred: u32,
    step: Step,
}

struct Pass<'a> {
    ir: &'a mut Ir,
    budget: &'a mut Budget,
    variables: usize,
    defs: Vec<Option<Value>>,
    incomplete: Vec<u32>,
    sealed: Vec<bool>,
    undefs: Vec<u32>,
    stack: Vec<ReadState>,
}

impl Pass<'_> {
    fn slot(&self, block: u32, variable: u32) -> usize {
        block as usize * self.variables + variable as usize
    }

    fn write(&mut self, variable: u32, block: u32, value: Value) {
        let slot = self.slot(block, variable);
        if let Some(def) = self.defs.get_mut(slot) {
            *def = Some(value);
        }
    }

    fn new_phi(&mut self, block: u32, variable: u32) -> Result<InstId, CompileError> {
        let preds = self.ir.predecessors(block).len();
        let pc = self.ir.blocks[block as usize].start_pc;
        self.budget.charge_usize(preds + 1)?;
        let id = self
            .ir
            .push(self.budget, Op::Phi, variable_type(variable), 0, pc, &[])?;
        // Reserve one operand slot per predecessor, filled in order.
        let start = u32::try_from(self.ir.args.len())
            .map_err(|_| CompileError::limit(LimitKind::IrOperands))?;
        let limits = self.budget.limits;
        self.budget.reserve(
            &mut self.ir.args,
            preds,
            limits.max_ir_operands,
            LimitKind::IrOperands,
        )?;
        self.ir.args.resize(self.ir.args.len() + preds, PENDING);
        let inst = self.ir.inst_mut(id);
        inst.args = start;
        inst.nargs = preds as u32;
        self.ir.prepend(block, id);
        Ok(id)
    }

    fn add_phi_operand(&mut self, phi: InstId, value: Value) -> Result<(), CompileError> {
        let filled = self.ir.inst(phi).aux as usize;
        if filled >= self.ir.nargs(phi) {
            return Err(CompileError::Internal("phi operand overflow"));
        }
        self.ir.set_arg(phi, filled, value);
        self.ir.inst_mut(phi).aux = filled as u32 + 1;
        Ok(())
    }

    /// A typed undefined value in the entry block for `variable`.
    fn undef(&mut self, variable: u32) -> Result<Value, CompileError> {
        let cached = self.undefs.get(variable as usize).copied().unwrap_or(NIL);
        if cached != NIL {
            return Ok(Value::Inst(self.ir.id_from_raw(cached)));
        }
        let pc = self.ir.blocks[0].start_pc;
        let id = self.ir.push(
            self.budget,
            Op::Undef,
            variable_type(variable),
            variable,
            pc,
            &[],
        )?;
        self.ir.prepend(0, id);
        if let Some(slot) = self.undefs.get_mut(variable as usize) {
            *slot = id.index() as u32;
        }
        Ok(Value::Inst(id))
    }

    /// Source `TryRemoveTrivialPhi` (users are simplified by the fold pass).
    fn try_remove_trivial_phi(
        &mut self,
        phi: InstId,
        variable: u32,
    ) -> Result<Value, CompileError> {
        let mut same: Option<Value> = None;
        for index in 0..self.ir.nargs(phi) {
            self.budget.charge(1)?;
            let operand = self.ir.resolve(self.budget, self.ir.arg(phi, index))?;
            if operand == Value::Inst(phi) || Some(operand) == same {
                continue;
            }
            if same.is_some() {
                return Ok(Value::Inst(phi));
            }
            same = Some(operand);
        }
        let same = match same {
            Some(value) => value,
            None => self.undef(variable)?,
        };
        self.ir.forward(self.budget, phi, same)?;
        Ok(same)
    }

    /// Source `Pass::Read`, iterative.
    fn read(&mut self, variable: u32, root: u32) -> Result<Value, CompileError> {
        self.stack.clear();
        let sentinel = ReadState {
            block: NIL,
            result: PENDING,
            phi: None,
            pred: 0,
            step: Step::Start,
        };
        self.budget
            .push(&mut self.stack, sentinel, usize::MAX, LimitKind::Bytes)?;
        self.budget.push(
            &mut self.stack,
            ReadState {
                block: root,
                ..sentinel
            },
            usize::MAX,
            LimitKind::Bytes,
        )?;
        while self.stack.len() > 1 {
            self.budget.charge(1)?;
            let top = self.stack.len() - 1;
            let state = self.stack[top];
            let block = state.block;
            match state.step {
                Step::Start => {
                    let slot = self.slot(block, variable);
                    let def = self.defs.get(slot).copied().flatten();
                    if let Some(def) = def {
                        self.stack[top].result = def;
                    } else if !self.sealed[block as usize] {
                        let phi = self.new_phi(block, variable)?;
                        if let Some(entry) = self.incomplete.get_mut(slot) {
                            *entry = phi.index() as u32;
                        }
                        self.stack[top].result = Value::Inst(phi);
                    } else if self.ir.predecessors(block).len() == 1 {
                        let pred = self.ir.predecessors(block)[0];
                        self.stack[top].step = Step::SetValue;
                        self.budget.push(
                            &mut self.stack,
                            ReadState {
                                block: pred,
                                ..sentinel
                            },
                            usize::MAX,
                            LimitKind::Bytes,
                        )?;
                        continue;
                    } else {
                        let phi = self.new_phi(block, variable)?;
                        self.write(variable, block, Value::Inst(phi));
                        self.stack[top].phi = Some(phi);
                        self.prepare_phi(variable)?;
                        continue;
                    }
                    self.finish_set_value(variable)?;
                }
                Step::SetValue => self.finish_set_value(variable)?,
                Step::PushPhiArgument => {
                    let phi = state.phi.ok_or(CompileError::Internal("phi state"))?;
                    self.add_phi_operand(phi, state.result)?;
                    self.stack[top].pred += 1;
                    self.prepare_phi(variable)?;
                }
            }
        }
        Ok(self.stack[0].result)
    }

    fn finish_set_value(&mut self, variable: u32) -> Result<(), CompileError> {
        let state = self
            .stack
            .pop()
            .ok_or(CompileError::Internal("SSA read stack"))?;
        self.write(variable, state.block, state.result);
        if let Some(parent) = self.stack.last_mut() {
            parent.result = state.result;
        }
        Ok(())
    }

    fn prepare_phi(&mut self, variable: u32) -> Result<(), CompileError> {
        let top = self.stack.len() - 1;
        let state = self.stack[top];
        let preds = self.ir.predecessors(state.block).len() as u32;
        if state.pred == preds {
            let phi = state.phi.ok_or(CompileError::Internal("phi state"))?;
            let result = self.try_remove_trivial_phi(phi, variable)?;
            self.stack.pop();
            if let Some(parent) = self.stack.last_mut() {
                parent.result = result;
            }
            self.write(variable, state.block, result);
            return Ok(());
        }
        self.stack[top].step = Step::PushPhiArgument;
        let pred = self.ir.predecessors(state.block)[state.pred as usize];
        self.budget.push(
            &mut self.stack,
            ReadState {
                block: pred,
                result: PENDING,
                phi: None,
                pred: 0,
                step: Step::Start,
            },
            usize::MAX,
            LimitKind::Bytes,
        )
    }

    fn seal(&mut self, block: u32) -> Result<(), CompileError> {
        for variable in 0..self.variables as u32 {
            self.budget.charge(1)?;
            let slot = self.slot(block, variable);
            let phi = self.incomplete.get(slot).copied().unwrap_or(NIL);
            if phi == NIL {
                continue;
            }
            let phi = self.ir.id_from_raw(phi);
            let preds = self.ir.predecessors(block).len();
            for index in 0..preds {
                let pred = self.ir.predecessors(block)[index];
                let value = self.read(variable, pred)?;
                self.add_phi_operand(phi, value)?;
            }
            self.try_remove_trivial_phi(phi, variable)?;
        }
        self.sealed[block as usize] = true;
        Ok(())
    }
}

/// Source `RewriteToSsa` (default mode: visit every block, then seal all).
pub(crate) fn rewrite_to_ssa(ir: &mut Ir, budget: &mut Budget) -> Result<(), CompileError> {
    let mut vgprs = 0u32;
    budget.charge_usize(ir.insts.len())?;
    for inst in &ir.insts {
        if matches!(inst.op, Op::GetVectorRegister | Op::SetVectorRegister) {
            vgprs = vgprs.max(inst.aux + 1);
        }
    }
    let variables = (var::VGPR + vgprs) as usize;
    let blocks = ir.blocks.len();
    let slots = blocks
        .checked_mul(variables)
        .ok_or(CompileError::limit(LimitKind::SsaSlots))?;
    let max_slots = budget.limits.max_ssa_slots;
    if slots > max_slots {
        return Err(CompileError::limit(LimitKind::SsaSlots));
    }
    let defs = budget.table(slots, None, max_slots, LimitKind::SsaSlots)?;
    let incomplete = budget.table(slots, NIL, max_slots, LimitKind::SsaSlots)?;
    let sealed = budget.table(blocks, false, usize::MAX, LimitKind::Bytes)?;
    let undefs = budget.table(variables, NIL, usize::MAX, LimitKind::Bytes)?;
    let mut pass = Pass {
        ir,
        budget,
        variables,
        defs,
        incomplete,
        sealed,
        undefs,
        stack: Vec::new(),
    };
    for block in 0..blocks as u32 {
        let ids: Vec<InstId> = collect_block(pass.ir, pass.budget, block)?;
        for id in ids {
            pass.budget.charge(1)?;
            let inst = *pass.ir.inst(id);
            let Some((variable, write)) = variable_of(inst.op, inst.aux) else {
                continue;
            };
            if variable as usize >= variables {
                return Err(CompileError::Internal("SSA variable out of range"));
            }
            if write {
                let value = pass.ir.arg(id, 0);
                pass.write(variable, block, value);
            } else {
                let value = pass.read(variable, block)?;
                pass.ir.forward(pass.budget, id, value)?;
            }
        }
    }
    for block in 0..blocks as u32 {
        pass.seal(block)?;
    }
    Ok(())
}

pub(crate) fn collect_block(
    ir: &Ir,
    budget: &mut Budget,
    block: u32,
) -> Result<Vec<InstId>, CompileError> {
    let mut ids = Vec::new();
    for id in ir.block_insts(block) {
        budget.push(&mut ids, id, usize::MAX, LimitKind::Bytes)?;
    }
    Ok(ids)
}
