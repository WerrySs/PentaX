// SPDX-License-Identifier: GPL-2.0-only
// Adapted from KytyPS5-Performance at 209dffe9032d72749257ce18680d506b891284de:
// ir/{ShaderIR.h,Value.{h,cpp},Block.{h,cpp},Type.{h,cpp},IREmitter.{h,cpp}}
// and ir/opcodes/ValueOpcodes.inc (the subset of operations produced by the
// ported translators, with the source signatures).
// PentaX modifications: one flat instruction arena with typed ids and an
// intrusive block order instead of heap nodes and use lists; Identity
// forwarding replaces ReplaceUsesWith; register immediates and metadata
// indices live in a typed `aux` word; every growth is budgeted.

use crate::CompileError;
use crate::budget::{Budget, LimitKind};

/// An instruction id minted only by `Ir::push`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct InstId(u32);

impl InstId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Operation result and operand types (source `IR::Type`, ported subset).
/// 8/16-bit values are not produced: the translators refuse those widths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Type {
    Void,
    U1,
    U32,
    U64,
    F32,
    U32x2,
    U32x3,
    U32x4,
    F32x2,
    F32x4,
    Resource,
}

/// A typed SSA value: an instruction result or an immediate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Value {
    Inst(InstId),
    U1(bool),
    U32(u32),
    U64(u64),
    /// IEEE-754 bit pattern.
    F32(u32),
}

impl Value {
    pub fn f32(value: f32) -> Self {
        Self::F32(value.to_bits())
    }
}

/// Placeholder for a phi operand not yet filled by SSA construction.
pub(crate) const PENDING: Value = Value::Inst(InstId(u32::MAX));

macro_rules! ops {
    ($($(#[$attr:meta])* $name:ident : $result:ident ($($arg:ident),*);)*) => {
        /// Ported value operations (source `ValueOpcode`).
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub(crate) enum Op { $($(#[$attr])* $name),* }

        impl Op {
    pub fn is_image(self) -> bool {
        matches!(self, Self::ImageSampleImplicit | Self::ImageSampleLod | Self::ImageQueryLod
            | Self::ImageSampleImplicitFeedback | Self::ImageSampleLodFeedback)
    }

            pub fn result(self) -> Type {
                match self { $(Op::$name => Type::$result),* }
            }
            pub fn args(self) -> &'static [Type] {
                match self { $(Op::$name => &[$(Type::$arg),*]),* }
            }
        }
    };
}

// Phi, Identity and Undef take their type from the instruction. `aux` holds
// the source flags or immediate register/table operand of an operation.
ops! {
    Phi: Void();
    Identity: Void();
    Undef: Void();
    GetUserData: U32();
    GetBuiltin: U32();
    GetScalarRegister: U32();
    SetScalarRegister: Void(U32);
    GetThreadBitScalarRegister: U1();
    SetThreadBitScalarRegister: Void(U1);
    GetScalarMaskTag: U1();
    SetScalarMaskTag: Void(U1);
    GetVectorRegister: U32();
    SetVectorRegister: Void(U32);
    GetScc: U1();
    SetScc: Void(U1);
    GetExec: U1();
    SetExec: Void(U1);
    GetExecLo: U32();
    SetExecLo: Void(U32);
    GetExecHi: U32();
    SetExecHi: Void(U32);
    GetVcc: U1();
    SetVcc: Void(U1);
    GetVccLo: U32();
    SetVccLo: Void(U32);
    GetVccHi: U32();
    SetVccHi: Void(U32);
    GetM0: U32();
    SetM0: Void(U32);
    BitCastU32F32: U32(F32);
    BitCastF32U32: F32(U32);
    ConvertS32F32: U32(F32);
    ConvertU32F32: U32(F32);
    ConvertF32S32: F32(U32);
    ConvertF32U32: F32(U32);
    CompositeConstructU64: U64(U32, U32);
    CompositeConstructU32x2: U32x2(U32, U32);
    CompositeConstructU32x3: U32x3(U32, U32, U32);
    CompositeConstructU32x4: U32x4(U32, U32, U32, U32);
    CompositeConstructF32x2: F32x2(F32, F32);
    CompositeExtractF32x4: F32(F32x4);
    CompositeExtractF32x2: F32(F32x2);
    CompositeExtractU64: U32(U64);
    CompositeExtractU32x2: U32(U32x2);
    CompositeExtractU32x3: U32(U32x3);
    CompositeExtractU32x4: U32(U32x4);
    #[expect(dead_code, reason = "Source emitter operation retained for the future 16-bit frontend")]
    PackHalf2x16: U32(F32x2);
    PackSnorm2x16: U32(F32x2);
    PackUnorm2x16: U32(F32x2);
    PackFloat2x16Rtz: U32(F32, F32);
    FPAbs32: F32(F32);
    FPNeg32: F32(F32);
    FPSaturate32: F32(F32);
    BitFieldInsert: U32(U32, U32, U32, U32);
    BitFieldUExtract: U32(U32, U32, U32);
    BitFieldSExtract: U32(U32, U32, U32);
    SelectU1: U1(U1, U1, U1);
    SelectU32: U32(U1, U32, U32);
    SelectF32: F32(U1, F32, F32);
    IAdd32: U32(U32, U32);
    IAdd64: U64(U64, U64);
    IAddCarry32: U32x2(U32, U32);
    ISub32: U32(U32, U32);
    ISub64: U64(U64, U64);
    IMul32: U32(U32, U32);
    SMulHi: U32(U32, U32);
    UMulHi: U32(U32, U32);
    UDiv32: U32(U32, U32);
    IAbs32: U32(U32);
    ShiftLeftLogical32: U32(U32, U32);
    ShiftLeftLogical64: U64(U64, U32);
    ShiftRightLogical32: U32(U32, U32);
    ShiftRightLogical64: U64(U64, U32);
    ShiftRightArithmetic32: U32(U32, U32);
    ShiftRightArithmetic64: U64(U64, U32);
    BitwiseAnd32: U32(U32, U32);
    BitwiseAnd64: U64(U64, U64);
    BitwiseOr32: U32(U32, U32);
    BitwiseXor32: U32(U32, U32);
    BitwiseNot32: U32(U32);
    BitReverse32: U32(U32);
    BitCount32: U32(U32);
    #[expect(dead_code, reason = "Source emitter operation retained; current frontend counts two 32-bit words")]
    BitCount64: U32(U64);
    FindUMsb32: U32(U32);
    FindUMsb64: U32(U64);
    FindILsb32: U32(U32);
    SMin32: U32(U32, U32);
    UMin32: U32(U32, U32);
    SMax32: U32(U32, U32);
    UMax32: U32(U32, U32);
    SMinTri32: U32(U32, U32, U32);
    UMinTri32: U32(U32, U32, U32);
    SMaxTri32: U32(U32, U32, U32);
    UMaxTri32: U32(U32, U32, U32);
    SMedTri32: U32(U32, U32, U32);
    UMedTri32: U32(U32, U32, U32);
    SLessThan32: U1(U32, U32);
    SLessThan64: U1(U64, U64);
    SLessThanEqual64: U1(U64, U64);
    ULessThanEqual64: U1(U64, U64);
    UGreaterThanEqual64: U1(U64, U64);
    ULessThan32: U1(U32, U32);
    ULessThan64: U1(U64, U64);
    IEqual32: U1(U32, U32);
    IEqual64: U1(U64, U64);
    SLessThanEqual32: U1(U32, U32);
    ULessThanEqual32: U1(U32, U32);
    SGreaterThan32: U1(U32, U32);
    UGreaterThan32: U1(U32, U32);
    UGreaterThan64: U1(U64, U64);
    INotEqual32: U1(U32, U32);
    INotEqual64: U1(U64, U64);
    SGreaterThanEqual32: U1(U32, U32);
    UGreaterThanEqual32: U1(U32, U32);
    LogicalOr: U1(U1, U1);
    LogicalAnd: U1(U1, U1);
    LogicalXor: U1(U1, U1);
    LogicalNot: U1(U1);
    FPOrdEqual32: U1(F32, F32);
    FPUnordEqual32: U1(F32, F32);
    FPOrdNotEqual32: U1(F32, F32);
    FPUnordNotEqual32: U1(F32, F32);
    FPOrdLessThan32: U1(F32, F32);
    FPUnordLessThan32: U1(F32, F32);
    FPOrdGreaterThan32: U1(F32, F32);
    FPUnordGreaterThan32: U1(F32, F32);
    FPOrdLessThanEqual32: U1(F32, F32);
    FPUnordLessThanEqual32: U1(F32, F32);
    FPOrdGreaterThanEqual32: U1(F32, F32);
    FPUnordGreaterThanEqual32: U1(F32, F32);
    FPIsNan32: U1(F32);
    FPCmpClass32: U1(F32, U32);
    FPAdd32: F32(F32, F32);
    FPSub32: F32(F32, F32);
    FPFma32: F32(F32, F32, F32);
    FPMad32: F32(F32, F32, F32);
    FPMul32: F32(F32, F32);
    FPMin32: F32(F32, F32);
    FPMax32: F32(F32, F32);
    FPMinTri32: F32(F32, F32, F32);
    FPMaxTri32: F32(F32, F32, F32);
    FPMedTri32: F32(F32, F32, F32);
    FPRecip32: F32(F32);
    FPRecipIFlag32: F32(F32);
    FPRecipSqrt32: F32(F32);
    FPSqrt: F32(F32);
    FPSin: F32(F32);
    FPCos: F32(F32);
    FPExp2: F32(F32);
    FPLog2: F32(F32);
    FPLdexp: F32(F32, U32);
    FPRoundEven32: F32(F32);
    FPFloor32: F32(F32);
    FPCeil32: F32(F32);
    FPTrunc32: F32(F32);
    FPFract32: F32(F32);
    LaneId: U32();
    Ballot: U32x4(U1);
    AnyLane: U1(U1);
    GetBufferResource: Resource(U32, U32, U32, U32);
    GetImageResource: Resource(U32, U32, U32, U32, U32, U32, U32, U32);
    GetSamplerResource: Resource(U32, U32, U32, U32);
    ImageSampleImplicit: F32x4(Resource, Resource, F32x2);
    ImageSampleLod: F32x4(Resource, Resource, F32x2, F32);
    ImageSampleImplicitFeedback: F32x4(Resource, Resource, F32x2, U1);
    ImageSampleLodFeedback: F32x4(Resource, Resource, F32x2, F32, U1);
    ImageQueryLod: F32x2(Resource, Resource, F32x2);
    ReadConstBuffer: U32(Resource, U32);
    LoadSharedU32: U32(U32, U1);
    StoreSharedU32: Void(U32, U32, U1);
    SharedCompletion: Void();
    LoadBufferU32: U32(Resource, U32, U32, U32, U1);
    LoadBufferU32x2: U32x2(Resource, U32, U32, U32, U1);
    LoadBufferU32x3: U32x3(Resource, U32, U32, U32, U1);
    LoadBufferU32x4: U32x4(Resource, U32, U32, U32, U1);
    StoreBufferU32: Void(Resource, U32, U32, U32, U32, U1);
    StoreBufferU32x2: Void(Resource, U32, U32, U32, U32x2, U1);
    StoreBufferU32x3: Void(Resource, U32, U32, U32, U32x3, U1);
    StoreBufferU32x4: Void(Resource, U32, U32, U32, U32x4, U1);
    GetAttribute: U32();
    InterpolationP1: U32(U32);
    GetAttributeWithBary: U32(U32, U32);
    GetInterpolationParameter: U32();
    SetAttribute: Void(U32x4, U1);
    ControlNop: Void();
    StoreCompletion: Void();
    Barrier: Void();
}

impl Op {
    pub fn has_lod_feedback(self) -> bool {
        matches!(
            self,
            Self::ImageSampleImplicitFeedback | Self::ImageSampleLodFeedback
        )
    }

    /// Source `HasSideEffects` for the ported operations. Register writes are
    /// only meaningful before SSA construction, where DCE does not run.
    pub fn has_side_effects(self) -> bool {
        matches!(
            self,
            Op::ImageSampleImplicitFeedback
                | Op::ImageSampleLodFeedback
                | Op::StoreBufferU32
                | Op::StoreBufferU32x2
                | Op::StoreBufferU32x3
                | Op::StoreBufferU32x4
                | Op::SetAttribute
                | Op::StoreSharedU32
                | Op::SharedCompletion
                | Op::StoreCompletion
                | Op::Barrier
        )
    }

    pub fn is_buffer_load(self) -> bool {
        matches!(
            self,
            Op::ReadConstBuffer
                | Op::LoadBufferU32
                | Op::LoadBufferU32x2
                | Op::LoadBufferU32x3
                | Op::LoadBufferU32x4
        )
    }

    pub fn is_buffer_store(self) -> bool {
        matches!(
            self,
            Op::StoreBufferU32 | Op::StoreBufferU32x2 | Op::StoreBufferU32x3 | Op::StoreBufferU32x4
        )
    }
}

/// Source `StageInputKind` (ported subset).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Builtin {
    VertexIndex = 0,
    InstanceIndex = 1,
    BaseInstance = 2,
    FragCoord = 3,
    FrontFacing = 4,
    BaryCoordSmooth = 5,
    BaryCoordSmoothCentroid = 6,
    WorkgroupId = 7,
    LocalInvocationId = 8,
    LocalInvocationIndex = 9,
    BaryCoordNoPerspective = 10,
    BaryCoordNoPerspectiveCentroid = 11,
    BaryCoordSmoothSample = 12,
    BaryCoordNoPerspectiveSample = 13,
}

impl Builtin {
    pub fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            0 => Self::VertexIndex,
            1 => Self::InstanceIndex,
            2 => Self::BaseInstance,
            3 => Self::FragCoord,
            4 => Self::FrontFacing,
            5 => Self::BaryCoordSmooth,
            6 => Self::BaryCoordSmoothCentroid,
            7 => Self::WorkgroupId,
            8 => Self::LocalInvocationId,
            9 => Self::LocalInvocationIndex,
            10 => Self::BaryCoordNoPerspective,
            11 => Self::BaryCoordNoPerspectiveCentroid,
            12 => Self::BaryCoordSmoothSample,
            13 => Self::BaryCoordNoPerspectiveSample,
            _ => return None,
        })
    }

    pub fn aux(self, component: u32) -> u32 {
        (self as u32) << 8 | component
    }
}

pub(crate) const NIL: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Inst {
    pub op: Op,
    pub ty: Type,
    pub aux: u32,
    pub pc: u32,
    pub args: u32,
    pub nargs: u32,
    pub block: u32,
    pub prev: u32,
    pub next: u32,
    pub alive: bool,
}

/// Source `CFG::Terminator` translated to IR block indices.
#[derive(Clone, Copy, Debug)]
pub(crate) struct IrTerminator {
    pub kind: crate::cfg::TerminatorKind,
    pub condition: crate::cfg::BranchCondition,
    pub true_block: u32,
    pub false_block: u32,
    pub merge_block: u32,
    pub continue_block: u32,
    pub loop_header: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct IrBlock {
    pub first: u32,
    pub last: u32,
    pub pred_start: u32,
    pub pred_len: u32,
    pub start_pc: u32,
    pub term: IrTerminator,
    /// Source `BlockInfo::condition`, set by AddBranchCondition.
    pub condition: Option<Value>,
}

/// Source `ExportTargetKind` and `ExportInfo`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExportKind {
    Null,
    Position,
    Parameter,
    Mrt,
    MrtZ,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ExportInfo {
    pub kind: ExportKind,
    pub index: u32,
    pub enabled: u8,
    #[expect(
        dead_code,
        reason = "Retained decoded export provenance; program termination is checked separately"
    )]
    pub done: bool,
    pub valid_mask: bool,
    pub pc: u32,
}

/// Source `MemoryInfo` fields used by the ported buffer operations.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MemoryInfo {
    pub scalar: bool,
    pub offset: u32,
    pub data_dwords: u32,
    pub coherent: bool,
    pub idxen: bool,
    pub offen: bool,
    /// Dense buffer resource index, written by resource tracking.
    pub resource: u32,
}

/// The typed program of one compilation (source `IR::Program`).
pub(crate) struct Ir {
    pub insts: Vec<Inst>,
    pub args: Vec<Value>,
    pub blocks: Vec<IrBlock>,
    pub preds: Vec<u32>,
    pub exports: Vec<ExportInfo>,
    pub memory: Vec<MemoryInfo>,
}

impl Ir {
    pub fn new() -> Self {
        Self {
            insts: Vec::new(),
            args: Vec::new(),
            blocks: Vec::new(),
            preds: Vec::new(),
            exports: Vec::new(),
            memory: Vec::new(),
        }
    }

    pub fn inst(&self, id: InstId) -> &Inst {
        &self.insts[id.index()]
    }

    pub fn inst_mut(&mut self, id: InstId) -> &mut Inst {
        &mut self.insts[id.index()]
    }

    /// Rebuild an id stored in a crate-internal u32 table (never guest data).
    pub fn id_from_raw(&self, raw: u32) -> InstId {
        InstId(raw)
    }

    pub fn arg(&self, id: InstId, index: usize) -> Value {
        let inst = self.inst(id);
        if index >= inst.nargs as usize {
            return PENDING;
        }
        self.args[inst.args as usize + index]
    }

    pub fn set_arg(&mut self, id: InstId, index: usize, value: Value) {
        let inst = *self.inst(id);
        if index < inst.nargs as usize {
            self.args[inst.args as usize + index] = value;
        }
    }

    pub fn nargs(&self, id: InstId) -> usize {
        self.inst(id).nargs as usize
    }

    /// Follow Identity forwarding (source `Value::Resolve`). Every visited
    /// instruction is work-charged, including repeated walks of a long chain.
    pub fn resolve(&self, budget: &Budget, mut value: Value) -> Result<Value, CompileError> {
        let mut steps = 0usize;
        while let Value::Inst(id) = value {
            if id == InstId(u32::MAX) {
                return Ok(value);
            }
            budget.charge(1)?;
            if steps >= self.insts.len() {
                return Err(CompileError::Internal("cyclic identity forwarding"));
            }
            let inst = self
                .insts
                .get(id.index())
                .ok_or(CompileError::Internal("identity instruction id"))?;
            if inst.op != Op::Identity {
                return Ok(value);
            }
            value = *self
                .args
                .get(inst.args as usize)
                .ok_or(CompileError::Internal("identity operand"))?;
            steps += 1;
        }
        Ok(value)
    }

    pub fn type_of(&self, budget: &Budget, value: Value) -> Result<Type, CompileError> {
        Ok(match self.resolve(budget, value)? {
            Value::Inst(id) if id.0 != u32::MAX => self.inst(id).ty,
            Value::Inst(_) => Type::Void,
            Value::U1(_) => Type::U1,
            Value::U32(_) => Type::U32,
            Value::U64(_) => Type::U64,
            Value::F32(_) => Type::F32,
        })
    }

    /// The producing instruction of a resolved value.
    pub fn producer(
        &self,
        budget: &Budget,
        value: Value,
    ) -> Result<Option<(InstId, &Inst)>, CompileError> {
        Ok(match self.resolve(budget, value)? {
            Value::Inst(id) if id.0 != u32::MAX => Some((id, self.inst(id))),
            _ => None,
        })
    }

    pub fn predecessors(&self, block: u32) -> &[u32] {
        match self.blocks.get(block as usize) {
            Some(b) => {
                let start = b.pred_start as usize;
                self.preds
                    .get(start..start + b.pred_len as usize)
                    .unwrap_or(&[])
            }
            None => &[],
        }
    }

    /// Ids of a block's live instructions, in order.
    pub fn block_insts(&self, block: u32) -> BlockIter<'_> {
        BlockIter {
            ir: self,
            next: self.blocks.get(block as usize).map_or(NIL, |b| b.first),
        }
    }

    /// Append a new instruction (not yet linked into a block).
    pub fn push(
        &mut self,
        budget: &mut Budget,
        op: Op,
        ty: Type,
        aux: u32,
        pc: u32,
        args: &[Value],
    ) -> Result<InstId, CompileError> {
        let limits = budget.limits;
        let start = u32::try_from(self.args.len())
            .map_err(|_| CompileError::limit(LimitKind::IrOperands))?;
        budget.reserve(
            &mut self.args,
            args.len(),
            limits.max_ir_operands,
            LimitKind::IrOperands,
        )?;
        budget.charge_usize(args.len())?;
        self.args.extend_from_slice(args);
        let id = u32::try_from(self.insts.len())
            .ok()
            .filter(|&id| id != u32::MAX)
            .ok_or(CompileError::limit(LimitKind::IrInstructions))?;
        budget.push(
            &mut self.insts,
            Inst {
                op,
                ty,
                aux,
                pc,
                args: start,
                nargs: args.len() as u32,
                block: NIL,
                prev: NIL,
                next: NIL,
                alive: true,
            },
            limits.max_ir_instructions,
            LimitKind::IrInstructions,
        )?;
        Ok(InstId(id))
    }

    pub fn append(&mut self, block: u32, id: InstId) {
        let last = self.blocks[block as usize].last;
        {
            let inst = self.inst_mut(id);
            inst.block = block;
            inst.prev = last;
            inst.next = NIL;
        }
        if last == NIL {
            self.blocks[block as usize].first = id.0;
        } else {
            self.insts[last as usize].next = id.0;
        }
        self.blocks[block as usize].last = id.0;
    }

    pub fn prepend(&mut self, block: u32, id: InstId) {
        let first = self.blocks[block as usize].first;
        {
            let inst = self.inst_mut(id);
            inst.block = block;
            inst.prev = NIL;
            inst.next = first;
        }
        if first == NIL {
            self.blocks[block as usize].last = id.0;
        } else {
            self.insts[first as usize].prev = id.0;
        }
        self.blocks[block as usize].first = id.0;
    }

    /// Insert `id` before the linked instruction `before`.
    pub fn insert_before(&mut self, before: InstId, id: InstId) {
        let at = *self.inst(before);
        {
            let inst = self.inst_mut(id);
            inst.block = at.block;
            inst.prev = at.prev;
            inst.next = before.0;
        }
        if at.prev == NIL {
            self.blocks[at.block as usize].first = id.0;
        } else {
            self.insts[at.prev as usize].next = id.0;
        }
        self.insts[before.index()].prev = id.0;
    }

    pub fn unlink(&mut self, id: InstId) {
        let inst = *self.inst(id);
        if inst.block == NIL {
            return;
        }
        if inst.prev == NIL {
            self.blocks[inst.block as usize].first = inst.next;
        } else {
            self.insts[inst.prev as usize].next = inst.next;
        }
        if inst.next == NIL {
            self.blocks[inst.block as usize].last = inst.prev;
        } else {
            self.insts[inst.next as usize].prev = inst.prev;
        }
        let entry = self.inst_mut(id);
        entry.block = NIL;
        entry.prev = NIL;
        entry.next = NIL;
        entry.alive = false;
    }

    /// Source `ReplaceUsesWith`: the instruction forwards to `value`.
    pub fn forward(
        &mut self,
        budget: &mut Budget,
        id: InstId,
        value: Value,
    ) -> Result<(), CompileError> {
        let value = self.resolve(budget, value)?;
        if value == Value::Inst(id) {
            return Err(CompileError::Internal("instruction forwards to itself"));
        }
        let limits = budget.limits;
        let start = u32::try_from(self.args.len())
            .map_err(|_| CompileError::limit(LimitKind::IrOperands))?;
        budget.push(
            &mut self.args,
            value,
            limits.max_ir_operands,
            LimitKind::IrOperands,
        )?;
        let inst = self.inst_mut(id);
        inst.op = Op::Identity;
        inst.args = start;
        inst.nargs = 1;
        Ok(())
    }
}

pub(crate) struct BlockIter<'a> {
    ir: &'a Ir,
    next: u32,
}

impl Iterator for BlockIter<'_> {
    type Item = InstId;

    fn next(&mut self) -> Option<InstId> {
        if self.next == NIL {
            return None;
        }
        let id = InstId(self.next);
        self.next = self
            .ir
            .insts
            .get(self.next as usize)
            .map_or(NIL, |i| i.next);
        Some(id)
    }
}

/// Source `IREmitter`: appends checked operations to one block.
pub(crate) struct Emitter<'a> {
    pub ir: &'a mut Ir,
    pub budget: &'a mut Budget,
    pub block: u32,
    pub pc: u32,
}

impl Emitter<'_> {
    pub fn emit_aux(&mut self, op: Op, aux: u32, args: &[Value]) -> Result<Value, CompileError> {
        let expected = op.args();
        if expected.len() != args.len() {
            return Err(CompileError::Internal("operation arity"));
        }
        for (&want, &arg) in expected.iter().zip(args) {
            if self.ir.type_of(self.budget, arg)? != want {
                return Err(CompileError::Internal("operation operand type"));
            }
        }
        let id = self
            .ir
            .push(self.budget, op, op.result(), aux, self.pc, args)
            .map_err(|e| e.at(self.pc))?;
        self.ir.append(self.block, id);
        Ok(Value::Inst(id))
    }

    pub fn emit(&mut self, op: Op, args: &[Value]) -> Result<Value, CompileError> {
        self.emit_aux(op, 0, args)
    }

    pub fn iadd(&mut self, a: Value, b: Value) -> Result<Value, CompileError> {
        self.emit(Op::IAdd32, &[a, b])
    }
    pub fn isub(&mut self, a: Value, b: Value) -> Result<Value, CompileError> {
        self.emit(Op::ISub32, &[a, b])
    }
    pub fn imul(&mut self, a: Value, b: Value) -> Result<Value, CompileError> {
        self.emit(Op::IMul32, &[a, b])
    }
    pub fn and(&mut self, a: Value, b: Value) -> Result<Value, CompileError> {
        self.emit(Op::BitwiseAnd32, &[a, b])
    }
    pub fn or(&mut self, a: Value, b: Value) -> Result<Value, CompileError> {
        self.emit(Op::BitwiseOr32, &[a, b])
    }
    pub fn xor(&mut self, a: Value, b: Value) -> Result<Value, CompileError> {
        self.emit(Op::BitwiseXor32, &[a, b])
    }
    pub fn not(&mut self, a: Value) -> Result<Value, CompileError> {
        self.emit(Op::BitwiseNot32, &[a])
    }
    pub fn shl(&mut self, a: Value, b: Value) -> Result<Value, CompileError> {
        self.emit(Op::ShiftLeftLogical32, &[a, b])
    }
    pub fn shr(&mut self, a: Value, b: Value) -> Result<Value, CompileError> {
        self.emit(Op::ShiftRightLogical32, &[a, b])
    }
    pub fn sar(&mut self, a: Value, b: Value) -> Result<Value, CompileError> {
        self.emit(Op::ShiftRightArithmetic32, &[a, b])
    }
    pub fn ieq(&mut self, a: Value, b: Value) -> Result<Value, CompileError> {
        self.emit(Op::IEqual32, &[a, b])
    }
    pub fn ine(&mut self, a: Value, b: Value) -> Result<Value, CompileError> {
        self.emit(Op::INotEqual32, &[a, b])
    }
    pub fn ult(&mut self, a: Value, b: Value) -> Result<Value, CompileError> {
        self.emit(Op::ULessThan32, &[a, b])
    }
    pub fn ugt(&mut self, a: Value, b: Value) -> Result<Value, CompileError> {
        self.emit(Op::UGreaterThan32, &[a, b])
    }
    pub fn land(&mut self, a: Value, b: Value) -> Result<Value, CompileError> {
        self.emit(Op::LogicalAnd, &[a, b])
    }
    pub fn lor(&mut self, a: Value, b: Value) -> Result<Value, CompileError> {
        self.emit(Op::LogicalOr, &[a, b])
    }
    pub fn lnot(&mut self, a: Value) -> Result<Value, CompileError> {
        self.emit(Op::LogicalNot, &[a])
    }
    /// Source `IREmitter::Select` (U32 or F32 by operand type).
    pub fn select(&mut self, c: Value, t: Value, f: Value) -> Result<Value, CompileError> {
        match self.ir.type_of(self.budget, t)? {
            Type::F32 => self.emit(Op::SelectF32, &[c, t, f]),
            Type::U1 => self.emit(Op::SelectU1, &[c, t, f]),
            _ => self.emit(Op::SelectU32, &[c, t, f]),
        }
    }
    pub fn bitcast_f32(&mut self, a: Value) -> Result<Value, CompileError> {
        self.emit(Op::BitCastF32U32, &[a])
    }
    pub fn bitcast_u32(&mut self, a: Value) -> Result<Value, CompileError> {
        self.emit(Op::BitCastU32F32, &[a])
    }
    pub fn extract(&mut self, composite: Value, index: u32) -> Result<Value, CompileError> {
        let op = match self.ir.type_of(self.budget, composite)? {
            Type::U64 => Op::CompositeExtractU64,
            Type::U32x2 => Op::CompositeExtractU32x2,
            Type::U32x3 => Op::CompositeExtractU32x3,
            Type::U32x4 => Op::CompositeExtractU32x4,
            _ => return Err(CompileError::Internal("composite extract type")),
        };
        self.emit_aux(op, index, &[composite])
    }
    pub fn construct_u64(&mut self, low: Value, high: Value) -> Result<Value, CompileError> {
        self.emit(Op::CompositeConstructU64, &[low, high])
    }
    pub fn get(&mut self, op: Op, aux: u32) -> Result<Value, CompileError> {
        self.emit_aux(op, aux, &[])
    }
    pub fn set(&mut self, op: Op, aux: u32, value: Value) -> Result<(), CompileError> {
        self.emit_aux(op, aux, &[value]).map(|_| ())
    }
}
