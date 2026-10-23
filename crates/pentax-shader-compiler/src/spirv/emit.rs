// SPDX-License-Identifier: GPL-2.0-only
// Direct Rust adaptation of KytyPS5-Performance at
// 209dffe9032d72749257ce18680d506b891284de, backend/spirv:
// SpirvEmitter.cpp (EmitProgram), spirvEmitterModule.cpp (types,
// DefineDescriptors, DefineInputs, DefineOutputs, DefineModule capabilities,
// float controls and execution modes), spirvEmitterProgram.cpp (labels,
// structured blocks, deferred phis, EmitStructuredTerminator, BranchCondition),
// spirvEmitterInstructions.h/spirvEmitterAlu.cpp/spirvEmitterAluHelpers.cpp
// (ALU lowerings with the default CodegenOptions: fast float min/max, fast
// PKRTZ, single f2i saturation, MadMode::Position), spirvEmitterFlow.cpp
// (builtins, attributes, exports, barriers, ballots) and
// spirvEmitterMemory{,Helpers}.cpp (shader-data loads, buffer byte offsets,
// bounds-checked storage-buffer words, ReadConstBuffer).
// PentaX modifications: typed failures instead of EXIT; only the ported IR
// subset; optional host workarounds (clip guard, loop guard, relaxed math,
// draw-data uniform buffers, device robustness shortcuts) are not emitted.

use pentax_shader::ShaderStage;

use super::builder::{Annotation, Builder, DeferredPhi};
use super::spec::*;
use crate::CompileError;
use crate::budget::{Budget, LimitKind};
use crate::cfg::TerminatorKind;
use crate::error::UnsupportedReason;
use crate::ir::{Builtin, ExportKind, InstId, Ir, NIL, Op, Type, Value};
use crate::passes::{InputKind, Layout, OutputKind, StageInput};
use crate::profile::{PIXEL_INPUTS, PUSH_DWORDS, Profile};

#[path = "emit/shared.rs"]
mod shared;

type R<T> = Result<T, CompileError>;

/// Source `DescriptorBindingKind` values used here, and `NativeBinding`.
pub(crate) const BINDING_BUFFERS: u32 = 0;
pub(crate) const BINDING_IMAGES: u32 = 1;
pub(crate) const BINDING_SAMPLERS: u32 = 17;
pub(crate) const BINDING_LOD_FEEDBACK: u32 = 33;
pub(crate) const BINDING_SHADER_DATA: u32 = 54;
const BINDING_GROUP: u32 = 57;

pub(crate) fn native_binding(stage: ShaderStage, kind: u32) -> u32 {
    let group = u32::from(stage == ShaderStage::Pixel);
    kind + group * BINDING_GROUP
}

#[derive(Default)]
struct Types {
    void: u32,
    bool_: u32,
    u32_: u32,
    i32_: u32,
    f32_: u32,
    u64_: u32,
}

struct Emit<'a, 'b> {
    b: Builder<'b>,
    ir: &'a Ir,
    layout: &'a Layout,
    profile: &'a Profile,
    t: Types,
    ids: Vec<u32>,
    labels: Vec<u32>,
    exit_labels: Vec<u32>,
    current_label: u32,
    deferred: Vec<(DeferredPhi, InstId)>,
    glsl: u32,
    main_func: u32,
    push_var: u32,
    buffers_var: u32,
    shared_var: u32,
    image_vars: Vec<u32>,
    sampler_vars: Vec<u32>,
    lod_feedback_var: u32,
    helper_var: u32,
    shader_data_var: u32,
    inputs: Vec<(StageInput, u32)>,
    outputs: Vec<(crate::passes::StageOutput, u32)>,
    interface: Vec<u32>,
    subgroup_lane_var: u32,
    local_lane_var: u32,
    memory_offsets: Vec<u32>,
    position_slice: Vec<bool>,
}

fn count_ops(ir: &Ir, predicate: impl Fn(Op) -> bool) -> bool {
    (0..ir.blocks.len() as u32).any(|b| ir.block_insts(b).any(|id| predicate(ir.inst(id).op)))
}

impl Emit<'_, '_> {
    // ------------------------------------------------------------ types

    fn ty_void(&mut self) -> R<u32> {
        if self.t.void == 0 {
            self.t.void = self.b.ty(OpTypeVoid, &[])?;
        }
        Ok(self.t.void)
    }
    fn ty_bool(&mut self) -> R<u32> {
        if self.t.bool_ == 0 {
            self.t.bool_ = self.b.ty(OpTypeBool, &[])?;
        }
        Ok(self.t.bool_)
    }
    fn ty_u32(&mut self) -> R<u32> {
        if self.t.u32_ == 0 {
            self.t.u32_ = self.b.ty(OpTypeInt, &[32, 0])?;
        }
        Ok(self.t.u32_)
    }
    fn ty_i32(&mut self) -> R<u32> {
        if self.t.i32_ == 0 {
            self.t.i32_ = self.b.ty(OpTypeInt, &[32, 1])?;
        }
        Ok(self.t.i32_)
    }
    fn ty_f32(&mut self) -> R<u32> {
        if self.t.f32_ == 0 {
            self.t.f32_ = self.b.ty(OpTypeFloat, &[32])?;
        }
        Ok(self.t.f32_)
    }
    fn ty_u64(&mut self) -> R<u32> {
        if self.t.u64_ == 0 {
            self.b.require_capability(CapabilityInt64)?;
            self.t.u64_ = self.b.ty(OpTypeInt, &[64, 0])?;
        }
        Ok(self.t.u64_)
    }
    fn ty_vec(&mut self, element: u32, count: u32) -> R<u32> {
        self.b.ty(OpTypeVector, &[element, count])
    }
    fn ty_uvec(&mut self, count: u32) -> R<u32> {
        let u = self.ty_u32()?;
        self.ty_vec(u, count)
    }
    fn ty_fvec(&mut self, count: u32) -> R<u32> {
        let f = self.ty_f32()?;
        self.ty_vec(f, count)
    }
    /// Source `TypeU32Pair`: a two-member struct (OpIAddCarry result type).
    fn ty_u32_pair(&mut self) -> R<u32> {
        let u = self.ty_u32()?;
        self.b.ty(OpTypeStruct, &[u, u])
    }
    fn ty_i32_pair(&mut self) -> R<u32> {
        let i = self.ty_i32()?;
        self.b.ty(OpTypeStruct, &[i, i])
    }
    /// Source `TypeU32Composite`.
    fn ty_u32_composite(&mut self, components: u32) -> R<u32> {
        if components == 2 {
            self.ty_u32_pair()
        } else {
            self.ty_uvec(components)
        }
    }
    fn ty_pointer(&mut self, storage: u32, pointee: u32) -> R<u32> {
        self.b.ty(OpTypePointer, &[storage, pointee])
    }
    /// Source `TypeId`.
    fn type_id(&mut self, ty: Type) -> R<u32> {
        match ty {
            Type::U1 => self.ty_bool(),
            Type::U32 => self.ty_u32(),
            Type::U64 => self.ty_u64(),
            Type::F32 => self.ty_f32(),
            Type::U32x2 => self.ty_u32_pair(),
            Type::U32x3 => self.ty_uvec(3),
            Type::U32x4 => self.ty_uvec(4),
            Type::F32x2 => self.ty_fvec(2),
            Type::F32x4 => self.ty_fvec(4),
            Type::Void | Type::Resource => {
                Err(CompileError::Internal("value type has no SPIR-V id"))
            }
        }
    }

    fn const_u32(&mut self, value: u32) -> R<u32> {
        let ty = self.ty_u32()?;
        self.b.constant(OpConstant, ty, &[value])
    }
    fn const_f32(&mut self, bits: u32) -> R<u32> {
        let ty = self.ty_f32()?;
        self.b.constant(OpConstant, ty, &[bits])
    }
    fn const_bool(&mut self, value: bool) -> R<u32> {
        let ty = self.ty_bool()?;
        self.b.constant(
            if value {
                OpConstantTrue
            } else {
                OpConstantFalse
            },
            ty,
            &[],
        )
    }
    fn const_u64(&mut self, value: u64) -> R<u32> {
        let ty = self.ty_u64()?;
        self.b
            .constant(OpConstant, ty, &[value as u32, (value >> 32) as u32])
    }
    fn const_u32_composite_zero(&mut self, components: u32) -> R<u32> {
        let zero = self.const_u32(0)?;
        let ty = self.ty_u32_composite(components)?;
        let operands = [zero; 4];
        self.b
            .constant(OpConstantComposite, ty, &operands[..components as usize])
    }

    // ----------------------------------------------------- instructions

    fn op(&mut self, op: u32, ty: u32, operands: &[u32]) -> R<u32> {
        let result = self.b.alloc_id()?;
        let mut words = [0u32; 8];
        if operands.len() + 2 > words.len() {
            return Err(CompileError::Internal("instruction operand count"));
        }
        words[0] = ty;
        words[1] = result;
        words[2..2 + operands.len()].copy_from_slice(operands);
        self.b.func(op, &words[..2 + operands.len()])?;
        Ok(result)
    }
    fn u32op(&mut self, op: u32, operands: &[u32]) -> R<u32> {
        let ty = self.ty_u32()?;
        self.op(op, ty, operands)
    }
    fn boolop(&mut self, op: u32, operands: &[u32]) -> R<u32> {
        let ty = self.ty_bool()?;
        self.op(op, ty, operands)
    }
    fn f32op(&mut self, op: u32, operands: &[u32]) -> R<u32> {
        let ty = self.ty_f32()?;
        self.op(op, ty, operands)
    }
    fn ext(&mut self, ty: u32, instruction: u32, operands: &[u32]) -> R<u32> {
        if self.glsl == 0 {
            self.glsl = self.b.import("GLSL.std.450")?;
        }
        let mut words = [0u32; 6];
        words[0] = self.glsl;
        words[1] = instruction;
        words[2..2 + operands.len()].copy_from_slice(operands);
        self.op(OpExtInst, ty, &words[..2 + operands.len()])
    }
    fn ext_f32(&mut self, instruction: u32, operands: &[u32]) -> R<u32> {
        let ty = self.ty_f32()?;
        self.ext(ty, instruction, operands)
    }
    fn select_u32(&mut self, c: u32, t: u32, f: u32) -> R<u32> {
        self.u32op(OpSelect, &[c, t, f])
    }
    fn and_const(&mut self, value: u32, mask: u32) -> R<u32> {
        let m = self.const_u32(mask)?;
        self.u32op(OpBitwiseAnd, &[value, m])
    }
    fn shr_const(&mut self, value: u32, shift: u32) -> R<u32> {
        let s = self.const_u32(shift)?;
        self.u32op(OpShiftRightLogical, &[value, s])
    }
    fn cmp_const(&mut self, op: u32, value: u32, constant: u32) -> R<u32> {
        let c = self.const_u32(constant)?;
        self.boolop(op, &[value, c])
    }
    fn bitcast_to_u32(&mut self, value: u32) -> R<u32> {
        self.u32op(OpBitcast, &[value])
    }
    fn bitcast_to_f32(&mut self, value: u32) -> R<u32> {
        self.f32op(OpBitcast, &[value])
    }
    fn label(&mut self, label: u32) -> R<()> {
        self.b.func(OpLabel, &[label])?;
        self.current_label = label;
        Ok(())
    }

    /// Source `EmitIfCondition`.
    fn if_condition(&mut self, condition: u32, body: impl FnOnce(&mut Self) -> R<()>) -> R<()> {
        let then_label = self.b.alloc_id()?;
        let merge_label = self.b.alloc_id()?;
        self.b
            .func(OpSelectionMerge, &[merge_label, SelectionControlMaskNone])?;
        self.b
            .func(OpBranchConditional, &[condition, then_label, merge_label])?;
        self.label(then_label)?;
        body(self)?;
        self.b.func(OpBranch, &[merge_label])?;
        self.label(merge_label)
    }

    /// Source `EmitValueOrDefaultIfCondition`.
    fn value_or_default_if(
        &mut self,
        condition: u32,
        ty: u32,
        default: u32,
        body: impl FnOnce(&mut Self) -> R<u32>,
    ) -> R<u32> {
        let then_label = self.b.alloc_id()?;
        let header = self.current_label;
        let merge_label = self.b.alloc_id()?;
        self.b
            .func(OpSelectionMerge, &[merge_label, SelectionControlMaskNone])?;
        self.b
            .func(OpBranchConditional, &[condition, then_label, merge_label])?;
        self.label(then_label)?;
        let value = body(self)?;
        let then_exit = self.current_label;
        self.b.func(OpBranch, &[merge_label])?;
        self.label(merge_label)?;
        self.op(OpPhi, ty, &[value, then_exit, default, header])
    }

    fn value_or_zero_if(
        &mut self,
        condition: u32,
        body: impl FnOnce(&mut Self) -> R<u32>,
    ) -> R<u32> {
        let ty = self.ty_u32()?;
        let zero = self.const_u32(0)?;
        self.value_or_default_if(condition, ty, zero, body)
    }

    /// Source `ValueEmitContext::Def`.
    fn def(&mut self, value: Value) -> R<u32> {
        match self.ir.resolve(self.b.budget_ref(), value)? {
            Value::U1(v) => self.const_bool(v),
            Value::U32(v) => self.const_u32(v),
            Value::U64(v) => self.const_u64(v),
            Value::F32(bits) => self.const_f32(bits),
            Value::Inst(id) => match self.ids.get(id.index()).copied() {
                Some(id) if id != 0 => Ok(id),
                _ => Err(CompileError::Internal("value used before its definition")),
            },
        }
    }

    fn arg(&mut self, id: InstId, index: usize) -> R<u32> {
        let value = self.ir.arg(id, index);
        self.def(value)
    }

    fn arg_bits(&self, id: InstId, index: usize) -> R<Option<u32>> {
        Ok(
            match self
                .ir
                .resolve(self.b.budget_ref(), self.ir.arg(id, index))?
            {
                Value::F32(bits) => Some(bits),
                _ => None,
            },
        )
    }

    // --------------------------------------------------- source helpers

    /// Source `EmitFlushF32DenormToSignedZero`.
    fn flush_denorm(&mut self, value: u32) -> R<u32> {
        let bits = self.bitcast_to_u32(value)?;
        let abs = self.and_const(bits, 0x7fff_ffff)?;
        let sign = self.and_const(bits, 0x8000_0000)?;
        let flush = self.cmp_const(OpULessThan, abs, 0x0080_0000)?;
        let selected = self.select_u32(flush, sign, bits)?;
        self.bitcast_to_f32(selected)
    }

    /// Source `EmitFastMinMaxF32` (the default fast_float_min_max path).
    fn min_max_f32(
        &mut self,
        lhs: (u32, Option<u32>),
        rhs: (u32, Option<u32>),
        max: bool,
    ) -> R<(u32, Option<u32>)> {
        let non_nan = |c: Option<u32>| c.is_some_and(|b| b & 0x7fff_ffff <= 0x7f80_0000);
        let plain = |c: Option<u32>| non_nan(c) && c.is_some_and(|b| b & 0x7fff_ffff != 0);
        let mut pick = self.boolop(
            if max {
                OpFOrdGreaterThanEqual
            } else {
                OpFOrdLessThan
            },
            &[lhs.0, rhs.0],
        )?;
        if !non_nan(rhs.1) {
            let rhs_nan = self.boolop(OpIsNan, &[rhs.0])?;
            pick = self.boolop(OpLogicalOr, &[pick, rhs_nan])?;
            if !non_nan(lhs.1) {
                let lhs_ordered = self.boolop(OpFOrdEqual, &[lhs.0, lhs.0])?;
                pick = self.boolop(OpLogicalAnd, &[pick, lhs_ordered])?;
            }
        }
        let lhs_bits = self.bitcast_to_u32(lhs.0)?;
        let rhs_bits = self.bitcast_to_u32(rhs.0)?;
        let mut value = self.select_u32(pick, lhs_bits, rhs_bits)?;
        if !plain(lhs.1) && !plain(rhs.1) {
            let either = self.u32op(OpBitwiseOr, &[lhs_bits, rhs_bits])?;
            let magnitude = self.and_const(either, 0x7fff_ffff)?;
            let both_zero = self.cmp_const(OpIEqual, magnitude, 0)?;
            let combined = if max {
                self.u32op(OpBitwiseAnd, &[lhs_bits, rhs_bits])?
            } else {
                either
            };
            value = self.select_u32(both_zero, combined, value)?;
        }
        Ok((self.bitcast_to_f32(value)?, None))
    }

    fn f32_operand(&mut self, id: InstId, index: usize) -> R<(u32, Option<u32>)> {
        let bits = self.arg_bits(id, index)?;
        Ok((self.arg(id, index)?, bits))
    }

    /// Source `EmitFastMedTri32`.
    fn med3_f32(&mut self, id: InstId) -> R<u32> {
        let a = self.f32_operand(id, 0)?;
        let b = self.f32_operand(id, 1)?;
        let c = self.f32_operand(id, 2)?;
        let min_ab = self.min_max_f32(a, b, false)?;
        let min3 = self.min_max_f32(min_ab, c, false)?;
        let max_ab = self.min_max_f32(a, b, true)?;
        let high_min = self.min_max_f32(max_ab, c, false)?;
        let median = self.min_max_f32(min_ab, high_min, true)?;
        let mut any_nan = 0;
        for operand in [a, b, c] {
            if operand
                .1
                .is_some_and(|bits| bits & 0x7fff_ffff <= 0x7f80_0000)
            {
                continue;
            }
            let nan = self.boolop(OpIsNan, &[operand.0])?;
            any_nan = if any_nan == 0 {
                nan
            } else {
                self.boolop(OpLogicalOr, &[any_nan, nan])?
            };
        }
        if any_nan == 0 {
            return Ok(median.0);
        }
        self.f32op(OpSelect, &[any_nan, min3.0, median.0])
    }

    fn int_min_max(&mut self, lhs: u32, rhs: u32, signed: bool, max: bool) -> R<u32> {
        let op = match (signed, max) {
            (false, false) => OpULessThan,
            (false, true) => OpUGreaterThan,
            (true, false) => OpSLessThan,
            (true, true) => OpSGreaterThan,
        };
        let cond = self.boolop(op, &[lhs, rhs])?;
        self.select_u32(cond, lhs, rhs)
    }

    /// Source `EmitTrigCycleF32` (non-relaxed).
    fn trig_cycle(&mut self, src: u32, preserve_signed_zero: bool) -> R<u32> {
        let fract = self.ext_f32(GLSLstd450Fract, &[src])?;
        let bits = self.bitcast_to_u32(src)?;
        let abs = self.and_const(bits, 0x7fff_ffff)?;
        let large = self.cmp_const(OpUGreaterThanEqual, abs, 0x4b00_0000)?;
        let finite = self.cmp_const(OpULessThan, abs, 0x7f80_0000)?;
        let large_finite = self.boolop(OpLogicalAnd, &[large, finite])?;
        let zero = self.const_f32(0)?;
        let reduced = self.f32op(OpSelect, &[large_finite, zero, fract])?;
        if !preserve_signed_zero {
            return Ok(reduced);
        }
        let is_zero = self.cmp_const(OpIEqual, abs, 0)?;
        self.f32op(OpSelect, &[is_zero, src, reduced])
    }

    /// Source `EmitFastF32ToF16RtzBits` (default fast_pkrtz path).
    fn f32_to_f16_rtz_bits(&mut self, f32: u32) -> R<u32> {
        let bits = self.bitcast_to_u32(f32)?;
        let abs = self.and_const(bits, 0x7fff_ffff)?;
        let shifted = self.shr_const(bits, 16)?;
        let sign = self.and_const(shifted, 0x8000)?;
        let high13 = self.shr_const(abs, 13)?;
        let bias = self.const_u32(112 << 10)?;
        let rebased = self.u32op(OpISub, &[high13, bias])?;
        let max = self.const_u32(0x7bff)?;
        let normal = self.int_min_max(rebased, max, false, false)?;
        let exponent = self.shr_const(abs, 23)?;
        let k126 = self.const_u32(126)?;
        let raw_shift = self.u32op(OpISub, &[k126, exponent])?;
        let k31 = self.const_u32(31)?;
        let shift = self.int_min_max(raw_shift, k31, false, false)?;
        let mantissa = self.and_const(abs, 0x007f_ffff)?;
        let hidden = self.const_u32(0x0080_0000)?;
        let full = self.u32op(OpBitwiseOr, &[mantissa, hidden])?;
        let subnormal = self.u32op(OpShiftRightLogical, &[full, shift])?;
        let small = self.cmp_const(OpULessThan, abs, 0x3880_0000)?;
        let mut result = self.select_u32(small, subnormal, normal)?;
        let is_nan = self.cmp_const(OpUGreaterThan, abs, 0x7f80_0000)?;
        let quiet_bit = self.const_u32(0x0200)?;
        let zero = self.const_u32(0)?;
        let quiet = self.select_u32(is_nan, quiet_bit, zero)?;
        let payload = self.and_const(high13, 0x7fff)?;
        let special = self.u32op(OpBitwiseOr, &[payload, quiet])?;
        let is_special = self.cmp_const(OpUGreaterThanEqual, abs, 0x7f80_0000)?;
        result = self.select_u32(is_special, special, result)?;
        self.u32op(OpBitwiseOr, &[result, sign])
    }

    /// Source `EmitClassMaskBits` for F32.
    fn class_mask_f32(&mut self, value: u32, mask: u32) -> R<u32> {
        let bits = self.bitcast_to_u32(value)?;
        let sign_bits = self.and_const(bits, 0x8000_0000)?;
        let abs_bits = self.and_const(bits, 0x7fff_ffff)?;
        let exponent_bits = self.and_const(abs_bits, 0x7f80_0000)?;
        let mantissa_bits = self.and_const(abs_bits, 0x007f_ffff)?;
        let quiet_bits = self.and_const(mantissa_bits, 0x0040_0000)?;
        let sign = self.cmp_const(OpINotEqual, sign_bits, 0)?;
        let positive = self.boolop(OpLogicalNot, &[sign])?;
        let exponent_zero = self.cmp_const(OpIEqual, exponent_bits, 0)?;
        let exponent_nonzero = self.boolop(OpLogicalNot, &[exponent_zero])?;
        let exponent_inf = self.cmp_const(OpIEqual, exponent_bits, 0x7f80_0000)?;
        let finite_exponent = self.boolop(OpLogicalNot, &[exponent_inf])?;
        let mantissa_zero = self.cmp_const(OpIEqual, mantissa_bits, 0)?;
        let mantissa_nonzero = self.boolop(OpLogicalNot, &[mantissa_zero])?;
        let quiet = self.cmp_const(OpINotEqual, quiet_bits, 0)?;
        let nan_common = self.boolop(OpLogicalAnd, &[exponent_inf, mantissa_nonzero])?;
        let not_quiet = self.boolop(OpLogicalNot, &[quiet])?;
        let snan = self.boolop(OpLogicalAnd, &[nan_common, not_quiet])?;
        let qnan = self.boolop(OpLogicalAnd, &[nan_common, quiet])?;
        let inf = self.boolop(OpLogicalAnd, &[exponent_inf, mantissa_zero])?;
        let normal = self.boolop(OpLogicalAnd, &[exponent_nonzero, finite_exponent])?;
        let denorm = self.boolop(OpLogicalAnd, &[exponent_zero, mantissa_nonzero])?;
        let zero = self.cmp_const(OpIEqual, abs_bits, 0)?;
        let classes = [
            (0, snan, None),
            (1, qnan, None),
            (2, inf, Some(sign)),
            (3, normal, Some(sign)),
            (4, denorm, Some(sign)),
            (5, zero, Some(sign)),
            (6, zero, Some(positive)),
            (7, denorm, Some(positive)),
            (8, normal, Some(positive)),
            (9, inf, Some(positive)),
        ];
        let mut result = 0;
        for (bit, class, side) in classes {
            let class = match side {
                Some(side) => self.boolop(OpLogicalAnd, &[class, side])?,
                None => class,
            };
            let selected_bit = self.and_const(mask, 1 << bit)?;
            let selected = self.cmp_const(OpINotEqual, selected_bit, 0)?;
            let matched = self.boolop(OpLogicalAnd, &[selected, class])?;
            result = if result == 0 {
                matched
            } else {
                self.boolop(OpLogicalOr, &[result, matched])?
            };
        }
        Ok(result)
    }

    fn mul_high(&mut self, lhs: u32, rhs: u32, signed: bool) -> R<u32> {
        if signed {
            let i = self.ty_i32()?;
            let a = self.op(OpBitcast, i, &[lhs])?;
            let b = self.op(OpBitcast, i, &[rhs])?;
            let pair = self.ty_i32_pair()?;
            let extended = self.op(OpSMulExtended, pair, &[a, b])?;
            let high = self.op(OpCompositeExtract, i, &[extended, 1])?;
            self.bitcast_to_u32(high)
        } else {
            let pair = self.ty_u32_pair()?;
            let extended = self.op(OpUMulExtended, pair, &[lhs, rhs])?;
            self.u32op(OpCompositeExtract, &[extended, 1])
        }
    }

    fn no_contraction(&self, id: InstId) -> bool {
        self.position_slice
            .get(id.index())
            .copied()
            .unwrap_or(false)
    }

    fn float_binary(&mut self, id: InstId, op: u32) -> R<u32> {
        let a = self.arg(id, 0)?;
        let b = self.arg(id, 1)?;
        let result = self.f32op(op, &[a, b])?;
        if self.no_contraction(id) {
            self.b
                .annotate(OpDecorate, &[result, DecorationNoContraction])?;
        }
        Ok(result)
    }

    // -------------------------------------------------- shader data

    /// Source `EmitShaderDataDwordLoad`.
    fn shader_data_dword(&mut self, dword: u32) -> R<u32> {
        if let Some(start) = self.layout.push_data_start {
            let u = self.ty_u32()?;
            let pointer_ty = self.ty_pointer(StorageClassPushConstant, u)?;
            let zero = self.const_u32(0)?;
            let index = self.const_u32(start + dword)?;
            let pointer = self.op(OpAccessChain, pointer_ty, &[self.push_var, zero, index])?;
            return self.u32op(OpLoad, &[pointer]);
        }
        if self.shader_data_var != 0 {
            let u = self.ty_u32()?;
            let pointer_ty = self.ty_pointer(StorageClassStorageBuffer, u)?;
            let zero = self.const_u32(0)?;
            let index = self.const_u32(dword)?;
            let pointer = self.op(
                OpAccessChain,
                pointer_ty,
                &[self.shader_data_var, zero, index],
            )?;
            return self.u32op(OpLoad, &[pointer]);
        }
        Err(CompileError::Internal("shader data without a binding"))
    }

    fn input_variable(&self, kind: InputKind, index: u32) -> u32 {
        self.inputs
            .iter()
            .find(|(input, _)| {
                input.kind == kind && (kind != InputKind::Parameter || input.index == index)
            })
            .map_or(0, |(_, var)| *var)
    }

    /// Source `EmitBuiltinU32` and `EmitInputComponentU32`.
    fn builtin_u32(&mut self, kind: Builtin, component: u32) -> R<u32> {
        let variable = self.input_variable(InputKind::Builtin(kind), 0);
        if variable == 0 {
            return Err(CompileError::Internal("builtin input was not declared"));
        }
        match kind {
            Builtin::VertexIndex | Builtin::InstanceIndex | Builtin::BaseInstance => {
                let i = self.ty_i32()?;
                let value = self.op(OpLoad, i, &[variable])?;
                self.bitcast_to_u32(value)
            }
            Builtin::FragCoord => {
                let f = self.ty_f32()?;
                let pointer_ty = self.ty_pointer(StorageClassInput, f)?;
                let index = self.const_u32(component)?;
                let pointer = self.op(OpAccessChain, pointer_ty, &[variable, index])?;
                let value = self.f32op(OpLoad, &[pointer])?;
                self.bitcast_to_u32(value)
            }
            Builtin::FrontFacing => {
                let b = self.ty_bool()?;
                let value = self.op(OpLoad, b, &[variable])?;
                // PS5 initializes v_front_face with float +1.0/-1.0 bits.
                let positive = self.const_u32(0x3f80_0000)?;
                let negative = self.const_u32(0xbf80_0000)?;
                self.select_u32(value, positive, negative)
            }
            Builtin::LocalInvocationIndex => self.u32op(OpLoad, &[variable]),
            Builtin::WorkgroupId | Builtin::LocalInvocationId => {
                let u = self.ty_u32()?;
                let pointer_ty = self.ty_pointer(StorageClassInput, u)?;
                let index = self.const_u32(component)?;
                let pointer = self.op(OpAccessChain, pointer_ty, &[variable, index])?;
                self.u32op(OpLoad, &[pointer])
            }
            _ => Err(CompileError::unsupported(
                0,
                None,
                UnsupportedReason::PerVertexInterpolation,
            )),
        }
    }

    /// Pixel parameter reads (source `EmitAttribute`, center interpolation).
    fn attribute(&mut self, attribute: u32, channel: u32) -> R<u32> {
        let variable = self.input_variable(InputKind::Parameter, attribute);
        if variable == 0 {
            return Err(CompileError::Internal("pixel parameter was not declared"));
        }
        let f = self.ty_f32()?;
        let pointer_ty = self.ty_pointer(StorageClassInput, f)?;
        let index = self.const_u32(channel)?;
        let pointer = self.op(OpAccessChain, pointer_ty, &[variable, index])?;
        let value = self.f32op(OpLoad, &[pointer])?;
        self.bitcast_to_u32(value)
    }

    // ----------------------------------------------------------- memory

    /// Source `PrepareStorageBufferResourceAccess`: (object, byte offset, length).
    fn buffer_access(&mut self, resource: u32) -> R<(u32, u32, u32)> {
        if self.buffers_var == 0 {
            return Err(CompileError::Internal(
                "storage buffer descriptor array was not emitted",
            ));
        }
        let u = self.ty_u32()?;
        let runtime = self.b.decorated_ty(
            OpTypeRuntimeArray,
            &[Annotation {
                op: OpDecorate,
                operands: &[DecorationArrayStride, 4],
            }],
            &[u],
        )?;
        let block = self.b.decorated_ty(
            OpTypeStruct,
            &[
                Annotation {
                    op: OpMemberDecorate,
                    operands: &[0, DecorationOffset, 0],
                },
                Annotation {
                    op: OpDecorate,
                    operands: &[DecorationBlock],
                },
            ],
            &[runtime],
        )?;
        let pointer_ty = self.ty_pointer(StorageClassStorageBuffer, block)?;
        let index = self.const_u32(resource)?;
        let object = self.op(OpAccessChain, pointer_ty, &[self.buffers_var, index])?;
        let byte_offset = *self
            .memory_offsets
            .get(resource as usize)
            .ok_or(CompileError::Internal("memory byte offset"))?;
        let length = self.u32op(OpArrayLength, &[object, 0])?;
        Ok((object, byte_offset, length))
    }

    fn element_pointer(&mut self, object: u32, index: u32) -> R<u32> {
        let u = self.ty_u32()?;
        let pointer_ty = self.ty_pointer(StorageClassStorageBuffer, u)?;
        let zero = self.const_u32(0)?;
        self.op(OpAccessChain, pointer_ty, &[object, zero, index])
    }

    /// Source `CalculateBufferAddress` + `BufferByteAddress` + `ByteAddress`
    /// + `DwordIndex` for a raw component at `immediate` bytes.
    fn buffer_dword_index(
        &mut self,
        id: InstId,
        resource: u32,
        immediate: u32,
        prefix: u32,
    ) -> R<u32> {
        let buffer = self
            .layout
            .buffers
            .get(resource as usize)
            .ok_or(CompileError::Internal("buffer resource index"))?;
        let description = self
            .profile
            .common()
            .buffers
            .get(buffer.profile_index)
            .ok_or(CompileError::Internal("buffer profile index"))?;
        let stride = u32::from(description.stride);
        let swizzle = stride != 0 && description.swizzle;
        let index_stride = u32::from(description.index_stride);
        let index = self.arg(id, 1)?;
        let mut offset = self.arg(id, 2)?;
        let soffset = self.arg(id, 3)?;
        let zero = self.const_u32(0)?;
        let one = self.const_u32(1)?;
        let add = |e: &mut Self, a: u32, b: u32| -> R<u32> {
            if a == zero {
                Ok(b)
            } else if b == zero {
                Ok(a)
            } else {
                e.u32op(OpIAdd, &[a, b])
            }
        };
        let mul = |e: &mut Self, a: u32, b: u32| -> R<u32> {
            if a == zero || b == zero {
                Ok(zero)
            } else if a == one {
                Ok(b)
            } else if b == one {
                Ok(a)
            } else {
                e.u32op(OpIMul, &[a, b])
            }
        };
        if immediate != 0 {
            let k = self.const_u32(immediate)?;
            offset = add(self, offset, k)?;
        }
        let stride_id = self.const_u32(stride)?;
        let scaled = mul(self, index, stride_id)?;
        let mut address = add(self, scaled, offset)?;
        if swizzle {
            let k3 = self.const_u32(3)?;
            let index_stride_id = self.const_u32(index_stride)?;
            let index_shift = self.u32op(OpIAdd, &[index_stride_id, k3])?;
            let indices = self.u32op(OpShiftLeftLogical, &[one, index_shift])?;
            let index_msb = self.u32op(OpShiftRightLogical, &[index, index_shift])?;
            let indices_minus = self.u32op(OpISub, &[indices, one])?;
            let index_lsb = self.u32op(OpBitwiseAnd, &[index, indices_minus])?;
            let offset_msb = self.and_const(offset, !3)?;
            let offset_lsb = self.and_const(offset, 3)?;
            let msb_scaled = mul(self, index_msb, stride_id)?;
            let msb_sum = add(self, msb_scaled, offset_msb)?;
            let msb = mul(self, msb_sum, indices)?;
            let k2 = self.const_u32(2)?;
            let lsb_shift = self.u32op(OpShiftLeftLogical, &[index_lsb, k2])?;
            let lsb = add(self, lsb_shift, offset_lsb)?;
            let swizzled = add(self, msb, lsb)?;
            let enabled = self.const_bool(true)?;
            address = self.select_u32(enabled, swizzled, address)?;
        }
        let byte = add(self, address, soffset)?;
        // Source ByteAddress: the host binding prefix must not wrap an
        // out-of-range guest address into the buffer.
        let prefixed = self.u32op(OpIAdd, &[byte, prefix])?;
        let no_wrap = self.boolop(OpUGreaterThanEqual, &[prefixed, prefix])?;
        let max = self.const_u32(u32::MAX)?;
        let guarded = self.select_u32(no_wrap, prefixed, max)?;
        self.shr_const(guarded, 2)
    }

    fn load_word_prepared(
        &mut self,
        id: InstId,
        resource: u32,
        immediate: u32,
        access: (u32, u32, u32),
    ) -> R<u32> {
        let index = self.buffer_dword_index(id, resource, immediate, access.1)?;
        let in_bounds = self.boolop(OpULessThan, &[index, access.2])?;
        let object = access.0;
        self.value_or_zero_if(in_bounds, |e| {
            let pointer = e.element_pointer(object, index)?;
            e.load_with_access(pointer, id)
        })
    }

    fn coherent(&self, id: InstId) -> bool {
        self.ir
            .memory
            .get(self.ir.inst(id).aux as usize)
            .is_some_and(|m| m.coherent)
    }

    fn load_with_access(&mut self, pointer: u32, id: InstId) -> R<u32> {
        let u = self.ty_u32()?;
        let result = self.b.alloc_id()?;
        if self.coherent(id) {
            self.b
                .func(OpLoad, &[u, result, pointer, MemoryAccessVolatileMask])?;
        } else {
            self.b.func(OpLoad, &[u, result, pointer])?;
        }
        Ok(result)
    }

    fn emit_buffer_load(&mut self, id: InstId, components: u32) -> R<u32> {
        let memory = *self
            .ir
            .memory
            .get(self.ir.inst(id).aux as usize)
            .ok_or(CompileError::Internal("memory info"))?;
        let exec = self.arg(id, 4)?;
        if components == 1 {
            return self.value_or_zero_if(exec, |e| {
                let access = e.buffer_access(memory.resource)?;
                e.load_word_prepared(id, memory.resource, memory.offset, access)
            });
        }
        let ty = self.ty_u32_composite(components)?;
        let zero = self.const_u32_composite_zero(components)?;
        self.value_or_default_if(exec, ty, zero, |e| {
            let access = e.buffer_access(memory.resource)?;
            let mut values = [0u32; 4];
            for component in 0..components {
                values[component as usize] = e.load_word_prepared(
                    id,
                    memory.resource,
                    memory.offset.wrapping_add(component * 4),
                    access,
                )?;
            }
            let ty = e.ty_u32_composite(components)?;
            e.op(OpCompositeConstruct, ty, &values[..components as usize])
        })
    }

    fn emit_buffer_store(&mut self, id: InstId, components: u32) -> R<()> {
        let memory = *self
            .ir
            .memory
            .get(self.ir.inst(id).aux as usize)
            .ok_or(CompileError::Internal("memory info"))?;
        let exec = self.arg(id, 5)?;
        let data = self.arg(id, 4)?;
        self.if_condition(exec, |e| {
            let access = e.buffer_access(memory.resource)?;
            for component in 0..components {
                let word = if components == 1 {
                    data
                } else {
                    e.u32op(OpCompositeExtract, &[data, component])?
                };
                let index = e.buffer_dword_index(
                    id,
                    memory.resource,
                    memory.offset.wrapping_add(component * 4),
                    access.1,
                )?;
                let in_bounds = e.boolop(OpULessThan, &[index, access.2])?;
                let coherent = e.coherent(id);
                e.if_condition(in_bounds, |e| {
                    let pointer = e.element_pointer(access.0, index)?;
                    if coherent {
                        e.b.func(OpStore, &[pointer, word, MemoryAccessVolatileMask])
                    } else {
                        e.b.func(OpStore, &[pointer, word])
                    }
                })?;
            }
            Ok(())
        })
    }

    /// Source `EmitReadConstBuffer` (explicit bounds check).
    fn emit_read_const_buffer(&mut self, id: InstId) -> R<u32> {
        let memory = *self
            .ir
            .memory
            .get(self.ir.inst(id).aux as usize)
            .ok_or(CompileError::Internal("memory info"))?;
        let base = self.arg(id, 1)?;
        let immediate = self.const_u32(memory.offset)?;
        let address = self.u32op(OpIAdd, &[base, immediate])?;
        let index = self.shr_const(address, 2)?;
        let (object, byte_offset, length) = self.buffer_access(memory.resource)?;
        let base_dwords = self.shr_const(byte_offset, 2)?;
        let element = self.u32op(OpIAdd, &[index, base_dwords])?;
        let in_bounds = self.boolop(OpULessThan, &[element, length])?;
        self.value_or_zero_if(in_bounds, |e| {
            let pointer = e.element_pointer(object, element)?;
            e.u32op(OpLoad, &[pointer])
        })
    }

    // ----------------------------------------------------------- export

    /// Source `EmitSetAttribute` / `ExportVector` for uncompressed exports.
    fn emit_set_attribute(&mut self, id: InstId) -> R<()> {
        let exp = *self
            .ir
            .exports
            .get(self.ir.inst(id).aux as usize)
            .ok_or(CompileError::Internal("export info"))?;
        if self.profile.stage() == ShaderStage::Pixel && exp.valid_mask {
            return Err(CompileError::unsupported(
                exp.pc,
                Some(pentax_shader::Opcode::Exp),
                UnsupportedReason::PixelValidMask,
            ));
        }
        if exp.kind == ExportKind::Null || exp.enabled == 0 {
            return Ok(());
        }
        let output_kind = match exp.kind {
            ExportKind::Position => OutputKind::Position,
            ExportKind::Parameter => OutputKind::Parameter,
            ExportKind::Mrt => OutputKind::Mrt,
            _ => return Ok(()),
        };
        let Some(&(output, variable)) = self
            .outputs
            .iter()
            .find(|(o, _)| o.kind == output_kind && o.index == exp.index)
        else {
            // Source: an MRT export without a colour target writes nothing.
            return Ok(());
        };
        let exec = self.arg(id, 1)?;
        let data = self.arg(id, 0)?;
        self.if_condition(exec, |e| {
            let uint = output.uint;
            let default_w = if uint { 1 } else { 0x3f80_0000 };
            let mut raw = [0u32; 4];
            for (component, slot) in raw.iter_mut().enumerate() {
                *slot = if exp.enabled >> component & 1 != 0 {
                    e.u32op(OpCompositeExtract, &[data, component as u32])?
                } else {
                    e.const_u32(if component == 3 { default_w } else { 0 })?
                };
            }
            let value = if uint {
                let ty = e.ty_uvec(4)?;
                e.op(OpCompositeConstruct, ty, &raw)?
            } else {
                let mut f = [0u32; 4];
                for (component, slot) in f.iter_mut().enumerate() {
                    *slot = e.bitcast_to_f32(raw[component])?;
                }
                let ty = e.ty_fvec(4)?;
                e.op(OpCompositeConstruct, ty, &f)?
            };
            if output.kind == OutputKind::Position {
                let v4 = e.ty_fvec(4)?;
                let pointer_ty = e.ty_pointer(StorageClassOutput, v4)?;
                let zero = e.const_u32(0)?;
                let pointer = e.op(OpAccessChain, pointer_ty, &[variable, zero])?;
                e.b.func(OpStore, &[pointer, value])
            } else {
                e.b.func(OpStore, &[variable, value])
            }
        })
    }

    // ------------------------------------------------------- dispatcher

    /// Source `EmitDirectInstruction` for the ported operations.
    fn emit_inst(&mut self, id: InstId) -> R<()> {
        let inst = *self.ir.inst(id);
        let result = match inst.op {
            Op::Phi => {
                let ty = self.type_id(inst.ty)?;
                let result = self.b.alloc_id()?;
                let phi = self.b.deferred_phi(ty, result, self.ir.nargs(id))?;
                self.b.budget().push(
                    &mut self.deferred,
                    (phi, id),
                    usize::MAX,
                    LimitKind::Bytes,
                )?;
                result
            }
            Op::Identity => self.arg(id, 0)?,
            Op::Undef => {
                let ty = self.type_id(inst.ty)?;
                self.op(OpUndef, ty, &[])?
            }
            Op::GetUserData => {
                let index = self
                    .layout
                    .user_data_registers
                    .iter()
                    .position(|&r| u32::from(r) == inst.aux)
                    .ok_or(CompileError::Internal("user data register layout"))?;
                self.shader_data_dword(index as u32)?
            }
            Op::GetBuiltin => {
                let kind = Builtin::from_code(inst.aux >> 8)
                    .ok_or(CompileError::Internal("builtin kind"))?;
                self.builtin_u32(kind, inst.aux & 0xff)?
            }
            Op::BitCastU32F32 => {
                let a = self.arg(id, 0)?;
                self.bitcast_to_u32(a)?
            }
            Op::BitCastF32U32 => {
                let a = self.arg(id, 0)?;
                self.bitcast_to_f32(a)?
            }
            Op::ConvertS32F32 => {
                let a = self.arg(id, 0)?;
                let i = self.ty_i32()?;
                let converted = self.op(OpConvertFToS, i, &[a])?;
                self.bitcast_to_u32(converted)?
            }
            Op::ConvertU32F32 => {
                let a = self.arg(id, 0)?;
                self.u32op(OpConvertFToU, &[a])?
            }
            Op::ConvertF32S32 => {
                let a = self.arg(id, 0)?;
                let i = self.ty_i32()?;
                let signed = self.op(OpBitcast, i, &[a])?;
                self.f32op(OpConvertSToF, &[signed])?
            }
            Op::ConvertF32U32 => {
                let a = self.arg(id, 0)?;
                self.f32op(OpConvertUToF, &[a])?
            }
            Op::CompositeConstructU64 => {
                let low = self.arg(id, 0)?;
                let u64t = self.ty_u64()?;
                if self.ir.resolve(self.b.budget_ref(), self.ir.arg(id, 1))? == Value::U32(0) {
                    self.b.require_capability(CapabilityInt64)?;
                    self.op(OpUConvert, u64t, &[low])?
                } else {
                    let high = self.arg(id, 1)?;
                    let v2 = self.ty_uvec(2)?;
                    let pair = self.op(OpCompositeConstruct, v2, &[low, high])?;
                    self.op(OpBitcast, u64t, &[pair])?
                }
            }
            Op::CompositeConstructU32x2
            | Op::CompositeConstructU32x3
            | Op::CompositeConstructU32x4
            | Op::CompositeConstructF32x2 => {
                let ty = self.type_id(inst.ty)?;
                let mut parts = [0u32; 4];
                let n = self.ir.nargs(id);
                for (index, part) in parts.iter_mut().enumerate().take(n) {
                    *part = self.arg(id, index)?;
                }
                self.op(OpCompositeConstruct, ty, &parts[..n])?
            }
            Op::CompositeExtractU64 => {
                let value = self.arg(id, 0)?;
                let shifted = if inst.aux == 0 {
                    value
                } else {
                    let u64t = self.ty_u64()?;
                    let k = self.const_u32(32)?;
                    self.op(OpShiftRightLogical, u64t, &[value, k])?
                };
                self.u32op(OpUConvert, &[shifted])?
            }
            Op::CompositeExtractU32x2 | Op::CompositeExtractU32x3 | Op::CompositeExtractU32x4 => {
                let value = self.arg(id, 0)?;
                self.u32op(OpCompositeExtract, &[value, inst.aux])?
            }
            Op::PackHalf2x16 | Op::PackSnorm2x16 | Op::PackUnorm2x16 => {
                let a = self.arg(id, 0)?;
                let u = self.ty_u32()?;
                let instruction = match inst.op {
                    Op::PackHalf2x16 => GLSLstd450PackHalf2x16,
                    Op::PackSnorm2x16 => GLSLstd450PackSnorm2x16,
                    _ => GLSLstd450PackUnorm2x16,
                };
                self.ext(u, instruction, &[a])?
            }
            Op::PackFloat2x16Rtz => {
                let x = self.arg(id, 0)?;
                let y = self.arg(id, 1)?;
                let low = self.f32_to_f16_rtz_bits(x)?;
                let high_bits = self.f32_to_f16_rtz_bits(y)?;
                let k16 = self.const_u32(16)?;
                let high = self.u32op(OpShiftLeftLogical, &[high_bits, k16])?;
                self.u32op(OpBitwiseOr, &[low, high])?
            }
            Op::FPAbs32 => {
                let a = self.arg(id, 0)?;
                self.ext_f32(GLSLstd450FAbs, &[a])?
            }
            Op::FPNeg32 => {
                let a = self.arg(id, 0)?;
                self.f32op(OpFNegate, &[a])?
            }
            Op::FPSaturate32 => {
                let a = self.arg(id, 0)?;
                let zero = self.const_f32(0)?;
                let one = self.const_f32(0x3f80_0000)?;
                self.ext_f32(GLSLstd450FClamp, &[a, zero, one])?
            }
            Op::BitFieldInsert => {
                let mut a = [0u32; 4];
                for (index, slot) in a.iter_mut().enumerate() {
                    *slot = self.arg(id, index)?;
                }
                self.u32op(OpBitFieldInsert, &a)?
            }
            Op::BitFieldUExtract | Op::BitFieldSExtract => {
                let mut a = [0u32; 3];
                for (index, slot) in a.iter_mut().enumerate() {
                    *slot = self.arg(id, index)?;
                }
                let op = if inst.op == Op::BitFieldUExtract {
                    OpBitFieldUExtract
                } else {
                    OpBitFieldSExtract
                };
                self.u32op(op, &a)?
            }
            Op::SelectU1 | Op::SelectU32 | Op::SelectF32 => {
                let ty = self.type_id(inst.ty)?;
                let c = self.arg(id, 0)?;
                let t = self.arg(id, 1)?;
                let f = self.arg(id, 2)?;
                self.op(OpSelect, ty, &[c, t, f])?
            }
            Op::IAdd32
            | Op::ISub32
            | Op::IMul32
            | Op::UDiv32
            | Op::BitwiseAnd32
            | Op::BitwiseOr32
            | Op::BitwiseXor32
            | Op::ShiftLeftLogical32
            | Op::ShiftRightLogical32
            | Op::ShiftRightArithmetic32 => {
                let op = match inst.op {
                    Op::IAdd32 => OpIAdd,
                    Op::ISub32 => OpISub,
                    Op::IMul32 => OpIMul,
                    Op::UDiv32 => OpUDiv,
                    Op::BitwiseAnd32 => OpBitwiseAnd,
                    Op::BitwiseOr32 => OpBitwiseOr,
                    Op::BitwiseXor32 => OpBitwiseXor,
                    Op::ShiftLeftLogical32 => OpShiftLeftLogical,
                    Op::ShiftRightLogical32 => OpShiftRightLogical,
                    _ => OpShiftRightArithmetic,
                };
                let a = self.arg(id, 0)?;
                let b = self.arg(id, 1)?;
                self.u32op(op, &[a, b])?
            }
            Op::IAdd64 | Op::ISub64 | Op::BitwiseAnd64 => {
                let op = match inst.op {
                    Op::IAdd64 => OpIAdd,
                    Op::ISub64 => OpISub,
                    _ => OpBitwiseAnd,
                };
                let ty = self.ty_u64()?;
                let a = self.arg(id, 0)?;
                let b = self.arg(id, 1)?;
                self.op(op, ty, &[a, b])?
            }
            Op::IAddCarry32 => {
                let ty = self.ty_u32_pair()?;
                let a = self.arg(id, 0)?;
                let b = self.arg(id, 1)?;
                self.op(OpIAddCarry, ty, &[a, b])?
            }
            Op::SMulHi | Op::UMulHi => {
                let a = self.arg(id, 0)?;
                let b = self.arg(id, 1)?;
                self.mul_high(a, b, inst.op == Op::SMulHi)?
            }
            Op::IAbs32 => {
                let value = self.arg(id, 0)?;
                let neg = self.u32op(OpSNegate, &[value])?;
                let zero = self.const_u32(0)?;
                let negative = self.boolop(OpSLessThan, &[value, zero])?;
                self.select_u32(negative, neg, value)?
            }
            Op::ShiftLeftLogical64 | Op::ShiftRightLogical64 | Op::ShiftRightArithmetic64 => {
                // Source EmitShift64: the count is masked to 63.
                let op = match inst.op {
                    Op::ShiftLeftLogical64 => OpShiftLeftLogical,
                    Op::ShiftRightLogical64 => OpShiftRightLogical,
                    _ => OpShiftRightArithmetic,
                };
                let value = self.arg(id, 0)?;
                let ty = self.ty_u64()?;
                match self.ir.resolve(self.b.budget_ref(), self.ir.arg(id, 1))? {
                    Value::U32(amount) => {
                        let amount = amount & 63;
                        if amount == 0 {
                            value
                        } else {
                            let k = self.const_u32(amount)?;
                            self.op(op, ty, &[value, k])?
                        }
                    }
                    _ => {
                        let shift = self.arg(id, 1)?;
                        let masked = self.and_const(shift, 63)?;
                        self.op(op, ty, &[value, masked])?
                    }
                }
            }
            Op::BitwiseNot32 => {
                let a = self.arg(id, 0)?;
                self.u32op(OpNot, &[a])?
            }
            Op::BitReverse32 => {
                let a = self.arg(id, 0)?;
                self.u32op(OpBitReverse, &[a])?
            }
            Op::BitCount32 => {
                let a = self.arg(id, 0)?;
                self.u32op(OpBitCount, &[a])?
            }
            Op::BitCount64 => {
                let a = self.arg(id, 0)?;
                let v2 = self.ty_uvec(2)?;
                let unpacked = self.op(OpBitcast, v2, &[a])?;
                let counts = self.op(OpBitCount, v2, &[unpacked])?;
                let low = self.u32op(OpCompositeExtract, &[counts, 0])?;
                let high = self.u32op(OpCompositeExtract, &[counts, 1])?;
                self.u32op(OpIAdd, &[low, high])?
            }
            Op::FindILsb32 | Op::FindUMsb32 => {
                let a = self.arg(id, 0)?;
                let i = self.ty_i32()?;
                let instruction = if inst.op == Op::FindILsb32 {
                    GLSLstd450FindILsb
                } else {
                    GLSLstd450FindUMsb
                };
                let value = self.ext(i, instruction, &[a])?;
                self.bitcast_to_u32(value)?
            }
            Op::FindUMsb64 => {
                let a = self.arg(id, 0)?;
                let v2 = self.ty_uvec(2)?;
                let unpacked = self.op(OpBitcast, v2, &[a])?;
                let msb = self.ext(v2, GLSLstd450FindUMsb, &[unpacked])?;
                let low = self.u32op(OpCompositeExtract, &[msb, 0])?;
                let high = self.u32op(OpCompositeExtract, &[msb, 1])?;
                let found = self.cmp_const(OpINotEqual, high, u32::MAX)?;
                let k32 = self.const_u32(32)?;
                let high32 = self.u32op(OpIAdd, &[high, k32])?;
                self.select_u32(found, high32, low)?
            }
            Op::SMin32 | Op::SMax32 | Op::UMin32 | Op::UMax32 => {
                let a = self.arg(id, 0)?;
                let b = self.arg(id, 1)?;
                let signed = matches!(inst.op, Op::SMin32 | Op::SMax32);
                let max = matches!(inst.op, Op::SMax32 | Op::UMax32);
                self.int_min_max(a, b, signed, max)?
            }
            Op::SMinTri32 | Op::SMaxTri32 | Op::UMinTri32 | Op::UMaxTri32 => {
                let a = self.arg(id, 0)?;
                let b = self.arg(id, 1)?;
                let c = self.arg(id, 2)?;
                let signed = matches!(inst.op, Op::SMinTri32 | Op::SMaxTri32);
                let max = matches!(inst.op, Op::SMaxTri32 | Op::UMaxTri32);
                let ab = self.int_min_max(a, b, signed, max)?;
                self.int_min_max(ab, c, signed, max)?
            }
            Op::SMedTri32 | Op::UMedTri32 => {
                // Source EmitMed3: a + b + c - min3 - max3.
                let a = self.arg(id, 0)?;
                let b = self.arg(id, 1)?;
                let c = self.arg(id, 2)?;
                let signed = inst.op == Op::SMedTri32;
                let min_ab = self.int_min_max(a, b, signed, false)?;
                let minimum = self.int_min_max(min_ab, c, signed, false)?;
                let max_ab = self.int_min_max(a, b, signed, true)?;
                let maximum = self.int_min_max(max_ab, c, signed, true)?;
                let ab = self.u32op(OpIAdd, &[a, b])?;
                let abc = self.u32op(OpIAdd, &[ab, c])?;
                let less = self.u32op(OpISub, &[abc, minimum])?;
                self.u32op(OpISub, &[less, maximum])?
            }
            Op::SLessThan32
            | Op::SLessThan64
            | Op::SLessThanEqual64
            | Op::ULessThanEqual64
            | Op::UGreaterThanEqual64
            | Op::ULessThan32
            | Op::ULessThan64
            | Op::IEqual32
            | Op::IEqual64
            | Op::SLessThanEqual32
            | Op::ULessThanEqual32
            | Op::SGreaterThan32
            | Op::UGreaterThan32
            | Op::UGreaterThan64
            | Op::INotEqual32
            | Op::INotEqual64
            | Op::SGreaterThanEqual32
            | Op::UGreaterThanEqual32 => {
                let op = match inst.op {
                    Op::SLessThan32 | Op::SLessThan64 => OpSLessThan,
                    Op::SLessThanEqual64 | Op::SLessThanEqual32 => OpSLessThanEqual,
                    Op::ULessThanEqual64 | Op::ULessThanEqual32 => OpULessThanEqual,
                    Op::UGreaterThanEqual64 | Op::UGreaterThanEqual32 => OpUGreaterThanEqual,
                    Op::ULessThan32 | Op::ULessThan64 => OpULessThan,
                    Op::IEqual32 | Op::IEqual64 => OpIEqual,
                    Op::SGreaterThan32 => OpSGreaterThan,
                    Op::UGreaterThan32 | Op::UGreaterThan64 => OpUGreaterThan,
                    Op::INotEqual32 | Op::INotEqual64 => OpINotEqual,
                    _ => OpSGreaterThanEqual,
                };
                let a = self.arg(id, 0)?;
                let b = self.arg(id, 1)?;
                self.boolop(op, &[a, b])?
            }
            Op::LogicalOr | Op::LogicalAnd | Op::LogicalXor => {
                let op = match inst.op {
                    Op::LogicalOr => OpLogicalOr,
                    Op::LogicalAnd => OpLogicalAnd,
                    _ => OpLogicalNotEqual,
                };
                let a = self.arg(id, 0)?;
                let b = self.arg(id, 1)?;
                self.boolop(op, &[a, b])?
            }
            Op::LogicalNot => {
                let a = self.arg(id, 0)?;
                self.boolop(OpLogicalNot, &[a])?
            }
            Op::FPOrdEqual32
            | Op::FPUnordEqual32
            | Op::FPOrdNotEqual32
            | Op::FPUnordNotEqual32
            | Op::FPOrdLessThan32
            | Op::FPUnordLessThan32
            | Op::FPOrdGreaterThan32
            | Op::FPUnordGreaterThan32
            | Op::FPOrdLessThanEqual32
            | Op::FPUnordLessThanEqual32
            | Op::FPOrdGreaterThanEqual32
            | Op::FPUnordGreaterThanEqual32 => {
                let op = match inst.op {
                    Op::FPOrdEqual32 => OpFOrdEqual,
                    Op::FPUnordEqual32 => OpFUnordEqual,
                    Op::FPOrdNotEqual32 => OpFOrdNotEqual,
                    Op::FPUnordNotEqual32 => OpFUnordNotEqual,
                    Op::FPOrdLessThan32 => OpFOrdLessThan,
                    Op::FPUnordLessThan32 => OpFUnordLessThan,
                    Op::FPOrdGreaterThan32 => OpFOrdGreaterThan,
                    Op::FPUnordGreaterThan32 => OpFUnordGreaterThan,
                    Op::FPOrdLessThanEqual32 => OpFOrdLessThanEqual,
                    Op::FPUnordLessThanEqual32 => OpFUnordLessThanEqual,
                    Op::FPOrdGreaterThanEqual32 => OpFOrdGreaterThanEqual,
                    _ => OpFUnordGreaterThanEqual,
                };
                // Source EmitFloatCompare32 with FPCompareFlags::flush_input_denorms.
                let flush = inst.aux & 1 != 0;
                let mut operands = [0u32; 2];
                for (index, slot) in operands.iter_mut().enumerate() {
                    *slot = match (flush, self.arg_bits(id, index)?) {
                        (true, Some(bits)) => {
                            let flushed = if bits & 0x7fff_ffff < 0x0080_0000 {
                                bits & 0x8000_0000
                            } else {
                                bits
                            };
                            self.const_f32(flushed)?
                        }
                        (true, None) => {
                            let value = self.arg(id, index)?;
                            self.flush_denorm(value)?
                        }
                        (false, _) => self.arg(id, index)?,
                    };
                }
                self.boolop(op, &operands)?
            }
            Op::FPIsNan32 => {
                let a = self.arg(id, 0)?;
                self.boolop(OpFUnordNotEqual, &[a, a])?
            }
            Op::FPCmpClass32 => {
                let value = self.arg(id, 0)?;
                let mask = self.arg(id, 1)?;
                self.class_mask_f32(value, mask)?
            }
            Op::FPAdd32 => self.float_binary(id, OpFAdd)?,
            Op::FPSub32 => self.float_binary(id, OpFSub)?,
            Op::FPMul32 => self.float_binary(id, OpFMul)?,
            Op::FPFma32 => {
                let a = self.arg(id, 0)?;
                let b = self.arg(id, 1)?;
                let c = self.arg(id, 2)?;
                self.ext_f32(GLSLstd450Fma, &[a, b, c])?
            }
            Op::FPMad32 => {
                let a = self.arg(id, 0)?;
                let b = self.arg(id, 1)?;
                let c = self.arg(id, 2)?;
                if !self.no_contraction(id) {
                    self.ext_f32(GLSLstd450Fma, &[a, b, c])?
                } else {
                    let product = self.f32op(OpFMul, &[a, b])?;
                    self.b
                        .annotate(OpDecorate, &[product, DecorationNoContraction])?;
                    let sum = self.f32op(OpFAdd, &[product, c])?;
                    self.b
                        .annotate(OpDecorate, &[sum, DecorationNoContraction])?;
                    sum
                }
            }
            Op::FPMin32 | Op::FPMax32 => {
                let a = self.f32_operand(id, 0)?;
                let b = self.f32_operand(id, 1)?;
                self.min_max_f32(a, b, inst.op == Op::FPMax32)?.0
            }
            Op::FPMinTri32 | Op::FPMaxTri32 => {
                let max = inst.op == Op::FPMaxTri32;
                let a = self.f32_operand(id, 0)?;
                let b = self.f32_operand(id, 1)?;
                let c = self.f32_operand(id, 2)?;
                let ab = self.min_max_f32(a, b, max)?;
                self.min_max_f32(ab, c, max)?.0
            }
            Op::FPMedTri32 => self.med3_f32(id)?,
            Op::FPRecip32 => {
                let a = self.arg(id, 0)?;
                let source = self.flush_denorm(a)?;
                let one = self.const_f32(0x3f80_0000)?;
                self.f32op(OpFDiv, &[one, source])?
            }
            Op::FPRecipIFlag32 => {
                let a = self.arg(id, 0)?;
                let one = self.const_f32(0x3f80_0000)?;
                self.f32op(OpFDiv, &[one, a])?
            }
            Op::FPRecipSqrt32 | Op::FPSqrt | Op::FPExp2 | Op::FPLog2 => {
                let instruction = match inst.op {
                    Op::FPRecipSqrt32 => GLSLstd450InverseSqrt,
                    Op::FPSqrt => GLSLstd450Sqrt,
                    Op::FPExp2 => GLSLstd450Exp2,
                    _ => GLSLstd450Log2,
                };
                let a = self.arg(id, 0)?;
                let source = self.flush_denorm(a)?;
                self.ext_f32(instruction, &[source])?
            }
            Op::FPLdexp => {
                let a = self.arg(id, 0)?;
                let b = self.arg(id, 1)?;
                let i = self.ty_i32()?;
                let exponent = self.op(OpBitcast, i, &[b])?;
                self.ext_f32(GLSLstd450Ldexp, &[a, exponent])?
            }
            Op::FPSin | Op::FPCos => {
                let a = self.arg(id, 0)?;
                let sin = inst.op == Op::FPSin;
                let cycle = self.trig_cycle(a, sin)?;
                let two_pi = self.const_f32(0x40c9_0fdb)?;
                let scaled = self.f32op(OpFMul, &[cycle, two_pi])?;
                self.ext_f32(if sin { GLSLstd450Sin } else { GLSLstd450Cos }, &[scaled])?
            }
            Op::FPRoundEven32 | Op::FPFloor32 | Op::FPCeil32 | Op::FPTrunc32 | Op::FPFract32 => {
                let instruction = match inst.op {
                    Op::FPRoundEven32 => GLSLstd450RoundEven,
                    Op::FPFloor32 => GLSLstd450Floor,
                    Op::FPCeil32 => GLSLstd450Ceil,
                    Op::FPTrunc32 => GLSLstd450Trunc,
                    _ => GLSLstd450Fract,
                };
                let a = self.arg(id, 0)?;
                self.ext_f32(instruction, &[a])?
            }
            Op::LaneId => {
                if self.local_lane_var != 0 {
                    let index = self.u32op(OpLoad, &[self.local_lane_var])?;
                    self.and_const(index, 63)?
                } else {
                    if self.subgroup_lane_var == 0 {
                        return Err(CompileError::Internal(
                            "subgroup lane input was not declared",
                        ));
                    }
                    self.u32op(OpLoad, &[self.subgroup_lane_var])?
                }
            }
            Op::Ballot => {
                let a = self.arg(id, 0)?;
                let ty = self.ty_uvec(4)?;
                let scope = self.const_u32(ScopeSubgroup)?;
                self.op(OpGroupNonUniformBallot, ty, &[scope, a])?
            }
            Op::AnyLane => {
                // Source EmitAnyLane: (ballot.x | ballot.y) != 0.
                let a = self.arg(id, 0)?;
                let ty = self.ty_uvec(4)?;
                let scope = self.const_u32(ScopeSubgroup)?;
                let ballot = self.op(OpGroupNonUniformBallot, ty, &[scope, a])?;
                let low = self.u32op(OpCompositeExtract, &[ballot, 0])?;
                let high = self.u32op(OpCompositeExtract, &[ballot, 1])?;
                let either = self.u32op(OpBitwiseOr, &[low, high])?;
                self.cmp_const(OpINotEqual, either, 0)?
            }
            Op::GetBufferResource | Op::GetImageResource | Op::GetSamplerResource => 0,
            Op::ImageSampleImplicit
            | Op::ImageSampleLod
            | Op::ImageQueryLod
            | Op::ImageSampleImplicitFeedback
            | Op::ImageSampleLodFeedback => self.emit_image(id)?,
            Op::CompositeExtractF32x2 | Op::CompositeExtractF32x4 => {
                let value = self.arg(id, 0)?;
                let ty = self.ty_f32()?;
                self.op(OpCompositeExtract, ty, &[value, inst.aux])?
            }
            Op::ReadConstBuffer => self.emit_read_const_buffer(id)?,
            Op::LoadSharedU32 => self.emit_shared_load(id)?,
            Op::StoreSharedU32 => {
                self.emit_shared_store(id)?;
                0
            }
            Op::SharedCompletion => {
                self.shared_completion()?;
                0
            }
            Op::LoadBufferU32 => self.emit_buffer_load(id, 1)?,
            Op::LoadBufferU32x2 => self.emit_buffer_load(id, 2)?,
            Op::LoadBufferU32x3 => self.emit_buffer_load(id, 3)?,
            Op::LoadBufferU32x4 => self.emit_buffer_load(id, 4)?,
            Op::StoreBufferU32 => {
                self.emit_buffer_store(id, 1)?;
                0
            }
            Op::StoreBufferU32x2 => {
                self.emit_buffer_store(id, 2)?;
                0
            }
            Op::StoreBufferU32x3 => {
                self.emit_buffer_store(id, 3)?;
                0
            }
            Op::StoreBufferU32x4 => {
                self.emit_buffer_store(id, 4)?;
                0
            }
            Op::GetAttribute | Op::GetInterpolationParameter => {
                self.attribute(inst.aux & 0xff, (inst.aux >> 8) & 0xff)?
            }
            Op::SetAttribute => {
                self.emit_set_attribute(id)?;
                0
            }
            Op::ControlNop => 0,
            Op::StoreCompletion => {
                let execution = self.const_u32(ScopeSubgroup)?;
                let memory = self.const_u32(ScopeDevice)?;
                let semantics = self.const_u32(
                    MemorySemanticsReleaseMask
                        | MemorySemanticsUniformMemoryMask
                        | MemorySemanticsImageMemoryMask,
                )?;
                self.b
                    .func(OpControlBarrier, &[execution, memory, semantics])?;
                0
            }
            Op::Barrier => {
                let workgroup = self.const_u32(ScopeWorkgroup)?;
                let semantics = self.const_u32(
                    MemorySemanticsAcquireReleaseMask
                        | MemorySemanticsWorkgroupMemoryMask
                        | if self.profile.stage() == ShaderStage::Compute {
                            MemorySemanticsUniformMemoryMask
                        } else {
                            0
                        },
                )?;
                self.b
                    .func(OpControlBarrier, &[workgroup, workgroup, semantics])?;
                0
            }
            _ => {
                return Err(CompileError::unsupported(
                    inst.pc,
                    None,
                    UnsupportedReason::EmitterValue,
                ));
            }
        };
        if let Some(slot) = self.ids.get_mut(id.index()) {
            *slot = result;
        }
        Ok(())
    }

    /// Source `EmitStructuredTerminator` (single-lane `BranchCondition`).
    fn emit_terminator(&mut self, block: u32) -> R<()> {
        let info = self.ir.blocks[block as usize];
        let term = info.term;
        let label_of =
            |e: &Self, target: u32| e.labels.get(target as usize).copied().filter(|&l| l != 0);
        let merge = |e: &mut Self| -> R<()> {
            if term.loop_header {
                if let (Some(merge), Some(cont)) = (
                    label_of(e, term.merge_block),
                    label_of(e, term.continue_block),
                ) {
                    e.b.func(OpLoopMerge, &[merge, cont, LoopControlMaskNone])?;
                }
            } else if term.kind == TerminatorKind::ConditionalBranch
                && term.merge_block != NIL
                && let Some(merge) = label_of(e, term.merge_block)
            {
                e.b.func(OpSelectionMerge, &[merge, SelectionControlMaskNone])?;
            }
            Ok(())
        };
        match term.kind {
            TerminatorKind::Branch => {
                let Some(target) = label_of(self, term.true_block) else {
                    return self.b.func(OpReturn, &[]);
                };
                merge(self)?;
                self.b.func(OpBranch, &[target])
            }
            TerminatorKind::ConditionalBranch => {
                let (Some(t), Some(f), Some(condition)) = (
                    label_of(self, term.true_block),
                    label_of(self, term.false_block),
                    info.condition,
                ) else {
                    return self.b.func(OpReturn, &[]);
                };
                if !term.loop_header && term.true_block == term.false_block {
                    return self.b.func(OpBranch, &[t]);
                }
                let condition = self.def(condition)?;
                merge(self)?;
                self.b.func(OpBranchConditional, &[condition, t, f])
            }
            TerminatorKind::Return => self.b.func(OpReturn, &[]),
        }
    }

    // ------------------------------------------------------- definitions

    fn define_interface(&mut self, ty: u32, storage: u32, name: &str) -> R<u32> {
        let pointer = self.ty_pointer(storage, ty)?;
        let variable = self.b.global_variable(pointer, storage)?;
        let limit = PIXEL_INPUTS * 4;
        self.b
            .budget()
            .push(&mut self.interface, variable, limit, LimitKind::SourceTable)?;
        self.b.name(variable, name)?;
        Ok(variable)
    }

    /// Source `DefineInputs`.
    fn define_inputs(&mut self, uses_lane: bool, logical_lane: bool) -> R<()> {
        let pixel = self.profile.stage() == ShaderStage::Pixel;
        for index in 0..self.layout.inputs.len() {
            let input = self.layout.inputs[index];
            if let InputKind::Parameter = input.kind
                && let Some(&(_, variable)) = self.inputs.iter().find(|(other, _)| {
                    other.kind == InputKind::Parameter && other.location == input.location
                })
            {
                self.b.budget().push(
                    &mut self.inputs,
                    (input, variable),
                    128,
                    LimitKind::SourceTable,
                )?;
                continue;
            }
            let (ty, name, builtin) = match input.kind {
                InputKind::Builtin(kind) => match kind {
                    Builtin::VertexIndex => (self.ty_i32()?, "gl_VertexIndex", BuiltInVertexIndex),
                    Builtin::InstanceIndex => {
                        (self.ty_i32()?, "gl_InstanceIndex", BuiltInInstanceIndex)
                    }
                    Builtin::BaseInstance => {
                        self.b.require_capability(CapabilityDrawParameters)?;
                        (self.ty_i32()?, "gl_BaseInstance", BuiltInBaseInstance)
                    }
                    Builtin::FragCoord => (self.ty_fvec(4)?, "gl_FragCoord", BuiltInFragCoord),
                    Builtin::FrontFacing => (self.ty_bool()?, "gl_FrontFacing", BuiltInFrontFacing),
                    Builtin::WorkgroupId => {
                        (self.ty_uvec(3)?, "gl_WorkGroupID", BuiltInWorkgroupId)
                    }
                    Builtin::LocalInvocationId => (
                        self.ty_uvec(3)?,
                        "gl_LocalInvocationID",
                        BuiltInLocalInvocationId,
                    ),
                    Builtin::LocalInvocationIndex => (
                        self.ty_u32()?,
                        "gl_LocalInvocationIndex",
                        BuiltInLocalInvocationIndex,
                    ),
                    _ => {
                        return Err(CompileError::unsupported(
                            0,
                            None,
                            UnsupportedReason::PerVertexInterpolation,
                        ));
                    }
                },
                InputKind::Parameter => (self.ty_fvec(4)?, "in_param", u32::MAX),
            };
            let variable = self.define_interface(ty, StorageClassInput, name)?;
            if builtin != u32::MAX {
                self.b
                    .annotate(OpDecorate, &[variable, DecorationBuiltIn, builtin])?;
            } else {
                let ps = self
                    .profile
                    .pixel()
                    .ok_or(CompileError::Internal("parameter outside pixel"))?;
                if input.flat {
                    self.b.annotate(OpDecorate, &[variable, DecorationFlat])?;
                } else if ps.no_perspective || self.linear_input(input.location) {
                    self.b
                        .annotate(OpDecorate, &[variable, DecorationNoPerspective])?;
                }
                self.b
                    .annotate(OpDecorate, &[variable, DecorationLocation, input.location])?;
            }
            self.b.budget().push(
                &mut self.inputs,
                (input, variable),
                128,
                LimitKind::SourceTable,
            )?;
        }
        if logical_lane {
            if let Some((_, variable)) = self
                .inputs
                .iter()
                .find(|(input, _)| input.kind == InputKind::Builtin(Builtin::LocalInvocationIndex))
            {
                self.local_lane_var = *variable;
            } else {
                let u = self.ty_u32()?;
                let variable =
                    self.define_interface(u, StorageClassInput, "gl_LocalInvocationIndex")?;
                self.b.annotate(
                    OpDecorate,
                    &[variable, DecorationBuiltIn, BuiltInLocalInvocationIndex],
                )?;
                self.local_lane_var = variable;
            }
        } else if uses_lane {
            let u = self.ty_u32()?;
            let variable =
                self.define_interface(u, StorageClassInput, "gl_SubgroupInvocationID")?;
            self.b.annotate(
                OpDecorate,
                &[
                    variable,
                    DecorationBuiltIn,
                    BuiltInSubgroupLocalInvocationId,
                ],
            )?;
            if pixel {
                self.b.annotate(OpDecorate, &[variable, DecorationFlat])?;
            }
            self.subgroup_lane_var = variable;
        }
        Ok(())
    }

    /// Source `InputInterpolation`: every read of the location used the
    /// linear-center pair (mode 4), so the input is NoPerspective.
    fn linear_input(&self, location: u32) -> bool {
        let Some(ps) = self.profile.pixel() else {
            return false;
        };
        let mut any = false;
        for block in 0..self.ir.blocks.len() as u32 {
            for id in self.ir.block_insts(block) {
                let inst = self.ir.inst(id);
                if inst.op != Op::GetAttribute {
                    continue;
                }
                let attribute = inst.aux & 0xff;
                if ps
                    .inputs
                    .get(attribute as usize)
                    .is_none_or(|i| u32::from(i.location) != location)
                {
                    continue;
                }
                if (inst.aux >> 16) & 0xff != 4 {
                    return false;
                }
                any = true;
            }
        }
        any
    }

    /// Source `DefineOutputs` (without the optional clip guard).
    fn define_outputs(&mut self) -> R<()> {
        for index in 0..self.layout.outputs.len() {
            let output = self.layout.outputs[index];
            let variable = match output.kind {
                OutputKind::Position => {
                    let v4 = self.ty_fvec(4)?;
                    // MadMode::Position: the position member is also Invariant.
                    let block = self.b.decorated_ty(
                        OpTypeStruct,
                        &[
                            Annotation {
                                op: OpMemberDecorate,
                                operands: &[0, DecorationBuiltIn, BuiltInPosition],
                            },
                            Annotation {
                                op: OpMemberDecorate,
                                operands: &[0, DecorationInvariant],
                            },
                            Annotation {
                                op: OpDecorate,
                                operands: &[DecorationBlock],
                            },
                        ],
                        &[v4],
                    )?;
                    self.b.name(block, "gl_PerVertex")?;
                    self.define_interface(block, StorageClassOutput, "outPerVertex")?
                }
                OutputKind::Parameter | OutputKind::Mrt => {
                    let ty = if output.uint {
                        self.ty_uvec(4)?
                    } else {
                        self.ty_fvec(4)?
                    };
                    let name = if output.kind == OutputKind::Mrt {
                        "out_mrt"
                    } else {
                        "out_param"
                    };
                    let variable = self.define_interface(ty, StorageClassOutput, name)?;
                    self.b
                        .annotate(OpDecorate, &[variable, DecorationLocation, output.location])?;
                    variable
                }
            };
            self.b.budget().push(
                &mut self.outputs,
                (output, variable),
                128,
                LimitKind::SourceTable,
            )?;
        }
        Ok(())
    }

    fn storage_block(&mut self) -> R<u32> {
        let u = self.ty_u32()?;
        let runtime = self.b.decorated_ty(
            OpTypeRuntimeArray,
            &[Annotation {
                op: OpDecorate,
                operands: &[DecorationArrayStride, 4],
            }],
            &[u],
        )?;
        self.b.decorated_ty(
            OpTypeStruct,
            &[
                Annotation {
                    op: OpMemberDecorate,
                    operands: &[0, DecorationOffset, 0],
                },
                Annotation {
                    op: OpDecorate,
                    operands: &[DecorationBlock],
                },
            ],
            &[runtime],
        )
    }

    fn define_binding(&mut self, ty: u32, storage: u32, name: &str, kind: u32) -> R<u32> {
        let pointer = self.ty_pointer(storage, ty)?;
        let variable = self.b.global_variable(pointer, storage)?;
        self.b.name(variable, name)?;
        self.b
            .annotate(OpDecorate, &[variable, DecorationDescriptorSet, 0])?;
        let binding = native_binding(self.profile.stage(), kind);
        self.b
            .annotate(OpDecorate, &[variable, DecorationBinding, binding])?;
        Ok(variable)
    }

    fn sampled_image_type(&mut self) -> R<u32> {
        let f32_type = self.ty_f32()?;
        // Sampled float 2D, no depth/array/multisampling, unknown storage format.
        self.b.ty(
            OpTypeImage,
            &[f32_type, Dim2D, 0, 0, 0, 1, ImageFormatUnknown],
        )
    }

    fn emit_image(&mut self, id: InstId) -> R<u32> {
        let inst = self.ir.inst(id);
        let image_index = (inst.aux & 0xffff) as usize;
        let sampler_index = (inst.aux >> 16) as usize;
        let image_var = *self
            .image_vars
            .get(image_index)
            .ok_or(CompileError::Internal("image binding index"))?;
        let sampler_var = *self
            .sampler_vars
            .get(sampler_index)
            .ok_or(CompileError::Internal("sampler binding index"))?;
        let image_type = self.sampled_image_type()?;
        let sampler_type = self.b.ty(OpTypeSampler, &[])?;
        let image = self.op(OpLoad, image_type, &[image_var])?;
        let sampler = self.op(OpLoad, sampler_type, &[sampler_var])?;
        let sampled_type = self.b.ty(OpTypeSampledImage, &[image_type])?;
        let sampled = self.op(OpSampledImage, sampled_type, &[image, sampler])?;
        let coords = self.arg(id, 2)?;
        let op = self.ir.inst(id).op;
        if op == Op::ImageQueryLod {
            self.b.require_capability(CapabilityImageQuery)?;
            let ty = self.ty_fvec(2)?;
            return self.op(OpImageQueryLod, ty, &[sampled, coords]);
        }
        let explicit = matches!(op, Op::ImageSampleLod | Op::ImageSampleLodFeedback);
        let ty = self.ty_fvec(4)?;
        let sample = if explicit {
            let lod = self.arg(id, 3)?;
            self.op(
                OpImageSampleExplicitLod,
                ty,
                &[sampled, coords, ImageOperandsLodMask, lod],
            )?
        } else {
            self.op(OpImageSampleImplicitLod, ty, &[sampled, coords])?
        };
        if op.has_lod_feedback() {
            self.b
                .budget()
                .charge_usize(self.layout.lod_counters.len())?;
            if let Some(counter) = self
                .layout
                .lod_counters
                .iter()
                .find(|counter| counter.image == image_index)
            {
                let description = self.profile.common().lod_counters[counter.profile_index];
                let lod = if explicit {
                    self.arg(id, 3)?
                } else {
                    self.b.require_capability(CapabilityImageQuery)?;
                    let query_type = self.ty_fvec(2)?;
                    let query = self.op(OpImageQueryLod, query_type, &[sampled, coords])?;
                    self.f32op(OpCompositeExtract, &[query, 1])?
                };
                let active = self.arg(id, if explicit { 4 } else { 3 })?;
                let bool_type = self.ty_bool()?;
                let helper = self.op(OpLoad, bool_type, &[self.helper_var])?;
                let ordinary = self.boolop(OpLogicalNot, &[helper])?;
                let eligible = self.boolop(OpLogicalAnd, &[active, ordinary])?;
                // Derivative-dependent query remains before this conditional.
                // Helpers and guest EXEC-inactive lanes cannot elect a writer.
                self.if_condition(eligible, |e| e.emit_lod_record(description, lod))?;
            }
        }
        Ok(sample)
    }

    fn lod_counter_pointer(&mut self, index: u32) -> R<u32> {
        let word = self.ty_u32()?;
        let pointer_type = self.ty_pointer(StorageClassStorageBuffer, word)?;
        let zero = self.const_u32(0)?;
        let index = self.const_u32(index)?;
        self.op(
            OpAccessChain,
            pointer_type,
            &[self.lod_feedback_var, zero, index],
        )
    }

    fn emit_lod_record(&mut self, counter: crate::LodCounterDescription, lod: u32) -> R<()> {
        self.b.require_capability(CapabilityGroupNonUniform)?;
        self.b
            .require_capability(CapabilityGroupNonUniformArithmetic)?;
        let base = self.const_u32(u32::from(counter.base_level))?;
        let base = self.f32op(OpConvertUToF, &[base])?;
        let absolute = self.f32op(OpFAdd, &[lod, base])?;
        let zero = self.const_f32(0)?;
        let fourteen = self.const_f32(0x4160_0000)?;
        let clamped = self.ext_f32(GLSLstd450FClamp, &[absolute, zero, fourteen])?;
        let floor = self.ext_f32(GLSLstd450Floor, &[clamped])?;
        let level = self.u32op(OpConvertFToU, &[floor])?;
        let scope = self.const_u32(ScopeSubgroup)?;
        let finest = self.u32op(OpGroupNonUniformUMin, &[scope, 0, level])?;
        let threshold = self.const_u32(u32::from(counter.min_lod_u4_8))?;
        let threshold = self.f32op(OpConvertUToF, &[threshold])?;
        let scale = self.const_f32(0x3b80_0000)?; // exactly 1/256
        let threshold = self.f32op(OpFMul, &[threshold, scale])?;
        // Magnification at MIN_LOD zero is not a minimum-LOD clamp.
        let below = self.boolop(OpFOrdLessThan, &[clamped, threshold])?;
        let one = self.const_u32(1)?;
        let zero = self.const_u32(0)?;
        let counted = self.select_u32(below, one, zero)?;
        let counted = self.u32op(OpGroupNonUniformUMax, &[scope, 0, counted])?;
        let elected = self.boolop(OpGroupNonUniformElect, &[scope])?;
        self.if_condition(elected, |e| {
            let pointer = e.lod_counter_pointer(u32::from(counter.counter_id))?;
            let device = e.const_u32(ScopeDevice)?;
            let semantics = e.const_u32(MemorySemanticsMaskNone)?;
            e.u32op(OpAtomicUMin, &[pointer, device, semantics, finest])?;
            let any = e.cmp_const(OpINotEqual, counted, 0)?;
            e.if_condition(any, |e| {
                let pointer = e.lod_counter_pointer(
                    crate::LOD_COUNTER_ENTRIES + u32::from(counter.counter_id),
                )?;
                let one = e.const_u32(1)?;
                e.u32op(OpAtomicIAdd, &[pointer, device, semantics, one])?;
                Ok(())
            })
        })
    }

    fn define_lod_feedback(&mut self) -> R<()> {
        if self.layout.lod_counters.is_empty() {
            return Ok(());
        }
        let word = self.ty_u32()?;
        let count = self.const_u32(crate::LOD_COUNTER_WORDS)?;
        let array = self.b.decorated_ty(
            OpTypeArray,
            &[Annotation {
                op: OpDecorate,
                operands: &[DecorationArrayStride, 4],
            }],
            &[word, count],
        )?;
        let block = self.b.decorated_ty(
            OpTypeStruct,
            &[
                Annotation {
                    op: OpMemberDecorate,
                    operands: &[0, DecorationOffset, 0],
                },
                Annotation {
                    op: OpDecorate,
                    operands: &[DecorationBlock],
                },
            ],
            &[array],
        )?;
        self.lod_feedback_var = self.define_binding(
            block,
            StorageClassStorageBuffer,
            "lod_feedback",
            BINDING_LOD_FEEDBACK,
        )?;
        let bool_type = self.ty_bool()?;
        self.helper_var =
            self.define_interface(bool_type, StorageClassInput, "gl_HelperInvocation")?;
        self.b.annotate(
            OpDecorate,
            &[self.helper_var, DecorationBuiltIn, BuiltInHelperInvocation],
        )
    }

    /// Source `DefineDescriptors` (push data, Buffers, ShaderData).
    fn define_descriptors(&mut self) -> R<()> {
        self.define_lod_feedback()?;
        for index in 0..self.layout.images.len() {
            let ty = self.sampled_image_type()?;
            let variable = self.define_binding(
                ty,
                StorageClassUniformConstant,
                "sampled_image",
                BINDING_IMAGES + index as u32,
            )?;
            self.b.budget().push(
                &mut self.image_vars,
                variable,
                crate::profile::MAX_IMAGES,
                LimitKind::SourceTable,
            )?;
        }
        for index in 0..self.layout.samplers.len() {
            let ty = self.b.ty(OpTypeSampler, &[])?;
            let variable = self.define_binding(
                ty,
                StorageClassUniformConstant,
                "sampler",
                BINDING_SAMPLERS + index as u32,
            )?;
            self.b.budget().push(
                &mut self.sampler_vars,
                variable,
                crate::profile::MAX_IMAGES,
                LimitKind::SourceTable,
            )?;
        }
        if self.layout.push_data_start.is_some() {
            let u = self.ty_u32()?;
            let count = self.const_u32(PUSH_DWORDS)?;
            let array = self.b.decorated_ty(
                OpTypeArray,
                &[Annotation {
                    op: OpDecorate,
                    operands: &[DecorationArrayStride, 4],
                }],
                &[u, count],
            )?;
            let block = self.b.decorated_ty(
                OpTypeStruct,
                &[
                    Annotation {
                        op: OpMemberDecorate,
                        operands: &[0, DecorationOffset, 0],
                    },
                    Annotation {
                        op: OpDecorate,
                        operands: &[DecorationBlock],
                    },
                ],
                &[array],
            )?;
            let pointer = self.ty_pointer(StorageClassPushConstant, block)?;
            self.push_var = self.b.global_variable(pointer, StorageClassPushConstant)?;
            self.b.name(block, "BufferResource")?;
            self.b.name(self.push_var, "vsharp")?;
        }
        if !self.layout.buffers.is_empty() {
            let block = self.storage_block()?;
            let count = self.const_u32(self.layout.buffers.len() as u32)?;
            let array = self.b.ty(OpTypeArray, &[block, count])?;
            let variable =
                self.define_binding(array, StorageClassStorageBuffer, "buffers", BINDING_BUFFERS)?;
            let writes = self.layout.buffers.iter().any(|b| b.written);
            // GLSL450 peer visibility needs coherent backing as well as the
            // actual barrier's storage-class semantics. Keep guest flags in
            // the manifest separate from this effective array decoration.
            let coherent = self.layout.buffers_coherent();
            if !writes && !coherent {
                self.b
                    .annotate(OpDecorate, &[variable, DecorationNonWritable])?;
            }
            if coherent {
                self.b
                    .annotate(OpDecorate, &[variable, DecorationCoherent])?;
            }
            self.buffers_var = variable;
        }
        if self.layout.shader_data_binding {
            let block = self.storage_block()?;
            self.shader_data_var = self.define_binding(
                block,
                StorageClassStorageBuffer,
                "shader_data",
                BINDING_SHADER_DATA,
            )?;
        }
        Ok(())
    }

    /// Source `CollectPositionSlice` (MadMode::Position NoContraction).
    fn collect_position_slice(&mut self) -> R<()> {
        let count = self.ir.insts.len();
        self.position_slice = self
            .b
            .budget()
            .table(count, false, usize::MAX, LimitKind::Bytes)?;
        let mut pending: Vec<InstId> = Vec::new();
        for block in 0..self.ir.blocks.len() as u32 {
            for id in self.ir.block_insts(block) {
                let inst = self.ir.inst(id);
                if inst.op == Op::SetAttribute
                    && self
                        .ir
                        .exports
                        .get(inst.aux as usize)
                        .is_some_and(|e| e.kind == ExportKind::Position)
                {
                    self.b
                        .budget()
                        .push(&mut pending, id, usize::MAX, LimitKind::Bytes)?;
                }
            }
        }
        while let Some(id) = pending.pop() {
            for index in 0..self.ir.nargs(id) {
                self.b.budget().charge(1)?;
                if let Some((producer, _)) = self
                    .ir
                    .producer(self.b.budget_ref(), self.ir.arg(id, index))?
                    && let Some(slot) = self.position_slice.get_mut(producer.index())
                    && !*slot
                {
                    *slot = true;
                    self.b
                        .budget()
                        .push(&mut pending, producer, usize::MAX, LimitKind::Bytes)?;
                }
            }
        }
        Ok(())
    }
}

/// Information about an emitted module needed by the manifest.
pub(crate) struct EmittedModule {
    pub words: Vec<u32>,
    pub uses_subgroup: bool,
    pub uses_logical_lane: bool,
    pub required_features: crate::manifest::RequiredFeatures,
}

/// Source `Spirv::EmitProgram` for vertex, pixel and compute programs.
pub(crate) fn emit_program(
    ir: &Ir,
    layout: &Layout,
    profile: &Profile,
    budget: &mut Budget,
) -> R<EmittedModule> {
    let stage = profile.stage();
    budget.charge_usize(ir.insts.len())?;
    let uses_lane = count_ops(ir, |op| op == Op::LaneId);
    budget.charge_usize(ir.insts.len())?;
    let uses_ballot = count_ops(ir, |op| matches!(op, Op::Ballot | Op::AnyLane));
    // A module that communicates through native ballots keeps native lane
    // numbering too. Only lane-local mask selection can use logical waves.
    let logical_lane = uses_lane && profile.local_wave64_lane_id() && !uses_ballot;
    let native_lane = uses_lane && !logical_lane;
    let insts = ir.insts.len();
    let blocks = ir.blocks.len();
    let ids = budget.table(insts, 0u32, usize::MAX, LimitKind::Bytes)?;
    let labels = budget.table(blocks, 0u32, usize::MAX, LimitKind::Bytes)?;
    let exit_labels = budget.table(blocks, 0u32, usize::MAX, LimitKind::Bytes)?;
    let mut e = Emit {
        b: Builder::new(budget, 0x0001_0300),
        ir,
        layout,
        profile,
        t: Types::default(),
        ids,
        labels,
        exit_labels,
        current_label: 0,
        deferred: Vec::new(),
        glsl: 0,
        main_func: 0,
        push_var: 0,
        buffers_var: 0,
        shared_var: 0,
        image_vars: Vec::new(),
        sampler_vars: Vec::new(),
        lod_feedback_var: 0,
        helper_var: 0,
        shader_data_var: 0,
        inputs: Vec::new(),
        outputs: Vec::new(),
        interface: Vec::new(),
        subgroup_lane_var: 0,
        local_lane_var: 0,
        memory_offsets: Vec::new(),
        position_slice: Vec::new(),
    };
    // Source DefineModule.
    e.define_inputs(uses_lane, logical_lane)?;
    e.define_outputs()?;
    e.define_descriptors()?;
    e.define_shared()?;
    e.main_func = e.b.alloc_id()?;
    let entry_label = e.b.alloc_id()?;
    e.b.require_capability(CapabilityShader)?;
    e.b.require_capability(CapabilitySignedZeroInfNanPreserve)?;
    // A host-subgroup completion barrier requires BASIC support, even when
    // logical lane indexing does not require an exact native subgroup size.
    if native_lane || uses_ballot || layout.subgroup_store_completion {
        e.b.require_capability(CapabilityGroupNonUniform)?;
    }
    if uses_ballot {
        e.b.require_capability(CapabilityGroupNonUniformBallot)?;
    }
    e.b.require_extension("SPV_KHR_float_controls")?;
    e.b.memory_model(AddressingModelLogical, MemoryModelGLSL450)?;
    e.b.execution_mode(e.main_func, ExecutionModeSignedZeroInfNanPreserve, &[32])?;
    if let Some(cs) = profile.compute() {
        let [x, y, z] = cs.workgroup_size;
        e.b.execution_mode(
            e.main_func,
            ExecutionModeLocalSize,
            &[x.max(1), y.max(1), z.max(1)],
        )?;
    }
    if stage == ShaderStage::Pixel {
        e.b.execution_mode(e.main_func, ExecutionModeOriginUpperLeft, &[])?;
    }
    e.b.name(e.main_func, "main")?;
    e.collect_position_slice()?;

    // Source EmitProgram (structured function).
    for block in 0..blocks {
        e.labels[block] = e.b.alloc_id()?;
    }
    let void = e.ty_void()?;
    let function_ty = e.b.ty(OpTypeFunction, &[void])?;
    e.b.func(
        OpFunction,
        &[void, e.main_func, FunctionControlMaskNone, function_ty],
    )?;
    e.label(entry_label)?;
    // Source EmitMemoryOffsets.
    for index in 0..layout.memory_offset_count {
        let word = e.shader_data_dword(layout.memory_offset_dword + index / 4)?;
        let shifted = e.shr_const(word, (index % 4) * 8)?;
        let byte = e.and_const(shifted, 0xff)?;
        e.b.budget()
            .push(&mut e.memory_offsets, byte, 64, LimitKind::SourceTable)?;
    }
    let first = e.labels[0];
    e.b.func(OpBranch, &[first])?;
    for block in 0..blocks as u32 {
        let label = e.labels[block as usize];
        e.label(label)?;
        let mut seen_non_phi = false;
        for id in ir.block_insts(block) {
            e.b.budget().charge(1)?;
            if ir.inst(id).op == Op::Phi {
                if seen_non_phi {
                    return Err(CompileError::Internal("phi after a non-phi instruction"));
                }
            } else {
                seen_non_phi = true;
            }
            e.emit_inst(id)?;
        }
        e.exit_labels[block as usize] = e.current_label;
        e.emit_terminator(block)?;
    }
    // Source PatchStructuredPhis.
    let deferred = std::mem::take(&mut e.deferred);
    for (phi, id) in deferred {
        let block = ir.inst(id).block;
        let preds = ir.predecessors(block).len();
        for index in 0..preds {
            let pred = ir.predecessors(block)[index];
            let parent = e.exit_labels.get(pred as usize).copied().unwrap_or(0);
            let value = e.arg(id, index)?;
            e.b.patch_phi(phi, index, value, parent)?;
        }
    }
    e.b.func(OpFunctionEnd, &[])?;
    let model = match stage {
        ShaderStage::Vertex => ExecutionModelVertex,
        ShaderStage::Pixel => ExecutionModelFragment,
        _ => ExecutionModelGLCompute,
    };
    let interface = std::mem::take(&mut e.interface);
    e.b.entry_point(model, e.main_func, "main", &interface)?;
    let required_features = e.b.required_features();
    let words = e.b.build()?;
    Ok(EmittedModule {
        words,
        uses_subgroup: native_lane || uses_ballot,
        uses_logical_lane: logical_lane,
        required_features,
    })
}
