// SPDX-License-Identifier: GPL-2.0-only
// Direct Rust adaptation of KytyPS5-Performance at
// 209dffe9032d72749257ce18680d506b891284de, frontend/translate/Translate.cpp
// (operand reads and writes, SDWA selection, VCC/EXEC/SCC and SGPR mask
// provenance, BallotMask/ThreadBit, AddBranchCondition and TranslateProgram's
// typed entry block) and Dispatch.cpp.
// PentaX modifications: typed refusals replace EXIT/FailMissingTranslation;
// DPP, 16/64-bit float and lane permutation forms are refused rather than
// lowered; register-pair overflow is rejected; source-defined defaults are
// kept and documented in the README.

mod memory;
mod scalar;
mod vector;

use pentax_shader::{Export, Family, Instruction, MemoryAccess, Opcode, OperandKind, ShaderStage};

use crate::CompileError;
use crate::budget::Budget;
use crate::cfg::{BranchCondition, Cfg, NONE, TerminatorKind};
use crate::error::UnsupportedReason;
use crate::ir::{Builtin, Emitter, Ir, IrBlock, IrTerminator, NIL, Op, Type, Value};
use crate::profile::Profile;

/// A decoder operand with the source `Decoder::Operand` fields.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Opnd {
    pub kind: OperandKind,
    pub value: u32,
    pub signed: i32,
    pub reg: u16,
    pub sel: u8,
    pub unused: u8,
    pub omod: u8,
    pub neg: bool,
    pub abs: bool,
    pub clamp: bool,
    pub sext: bool,
    pub op_sel: bool,
    pub op_sel_hi: bool,
    pub neg_hi: bool,
    pub dpp: bool,
}

impl Opnd {
    pub fn special(kind: OperandKind) -> Self {
        Self {
            kind,
            value: 0,
            signed: 0,
            reg: 0,
            sel: 6,
            unused: 2,
            omod: 0,
            neg: false,
            abs: false,
            clamp: false,
            sext: false,
            op_sel: false,
            op_sel_hi: false,
            neg_hi: false,
            dpp: false,
        }
    }

    /// Source `PlainOperand`.
    pub fn plain(self) -> Self {
        Self {
            kind: self.kind,
            value: self.value,
            signed: self.signed,
            reg: self.reg,
            ..Self::special(self.kind)
        }
    }

    fn from_decoded(op: pentax_shader::Operand) -> Self {
        Self {
            kind: op.kind(),
            value: op.value(),
            signed: op.signed_value(),
            reg: op.register(),
            sel: op.sdwa_select().bits(),
            unused: op.sdwa_unused().bits(),
            omod: op.output_modifier(),
            neg: op.negate(),
            abs: op.absolute(),
            clamp: op.clamp(),
            sext: op.sdwa_sign_extend(),
            op_sel: op.op_sel(),
            op_sel_hi: op.op_sel_hi(),
            neg_hi: op.negate_hi(),
            dpp: op.dpp().is_some(),
        }
    }

    pub fn is_constant(self) -> bool {
        matches!(
            self.kind,
            OperandKind::LiteralConstant
                | OperandKind::IntegerInlineConstant
                | OperandKind::FloatInlineConstant
        )
    }
}

/// The decoded fields one translation step reads.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Decoded {
    pub opcode: Opcode,
    pub family: Family,
    pub pc: u32,
    pub dst: Opnd,
    pub dst2: Opnd,
    pub src: [Opnd; 4],
    pub src_count: usize,
    pub memory: Option<MemoryAccess>,
    pub export: Option<Export>,
}

impl Decoded {
    fn from_instruction(inst: Instruction<'_>) -> Self {
        let encoded = inst.encoded_sources();
        Self {
            opcode: inst.opcode(),
            family: inst.family(),
            pc: inst.byte_offset(),
            dst: Opnd::from_decoded(inst.destination()),
            dst2: Opnd::from_decoded(inst.secondary_destination()),
            src: encoded.map(Opnd::from_decoded),
            src_count: inst.sources().len(),
            memory: inst.memory(),
            export: inst.export(),
        }
    }

    pub fn src(&self, index: usize) -> Opnd {
        self.src
            .get(index)
            .copied()
            .unwrap_or(Opnd::special(OperandKind::Unknown))
    }
}

/// SSA variable numbering (source SsaRewrite `Variable`).
pub(crate) mod var {
    pub const SGPRS: u32 = 106;
    pub const THREAD_BIT: u32 = 106;
    pub const MASK_TAG: u32 = 212;
    pub const SCC: u32 = 318;
    pub const EXEC: u32 = 319;
    pub const EXEC_LO: u32 = 320;
    pub const EXEC_HI: u32 = 321;
    pub const VCC: u32 = 322;
    pub const VCC_LO: u32 = 323;
    pub const VCC_HI: u32 = 324;
    pub const M0: u32 = 325;
    pub const VGPR: u32 = 326;
}

pub(crate) struct Translator<'a> {
    pub e: Emitter<'a>,
    pub profile: &'a Profile,
    pub wave64: bool,
    pub flush_f32_inputs: bool,
    pub inst: Decoded,
    /// Source `instruction_branch_condition` (subvector loops; refused).
    pub branch_condition: Option<Value>,
}

pub(crate) fn u32v(value: u32) -> Value {
    Value::U32(value)
}

impl Translator<'_> {
    pub fn unsupported(&self, reason: UnsupportedReason) -> CompileError {
        CompileError::unsupported(self.inst.pc, Some(self.inst.opcode), reason)
    }

    fn invalid_pair(&self) -> CompileError {
        self.unsupported(UnsupportedReason::NotPorted)
    }

    // ---------------------------------------------------------- operands

    /// Source `OffsetOperand`.
    pub fn offset_operand(&self, op: Opnd, offset: u16) -> Result<Opnd, CompileError> {
        if offset == 0 {
            return Ok(op);
        }
        let mut result = op.plain();
        match result.kind {
            OperandKind::Sgpr => {
                result.reg += offset;
                if result.reg >= 106 {
                    return Err(self.invalid_pair());
                }
            }
            OperandKind::Vgpr => {
                result.reg += offset;
                if result.reg >= 256 {
                    return Err(self.invalid_pair());
                }
            }
            OperandKind::VccLo if offset == 1 => result.kind = OperandKind::VccHi,
            OperandKind::ExecLo if offset == 1 => result.kind = OperandKind::ExecHi,
            OperandKind::VccLo | OperandKind::ExecLo | OperandKind::VccHi | OperandKind::ExecHi => {
                return Err(self.invalid_pair());
            }
            _ => return Ok(op),
        }
        Ok(result)
    }

    pub fn ballot_mask(&mut self, value: Value) -> Result<[Value; 2], CompileError> {
        let mask = self.e.emit(Op::Ballot, &[value])?;
        let low = self.e.extract(mask, 0)?;
        let high = if self.wave64 {
            self.e.extract(mask, 1)?
        } else {
            u32v(0)
        };
        Ok([low, high])
    }

    /// Source `ReadRawU32`.
    pub fn read_raw(&mut self, op: Opnd) -> Result<Value, CompileError> {
        match op.kind {
            OperandKind::LiteralConstant
            | OperandKind::IntegerInlineConstant
            | OperandKind::FloatInlineConstant => Ok(u32v(op.value)),
            OperandKind::Null => Ok(u32v(0)),
            OperandKind::PopsExitingWaveId => Err(self.unsupported(UnsupportedReason::NotPorted)),
            OperandKind::Sgpr => self.e.get(Op::GetScalarRegister, u32::from(op.reg)),
            OperandKind::Vgpr => self.e.get(Op::GetVectorRegister, u32::from(op.reg)),
            OperandKind::VccLo => self.e.get(Op::GetVccLo, 0),
            OperandKind::VccHi => self.e.get(Op::GetVccHi, 0),
            OperandKind::M0 => self.e.get(Op::GetM0, 0),
            OperandKind::ExecLo => self.e.get(Op::GetExecLo, 0),
            OperandKind::ExecHi => self.e.get(Op::GetExecHi, 0),
            OperandKind::Scc => {
                let scc = self.e.get(Op::GetScc, 0)?;
                self.e.select(scc, u32v(1), u32v(0))
            }
            OperandKind::VccZ | OperandKind::ExecZ => {
                let vcc = op.kind == OperandKind::VccZ;
                let mut mask = self
                    .e
                    .get(if vcc { Op::GetVccLo } else { Op::GetExecLo }, 0)?;
                if self.wave64 {
                    let high = self
                        .e
                        .get(if vcc { Op::GetVccHi } else { Op::GetExecHi }, 0)?;
                    mask = self.e.or(mask, high)?;
                }
                let zero = self.e.ieq(mask, u32v(0))?;
                self.e.select(zero, u32v(1), u32v(0))
            }
            OperandKind::Unknown => Err(CompileError::Internal("unknown operand read")),
        }
    }

    /// Source `ReadScalarCode` for descriptor dwords.
    pub fn read_scalar_code(&mut self, code: u32) -> Result<Value, CompileError> {
        if code < 106 {
            return self.e.get(Op::GetScalarRegister, code);
        }
        Err(self.unsupported(UnsupportedReason::DynamicDescriptor))
    }

    /// Source `ApplyBitSourceModifiers` (DPP is refused before dispatch).
    pub fn bit_source_modifiers(&mut self, op: Opnd, value: Value) -> Result<Value, CompileError> {
        if op.sel == 6 {
            return Ok(value);
        }
        let (offset, width) = match op.sel {
            0..=3 => (u32::from(op.sel) * 8, 8),
            4 => (0, 16),
            _ => (16, 16),
        };
        let opcode = if op.sext {
            Op::BitFieldSExtract
        } else {
            Op::BitFieldUExtract
        };
        self.e.emit(opcode, &[value, u32v(offset), u32v(width)])
    }

    /// Source `ReadOperand` for U1, U32, U64 and F32.
    pub fn read(&mut self, op: Opnd, ty: Type) -> Result<Value, CompileError> {
        match ty {
            Type::U1 => match op.kind {
                OperandKind::Scc => self.e.get(Op::GetScc, 0),
                OperandKind::ExecLo | OperandKind::ExecHi => self.e.get(Op::GetExec, 0),
                OperandKind::VccLo | OperandKind::VccHi => self.e.get(Op::GetVcc, 0),
                OperandKind::VccZ => {
                    let vcc = self.e.get(Op::GetVcc, 0)?;
                    self.e.lnot(vcc)
                }
                OperandKind::ExecZ => {
                    let exec = self.e.get(Op::GetExec, 0)?;
                    self.e.lnot(exec)
                }
                _ => {
                    let raw = self.read_raw(op)?;
                    self.e.ine(raw, u32v(0))
                }
            },
            Type::U64 => {
                let [low, high] = self.read_pair(op)?;
                self.e.construct_u64(low, high)
            }
            Type::F32 => {
                let raw = self.read_raw(op)?;
                let bits = self.bit_source_modifiers(op, raw)?;
                let mut value = self.e.bitcast_f32(bits)?;
                if op.abs {
                    value = self.e.emit(Op::FPAbs32, &[value])?;
                }
                if op.neg {
                    value = self.e.emit(Op::FPNeg32, &[value])?;
                }
                Ok(value)
            }
            Type::U32 => {
                let raw = self.read_raw(op)?;
                let mut bits = self.bit_source_modifiers(op, raw)?;
                if op.abs {
                    bits = self.e.and(bits, u32v(0x7fff_ffff))?;
                }
                if op.neg {
                    bits = self.e.xor(bits, u32v(0x8000_0000))?;
                }
                Ok(bits)
            }
            _ => Err(CompileError::Internal("unsupported operand read type")),
        }
    }

    pub fn read_u32(&mut self, op: Opnd) -> Result<Value, CompileError> {
        self.read(op, Type::U32)
    }

    pub fn read_f32(&mut self, op: Opnd) -> Result<Value, CompileError> {
        self.read(op, Type::F32)
    }

    /// Source `ReadU32Pair`.
    pub fn read_pair(&mut self, op: Opnd) -> Result<[Value; 2], CompileError> {
        match op.kind {
            OperandKind::ExecLo => {
                Ok([self.e.get(Op::GetExecLo, 0)?, self.e.get(Op::GetExecHi, 0)?])
            }
            OperandKind::VccLo => Ok([self.e.get(Op::GetVccLo, 0)?, self.e.get(Op::GetVccHi, 0)?]),
            _ => {
                let raw = self.read_raw(op)?;
                let low = self.bit_source_modifiers(op, raw)?;
                let high = match op.kind {
                    OperandKind::Sgpr | OperandKind::Vgpr => {
                        let next = self.offset_operand(op, 1)?;
                        self.read_raw(next)?
                    }
                    OperandKind::IntegerInlineConstant if op.signed < 0 => u32v(0xffff_ffff),
                    _ => u32v(0),
                };
                Ok([low, high])
            }
        }
    }

    /// Source `ThreadBit`.
    pub fn thread_bit(&mut self, mask: [Value; 2]) -> Result<Value, CompileError> {
        let constant = |word: Value| match word {
            Value::U32(0) => Some(false),
            Value::U32(u32::MAX) => Some(true),
            _ => None,
        };
        let low = constant(self.e.ir.resolve(self.e.budget, mask[0])?);
        let high = if self.wave64 {
            constant(self.e.ir.resolve(self.e.budget, mask[1])?)
        } else {
            low
        };
        if let (Some(l), Some(h)) = (low, high)
            && l == h
        {
            return Ok(Value::U1(l));
        }
        let lane = self.e.get(Op::LaneId, 0)?;
        let word = if self.wave64 {
            let low_half = self.e.ult(lane, u32v(32))?;
            self.e.select(low_half, mask[0], mask[1])?
        } else {
            mask[0]
        };
        let bit = self.e.and(lane, u32v(31))?;
        let shifted = self.e.shr(word, bit)?;
        let masked = self.e.and(shifted, u32v(1))?;
        self.e.ine(masked, u32v(0))
    }

    /// Source `WriteRawU32`.
    pub fn write_raw(&mut self, op: Opnd, mut value: Value) -> Result<(), CompileError> {
        if op.kind == OperandKind::Null {
            return Ok(());
        }
        if op.sel != 6 {
            let (offset, width) = match op.sel {
                0..=3 => (u32::from(op.sel) * 8, 8),
                4 => (0, 16),
                _ => (16, 16),
            };
            value = match op.unused {
                0 => self.e.emit(
                    Op::BitFieldInsert,
                    &[u32v(0), value, u32v(offset), u32v(width)],
                )?,
                1 => {
                    let extended = self
                        .e
                        .emit(Op::BitFieldSExtract, &[value, u32v(0), u32v(width)])?;
                    self.e.shl(extended, u32v(offset))?
                }
                _ => {
                    let field = if width == 32 {
                        u32::MAX
                    } else {
                        (1 << width) - 1
                    };
                    let low = self.e.and(value, u32v(field))?;
                    let inserted = self.e.shl(low, u32v(offset))?;
                    let old = self.read_raw(op.plain())?;
                    let cleared = self.e.and(old, u32v(!(field << offset)))?;
                    self.e.or(cleared, inserted)?
                }
            };
        }
        match op.kind {
            OperandKind::Sgpr => {
                let reg = u32::from(op.reg);
                self.e.set(Op::SetScalarRegister, reg, value)?;
                self.e.set(Op::SetScalarMaskTag, reg, Value::U1(false))?;
                if reg > 0 {
                    self.e
                        .set(Op::SetScalarMaskTag, reg - 1, Value::U1(false))?;
                }
            }
            OperandKind::Vgpr => {
                let reg = u32::from(op.reg);
                let old = self.e.get(Op::GetVectorRegister, reg)?;
                let exec = self.e.get(Op::GetExec, 0)?;
                let merged = self.e.select(exec, value, old)?;
                self.e.set(Op::SetVectorRegister, reg, merged)?;
            }
            OperandKind::VccLo => {
                self.e.set(Op::SetVccLo, 0, value)?;
                let high = self.e.get(Op::GetVccHi, 0)?;
                let bit = self.thread_bit([value, high])?;
                self.e.set(Op::SetVcc, 0, bit)?;
            }
            OperandKind::VccHi => {
                self.e.set(Op::SetVccHi, 0, value)?;
                let low = self.e.get(Op::GetVccLo, 0)?;
                let bit = self.thread_bit([low, value])?;
                self.e.set(Op::SetVcc, 0, bit)?;
            }
            OperandKind::M0 => self.e.set(Op::SetM0, 0, value)?,
            OperandKind::ExecLo => {
                self.e.set(Op::SetExecLo, 0, value)?;
                let high = self.e.get(Op::GetExecHi, 0)?;
                let bit = self.thread_bit([value, high])?;
                self.e.set(Op::SetExec, 0, bit)?;
            }
            OperandKind::ExecHi => {
                self.e.set(Op::SetExecHi, 0, value)?;
                let low = self.e.get(Op::GetExecLo, 0)?;
                let bit = self.thread_bit([low, value])?;
                self.e.set(Op::SetExec, 0, bit)?;
            }
            OperandKind::Scc => {
                let bit = self.e.ine(value, u32v(0))?;
                self.e.set(Op::SetScc, 0, bit)?;
            }
            _ => return Err(CompileError::Internal("invalid destination operand")),
        }
        Ok(())
    }

    /// Source `ApplyF32ResultModifiers`.
    pub fn f32_result_modifiers(
        &mut self,
        op: Opnd,
        mut value: Value,
    ) -> Result<Value, CompileError> {
        if op.omod != 0 {
            let multiplier = match op.omod {
                1 => 2.0f32,
                2 => 4.0,
                _ => 0.5,
            };
            value = self.e.emit(Op::FPMul32, &[value, Value::f32(multiplier)])?;
        }
        if op.clamp {
            value = self.e.emit(Op::FPSaturate32, &[value])?;
        }
        Ok(value)
    }

    /// Source `WriteOperand`.
    pub fn write(&mut self, op: Opnd, value: Value) -> Result<(), CompileError> {
        if op.kind == OperandKind::Null {
            return Ok(());
        }
        let ty = self.e.ir.type_of(self.e.budget, value)?;
        if ty != Type::F32 && (op.clamp || op.omod != 0) {
            // The source drops clamp/omod on integer results; hardware saturates.
            return Err(self.unsupported(UnsupportedReason::IntegerResultModifier));
        }
        match ty {
            Type::F32 => {
                let value = self.f32_result_modifiers(op, value)?;
                let bits = self.e.bitcast_u32(value)?;
                self.write_raw(op, bits)
            }
            Type::U1 => match op.kind {
                OperandKind::Scc => self.e.set(Op::SetScc, 0, value),
                OperandKind::ExecLo | OperandKind::ExecHi => {
                    let mask = self.ballot_mask(value)?;
                    self.e.set(Op::SetExec, 0, value)?;
                    self.e.set(Op::SetExecLo, 0, mask[0])?;
                    self.e.set(Op::SetExecHi, 0, mask[1])
                }
                OperandKind::VccLo | OperandKind::VccHi => {
                    let mask = self.ballot_mask(value)?;
                    self.e.set(Op::SetVcc, 0, value)?;
                    self.e.set(Op::SetVccLo, 0, mask[0])?;
                    self.e.set(Op::SetVccHi, 0, mask[1])
                }
                _ => {
                    let word = self.e.select(value, u32v(1), u32v(0))?;
                    self.write_raw(op, word)
                }
            },
            Type::U64 => {
                let low = self.e.extract(value, 0)?;
                let high = self.e.extract(value, 1)?;
                self.write_pair(op, [low, high])
            }
            Type::U32 => self.write_raw(op, value),
            _ => Err(CompileError::Internal("untyped result write")),
        }
    }

    /// Source `WriteU32Pair`.
    pub fn write_pair(&mut self, op: Opnd, value: [Value; 2]) -> Result<(), CompileError> {
        match op.kind {
            OperandKind::Null => Ok(()),
            OperandKind::ExecLo => {
                let bit = self.thread_bit(value)?;
                self.e.set(Op::SetExec, 0, bit)?;
                self.e.set(Op::SetExecLo, 0, value[0])?;
                self.e.set(Op::SetExecHi, 0, value[1])
            }
            OperandKind::VccLo => {
                let bit = self.thread_bit(value)?;
                self.e.set(Op::SetVcc, 0, bit)?;
                self.e.set(Op::SetVccLo, 0, value[0])?;
                self.e.set(Op::SetVccHi, 0, value[1])
            }
            _ => {
                self.write_raw(op, value[0])?;
                let high = self.offset_operand(op, 1)?;
                self.write_raw(high, value[1])
            }
        }
    }

    /// Source `ConditionBit`.
    pub fn condition_bit(&mut self, op: Opnd) -> Result<Value, CompileError> {
        let mask = self.read_mask(op)?;
        self.e.select(mask, u32v(1), u32v(0))
    }

    /// Source `ReadMask`.
    pub fn read_mask(&mut self, op: Opnd) -> Result<Value, CompileError> {
        if op.is_constant() {
            let pair = self.read_pair(op)?;
            return self.thread_bit(pair);
        }
        match op.kind {
            OperandKind::Sgpr => {
                let reg = u32::from(op.reg);
                let mask = if self.wave64 {
                    self.read_pair(op)?
                } else {
                    [self.read_raw(op)?, u32v(0)]
                };
                let tag = self.e.get(Op::GetScalarMaskTag, reg)?;
                let bit = self.e.get(Op::GetThreadBitScalarRegister, reg)?;
                let fallback = self.thread_bit(mask)?;
                self.e.emit(Op::SelectU1, &[tag, bit, fallback])
            }
            OperandKind::ExecLo | OperandKind::ExecHi => self.e.get(Op::GetExec, 0),
            OperandKind::VccLo | OperandKind::VccHi => {
                if self.wave64 {
                    self.e.get(Op::GetVcc, 0)
                } else {
                    let raw = self.read_raw(op)?;
                    self.thread_bit([raw, u32v(0)])
                }
            }
            OperandKind::Scc => self.e.get(Op::GetScc, 0),
            OperandKind::VccZ => {
                let vcc = self.e.get(Op::GetVcc, 0)?;
                self.e.lnot(vcc)
            }
            OperandKind::ExecZ => {
                let exec = self.e.get(Op::GetExec, 0)?;
                self.e.lnot(exec)
            }
            _ => {
                let raw = self.read_raw(op)?;
                self.e.ine(raw, u32v(0))
            }
        }
    }

    /// Source `ReadMaskValid`.
    pub fn read_mask_valid(&mut self, op: Opnd) -> Result<Value, CompileError> {
        if op.is_constant() {
            let zero = op.value == 0;
            let ones = op.value == u32::MAX
                && op.kind == OperandKind::IntegerInlineConstant
                && op.signed < 0;
            return Ok(Value::U1(zero || ones));
        }
        match op.kind {
            OperandKind::Null
            | OperandKind::ExecLo
            | OperandKind::ExecHi
            | OperandKind::VccLo
            | OperandKind::VccHi
            | OperandKind::VccZ
            | OperandKind::ExecZ
            | OperandKind::Scc => Ok(Value::U1(true)),
            OperandKind::Sgpr => self.e.get(Op::GetScalarMaskTag, u32::from(op.reg)),
            _ => Ok(Value::U1(false)),
        }
    }

    /// Source `WriteMask`.
    pub fn write_mask(
        &mut self,
        op: Opnd,
        value: Value,
        write_64: bool,
    ) -> Result<[Value; 2], CompileError> {
        let mask = self.ballot_mask(value)?;
        match op.kind {
            OperandKind::Sgpr => {
                let reg = u32::from(op.reg);
                self.e.set(Op::SetThreadBitScalarRegister, reg, value)?;
                self.e.set(Op::SetScalarMaskTag, reg, Value::U1(true))?;
                if reg > 0 {
                    self.e
                        .set(Op::SetScalarMaskTag, reg - 1, Value::U1(false))?;
                }
                self.e.set(Op::SetScalarRegister, reg, mask[0])?;
                if (write_64 || self.wave64) && reg + 1 < 106 {
                    self.e.set(Op::SetScalarRegister, reg + 1, mask[1])?;
                    self.e
                        .set(Op::SetThreadBitScalarRegister, reg + 1, Value::U1(false))?;
                    self.e
                        .set(Op::SetScalarMaskTag, reg + 1, Value::U1(false))?;
                }
            }
            OperandKind::ExecLo | OperandKind::ExecHi => {
                self.e.set(Op::SetExec, 0, value)?;
                self.e.set(Op::SetExecLo, 0, mask[0])?;
                self.e.set(Op::SetExecHi, 0, mask[1])?;
            }
            OperandKind::VccLo | OperandKind::VccHi => {
                if !write_64 && !self.wave64 {
                    self.write_raw(op, mask[0])?;
                    return Ok(mask);
                }
                self.e.set(Op::SetVcc, 0, value)?;
                self.e.set(Op::SetVccLo, 0, mask[0])?;
                self.e.set(Op::SetVccHi, 0, mask[1])?;
            }
            OperandKind::Scc => self.e.set(Op::SetScc, 0, value)?,
            _ => {
                self.write_raw(op, mask[0])?;
                let high = self.offset_operand(op, 1)?;
                self.write_raw(high, mask[1])?;
            }
        }
        Ok(mask)
    }

    /// Source `WriteCompareResult`.
    pub fn write_compare_result(&mut self, op: Opnd, value: Value) -> Result<(), CompileError> {
        if op.kind == OperandKind::Scc {
            return self.write(op, value);
        }
        let exec = self.e.get(Op::GetExec, 0)?;
        let masked = self.e.land(exec, value)?;
        self.write_mask(op, masked, false).map(|_| ())
    }

    /// Source `EmitCompareResult`.
    pub fn compare_result(
        &mut self,
        value: Value,
        scalar: bool,
        cmpx: bool,
    ) -> Result<(), CompileError> {
        if scalar {
            return self.e.set(Op::SetScc, 0, value);
        }
        let exec = self.e.get(Op::GetExec, 0)?;
        let masked = self.e.land(exec, value)?;
        if cmpx {
            let mask = self.ballot_mask(masked)?;
            self.e.set(Op::SetExec, 0, masked)?;
            self.e.set(Op::SetExecLo, 0, mask[0])?;
            return self.e.set(Op::SetExecHi, 0, mask[1]);
        }
        let dst = self.inst.dst;
        self.write_mask(dst, masked, false).map(|_| ())
    }

    // ------------------------------------------------------------ dispatch

    /// Source `Translator::TranslateInstruction`.
    fn translate(&mut self) -> Result<(), CompileError> {
        let inst = self.inst;
        for op in inst.src.iter().take(inst.src_count).chain([&inst.dst]) {
            if op.dpp {
                return Err(self.unsupported(UnsupportedReason::DppModifier));
            }
        }
        match inst.family {
            Family::Sop1 | Family::Sop2 | Family::Sopk | Family::Sopc | Family::Sopp => {
                self.emit_scalar()
            }
            Family::Vop1 | Family::Vop2 | Family::Vop3 | Family::Vop3p | Family::Vopc => {
                self.emit_vector()
            }
            Family::Smem
            | Family::Mubuf
            | Family::Mtbuf
            | Family::Flat
            | Family::Ds
            | Family::Mimg => self.emit_memory(),
            Family::Vintrp => self.emit_interpolation(),
            Family::Exp => self.export(),
            Family::Unknown => Err(self.unsupported(UnsupportedReason::NoSourceTranslation)),
        }
    }

    /// Source `AddBranchCondition`.
    fn branch_condition(&mut self, term: &IrTerminator) -> Result<Option<Value>, CompileError> {
        if term.kind != TerminatorKind::ConditionalBranch {
            return Ok(None);
        }
        let any = |t: &mut Self, op: Op| -> Result<Value, CompileError> {
            let value = t.e.get(op, 0)?;
            t.e.emit(Op::AnyLane, &[value])
        };
        let condition = match term.condition {
            BranchCondition::Always => Value::U1(true),
            BranchCondition::SccZero => {
                let scc = self.e.get(Op::GetScc, 0)?;
                self.e.lnot(scc)?
            }
            BranchCondition::SccNonZero => self.e.get(Op::GetScc, 0)?,
            BranchCondition::VccZero => {
                let anyv = any(self, Op::GetVcc)?;
                self.e.lnot(anyv)?
            }
            BranchCondition::VccNonZero => any(self, Op::GetVcc)?,
            BranchCondition::ExecZero => {
                let anye = any(self, Op::GetExec)?;
                self.e.lnot(anye)?
            }
            BranchCondition::ExecNonZero => any(self, Op::GetExec)?,
            BranchCondition::ScalarInstruction => self
                .branch_condition
                .ok_or(self.unsupported(UnsupportedReason::SubvectorLoop))?,
        };
        Ok(Some(condition))
    }
}

/// Source `Frontend::TranslateProgram`: one typed entry block, then every CFG
/// block in order, then the per-block branch conditions.
pub(crate) fn translate_program(
    cfg: &Cfg<'_>,
    profile: &Profile,
    budget: &mut Budget,
) -> Result<Ir, CompileError> {
    let program = cfg.program();
    let stage = program.stage();
    let mut ir = Ir::new();
    let limits = budget.limits;
    let block_count = cfg
        .block_count()
        .checked_add(1)
        .ok_or(CompileError::limit(crate::LimitKind::Blocks))?;
    if block_count > limits.max_blocks {
        return Err(CompileError::limit(crate::LimitKind::Blocks));
    }
    if cfg.entry() != 0 {
        return Err(CompileError::Internal(
            "structured entry is not the first block",
        ));
    }
    let shift = |id: u32| if id == NONE { NIL } else { id + 1 };
    budget.reserve(
        &mut ir.blocks,
        block_count,
        limits.max_blocks,
        crate::LimitKind::Blocks,
    )?;
    let first_pc = cfg.block(0).map_or(0, |b| b.start_pc());
    ir.blocks.push(IrBlock {
        first: NIL,
        last: NIL,
        pred_start: 0,
        pred_len: 0,
        start_pc: first_pc,
        term: IrTerminator {
            kind: TerminatorKind::Branch,
            condition: BranchCondition::Always,
            true_block: 1,
            false_block: NIL,
            merge_block: NIL,
            continue_block: NIL,
            loop_header: false,
        },
        condition: None,
    });
    for block in cfg.blocks() {
        budget.charge(1)?;
        ir.blocks.push(IrBlock {
            first: NIL,
            last: NIL,
            pred_start: 0,
            pred_len: 0,
            start_pc: block.start_pc(),
            term: IrTerminator {
                kind: block.terminator(),
                condition: block.condition(),
                true_block: shift(block.true_target().unwrap_or(NONE)),
                false_block: shift(block.false_target().unwrap_or(NONE)),
                merge_block: shift(block.merge().unwrap_or(NONE)),
                continue_block: shift(block.continue_target().unwrap_or(NONE)),
                loop_header: block.is_loop_header(),
            },
            condition: None,
        });
    }
    // Predecessors: the typed entry precedes CFG block 0; others shift by one.
    let mut total = 1usize;
    for block in cfg.blocks() {
        total += block.predecessors().len();
    }
    budget.reserve(
        &mut ir.preds,
        total,
        limits.max_edges,
        crate::LimitKind::Edges,
    )?;
    budget.charge_usize(total)?;
    for (index, block) in cfg.blocks().enumerate() {
        let start = ir.preds.len() as u32;
        if index == 0 {
            ir.preds.push(0);
        }
        for &pred in block.predecessors() {
            ir.preds.push(pred + 1);
        }
        let entry = &mut ir.blocks[index + 1];
        entry.pred_start = start;
        entry.pred_len = ir.preds.len() as u32 - start;
    }

    let wave64 = profile.wave64();
    let flush_f32_inputs = profile
        .compute()
        .is_some_and(|compute| compute.float_mode & 0x10 == 0);
    entry_block(&mut ir, budget, profile, stage, first_pc)?;

    for (index, block) in cfg.blocks().enumerate() {
        let ir_block = index as u32 + 1;
        let mut t = Translator {
            e: Emitter {
                ir: &mut ir,
                budget,
                block: ir_block,
                pc: block.start_pc(),
            },
            profile,
            wave64,
            flush_f32_inputs,
            inst: Decoded {
                opcode: Opcode::SNop,
                family: Family::Sopp,
                pc: block.start_pc(),
                dst: Opnd::special(OperandKind::Unknown),
                dst2: Opnd::special(OperandKind::Unknown),
                src: [Opnd::special(OperandKind::Unknown); 4],
                src_count: 0,
                memory: None,
                export: None,
            },
            branch_condition: None,
        };
        for inst_index in block.instructions() {
            let inst = program
                .instruction(inst_index)
                .ok_or(CompileError::Internal("CFG instruction index"))?;
            t.inst = Decoded::from_instruction(inst);
            t.e.pc = t.inst.pc;
            t.translate()?;
        }
        let term = t.e.ir.blocks[ir_block as usize].term;
        let condition = t.branch_condition(&term)?;
        t.e.ir.blocks[ir_block as usize].condition = condition;
    }
    Ok(ir)
}

fn entry_block(
    ir: &mut Ir,
    budget: &mut Budget,
    profile: &Profile,
    stage: ShaderStage,
    pc: u32,
) -> Result<(), CompileError> {
    let mut e = Emitter {
        ir,
        budget,
        block: 0,
        pc,
    };
    let common = profile.common();
    // These are compiler cache metadata, not guest register values. Every
    // ordinary/system SGPR starts with no cached lane-mask interpretation.
    // Initialize the cached bit too: a later phi may join a tagged-mask path
    // with an ordinary-value path whose false tag selects the raw fallback.
    for reg in 0..var::SGPRS {
        e.set(Op::SetScalarMaskTag, reg, Value::U1(false))?;
        e.set(Op::SetThreadBitScalarRegister, reg, Value::U1(false))?;
    }
    for index in 0..u32::from(common.user_sgpr_count) {
        let reg = u32::from(common.user_sgpr_base) + index;
        let value = e.get(Op::GetUserData, reg)?;
        e.set(Op::SetScalarRegister, reg, value)?;
    }
    // Source initial EXEC: every invocation (no live/helper or dispatch-extent gating).
    let initial_exec = Value::U1(true);
    e.set(Op::SetExec, 0, initial_exec)?;
    let mask = e.emit(Op::Ballot, &[initial_exec])?;
    let low = e.extract(mask, 0)?;
    e.set(Op::SetExecLo, 0, low)?;
    let high = if profile.wave64() {
        e.extract(mask, 1)?
    } else {
        u32v(0)
    };
    e.set(Op::SetExecHi, 0, high)?;
    let builtin = |e: &mut Emitter<'_>, kind: Builtin, component: u32| {
        e.get(Op::GetBuiltin, kind.aux(component))
    };
    match stage {
        ShaderStage::Compute => {
            let cs = profile
                .compute()
                .ok_or(CompileError::Internal("compute profile"))?;
            for index in 0..u32::from(cs.thread_ids.min(3)) {
                let id = builtin(&mut e, Builtin::LocalInvocationId, index)?;
                e.set(Op::SetVectorRegister, index, id)?;
            }
            let mut reg = u32::from(cs.workgroup_register);
            for index in 0..3u32 {
                if cs.group_id[index as usize] {
                    let id = builtin(&mut e, Builtin::WorkgroupId, index)?;
                    e.set(Op::SetScalarRegister, reg, id)?;
                    reg += 1;
                }
            }
            if cs.tg_size {
                let [x, y, z] = cs.workgroup_size;
                let total = x.max(1) * y.max(1) * z.max(1);
                let wave = profile.common().wave_size.lanes();
                let waves = total.div_ceil(wave).min(0x3f);
                let local = builtin(&mut e, Builtin::LocalInvocationIndex, 0)?;
                let wave_id = e.emit(Op::UDiv32, &[local, u32v(wave)])?;
                let wave_bits = e.shl(wave_id, u32v(20))?;
                let first = e.ieq(wave_id, u32v(0))?;
                let first_bit = e.select(first, u32v(0x8000_0000), u32v(0))?;
                let packed = e.or(wave_bits, u32v(waves))?;
                let value = e.or(packed, first_bit)?;
                e.set(Op::SetScalarRegister, reg, value)?;
            }
        }
        ShaderStage::Pixel => {
            let ps = profile
                .pixel()
                .ok_or(CompileError::Internal("pixel profile"))?;
            let pairs = [
                (ps.perspective_center_vgpr, Builtin::BaryCoordSmooth),
                (
                    ps.perspective_centroid_vgpr,
                    Builtin::BaryCoordSmoothCentroid,
                ),
                (ps.perspective_sample_vgpr, Builtin::BaryCoordSmoothSample),
                (ps.linear_sample_vgpr, Builtin::BaryCoordNoPerspectiveSample),
                (ps.linear_center_vgpr, Builtin::BaryCoordNoPerspective),
                (
                    ps.linear_centroid_vgpr,
                    Builtin::BaryCoordNoPerspectiveCentroid,
                ),
            ];
            for (reg, kind) in pairs {
                if let Some(reg) = reg {
                    let i = builtin(&mut e, kind, 0)?;
                    e.set(Op::SetVectorRegister, u32::from(reg), i)?;
                    let j = builtin(&mut e, kind, 1)?;
                    e.set(Op::SetVectorRegister, u32::from(reg) + 1, j)?;
                }
            }
            let mut reg = u32::from(ps.system_input_base);
            for component in 0..3u32 {
                if ps.position[component as usize] {
                    let value = builtin(&mut e, Builtin::FragCoord, component)?;
                    e.set(Op::SetVectorRegister, reg, value)?;
                    reg += 1;
                }
            }
            if ps.position[3] {
                let raw = builtin(&mut e, Builtin::FragCoord, 3)?;
                let reciprocal = e.bitcast_f32(raw)?;
                let w = e.emit(Op::FPRecip32, &[reciprocal])?;
                let bits = e.bitcast_u32(w)?;
                e.set(Op::SetVectorRegister, reg, bits)?;
                reg += 1;
            }
            if ps.front_face {
                let value = builtin(&mut e, Builtin::FrontFacing, 0)?;
                e.set(Op::SetVectorRegister, reg, value)?;
            }
        }
        ShaderStage::Vertex => {
            let wave = profile.common().wave_size.lanes();
            e.set(Op::SetScalarRegister, 2, u32v(wave << 12))?;
            e.set(Op::SetScalarRegister, 3, u32v((1 << 28) | wave))?;
            let vertex = builtin(&mut e, Builtin::VertexIndex, 0)?;
            e.set(Op::SetVectorRegister, 5, vertex)?;
            let base = builtin(&mut e, Builtin::BaseInstance, 0)?;
            let instance = builtin(&mut e, Builtin::InstanceIndex, 0)?;
            let relative = e.isub(instance, base)?;
            e.set(Op::SetVectorRegister, 8, relative)?;
            if let Some(reg) = profile.vertex().and_then(|v| v.start_instance_sgpr) {
                e.set(Op::SetScalarRegister, u32::from(reg), base)?;
            }
        }
        _ => {
            return Err(CompileError::unsupported(
                pc,
                None,
                UnsupportedReason::StageNotPorted,
            ));
        }
    }
    Ok(())
}
