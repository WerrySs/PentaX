// SPDX-License-Identifier: GPL-2.0-only
// Direct Rust adaptation of KytyPS5-Performance at
// 209dffe9032d72749257ce18680d506b891284de, ir/passes/ConstantPropagation.cpp
// `FoldInstruction` for the operations this port produces (integer, bit,
// composite, select, phi, logical and bit-cast folds; GetAttributeWithBary
// interpolation-mode resolution). Like the source, float arithmetic is never
// folded. PentaX modifications: P1/P2 interpolation requires matching genuine
// builtin-coordinate and partial-result provenance; unknown values are never
// replaced by implicit interpolation. The pass repeats to a fixed point under the
// work budget (it re-simplifies phi users that the source reaches through use
// lists); value-set, known-zero and lane-mask projections are not ported.

use crate::CompileError;
use crate::budget::Budget;
use crate::error::UnsupportedReason;
use crate::ir::{Builtin, InstId, Ir, Op, Value};
use crate::passes::ssa::collect_block;

struct Folder<'a> {
    ir: &'a mut Ir,
    budget: &'a mut Budget,
    changed: bool,
}

fn u32_of(value: Value) -> Option<u32> {
    match value {
        Value::U32(v) => Some(v),
        _ => None,
    }
}

fn u64_of(value: Value) -> Option<u64> {
    match value {
        Value::U64(v) => Some(v),
        _ => None,
    }
}

fn u1_of(value: Value) -> Option<bool> {
    match value {
        Value::U1(v) => Some(v),
        _ => None,
    }
}

impl Folder<'_> {
    fn arg(&self, id: InstId, index: usize) -> Result<Value, CompileError> {
        self.ir.resolve(self.budget, self.ir.arg(id, index))
    }

    fn replace(&mut self, id: InstId, value: Value) -> Result<bool, CompileError> {
        self.ir.forward(self.budget, id, value)?;
        self.changed = true;
        Ok(true)
    }

    fn insert(
        &mut self,
        before: InstId,
        op: Op,
        aux: u32,
        args: &[Value],
    ) -> Result<Value, CompileError> {
        let pc = self.ir.inst(before).pc;
        let ty = op.result();
        let id = self.ir.push(self.budget, op, ty, aux, pc, args)?;
        self.ir.insert_before(before, id);
        Ok(Value::Inst(id))
    }

    fn fold_u32(&mut self, id: InstId, f: impl Fn(u32, u32) -> u32) -> Result<bool, CompileError> {
        match (u32_of(self.arg(id, 0)?), u32_of(self.arg(id, 1)?)) {
            (Some(a), Some(b)) => self.replace(id, Value::U32(f(a, b))),
            _ => Ok(false),
        }
    }

    fn fold_u64(&mut self, id: InstId, f: impl Fn(u64, u64) -> u64) -> Result<bool, CompileError> {
        match (u64_of(self.arg(id, 0)?), u64_of(self.arg(id, 1)?)) {
            (Some(a), Some(b)) => self.replace(id, Value::U64(f(a, b))),
            _ => Ok(false),
        }
    }

    fn fold_u64_shift(
        &mut self,
        id: InstId,
        f: impl Fn(u64, u32) -> u64,
    ) -> Result<bool, CompileError> {
        match (u64_of(self.arg(id, 0)?), u32_of(self.arg(id, 1)?)) {
            (Some(a), Some(b)) => self.replace(id, Value::U64(f(a, b))),
            _ => Ok(false),
        }
    }

    fn fold_u32_compare(
        &mut self,
        id: InstId,
        f: impl Fn(u32, u32) -> bool,
    ) -> Result<bool, CompileError> {
        match (u32_of(self.arg(id, 0)?), u32_of(self.arg(id, 1)?)) {
            (Some(a), Some(b)) => self.replace(id, Value::U1(f(a, b))),
            _ => Ok(false),
        }
    }

    fn fold_u64_compare(
        &mut self,
        id: InstId,
        f: impl Fn(u64, u64) -> bool,
    ) -> Result<bool, CompileError> {
        match (u64_of(self.arg(id, 0)?), u64_of(self.arg(id, 1)?)) {
            (Some(a), Some(b)) => self.replace(id, Value::U1(f(a, b))),
            _ => Ok(false),
        }
    }

    fn fold_logical(
        &mut self,
        id: InstId,
        f: impl Fn(bool, bool) -> bool,
    ) -> Result<bool, CompileError> {
        match (u1_of(self.arg(id, 0)?), u1_of(self.arg(id, 1)?)) {
            (Some(a), Some(b)) => self.replace(id, Value::U1(f(a, b))),
            _ => Ok(false),
        }
    }

    fn binary_identity(&mut self, id: InstId, identity: Value) -> Result<bool, CompileError> {
        let lhs = self.arg(id, 0)?;
        let rhs = self.arg(id, 1)?;
        if lhs == identity {
            return self.replace(id, rhs);
        }
        if rhs == identity {
            return self.replace(id, lhs);
        }
        Ok(false)
    }

    fn fold_select(&mut self, id: InstId) -> Result<bool, CompileError> {
        let condition = self.arg(id, 0)?;
        let t = self.arg(id, 1)?;
        let f = self.arg(id, 2)?;
        if let Some(c) = u1_of(condition) {
            return self.replace(id, if c { t } else { f });
        }
        if t == f {
            return self.replace(id, t);
        }
        Ok(false)
    }

    fn fold_phi(&mut self, id: InstId) -> Result<bool, CompileError> {
        let mut same = None;
        for index in 0..self.ir.nargs(id) {
            let value = self.arg(id, index)?;
            if value == Value::Inst(id) {
                continue;
            }
            match same {
                None => same = Some(value),
                Some(s) if s != value => return Ok(false),
                _ => {}
            }
        }
        match same {
            Some(value) => self.replace(id, value),
            None => Ok(false),
        }
    }

    fn fold_bitcast(&mut self, id: InstId, reverse: Op) -> Result<bool, CompileError> {
        let value = self.arg(id, 0)?;
        let op = self.ir.inst(id).op;
        match (op, value) {
            (Op::BitCastU32F32, Value::F32(bits)) => return self.replace(id, Value::U32(bits)),
            (Op::BitCastF32U32, Value::U32(bits)) => return self.replace(id, Value::F32(bits)),
            _ => {}
        }
        if let Some((producer, inst)) = self.ir.producer(self.budget, value)?
            && inst.op == reverse
        {
            let inner = self.ir.arg(producer, 0);
            return self.replace(id, inner);
        }
        Ok(false)
    }

    fn fold_composite_extract(
        &mut self,
        id: InstId,
        construct: Op,
        components: u32,
    ) -> Result<bool, CompileError> {
        let composite = self.arg(id, 0)?;
        let component = self.ir.inst(id).aux;
        if component >= components {
            return Ok(false);
        }
        if let Value::U64(value) = composite {
            return self.replace(id, Value::U32((value >> (component * 32)) as u32));
        }
        if let Some((producer, inst)) = self.ir.producer(self.budget, composite)? {
            if inst.op == construct {
                let part = self.ir.arg(producer, component as usize);
                return self.replace(id, part);
            }
            if component < 2 && inst.op == Op::IAddCarry32 {
                let lhs = self.ir.resolve(self.budget, self.ir.arg(producer, 0))?;
                let rhs = self.ir.resolve(self.budget, self.ir.arg(producer, 1))?;
                if let (Some(a), Some(b)) = (u32_of(lhs), u32_of(rhs)) {
                    let sum = u64::from(a) + u64::from(b);
                    let value = if component == 0 {
                        sum as u32
                    } else {
                        (sum >> 32) as u32
                    };
                    return self.replace(id, Value::U32(value));
                }
            }
        }
        Ok(false)
    }

    fn fold(&mut self, id: InstId) -> Result<(), CompileError> {
        let op = self.ir.inst(id).op;
        match op {
            Op::Phi => {
                self.fold_phi(id)?;
            }
            Op::SelectU1 => {
                if !self.fold_select(id)? && self.arg(id, 2)? == Value::U1(false) {
                    let a = self.arg(id, 0)?;
                    let b = self.arg(id, 1)?;
                    let and = self.insert(id, Op::LogicalAnd, 0, &[a, b])?;
                    self.replace(id, and)?;
                }
            }
            Op::SelectU32 | Op::SelectF32 => {
                self.fold_select(id)?;
            }
            Op::GetAttributeWithBary => {
                let provenance = (|| -> Result<Option<u32>, CompileError> {
                    let partial = self.arg(id, 0)?;
                    let Some((p1_id, p1)) = self.ir.producer(self.budget, partial)? else {
                        return Ok(None);
                    };
                    if p1.op != Op::InterpolationP1 || p1.aux != self.ir.inst(id).aux {
                        return Ok(None);
                    }
                    let i = self.arg(p1_id, 0)?;
                    let j = self.arg(id, 1)?;
                    let Some((_, i)) = self.ir.producer(self.budget, i)? else {
                        return Ok(None);
                    };
                    let Some((_, j)) = self.ir.producer(self.budget, j)? else {
                        return Ok(None);
                    };
                    if i.op != Op::GetBuiltin
                        || j.op != Op::GetBuiltin
                        || i.aux & 0xff != 0
                        || j.aux != i.aux + 1
                    {
                        return Ok(None);
                    }
                    Ok(match Builtin::from_code(i.aux >> 8) {
                        Some(Builtin::BaryCoordSmooth) => Some(1),
                        Some(Builtin::BaryCoordSmoothCentroid) => Some(2),
                        Some(Builtin::BaryCoordSmoothSample) => Some(3),
                        Some(Builtin::BaryCoordNoPerspective) => Some(4),
                        Some(Builtin::BaryCoordNoPerspectiveCentroid) => Some(5),
                        Some(Builtin::BaryCoordNoPerspectiveSample) => Some(6),
                        _ => None,
                    })
                })()?;
                let mode = provenance.ok_or_else(|| {
                    CompileError::unsupported(
                        self.ir.inst(id).pc,
                        None,
                        UnsupportedReason::PerVertexInterpolation,
                    )
                })?;
                let aux = self.ir.inst(id).aux & 0xffff | mode << 16;
                let read = self.insert(id, Op::GetAttribute, aux, &[])?;
                self.replace(id, read)?;
            }
            Op::BitFieldInsert => {
                let args = [
                    u32_of(self.arg(id, 0)?),
                    u32_of(self.arg(id, 1)?),
                    u32_of(self.arg(id, 2)?),
                    u32_of(self.arg(id, 3)?),
                ];
                if let [Some(base), Some(insert), Some(offset), Some(count)] = args
                    && offset <= 32
                    && count <= 32 - offset
                {
                    if count == 0 {
                        self.replace(id, Value::U32(base))?;
                    } else {
                        let mask = if count == 32 {
                            u32::MAX
                        } else {
                            ((1u32 << count) - 1) << offset
                        };
                        let shifted = insert.checked_shl(offset).unwrap_or(0);
                        self.replace(id, Value::U32((base & !mask) | (shifted & mask)))?;
                    }
                }
            }
            Op::BitFieldUExtract | Op::BitFieldSExtract => {
                let value = self.arg(id, 0)?;
                let offset = u32_of(self.arg(id, 1)?);
                let count = u32_of(self.arg(id, 2)?);
                if let (Some((source, inst)), Some(offset), Some(count)) =
                    (self.ir.producer(self.budget, value)?, offset, count)
                    && inst.op == Op::ShiftLeftLogical32
                    && let Some(shift) =
                        u32_of(self.ir.resolve(self.budget, self.ir.arg(source, 1))?)
                    && shift < 32
                    && offset <= shift
                    && count <= shift - offset
                {
                    self.replace(id, Value::U32(0))?;
                    return Ok(());
                }
                if let (Some(value), Some(offset), Some(count)) = (u32_of(value), offset, count)
                    && offset <= 32
                    && count <= 32 - offset
                {
                    let result = if count == 0 {
                        0
                    } else if op == Op::BitFieldUExtract {
                        let mask = if count == 32 {
                            u32::MAX
                        } else {
                            (1 << count) - 1
                        };
                        value.checked_shr(offset).unwrap_or(0) & mask
                    } else {
                        let left = 32 - offset - count;
                        let bits = value << left;
                        ((bits as i32) >> (left + offset)) as u32
                    };
                    self.replace(id, Value::U32(result))?;
                }
            }
            Op::BitCastU32F32 => {
                self.fold_bitcast(id, Op::BitCastF32U32)?;
            }
            Op::BitCastF32U32 => {
                self.fold_bitcast(id, Op::BitCastU32F32)?;
            }
            Op::CompositeExtractU64 => {
                self.fold_composite_extract(id, Op::CompositeConstructU64, 2)?;
            }
            Op::CompositeExtractU32x2 => {
                self.fold_composite_extract(id, Op::CompositeConstructU32x2, 2)?;
            }
            Op::CompositeExtractU32x3 => {
                self.fold_composite_extract(id, Op::CompositeConstructU32x3, 3)?;
            }
            Op::CompositeExtractU32x4 => {
                self.fold_composite_extract(id, Op::CompositeConstructU32x4, 4)?;
            }
            Op::CompositeConstructU64 => {
                let low = self.arg(id, 0)?;
                let high = self.arg(id, 1)?;
                if let (Some(l), Some(h)) = (u32_of(low), u32_of(high)) {
                    self.replace(id, Value::U64(u64::from(l) | u64::from(h) << 32))?;
                } else if let (Some((lid, li)), Some((hid, hi))) = (
                    self.ir.producer(self.budget, low)?,
                    self.ir.producer(self.budget, high)?,
                ) && li.op == Op::CompositeExtractU64
                    && hi.op == Op::CompositeExtractU64
                    && li.aux == 0
                    && hi.aux == 1
                {
                    let lsrc = self.ir.resolve(self.budget, self.ir.arg(lid, 0))?;
                    let hsrc = self.ir.resolve(self.budget, self.ir.arg(hid, 0))?;
                    if lsrc == hsrc {
                        self.replace(id, lsrc)?;
                    }
                }
            }
            Op::IAdd32 => {
                if !self.fold_u32(id, u32::wrapping_add)? {
                    self.binary_identity(id, Value::U32(0))?;
                }
            }
            Op::IAdd64 => {
                if !self.fold_u64(id, u64::wrapping_add)? {
                    self.binary_identity(id, Value::U64(0))?;
                }
            }
            Op::ISub32 => {
                if !self.fold_u32(id, u32::wrapping_sub)? && self.arg(id, 1)? == Value::U32(0) {
                    let lhs = self.arg(id, 0)?;
                    self.replace(id, lhs)?;
                }
            }
            Op::ISub64 => {
                if !self.fold_u64(id, u64::wrapping_sub)? && self.arg(id, 1)? == Value::U64(0) {
                    let lhs = self.arg(id, 0)?;
                    self.replace(id, lhs)?;
                }
            }
            Op::IMul32 => {
                if !self.fold_u32(id, u32::wrapping_mul)? {
                    self.binary_identity(id, Value::U32(1))?;
                }
            }
            Op::UDiv32 => match u32_of(self.arg(id, 1)?) {
                Some(1) => {
                    let lhs = self.arg(id, 0)?;
                    self.replace(id, lhs)?;
                }
                Some(d) if d != 0 => {
                    self.fold_u32(id, |a, b| a / b)?;
                }
                _ => {}
            },
            Op::SMulHi => {
                self.fold_u32(id, |a, b| {
                    ((i64::from(a as i32) * i64::from(b as i32)) as u64 >> 32) as u32
                })?;
            }
            Op::UMulHi => {
                self.fold_u32(id, |a, b| ((u64::from(a) * u64::from(b)) >> 32) as u32)?;
            }
            Op::IAbs32 => {
                if let Some(v) = u32_of(self.arg(id, 0)?) {
                    let abs = if v & 0x8000_0000 != 0 {
                        0u32.wrapping_sub(v)
                    } else {
                        v
                    };
                    self.replace(id, Value::U32(abs))?;
                }
            }
            Op::ShiftLeftLogical32 => {
                if !self.fold_u32(id, |a, b| a << (b & 31))?
                    && u32_of(self.arg(id, 1)?).is_some_and(|s| s & 31 == 0)
                {
                    let lhs = self.arg(id, 0)?;
                    self.replace(id, lhs)?;
                }
            }
            Op::ShiftRightLogical32 => {
                if !self.fold_u32(id, |a, b| a >> (b & 31))? {
                    if self.arg(id, 0)? == Value::U32(0) {
                        self.replace(id, Value::U32(0))?;
                    } else if u32_of(self.arg(id, 1)?).is_some_and(|s| s & 31 == 0) {
                        let lhs = self.arg(id, 0)?;
                        self.replace(id, lhs)?;
                    }
                }
            }
            Op::ShiftRightArithmetic32 => {
                self.fold_u32(id, |a, b| ((a as i32) >> (b & 31)) as u32)?;
            }
            Op::ShiftLeftLogical64 => {
                if !self.fold_u64_shift(id, |a, b| a << (b & 63))?
                    && u32_of(self.arg(id, 1)?).is_some_and(|s| s & 63 == 0)
                {
                    let lhs = self.arg(id, 0)?;
                    self.replace(id, lhs)?;
                }
            }
            Op::ShiftRightLogical64 => {
                if !self.fold_u64_shift(id, |a, b| a >> (b & 63))?
                    && u32_of(self.arg(id, 1)?).is_some_and(|s| s & 63 == 0)
                {
                    let lhs = self.arg(id, 0)?;
                    self.replace(id, lhs)?;
                }
            }
            Op::ShiftRightArithmetic64 => {
                self.fold_u64_shift(id, |a, b| ((a as i64) >> (b & 63)) as u64)?;
            }
            Op::BitwiseAnd32 => {
                if !self.fold_u32(id, |a, b| a & b)? {
                    self.binary_identity(id, Value::U32(u32::MAX))?;
                }
            }
            Op::BitwiseAnd64 => {
                if !self.fold_u64(id, |a, b| a & b)? {
                    self.binary_identity(id, Value::U64(u64::MAX))?;
                }
            }
            Op::BitwiseOr32 => {
                if !self.fold_u32(id, |a, b| a | b)? {
                    self.binary_identity(id, Value::U32(0))?;
                }
            }
            Op::BitwiseXor32 => {
                if !self.fold_u32(id, |a, b| a ^ b)? {
                    self.binary_identity(id, Value::U32(0))?;
                }
            }
            Op::BitwiseNot32 => {
                let value = self.arg(id, 0)?;
                if let Some(v) = u32_of(value) {
                    self.replace(id, Value::U32(!v))?;
                } else if let Some((producer, inst)) = self.ir.producer(self.budget, value)?
                    && inst.op == Op::BitwiseNot32
                {
                    let inner = self.ir.arg(producer, 0);
                    self.replace(id, inner)?;
                }
            }
            Op::BitReverse32 => {
                if let Some(v) = u32_of(self.arg(id, 0)?) {
                    self.replace(id, Value::U32(v.reverse_bits()))?;
                }
            }
            Op::BitCount32 => {
                if let Some(v) = u32_of(self.arg(id, 0)?) {
                    self.replace(id, Value::U32(v.count_ones()))?;
                }
            }
            Op::BitCount64 => {
                if let Some(v) = u64_of(self.arg(id, 0)?) {
                    self.replace(id, Value::U32(v.count_ones()))?;
                }
            }
            Op::SMin32 => {
                self.fold_u32(id, |a, b| if (a as i32) < (b as i32) { a } else { b })?;
            }
            Op::SMax32 => {
                self.fold_u32(id, |a, b| if (a as i32) > (b as i32) { a } else { b })?;
            }
            Op::UMin32 => {
                self.fold_u32(id, u32::min)?;
            }
            Op::UMax32 => {
                self.fold_u32(id, u32::max)?;
            }
            Op::IEqual32 => {
                self.fold_u32_compare(id, |a, b| a == b)?;
            }
            Op::INotEqual32 => {
                self.fold_u32_compare(id, |a, b| a != b)?;
            }
            Op::ULessThan32 => {
                self.fold_u32_compare(id, |a, b| a < b)?;
            }
            Op::ULessThanEqual32 => {
                self.fold_u32_compare(id, |a, b| a <= b)?;
            }
            Op::UGreaterThan32 => {
                self.fold_u32_compare(id, |a, b| a > b)?;
            }
            Op::UGreaterThanEqual32 => {
                self.fold_u32_compare(id, |a, b| a >= b)?;
            }
            Op::SLessThan32 => {
                self.fold_u32_compare(id, |a, b| (a as i32) < (b as i32))?;
            }
            Op::SLessThanEqual32 => {
                self.fold_u32_compare(id, |a, b| (a as i32) <= (b as i32))?;
            }
            Op::SGreaterThan32 => {
                self.fold_u32_compare(id, |a, b| (a as i32) > (b as i32))?;
            }
            Op::SGreaterThanEqual32 => {
                self.fold_u32_compare(id, |a, b| (a as i32) >= (b as i32))?;
            }
            Op::IEqual64 => {
                self.fold_u64_compare(id, |a, b| a == b)?;
            }
            Op::INotEqual64 => {
                self.fold_u64_compare(id, |a, b| a != b)?;
            }
            Op::ULessThan64 => {
                self.fold_u64_compare(id, |a, b| a < b)?;
            }
            Op::UGreaterThan64 => {
                self.fold_u64_compare(id, |a, b| a > b)?;
            }
            Op::ULessThanEqual64 => {
                self.fold_u64_compare(id, |a, b| a <= b)?;
            }
            Op::UGreaterThanEqual64 => {
                self.fold_u64_compare(id, |a, b| a >= b)?;
            }
            Op::SLessThanEqual64 => {
                self.fold_u64_compare(id, |a, b| (a as i64) <= (b as i64))?;
            }
            Op::SLessThan64 => {
                self.fold_u64_compare(id, |a, b| (a as i64) < (b as i64))?;
            }
            Op::LogicalAnd => {
                if !self.fold_logical(id, |a, b| a && b)? {
                    let lhs = self.arg(id, 0)?;
                    let rhs = self.arg(id, 1)?;
                    if self.simplify_and(id, lhs, rhs)? || self.simplify_and(id, rhs, lhs)? {
                        return Ok(());
                    }
                    if let Some(l) = u1_of(lhs) {
                        self.replace(id, if l { rhs } else { lhs })?;
                    } else if let Some(r) = u1_of(rhs) {
                        self.replace(id, if r { lhs } else { rhs })?;
                    }
                }
            }
            Op::LogicalOr => {
                if !self.fold_logical(id, |a, b| a || b)? {
                    let lhs = self.arg(id, 0)?;
                    let rhs = self.arg(id, 1)?;
                    if let Some(l) = u1_of(lhs) {
                        self.replace(id, if l { lhs } else { rhs })?;
                    } else if let Some(r) = u1_of(rhs) {
                        self.replace(id, if r { rhs } else { lhs })?;
                    }
                }
            }
            Op::LogicalXor => {
                if !self.fold_logical(id, |a, b| a != b)? {
                    let lhs = self.arg(id, 0)?;
                    let rhs = self.arg(id, 1)?;
                    if lhs == Value::U1(false) {
                        self.replace(id, rhs)?;
                    } else if rhs == Value::U1(false) {
                        self.replace(id, lhs)?;
                    }
                }
            }
            Op::LogicalNot => {
                let value = self.arg(id, 0)?;
                if let Some(v) = u1_of(value) {
                    self.replace(id, Value::U1(!v))?;
                } else if let Some((producer, inst)) = self.ir.producer(self.budget, value)?
                    && inst.op == Op::LogicalNot
                {
                    let inner = self.ir.arg(producer, 0);
                    self.replace(id, inner)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Source LogicalAnd `simplify`: a && (!a || b) => a && b.
    fn simplify_and(
        &mut self,
        id: InstId,
        assumption: Value,
        expression: Value,
    ) -> Result<bool, CompileError> {
        let Some((disjunction, inst)) = self.ir.producer(self.budget, expression)? else {
            return Ok(false);
        };
        if inst.op != Op::LogicalOr {
            return Ok(false);
        }
        for i in 0..2 {
            let candidate = self.ir.resolve(self.budget, self.ir.arg(disjunction, i))?;
            if let Some((inverse, inv)) = self.ir.producer(self.budget, candidate)?
                && inv.op == Op::LogicalNot
                && self.ir.resolve(self.budget, self.ir.arg(inverse, 0))? == assumption
            {
                let other = self.ir.arg(disjunction, i ^ 1);
                let lhs = self.arg(id, 0)?;
                let index = if assumption == lhs { 1 } else { 0 };
                self.ir.set_arg(id, index, other);
                self.changed = true;
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Source `ConstantPropagationPass`, repeated until no fold applies.
pub(crate) fn constant_propagation(ir: &mut Ir, budget: &mut Budget) -> Result<(), CompileError> {
    let mut folder = Folder {
        ir,
        budget,
        changed: true,
    };
    let rounds = folder.ir.insts.len() + 2;
    for _ in 0..rounds {
        if !folder.changed {
            return Ok(());
        }
        folder.changed = false;
        for block in 0..folder.ir.blocks.len() as u32 {
            let ids = collect_block(folder.ir, folder.budget, block)?;
            for id in ids {
                folder.budget.charge(4)?;
                let inst = folder.ir.inst(id);
                if !inst.alive || inst.op == Op::Identity {
                    continue;
                }
                folder.fold(id)?;
            }
        }
    }
    Ok(())
}
