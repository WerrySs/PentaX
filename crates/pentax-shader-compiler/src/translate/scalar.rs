// SPDX-License-Identifier: GPL-2.0-only
// Direct Rust adaptation of KytyPS5-Performance at
// 209dffe9032d72749257ce18680d506b891284de: frontend/translate/Scalar.cpp
// (scalar opcode map), Control.cpp (SAVEEXEC, carries, selects, moves,
// waits, barrier) and Integer.cpp (integer and 64-bit mask helpers shared
// with the vector map).
// PentaX modifications: S_TRAP, S_SETREG, S_SENDMSG, S_TTRACEDATA, WQM,
// GETPC and subvector loops are typed refusals (the source drops several of
// them as no-ops); S_BARRIER outside compute is refused instead of dropped.

use pentax_shader::{Opcode as O, OperandKind};

use super::{Opnd, Translator, u32v};
use crate::CompileError;
use crate::error::UnsupportedReason;
use crate::ir::{Op, Type, Value};

type R = Result<(), CompileError>;

fn scc() -> Opnd {
    Opnd::special(OperandKind::Scc)
}

impl Translator<'_> {
    pub(super) fn emit_scalar(&mut self) -> R {
        use UnsupportedReason as U;
        let inst = self.inst;
        match inst.opcode {
            O::SMovB32 | O::SMovkI32 => self.mov_b32(false),
            O::SMovB64 => self.s_mov_b64(),
            O::SWqmB32 | O::SWqmB64 => Err(self.unsupported(U::WholeQuadMode)),
            O::SGetpcB64 | O::SSetpcB64 => Err(self.unsupported(U::ProgramCounter)),
            O::SSubvectorLoopBegin | O::SSubvectorLoopEnd => {
                Err(self.unsupported(U::SubvectorLoop))
            }
            O::SCselectB32 => {
                let scc = self.e.get(Op::GetScc, 0)?;
                let a = self.read_u32(inst.src(0))?;
                let b = self.read_u32(inst.src(1))?;
                let result = self.e.select(scc, a, b)?;
                self.write(inst.dst, result)
            }
            O::SCselectB64 => self.scalar_select64(inst.src(1)),
            O::SCmovB64 => self.scalar_select64(inst.dst),
            O::SCmovB32 | O::SCmovkI32 => {
                if inst.dst.kind == OperandKind::Null {
                    return Ok(());
                }
                let value = self.read_u32(inst.src(0))?;
                let scc = self.e.get(Op::GetScc, 0)?;
                let old = self.read_u32(inst.dst)?;
                let result = self.e.select(scc, value, old)?;
                self.write(inst.dst, result)
            }
            O::SSetregB32 => Err(self.unsupported(U::SetHardwareRegister)),
            O::SWaitcntVscnt => {
                let count = inst.src(1).value & 63;
                if inst.src(0).kind == OperandKind::Null && count == 63 {
                    return Ok(());
                }
                self.e.emit(Op::StoreCompletion, &[]).map(|_| ())
            }
            // No LDS write can be pending: DS operations are refused.
            O::SWaitcnt => Ok(()),
            O::SAndSaveexecB32 => self.s_saveexec(Op::LogicalAnd, false, false, false, false),
            O::SAndn1SaveexecB32 => self.s_saveexec(Op::LogicalAnd, false, true, false, false),
            O::SOrn2SaveexecB32 => self.s_saveexec(Op::LogicalOr, true, false, false, false),
            O::SAndSaveexecB64 => self.s_saveexec(Op::LogicalAnd, false, false, true, false),
            O::SAndn1SaveexecB64 => self.s_saveexec(Op::LogicalAnd, false, true, true, false),
            O::SOrn2SaveexecB64 => self.s_saveexec(Op::LogicalOr, true, false, true, false),
            O::SOrSaveexecB32 => self.s_saveexec(Op::LogicalOr, false, false, false, false),
            O::SOrSaveexecB64 => self.s_saveexec(Op::LogicalOr, false, false, true, false),
            O::SXorSaveexecB32 => self.s_saveexec(Op::LogicalXor, false, false, false, false),
            O::SXorSaveexecB64 => self.s_saveexec(Op::LogicalXor, false, false, true, false),
            O::SAndn2SaveexecB32 => self.s_saveexec(Op::LogicalAnd, true, false, false, false),
            O::SAndn2SaveexecB64 => self.s_saveexec(Op::LogicalAnd, true, false, true, false),
            O::SOrn1SaveexecB32 => self.s_saveexec(Op::LogicalOr, false, true, false, false),
            O::SOrn1SaveexecB64 => self.s_saveexec(Op::LogicalOr, false, true, true, false),
            O::SNandSaveexecB32 => self.s_saveexec(Op::LogicalAnd, false, false, false, true),
            O::SNandSaveexecB64 => self.s_saveexec(Op::LogicalAnd, false, false, true, true),
            O::SNorSaveexecB32 => self.s_saveexec(Op::LogicalOr, false, false, false, true),
            O::SNorSaveexecB64 => self.s_saveexec(Op::LogicalOr, false, false, true, true),
            O::SXnorSaveexecB32 => self.s_saveexec(Op::LogicalXor, false, false, false, true),
            O::SXnorSaveexecB64 => self.s_saveexec(Op::LogicalXor, false, false, true, true),
            O::SAddU32 => self.add_u32(false, false),
            O::SAddcU32 => self.add_u32(false, true),
            O::SSubU32 => self.sub_u32(false, false),
            O::SSubbU32 => self.subb_u32(false, false),
            O::SAbsdiffI32 => {
                let a = self.read_u32(inst.src(0))?;
                let b = self.read_u32(inst.src(1))?;
                let difference = self.e.isub(a, b)?;
                let result = self.e.emit(Op::IAbs32, &[difference])?;
                self.write(inst.dst, result)?;
                let nonzero = self.e.ine(result, u32v(0))?;
                self.e.set(Op::SetScc, 0, nonzero)
            }
            O::SAddI32 => self.s_add_sub_i32(false),
            O::SSubI32 => self.s_add_sub_i32(true),
            O::SLshl1AddU32 => self.s_lshl_add_u32(1),
            O::SLshl2AddU32 => self.s_lshl_add_u32(2),
            O::SLshl3AddU32 => self.s_lshl_add_u32(3),
            O::SLshl4AddU32 => self.s_lshl_add_u32(4),
            O::SMinI32 => self.scalar_min_max(Op::SMin32, Op::SLessThan32),
            O::SMaxI32 => self.scalar_min_max(Op::SMax32, Op::SGreaterThan32),
            O::SMinU32 => self.scalar_min_max(Op::UMin32, Op::ULessThan32),
            O::SMaxU32 => self.scalar_min_max(Op::UMax32, Op::UGreaterThan32),
            O::SCmpEqU32 | O::SCmpEqI32 => {
                self.integer_compare(Op::IEqual32, Type::U32, true, false)
            }
            O::SCmpLgU32 | O::SCmpLgI32 => {
                self.integer_compare(Op::INotEqual32, Type::U32, true, false)
            }
            O::SCmpGtU32 => self.integer_compare(Op::UGreaterThan32, Type::U32, true, false),
            O::SCmpGeU32 => self.integer_compare(Op::UGreaterThanEqual32, Type::U32, true, false),
            O::SCmpLtU32 => self.integer_compare(Op::ULessThan32, Type::U32, true, false),
            O::SCmpLeU32 => self.integer_compare(Op::ULessThanEqual32, Type::U32, true, false),
            O::SCmpGtI32 => self.integer_compare(Op::SGreaterThan32, Type::U32, true, false),
            O::SCmpGeI32 => self.integer_compare(Op::SGreaterThanEqual32, Type::U32, true, false),
            O::SCmpLtI32 => self.integer_compare(Op::SLessThan32, Type::U32, true, false),
            O::SCmpLeI32 => self.integer_compare(Op::SLessThanEqual32, Type::U32, true, false),
            O::SCmpEqU64 => self.integer_compare(Op::IEqual64, Type::U64, true, false),
            O::SCmpLgU64 => self.integer_compare(Op::INotEqual64, Type::U64, true, false),
            O::SAndB64 => self.s_u64_mask(Op::LogicalAnd, Op::BitwiseAnd32, false, false, false),
            O::SAndn2B64 => self.s_u64_mask(Op::LogicalAnd, Op::BitwiseAnd32, true, false, false),
            O::SOrB64 => self.s_u64_mask(Op::LogicalOr, Op::BitwiseOr32, false, false, false),
            O::SOrn2B64 => self.s_u64_mask(Op::LogicalOr, Op::BitwiseOr32, true, false, false),
            O::SXorB64 => self.s_u64_mask(Op::LogicalXor, Op::BitwiseXor32, false, false, false),
            O::SNandB64 => self.s_u64_mask(Op::LogicalAnd, Op::BitwiseAnd32, false, true, false),
            O::SNorB64 => self.s_u64_mask(Op::LogicalOr, Op::BitwiseOr32, false, true, false),
            O::SXnorB64 => self.s_u64_mask(Op::LogicalXor, Op::BitwiseXor32, false, true, false),
            O::SNotB64 => self.s_u64_mask(Op::LogicalAnd, Op::BitwiseAnd32, false, false, true),
            O::SAbsI32 => self.simple_integer(Op::IAbs32, Type::U32, false, false, true),
            O::SSextI32I8 | O::SSextI32I16 => {
                let bits = if inst.opcode == O::SSextI32I8 { 8 } else { 16 };
                let source = self.read_u32(inst.src(0))?;
                let value = self
                    .e
                    .emit(Op::BitFieldSExtract, &[source, u32v(0), u32v(bits)])?;
                self.write(inst.dst, value)
            }
            O::SMulI32 | O::SMulkI32 => {
                self.simple_integer(Op::IMul32, Type::U32, false, false, false)
            }
            O::SMulHiU32 => self.simple_integer(Op::UMulHi, Type::U32, false, false, false),
            O::SMulHiI32 => self.simple_integer(Op::SMulHi, Type::U32, false, false, false),
            O::SAndB32 => self.simple_integer(Op::BitwiseAnd32, Type::U32, false, false, true),
            O::SOrB32 => self.simple_integer(Op::BitwiseOr32, Type::U32, false, false, true),
            O::SXorB32 => self.simple_integer(Op::BitwiseXor32, Type::U32, false, false, true),
            O::SNotB32 => self.simple_integer(Op::BitwiseNot32, Type::U32, false, false, true),
            O::SBrevB32 => self.simple_integer(Op::BitReverse32, Type::U32, false, false, false),
            O::SBrevB64 => {
                let value = self.read_pair(inst.src(0))?;
                let low = self.e.emit(Op::BitReverse32, &[value[1]])?;
                let high = self.e.emit(Op::BitReverse32, &[value[0]])?;
                self.write_pair(inst.dst, [low, high])
            }
            O::SBcnt1I32B32 => self.simple_integer(Op::BitCount32, Type::U32, false, false, true),
            O::SBcnt1I32B64 => {
                let value = self.read_pair(inst.src(0))?;
                let low = self.e.emit(Op::BitCount32, &[value[0]])?;
                let high = self.e.emit(Op::BitCount32, &[value[1]])?;
                let result = self.e.iadd(low, high)?;
                self.write(inst.dst, result)?;
                let nonzero = self.e.ine(result, u32v(0))?;
                self.e.set(Op::SetScc, 0, nonzero)
            }
            O::SFf1I32B32 => self.simple_integer(Op::FindILsb32, Type::U32, false, false, false),
            O::SLshlB32 => {
                self.simple_integer(Op::ShiftLeftLogical32, Type::U32, false, true, true)
            }
            O::SLshrB32 => {
                self.simple_integer(Op::ShiftRightLogical32, Type::U32, false, true, true)
            }
            O::SAshrI32 => {
                self.simple_integer(Op::ShiftRightArithmetic32, Type::U32, false, true, true)
            }
            O::SLshlB64 => {
                self.simple_integer(Op::ShiftLeftLogical64, Type::U64, false, false, true)
            }
            O::SLshrB64 => {
                self.simple_integer(Op::ShiftRightLogical64, Type::U64, false, false, true)
            }
            O::SAshrI64 => self.s_ashr_i64(),
            O::SAndn2B32 => self.composed_integer_binary(Op::BitwiseAnd32, true, false, true),
            O::SOrn2B32 => self.composed_integer_binary(Op::BitwiseOr32, true, false, true),
            O::SNandB32 => self.composed_integer_binary(Op::BitwiseAnd32, false, true, true),
            O::SNorB32 => self.composed_integer_binary(Op::BitwiseOr32, false, true, true),
            O::SXnorB32 => self.composed_integer_binary(Op::BitwiseXor32, false, true, true),
            O::SFf1I32B64 => self.s_ff1_i32_b64(),
            O::SFlbitI32B32 => self.ffbh_32(false),
            O::SFlbitI32B64 => self.s_flbit_i32_b64(),
            O::SBitset0B32 => self.s_bitset_b32(false),
            O::SBitset1B32 => self.s_bitset_b32(true),
            O::SBitset0B64 => self.s_bitset_b64(false),
            O::SBitset1B64 => self.s_bitset_b64(true),
            O::SBitreplicateB64B32 => self.s_bitreplicate(),
            O::SQuadmaskB64 => self.s_quadmask(),
            O::SBfmB32 => self.bfm_b32(),
            O::SBfmB64 => {
                let count = self.read_u32(inst.src(0))?;
                let offset = self.read_u32(inst.src(1))?;
                let limit = self
                    .e
                    .emit(Op::ShiftLeftLogical64, &[Value::U64(1), count])?;
                let mask = self.e.emit(Op::ISub64, &[limit, Value::U64(1)])?;
                let result = self.e.emit(Op::ShiftLeftLogical64, &[mask, offset])?;
                self.write(inst.dst, result)
            }
            O::SBfeU32 => self.s_bfe_u32(false),
            O::SBfeI32 => self.s_bfe_u32(true),
            O::SBfeU64 => self.s_bfe_u64(),
            O::SBitcmp0B32 => self.s_bitcmp_b32(false),
            O::SBitcmp1B32 => self.s_bitcmp_b32(true),
            O::SBitcmp0B64 => self.s_bitcmp_b64(false),
            O::SBitcmp1B64 => self.s_bitcmp_b64(true),
            O::SPackLlB32B16 => self.pack_b16(false, false),
            O::SPackLhB32B16 => self.pack_b16(false, true),
            O::SPackHhB32B16 => self.pack_b16(true, true),
            O::SNop | O::SSleep | O::SSetprio | O::SClause => {
                self.e.emit(Op::ControlNop, &[]).map(|_| ())
            }
            O::STrap => Err(self.unsupported(U::Trap)),
            O::SWaitcntDepctr | O::SInstPrefetch => Ok(()),
            O::SBarrier => {
                if self.profile.compute().is_none() {
                    return Err(self.unsupported(U::BarrierOutsideWorkgroup));
                }
                self.e.emit(Op::Barrier, &[]).map(|_| ())
            }
            O::SSendmsg => Err(self.unsupported(U::SendMessage)),
            O::STtracedata => Err(self.unsupported(U::ThreadTrace)),
            O::SCbranchCdbgsys | O::SCbranchCdbgsysOrUser => {
                Err(self.unsupported(U::ConditionalDebugBranch))
            }
            O::SBranch
            | O::SCbranchScc0
            | O::SCbranchScc1
            | O::SCbranchVccz
            | O::SCbranchVccnz
            | O::SCbranchExecz
            | O::SCbranchExecnz
            | O::SEndpgm => Ok(()),
            _ => Err(self.unsupported(U::NotPorted)),
        }
    }

    // ------------------------------------------------------ Control.cpp

    pub(super) fn mov_b32(&mut self, float_modifiers: bool) -> R {
        let inst = self.inst;
        let source = inst.src(0);
        let value = if float_modifiers && (source.neg || source.abs) {
            self.read_f32(source)?
        } else {
            self.read_u32(source)?
        };
        self.write(inst.dst, value)
    }

    fn s_mov_b64(&mut self) -> R {
        let inst = self.inst;
        let source = inst.src(0);
        let mask_source = matches!(
            source.kind,
            OperandKind::Sgpr | OperandKind::ExecLo | OperandKind::VccLo
        );
        let mut mask = None;
        if mask_source {
            let live = if source.kind == OperandKind::VccLo {
                self.e.get(Op::GetVcc, 0)?
            } else {
                self.read_mask(source)?
            };
            let valid = self.read_mask_valid(source)?;
            mask = Some((live, valid));
        }
        let pair = self.read_pair(source)?;
        self.write_pair(inst.dst, pair)?;
        if let Some((live, valid)) = mask {
            match inst.dst.kind {
                OperandKind::ExecLo => self.e.set(Op::SetExec, 0, live)?,
                OperandKind::VccLo => self.e.set(Op::SetVcc, 0, live)?,
                OperandKind::Sgpr => {
                    let reg = u32::from(inst.dst.reg);
                    self.e.set(Op::SetThreadBitScalarRegister, reg, live)?;
                    self.e.set(Op::SetScalarMaskTag, reg, valid)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn scalar_select64(&mut self, false_source: Opnd) -> R {
        let inst = self.inst;
        let condition = self.e.get(Op::GetScc, 0)?;
        let lhs = self.read_pair(inst.src(0))?;
        let rhs = self.read_pair(false_source)?;
        let lhs_mask = self.read_mask(inst.src(0))?;
        let rhs_mask = self.read_mask(false_source)?;
        let selected = self
            .e
            .emit(Op::SelectU1, &[condition, lhs_mask, rhs_mask])?;
        let lhs_valid = self.read_mask_valid(inst.src(0))?;
        let rhs_valid = self.read_mask_valid(false_source)?;
        let selected_valid = self
            .e
            .emit(Op::SelectU1, &[condition, lhs_valid, rhs_valid])?;
        let low = self.e.select(condition, lhs[0], rhs[0])?;
        let high = self.e.select(condition, lhs[1], rhs[1])?;
        self.write_pair(inst.dst, [low, high])?;
        match inst.dst.kind {
            OperandKind::ExecLo => self.e.set(Op::SetExec, 0, selected),
            OperandKind::VccLo => self.e.set(Op::SetVcc, 0, selected),
            OperandKind::Sgpr => {
                let reg = u32::from(inst.dst.reg);
                self.e.set(Op::SetThreadBitScalarRegister, reg, selected)?;
                self.e.set(Op::SetScalarMaskTag, reg, selected_valid)
            }
            _ => Ok(()),
        }
    }

    fn s_saveexec(
        &mut self,
        operation: Op,
        negate_exec: bool,
        negate_source: bool,
        write_64: bool,
        negate_result: bool,
    ) -> R {
        let inst = self.inst;
        if !write_64 {
            let old = self.e.get(Op::GetExecLo, 0)?;
            let src = self.read_u32(inst.src(0))?;
            let lhs = if negate_exec { self.e.not(old)? } else { old };
            let rhs = if negate_source { self.e.not(src)? } else { src };
            let mut result = match operation {
                Op::LogicalAnd => self.e.and(lhs, rhs)?,
                Op::LogicalOr => self.e.or(lhs, rhs)?,
                _ => self.e.xor(lhs, rhs)?,
            };
            if negate_result {
                result = self.e.not(result)?;
            }
            self.write_raw(inst.dst, old)?;
            self.write_raw(Opnd::special(OperandKind::ExecLo), result)?;
            let nonzero = self.e.ine(result, u32v(0))?;
            return self.e.set(Op::SetScc, 0, nonzero);
        }
        let old = self.e.get(Op::GetExec, 0)?;
        let src = self.read_mask(inst.src(0))?;
        let lhs = if negate_exec { self.e.lnot(old)? } else { old };
        let rhs = if negate_source {
            self.e.lnot(src)?
        } else {
            src
        };
        let mut result = self.e.emit(operation, &[lhs, rhs])?;
        if negate_result {
            result = self.e.lnot(result)?;
        }
        self.write_mask(inst.dst, old, true)?;
        let mask = self.ballot_mask(result)?;
        self.e.set(Op::SetExec, 0, result)?;
        self.e.set(Op::SetExecLo, 0, mask[0])?;
        self.e.set(Op::SetExecHi, 0, mask[1])?;
        let either = self.e.or(mask[0], mask[1])?;
        let nonzero = self.e.ine(either, u32v(0))?;
        self.e.set(Op::SetScc, 0, nonzero)
    }

    pub(super) fn add_u32(&mut self, vector: bool, carry_in: bool) -> R {
        let inst = self.inst;
        let lhs = self.read_u32(inst.src(0))?;
        let rhs = self.read_u32(inst.src(1))?;
        let carry = if !carry_in {
            u32v(0)
        } else if vector {
            self.condition_bit(inst.src(2))?
        } else {
            self.condition_bit(scc())?
        };
        let add0 = self.e.emit(Op::IAddCarry32, &[lhs, rhs])?;
        let partial = self.e.extract(add0, 0)?;
        let carry0 = self.e.extract(add0, 1)?;
        let add1 = self.e.emit(Op::IAddCarry32, &[partial, carry])?;
        let result = self.e.extract(add1, 0)?;
        let carry1 = self.e.extract(add1, 1)?;
        let either = self.e.or(carry0, carry1)?;
        let carry_out = self.e.ine(either, u32v(0))?;
        self.write(inst.dst, result)?;
        if vector {
            let exec = self.e.get(Op::GetExec, 0)?;
            let masked = self.e.land(exec, carry_out)?;
            self.write_mask(inst.dst2, masked, false).map(|_| ())
        } else {
            self.e.set(Op::SetScc, 0, carry_out)
        }
    }

    pub(super) fn sub_u32(&mut self, vector: bool, reverse: bool) -> R {
        let inst = self.inst;
        let (a, b) = if reverse { (1, 0) } else { (0, 1) };
        let lhs = self.read_u32(inst.src(a))?;
        let rhs = self.read_u32(inst.src(b))?;
        let result = self.e.isub(lhs, rhs)?;
        let borrow = self.e.ugt(rhs, lhs)?;
        self.write(inst.dst, result)?;
        if vector {
            let exec = self.e.get(Op::GetExec, 0)?;
            let masked = self.e.land(exec, borrow)?;
            self.write_mask(inst.dst2, masked, false).map(|_| ())
        } else {
            self.e.set(Op::SetScc, 0, borrow)
        }
    }

    pub(super) fn subb_u32(&mut self, vector: bool, reverse: bool) -> R {
        let inst = self.inst;
        let (a, b) = if reverse { (1, 0) } else { (0, 1) };
        let lhs = self.read_u32(inst.src(a))?;
        let rhs = self.read_u32(inst.src(b))?;
        let borrow_in = if vector {
            self.condition_bit(inst.src(2))?
        } else {
            self.condition_bit(scc())?
        };
        let partial = self.e.isub(lhs, rhs)?;
        let result = self.e.isub(partial, borrow_in)?;
        let borrow0 = self.e.ugt(rhs, lhs)?;
        let borrow1 = self.e.ugt(borrow_in, partial)?;
        let borrow = self.e.lor(borrow0, borrow1)?;
        self.write(inst.dst, result)?;
        if vector {
            let exec = self.e.get(Op::GetExec, 0)?;
            let masked = self.e.land(exec, borrow)?;
            self.write_mask(inst.dst2, masked, false).map(|_| ())
        } else {
            self.e.set(Op::SetScc, 0, borrow)
        }
    }

    fn s_add_sub_i32(&mut self, subtract: bool) -> R {
        let inst = self.inst;
        let lhs = self.read_u32(inst.src(0))?;
        let rhs = self.read_u32(inst.src(1))?;
        let result = if subtract {
            self.e.isub(lhs, rhs)?
        } else {
            self.e.iadd(lhs, rhs)?
        };
        let lhs_sign = self.e.shr(lhs, u32v(31))?;
        let rhs_sign = self.e.shr(rhs, u32v(31))?;
        let out_sign = self.e.shr(result, u32v(31))?;
        let inputs = if subtract {
            self.e.ine(lhs_sign, rhs_sign)?
        } else {
            self.e.ieq(lhs_sign, rhs_sign)?
        };
        let changed = self.e.ine(lhs_sign, out_sign)?;
        self.write(inst.dst, result)?;
        let overflow = self.e.land(inputs, changed)?;
        self.e.set(Op::SetScc, 0, overflow)
    }

    fn s_lshl_add_u32(&mut self, shift_amount: u32) -> R {
        let inst = self.inst;
        let lhs = self.read_u32(inst.src(0))?;
        let rhs = self.read_u32(inst.src(1))?;
        let shifted = self.e.shl(lhs, u32v(shift_amount))?;
        let result = self.e.iadd(shifted, rhs)?;
        let add_carry = self.e.ult(result, shifted)?;
        let inverse = self.e.isub(u32v(32), u32v(shift_amount))?;
        let shifted_out = self.e.shr(lhs, inverse)?;
        let shift_carry = self.e.ine(shifted_out, u32v(0))?;
        self.write(inst.dst, result)?;
        let carry = self.e.lor(add_carry, shift_carry)?;
        self.e.set(Op::SetScc, 0, carry)
    }

    fn scalar_min_max(&mut self, value_op: Op, compare_op: Op) -> R {
        let inst = self.inst;
        let lhs = self.read_u32(inst.src(0))?;
        let rhs = self.read_u32(inst.src(1))?;
        let value = self.e.emit(value_op, &[lhs, rhs])?;
        self.write(inst.dst, value)?;
        let compare = self.e.emit(compare_op, &[lhs, rhs])?;
        self.e.set(Op::SetScc, 0, compare)
    }

    // ------------------------------------------------------ Integer.cpp

    fn u64_mask_binary(
        &mut self,
        op: Op,
        negate_rhs: bool,
        negate_result: bool,
    ) -> Result<Value, CompileError> {
        let inst = self.inst;
        let lhs = self.read_mask(inst.src(0))?;
        let mut rhs = self.read_mask(inst.src(1))?;
        if negate_rhs {
            rhs = self.e.lnot(rhs)?;
        }
        let result = self.e.emit(op, &[lhs, rhs])?;
        if negate_result {
            self.e.lnot(result)
        } else {
            Ok(result)
        }
    }

    fn s_u64_mask(
        &mut self,
        logical: Op,
        bit_op: Op,
        negate_rhs: bool,
        negate_result: bool,
        unary: bool,
    ) -> R {
        let inst = self.inst;
        let invocation = if unary {
            let mask = self.read_mask(inst.src(0))?;
            self.e.lnot(mask)?
        } else {
            self.u64_mask_binary(logical, negate_rhs, negate_result)?
        };
        let exec_or_vcc = |op: Opnd| {
            matches!(
                op.kind,
                OperandKind::ExecLo | OperandKind::ExecHi | OperandKind::VccLo | OperandKind::VccHi
            )
        };
        if exec_or_vcc(inst.dst)
            || exec_or_vcc(inst.src(0))
            || (inst.src_count > 1 && exec_or_vcc(inst.src(1)))
        {
            let mask = self.write_mask(inst.dst, invocation, true)?;
            let either = self.e.or(mask[0], mask[1])?;
            let nonzero = self.e.ine(either, u32v(0))?;
            return self.e.set(Op::SetScc, 0, nonzero);
        }
        let lhs = self.read_pair(inst.src(0))?;
        let mut valid = self.read_mask_valid(inst.src(0))?;
        if inst.src_count > 1 {
            let rhs_valid = self.read_mask_valid(inst.src(1))?;
            valid = self.e.land(valid, rhs_valid)?;
        }
        let mut result = [u32v(0); 2];
        if unary {
            result = [self.e.not(lhs[0])?, self.e.not(lhs[1])?];
        } else {
            let rhs = self.read_pair(inst.src(1))?;
            for component in 0..2 {
                let operand = if negate_rhs {
                    self.e.not(rhs[component])?
                } else {
                    rhs[component]
                };
                let value = self.e.emit(bit_op, &[lhs[component], operand])?;
                result[component] = if negate_result {
                    self.e.not(value)?
                } else {
                    value
                };
            }
        }
        self.write_pair(inst.dst, result)?;
        if inst.dst.kind == OperandKind::Sgpr {
            let reg = u32::from(inst.dst.reg);
            self.e
                .set(Op::SetThreadBitScalarRegister, reg, invocation)?;
            self.e.set(Op::SetScalarMaskTag, reg, valid)?;
        }
        let either = self.e.or(result[0], result[1])?;
        let nonzero = self.e.ine(either, u32v(0))?;
        self.e.set(Op::SetScc, 0, nonzero)
    }

    /// Source `SimpleInteger`.
    pub(super) fn simple_integer(
        &mut self,
        op: Op,
        ty: Type,
        reverse: bool,
        mask_shift_count: bool,
        update_scc: bool,
    ) -> R {
        let inst = self.inst;
        let count = inst.src_count;
        if count == 0 || count > 3 || count != op.args().len() {
            return Err(CompileError::Internal("simple integer source count"));
        }
        let mut args = [u32v(0); 3];
        for (index, &want) in op.args().iter().enumerate() {
            let source = if reverse && index < 2 {
                1 - index
            } else {
                index
            };
            let arg_ty = if want == Type::U64 || want == Type::U32 {
                want
            } else {
                ty
            };
            args[index] = self.read(inst.src(source), arg_ty)?;
            if mask_shift_count && index == 1 {
                args[index] = self.e.and(args[index], u32v(31))?;
            }
        }
        let result = self.e.emit(op, &args[..count])?;
        self.write(inst.dst, result)?;
        if update_scc {
            let nonzero = if op.result() == Type::U64 {
                self.e.emit(Op::INotEqual64, &[result, Value::U64(0)])?
            } else {
                self.e.ine(result, u32v(0))?
            };
            self.e.set(Op::SetScc, 0, nonzero)?;
        }
        Ok(())
    }

    fn s_ashr_i64(&mut self) -> R {
        let inst = self.inst;
        let src = inst.src(0);
        let source = if src.kind == OperandKind::LiteralConstant {
            let low = self.read_u32(src)?;
            let high = if src.value & 0x8000_0000 != 0 {
                u32::MAX
            } else {
                0
            };
            self.e.construct_u64(low, u32v(high))?
        } else {
            self.read(src, Type::U64)?
        };
        let shift = self.read_u32(inst.src(1))?;
        let result = self.e.emit(Op::ShiftRightArithmetic64, &[source, shift])?;
        self.write(inst.dst, result)?;
        let nonzero = self.e.emit(Op::INotEqual64, &[result, Value::U64(0)])?;
        self.e.set(Op::SetScc, 0, nonzero)
    }

    pub(super) fn composed_integer_binary(
        &mut self,
        op: Op,
        negate_rhs: bool,
        negate_result: bool,
        update_scc: bool,
    ) -> R {
        let inst = self.inst;
        let lhs = self.read_u32(inst.src(0))?;
        let mut rhs = self.read_u32(inst.src(1))?;
        if negate_rhs {
            rhs = self.e.not(rhs)?;
        }
        let mut result = self.e.emit(op, &[lhs, rhs])?;
        if negate_result {
            result = self.e.not(result)?;
        }
        self.write(inst.dst, result)?;
        if update_scc {
            let nonzero = self.e.ine(result, u32v(0))?;
            self.e.set(Op::SetScc, 0, nonzero)?;
        }
        Ok(())
    }

    fn s_ff1_i32_b64(&mut self) -> R {
        let inst = self.inst;
        let source = self.read(inst.src(0), Type::U64)?;
        let low = self.e.extract(source, 0)?;
        let high = self.e.extract(source, 1)?;
        let low_lsb = self.e.emit(Op::FindILsb32, &[low])?;
        let high_lsb = self.e.emit(Op::FindILsb32, &[high])?;
        let high_position = self.e.iadd(high_lsb, u32v(32))?;
        let low_nonzero = self.e.ine(low, u32v(0))?;
        let high_nonzero = self.e.ine(high, u32v(0))?;
        let fallback = self.e.select(high_nonzero, high_position, u32v(u32::MAX))?;
        let result = self.e.select(low_nonzero, low_lsb, fallback)?;
        self.write(inst.dst, result)
    }

    /// Source `V_FFBH_32` (also S_FLBIT_I32_B32).
    pub(super) fn ffbh_32(&mut self, sign: bool) -> R {
        let inst = self.inst;
        let source = self.read_u32(inst.src(0))?;
        let value = if sign {
            let spread = self.e.sar(source, u32v(31))?;
            self.e.xor(source, spread)?
        } else {
            source
        };
        let msb = self.e.emit(Op::FindUMsb32, &[value])?;
        let position = self.e.isub(u32v(31), msb)?;
        let nonzero = self.e.ine(value, u32v(0))?;
        let result = self.e.select(nonzero, position, u32v(u32::MAX))?;
        self.write(inst.dst, result)
    }

    fn s_flbit_i32_b64(&mut self) -> R {
        let inst = self.inst;
        let source = self.read(inst.src(0), Type::U64)?;
        let msb = self.e.emit(Op::FindUMsb64, &[source])?;
        let position = self.e.isub(u32v(63), msb)?;
        let zero = self.e.construct_u64(u32v(0), u32v(0))?;
        let nonzero = self.e.emit(Op::INotEqual64, &[source, zero])?;
        let result = self.e.select(nonzero, position, u32v(u32::MAX))?;
        self.write(inst.dst, result)
    }

    fn s_bitset_b32(&mut self, set: bool) -> R {
        let inst = self.inst;
        let raw = self.read_u32(inst.src(0))?;
        let offset = self.e.and(raw, u32v(31))?;
        let bit = self.e.shl(u32v(1), offset)?;
        let old = self.read_u32(inst.dst)?;
        let result = if set {
            self.e.or(old, bit)?
        } else {
            let inverse = self.e.not(bit)?;
            self.e.and(old, inverse)?
        };
        self.write(inst.dst, result)
    }

    fn s_bitset_b64(&mut self, set: bool) -> R {
        let inst = self.inst;
        let raw = self.read_u32(inst.src(0))?;
        let offset = self.e.and(raw, u32v(63))?;
        let word_bit = self.e.and(offset, u32v(31))?;
        let bit = self.e.shl(u32v(1), word_bit)?;
        let old = self.read_pair(inst.dst)?;
        let (low, high) = if set {
            (self.e.or(old[0], bit)?, self.e.or(old[1], bit)?)
        } else {
            let inverse = self.e.not(bit)?;
            (self.e.and(old[0], inverse)?, self.e.and(old[1], inverse)?)
        };
        let upper = self.e.emit(Op::UGreaterThanEqual32, &[offset, u32v(32)])?;
        let new_low = self.e.select(upper, old[0], low)?;
        let new_high = self.e.select(upper, high, old[1])?;
        self.write_pair(inst.dst, [new_low, new_high])
    }

    fn s_bitreplicate(&mut self) -> R {
        let inst = self.inst;
        let source = self.read_u32(inst.src(0))?;
        let low = self.e.and(source, u32v(0xffff))?;
        let high = self.e.shr(source, u32v(16))?;
        let replicate = |t: &mut Self, value: Value| -> Result<Value, CompileError> {
            let mut bits = value;
            for (shift, mask) in [
                (8, 0x00ff_00ff),
                (4, 0x0f0f_0f0f),
                (2, 0x3333_3333),
                (1, 0x5555_5555),
            ] {
                let shifted = t.e.shl(bits, u32v(shift))?;
                let merged = t.e.or(bits, shifted)?;
                bits = t.e.and(merged, u32v(mask))?;
            }
            let shifted = t.e.shl(bits, u32v(1))?;
            t.e.or(bits, shifted)
        };
        let low = replicate(self, low)?;
        let high = replicate(self, high)?;
        let result = self.e.construct_u64(low, high)?;
        self.write(inst.dst, result)
    }

    fn s_quadmask(&mut self) -> R {
        let inst = self.inst;
        let source = self.read_pair(inst.src(0))?;
        let compact = |t: &mut Self, value: Value| -> Result<Value, CompileError> {
            let s1 = t.e.shr(value, u32v(1))?;
            let mut bits = t.e.or(value, s1)?;
            let s2 = t.e.shr(bits, u32v(2))?;
            bits = t.e.or(bits, s2)?;
            bits = t.e.and(bits, u32v(0x1111_1111))?;
            for (shift, mask) in [(3, 0x0303_0303), (6, 0x000f_000f), (12, 0xff)] {
                let shifted = t.e.shr(bits, u32v(shift))?;
                let merged = t.e.or(bits, shifted)?;
                bits = t.e.and(merged, u32v(mask))?;
            }
            Ok(bits)
        };
        let low = compact(self, source[0])?;
        let high = compact(self, source[1])?;
        let high = self.e.shl(high, u32v(8))?;
        let quads = self.e.or(low, high)?;
        let result = self.e.construct_u64(quads, u32v(0))?;
        self.write(inst.dst, result)?;
        let nonzero = self.e.emit(Op::INotEqual64, &[result, Value::U64(0)])?;
        self.e.set(Op::SetScc, 0, nonzero)
    }

    pub(super) fn bfm_b32(&mut self) -> R {
        let inst = self.inst;
        let count_raw = self.read_u32(inst.src(0))?;
        let count = self.e.and(count_raw, u32v(31))?;
        let offset_raw = self.read_u32(inst.src(1))?;
        let offset = self.e.and(offset_raw, u32v(31))?;
        let result = self.e.emit(
            Op::BitFieldInsert,
            &[u32v(0), u32v(u32::MAX), offset, count],
        )?;
        self.write(inst.dst, result)
    }

    fn right_mask_32(&mut self, count: Value) -> Result<Value, CompileError> {
        self.e.emit(
            Op::BitFieldInsert,
            &[u32v(0), u32v(u32::MAX), u32v(0), count],
        )
    }

    fn s_bfe_u32(&mut self, sign: bool) -> R {
        let inst = self.inst;
        let source = self.read_u32(inst.src(0))?;
        let field = self.read_u32(inst.src(1))?;
        let offset = self
            .e
            .emit(Op::BitFieldUExtract, &[field, u32v(0), u32v(5)])?;
        let raw_count = self
            .e
            .emit(Op::BitFieldUExtract, &[field, u32v(16), u32v(7)])?;
        let available = self.e.isub(u32v(32), offset)?;
        let count = self.e.emit(Op::UMin32, &[raw_count, available])?;
        let op = if sign {
            Op::BitFieldSExtract
        } else {
            Op::BitFieldUExtract
        };
        let result = self.e.emit(op, &[source, offset, count])?;
        self.write(inst.dst, result)?;
        let nonzero = self.e.ine(result, u32v(0))?;
        self.e.set(Op::SetScc, 0, nonzero)
    }

    fn s_bfe_u64(&mut self) -> R {
        let inst = self.inst;
        let source = self.read(inst.src(0), Type::U64)?;
        let field = self.read_u32(inst.src(1))?;
        let offset = self
            .e
            .emit(Op::BitFieldUExtract, &[field, u32v(0), u32v(6)])?;
        let raw_count = self
            .e
            .emit(Op::BitFieldUExtract, &[field, u32v(16), u32v(7)])?;
        let available = self.e.isub(u32v(64), offset)?;
        let count = self.e.emit(Op::UMin32, &[raw_count, available])?;
        let shifted = self.e.emit(Op::ShiftRightLogical64, &[source, offset])?;
        // Source RightMask64.
        let below32 = self.e.ult(count, u32v(32))?;
        let above32 = self.e.ugt(count, u32v(32))?;
        let low_count = self.e.select(below32, count, u32v(32))?;
        let excess = self.e.isub(count, u32v(32))?;
        let high_count = self.e.select(above32, excess, u32v(0))?;
        let low_mask = self.right_mask_32(low_count)?;
        let high_mask = self.right_mask_32(high_count)?;
        let mask = self.e.construct_u64(low_mask, high_mask)?;
        let result = self.e.emit(Op::BitwiseAnd64, &[shifted, mask])?;
        self.write(inst.dst, result)?;
        let nonzero = self.e.emit(Op::INotEqual64, &[result, Value::U64(0)])?;
        self.e.set(Op::SetScc, 0, nonzero)
    }

    fn s_bitcmp_b32(&mut self, expected: bool) -> R {
        let inst = self.inst;
        let value = self.read_u32(inst.src(0))?;
        let raw = self.read_u32(inst.src(1))?;
        let offset = self.e.and(raw, u32v(31))?;
        let bit = self
            .e
            .emit(Op::BitFieldUExtract, &[value, offset, u32v(1)])?;
        let equal = self.e.ieq(bit, u32v(u32::from(expected)))?;
        self.write_compare_result(inst.dst, equal)
    }

    fn s_bitcmp_b64(&mut self, expected: bool) -> R {
        let inst = self.inst;
        let value = self.read_pair(inst.src(0))?;
        let raw = self.read_u32(inst.src(1))?;
        let offset = self.e.and(raw, u32v(63))?;
        let word_bit = self.e.and(offset, u32v(31))?;
        let low = self.e.ult(offset, u32v(32))?;
        let word = self.e.select(low, value[0], value[1])?;
        let bit = self
            .e
            .emit(Op::BitFieldUExtract, &[word, word_bit, u32v(1)])?;
        let equal = self.e.ieq(bit, u32v(u32::from(expected)))?;
        self.write_compare_result(inst.dst, equal)
    }

    pub(super) fn pack_b16(&mut self, high0: bool, high1: bool) -> R {
        let inst = self.inst;
        let a = self.read_u32(inst.src(0))?;
        let lo = if high0 { self.e.shr(a, u32v(16))? } else { a };
        let b = self.read_u32(inst.src(1))?;
        let hi = if high1 { self.e.shr(b, u32v(16))? } else { b };
        let result = self.pack_u16_lanes(lo, hi)?;
        self.write(inst.dst, result)
    }

    /// Source `PackU16Lanes`.
    pub(super) fn pack_u16_lanes(
        &mut self,
        low: Value,
        high: Value,
    ) -> Result<Value, CompileError> {
        let lo = self.e.and(low, u32v(0xffff))?;
        let hi_masked = self.e.and(high, u32v(0xffff))?;
        let hi = self.e.shl(hi_masked, u32v(16))?;
        self.e.or(lo, hi)
    }

    /// Source `EmitIntegerCompare`.
    pub(super) fn integer_compare(&mut self, op: Op, ty: Type, scalar: bool, cmpx: bool) -> R {
        let inst = self.inst;
        let signed_64 = matches!(op, Op::SLessThan64 | Op::SLessThanEqual64)
            || matches!(
                inst.opcode,
                O::VCmpEqI64 | O::VCmpxEqI64 | O::VCmpNeI64 | O::VCmpxNeI64
            );
        let read = |t: &mut Self, operand: Opnd| -> Result<Value, CompileError> {
            if signed_64 && operand.kind == OperandKind::LiteralConstant {
                return Ok(Value::U64(i64::from(operand.value as i32) as u64));
            }
            t.read(operand, ty)
        };
        let lhs = read(self, inst.src(0))?;
        let rhs = read(self, inst.src(1))?;
        let value = self.e.emit(op, &[lhs, rhs])?;
        self.compare_result(value, scalar, cmpx)
    }
}
