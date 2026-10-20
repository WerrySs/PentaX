// SPDX-License-Identifier: GPL-2.0-only
// Direct Rust adaptation of KytyPS5-Performance at
// 209dffe9032d72749257ce18680d506b891284de: frontend/translate/Vector.cpp
// (vector opcode map), Compare.cpp, Float.cpp, Convert.cpp and the vector
// operations of Integer.cpp and Control.cpp.
// PentaX modifications: 16-bit, packed, 64-bit float, lane read/write/permute
// and M0-relative forms are typed refusals instead of lowerings.

use pentax_shader::{Opcode as O, OperandKind};

use super::{Opnd, Translator, u32v};
use crate::CompileError;
use crate::error::UnsupportedReason;
use crate::ir::{Op, Type, Value};

type R = Result<(), CompileError>;

impl Translator<'_> {
    pub(super) fn emit_vector(&mut self) -> R {
        use UnsupportedReason as U;
        let inst = self.inst;
        match inst.opcode {
            O::VNop => Ok(()),
            O::VAddI32 => self.add_u32(true, false),
            O::VAddcU32 => self.add_u32(true, true),
            O::VSubI32 => self.sub_u32(true, false),
            O::VSubrevI32 => self.sub_u32(true, true),
            O::VSubCoCiU32 => self.subb_u32(true, false),
            O::VSubrevCoCiU32 => self.subb_u32(true, true),
            O::VMovB32 => self.mov_b32(true),
            O::VMovrelsB32 | O::VMovreldB32 => Err(self.unsupported(U::NotPorted)),
            O::VReadfirstlaneB32
            | O::VReadlaneB32
            | O::VWritelaneB32
            | O::VPermlane16B32
            | O::VPermlanex16B32 => Err(self.unsupported(U::CrossLaneOperation)),

            O::VCmpFI32 | O::VCmpFU32 | O::VCmpFI64 | O::VCmpFU64 | O::VCmpFF32 => {
                self.compare_result(Value::U1(false), false, false)
            }
            O::VCmpTI32 | O::VCmpTU32 | O::VCmpTI64 | O::VCmpTU64 | O::VCmpTruF32 => {
                self.compare_result(Value::U1(true), false, false)
            }
            O::VCmpxFI64 | O::VCmpxFU64 => self.compare_result(Value::U1(false), false, true),
            O::VCmpxTI64 | O::VCmpxTU64 => self.compare_result(Value::U1(true), false, true),
            O::VCmpEqU32 | O::VCmpEqI32 => {
                self.integer_compare(Op::IEqual32, Type::U32, false, false)
            }
            O::VCmpxEqU32 | O::VCmpxEqI32 => {
                self.integer_compare(Op::IEqual32, Type::U32, false, true)
            }
            O::VCmpNeU32 | O::VCmpNeI32 => {
                self.integer_compare(Op::INotEqual32, Type::U32, false, false)
            }
            O::VCmpxNeU32 | O::VCmpxNeI32 => {
                self.integer_compare(Op::INotEqual32, Type::U32, false, true)
            }
            O::VCmpGtU32 => self.integer_compare(Op::UGreaterThan32, Type::U32, false, false),
            O::VCmpxGtU32 => self.integer_compare(Op::UGreaterThan32, Type::U32, false, true),
            O::VCmpGeU32 => self.integer_compare(Op::UGreaterThanEqual32, Type::U32, false, false),
            O::VCmpxGeU32 => self.integer_compare(Op::UGreaterThanEqual32, Type::U32, false, true),
            O::VCmpLtU32 => self.integer_compare(Op::ULessThan32, Type::U32, false, false),
            O::VCmpxLtU32 => self.integer_compare(Op::ULessThan32, Type::U32, false, true),
            O::VCmpLeU32 => self.integer_compare(Op::ULessThanEqual32, Type::U32, false, false),
            O::VCmpxLeU32 => self.integer_compare(Op::ULessThanEqual32, Type::U32, false, true),
            O::VCmpGtI32 => self.integer_compare(Op::SGreaterThan32, Type::U32, false, false),
            O::VCmpxGtI32 => self.integer_compare(Op::SGreaterThan32, Type::U32, false, true),
            O::VCmpGeI32 => self.integer_compare(Op::SGreaterThanEqual32, Type::U32, false, false),
            O::VCmpxGeI32 => self.integer_compare(Op::SGreaterThanEqual32, Type::U32, false, true),
            O::VCmpLtI32 => self.integer_compare(Op::SLessThan32, Type::U32, false, false),
            O::VCmpxLtI32 => self.integer_compare(Op::SLessThan32, Type::U32, false, true),
            O::VCmpLeI32 => self.integer_compare(Op::SLessThanEqual32, Type::U32, false, false),
            O::VCmpxLeI32 => self.integer_compare(Op::SLessThanEqual32, Type::U32, false, true),
            O::VCmpEqI64 | O::VCmpEqU64 => {
                self.integer_compare(Op::IEqual64, Type::U64, false, false)
            }
            O::VCmpxEqI64 | O::VCmpxEqU64 => {
                self.integer_compare(Op::IEqual64, Type::U64, false, true)
            }
            O::VCmpLtU64 => self.integer_compare(Op::ULessThan64, Type::U64, false, false),
            O::VCmpxLtU64 => self.integer_compare(Op::ULessThan64, Type::U64, false, true),
            O::VCmpGtU64 => self.integer_compare(Op::UGreaterThan64, Type::U64, false, false),
            O::VCmpxGtU64 => self.integer_compare(Op::UGreaterThan64, Type::U64, false, true),
            O::VCmpNeI64 | O::VCmpNeU64 => {
                self.integer_compare(Op::INotEqual64, Type::U64, false, false)
            }
            O::VCmpxNeI64 | O::VCmpxNeU64 => {
                self.integer_compare(Op::INotEqual64, Type::U64, false, true)
            }
            O::VCmpLtI64 => self.integer_compare(Op::SLessThan64, Type::U64, false, false),
            O::VCmpxLtI64 => self.integer_compare(Op::SLessThan64, Type::U64, false, true),
            O::VCmpLeI64 => self.integer_compare(Op::SLessThanEqual64, Type::U64, false, false),
            O::VCmpxLeI64 => self.integer_compare(Op::SLessThanEqual64, Type::U64, false, true),
            O::VCmpLeU64 => self.integer_compare(Op::ULessThanEqual64, Type::U64, false, false),
            O::VCmpxLeU64 => self.integer_compare(Op::ULessThanEqual64, Type::U64, false, true),
            O::VCmpGeU64 => self.integer_compare(Op::UGreaterThanEqual64, Type::U64, false, false),
            O::VCmpxGeU64 => self.integer_compare(Op::UGreaterThanEqual64, Type::U64, false, true),
            O::VCmpEqU16
            | O::VCmpEqI16
            | O::VCmpxEqI16
            | O::VCmpNeU16
            | O::VCmpxNeU16
            | O::VCmpNeI16
            | O::VCmpxNeI16
            | O::VCmpGtU16
            | O::VCmpxGtU16
            | O::VCmpGeU16
            | O::VCmpLtU16
            | O::VCmpxLtU16
            | O::VCmpxEqU16
            | O::VCmpLeU16
            | O::VCmpGtI16
            | O::VCmpxGtI16
            | O::VCmpGeI16
            | O::VCmpxGeI16
            | O::VCmpLtI16
            | O::VCmpxLtI16
            | O::VCmpLeI16
            | O::VCmpxLeI16 => Err(self.unsupported(U::Integer16Operation)),
            O::VCmpEqF32 => self.float_compare(Op::FPOrdEqual32, false),
            O::VCmpxEqF32 => self.float_compare(Op::FPOrdEqual32, true),
            O::VCmpLgF32 => self.float_compare(Op::FPOrdNotEqual32, false),
            O::VCmpxLgF32 => self.float_compare(Op::FPOrdNotEqual32, true),
            O::VCmpGtF32 => self.float_compare(Op::FPOrdGreaterThan32, false),
            O::VCmpxGtF32 => self.float_compare(Op::FPOrdGreaterThan32, true),
            O::VCmpGeF32 => self.float_compare(Op::FPOrdGreaterThanEqual32, false),
            O::VCmpxGeF32 => self.float_compare(Op::FPOrdGreaterThanEqual32, true),
            O::VCmpLtF32 => self.float_compare(Op::FPOrdLessThan32, false),
            O::VCmpxLtF32 => self.float_compare(Op::FPOrdLessThan32, true),
            O::VCmpLeF32 => self.float_compare(Op::FPOrdLessThanEqual32, false),
            O::VCmpxLeF32 => self.float_compare(Op::FPOrdLessThanEqual32, true),
            O::VCmpNlgF32 => self.float_compare(Op::FPUnordEqual32, false),
            O::VCmpxNlgF32 => self.float_compare(Op::FPUnordEqual32, true),
            O::VCmpNeqF32 => self.float_compare(Op::FPUnordNotEqual32, false),
            O::VCmpxNeqF32 => self.float_compare(Op::FPUnordNotEqual32, true),
            O::VCmpNleF32 => self.float_compare(Op::FPUnordGreaterThan32, false),
            O::VCmpxNleF32 => self.float_compare(Op::FPUnordGreaterThan32, true),
            O::VCmpNltF32 => self.float_compare(Op::FPUnordGreaterThanEqual32, false),
            O::VCmpxNltF32 => self.float_compare(Op::FPUnordGreaterThanEqual32, true),
            O::VCmpNgeF32 => self.float_compare(Op::FPUnordLessThan32, false),
            O::VCmpxNgeF32 => self.float_compare(Op::FPUnordLessThan32, true),
            O::VCmpNgtF32 => self.float_compare(Op::FPUnordLessThanEqual32, false),
            O::VCmpxNgtF32 => self.float_compare(Op::FPUnordLessThanEqual32, true),
            O::VCmpOF32 => self.float_ordered_compare(true, false),
            O::VCmpUF32 => self.float_ordered_compare(false, false),
            O::VCmpxOF32 => self.float_ordered_compare(true, true),
            O::VCmpxUF32 => self.float_ordered_compare(false, true),
            O::VCmpClassF32 => self.float_class_compare(false),
            O::VCmpxClassF32 => self.float_class_compare(true),
            O::VCmpEqF64 | O::VCmpLeF64 | O::VCmpxLeF64 | O::VCmpxGeF64 => {
                Err(self.unsupported(U::Float64Operation))
            }
            O::VCmpEqF16
            | O::VCmpxEqF16
            | O::VCmpLgF16
            | O::VCmpGtF16
            | O::VCmpxGtF16
            | O::VCmpGeF16
            | O::VCmpxGeF16
            | O::VCmpLtF16
            | O::VCmpxLtF16
            | O::VCmpLeF16
            | O::VCmpxLeF16
            | O::VCmpNgeF16
            | O::VCmpNgtF16
            | O::VCmpxNgtF16
            | O::VCmpxNleF16
            | O::VCmpNeqF16
            | O::VCmpxNeqF16
            | O::VCmpNltF16
            | O::VCmpxNltF16
            | O::VCmpClassF16
            | O::VCmpxClassF16 => Err(self.unsupported(U::Float16Operation)),

            O::VCvtF32Ubyte0 => self.cvt_f32_ubyte(0),
            O::VCvtF32Ubyte1 => self.cvt_f32_ubyte(1),
            O::VCvtF32Ubyte2 => self.cvt_f32_ubyte(2),
            O::VCvtF32Ubyte3 => self.cvt_f32_ubyte(3),
            O::VCvtF32U32 => {
                let value = self.read_u32(inst.src(0))?;
                let result = self.e.emit(Op::ConvertF32U32, &[value])?;
                self.write(inst.dst, result)
            }
            O::VCvtF32I32 => {
                let value = self.read_u32(inst.src(0))?;
                let result = self.e.emit(Op::ConvertF32S32, &[value])?;
                self.write(inst.dst, result)
            }
            O::VCvtU32F32 => {
                let value = self.read_f32(inst.src(0))?;
                let result =
                    self.f32_to_u32_saturated(value, 4_294_967_296.0, 4_294_967_040.0, u32::MAX)?;
                self.write(inst.dst, result)
            }
            O::VCvtI32F32 => {
                let value = self.read_f32(inst.src(0))?;
                let result = self.f32_to_i32_full(value)?;
                self.write(inst.dst, result)
            }
            O::VCvtRpiI32F32 => {
                let source = self.read_f32(inst.src(0))?;
                let biased = self.e.emit(Op::FPAdd32, &[source, Value::f32(0.5)])?;
                let rounded = self.e.emit(Op::FPFloor32, &[biased])?;
                let result = self.f32_to_i32_full(rounded)?;
                self.write(inst.dst, result)
            }
            O::VCvtFlrI32F32 => {
                let source = self.read_f32(inst.src(0))?;
                let rounded = self.e.emit(Op::FPFloor32, &[source])?;
                let result = self.f32_to_i32_full(rounded)?;
                self.write(inst.dst, result)
            }
            O::VFrexpExpI32F32 => self.frexp_exp(),
            O::VCvtOffF32I4 => {
                let raw = self.read_u32(inst.src(0))?;
                let nibble = self
                    .e
                    .emit(Op::BitFieldSExtract, &[raw, u32v(0), u32v(4)])?;
                let value = self.e.emit(Op::ConvertF32S32, &[nibble])?;
                let result = self.e.emit(Op::FPMul32, &[value, Value::f32(1.0 / 16.0)])?;
                self.write(inst.dst, result)
            }
            O::VCvtPkrtzF16F32 => {
                let a = self.read_f32(inst.src(0))?;
                let lhs = self.f32_result_modifiers(inst.dst, a)?;
                let b = self.read_f32(inst.src(1))?;
                let rhs = self.f32_result_modifiers(inst.dst, b)?;
                let result = self.e.emit(Op::PackFloat2x16Rtz, &[lhs, rhs])?;
                let mut dst = inst.dst;
                dst.omod = 0;
                dst.clamp = false;
                self.write(dst, result)
            }
            O::VCvtPknormI16F32 => self.cvt_pknorm(Op::PackSnorm2x16),
            O::VCvtPknormU16F32 => self.cvt_pknorm(Op::PackUnorm2x16),
            O::VCvtPkU8F32 => {
                let source = self.read_f32(inst.src(0))?;
                let byte = self.f32_to_u32_saturated(source, 255.0, 255.0, 255)?;
                let raw_index = self.read_u32(inst.src(1))?;
                let index = self.e.and(raw_index, u32v(3))?;
                let shift = self.e.shl(index, u32v(3))?;
                let mask = self.e.shl(u32v(0xff), shift)?;
                let raw_base = self.read_u32(inst.src(2))?;
                let inverse = self.e.not(mask)?;
                let base = self.e.and(raw_base, inverse)?;
                let placed = self.e.shl(byte, shift)?;
                let result = self.e.or(base, placed)?;
                self.write(inst.dst, result)
            }
            O::VCvtPkU16U32 | O::VCvtPkI16I32 => self.pack_b16(false, false),
            O::VCvtF64I32
            | O::VCvtF32F64
            | O::VCvtF64F32
            | O::VCvtF64U32
            | O::VRcpF64
            | O::VFractF64
            | O::VTruncF64
            | O::VCeilF64
            | O::VFloorF64
            | O::VAddF64
            | O::VMulF64
            | O::VMinF64
            | O::VMaxF64
            | O::VFmaF64 => Err(self.unsupported(U::Float64Operation)),
            O::VCvtF16F32
            | O::VCvtF32F16
            | O::VCvtF16U16
            | O::VCvtF16I16
            | O::VCvtU16F16
            | O::VCvtI16F16
            | O::VPackB32F16
            | O::VPkAddF16
            | O::VPkMulF16
            | O::VPkMinF16
            | O::VPkMaxF16
            | O::VPkFmaF16
            | O::VPkFmacF16
            | O::VAddF16
            | O::VSubF16
            | O::VSubrevF16
            | O::VMulF16
            | O::VMinF16
            | O::VMaxF16
            | O::VFmacF16
            | O::VFmamkF16
            | O::VFmaakF16
            | O::VFmaF16
            | O::VMadMixloF16
            | O::VMadMixhiF16
            | O::VRcpF16
            | O::VSqrtF16
            | O::VRsqF16
            | O::VLogF16
            | O::VExpF16
            | O::VFloorF16
            | O::VCeilF16
            | O::VTruncF16
            | O::VRndneF16
            | O::VFractF16
            | O::VSinF16
            | O::VCosF16
            | O::VMin3F16
            | O::VMax3F16
            | O::VMed3F16
            | O::VDot2cF32F16 => Err(self.unsupported(U::Float16Operation)),
            O::VLshlrevB16
            | O::VLshrrevB16
            | O::VAshrrevI16
            | O::VAddNcU16
            | O::VAddNcI16
            | O::VSubNcU16
            | O::VSubNcI16
            | O::VMulLoU16
            | O::VMadI16
            | O::VMed3I16
            | O::VMin3U16
            | O::VMinI16
            | O::VMaxI16
            | O::VMinU16
            | O::VMaxU16 => Err(self.unsupported(U::Integer16Operation)),
            O::VPkLshlrevB16
            | O::VPkLshrrevB16
            | O::VPkAshrrevI16
            | O::VPkMadI16
            | O::VPkMadU16
            | O::VPkMulLoU16
            | O::VPkAddI16
            | O::VPkAddU16
            | O::VPkSubI16
            | O::VPkSubU16
            | O::VPkMaxI16
            | O::VPkMinI16
            | O::VPkMaxU16
            | O::VPkMinU16 => Err(self.unsupported(U::PackedOperation)),

            O::VFrexpMantF32 => self.frexp_mant(),
            O::VRcpF32 => self.float_unary(Op::FPRecip32),
            O::VRcpIflagF32 => self.float_unary(Op::FPRecipIFlag32),
            O::VFractF32 => self.float_unary(Op::FPFract32),
            O::VTruncF32 => self.float_unary(Op::FPTrunc32),
            O::VCeilF32 => self.float_unary(Op::FPCeil32),
            O::VRndneF32 => self.float_unary(Op::FPRoundEven32),
            O::VFloorF32 => self.float_unary(Op::FPFloor32),
            O::VExpF32 => self.float_unary(Op::FPExp2),
            O::VLogF32 => self.float_unary(Op::FPLog2),
            O::VRsqF32 => self.float_unary(Op::FPRecipSqrt32),
            O::VSqrtF32 => self.float_unary(Op::FPSqrt),
            O::VSinF32 => self.float_unary(Op::FPSin),
            O::VCosF32 => self.float_unary(Op::FPCos),
            O::VAddF32 => self.float_binary(Op::FPAdd32, false),
            O::VSubF32 => self.float_binary(Op::FPSub32, false),
            O::VSubrevF32 => self.float_binary(Op::FPSub32, true),
            O::VMulF32 => self.float_binary(Op::FPMul32, false),
            O::VMinF32 => self.float_binary(Op::FPMin32, false),
            O::VMaxF32 => self.float_binary(Op::FPMax32, false),
            O::VLdexpF32 => self.float_binary(Op::FPLdexp, false),
            O::VMacF32 => self.float_ternary(Op::FPMad32, true, true),
            O::VMadmkF32 | O::VMadakF32 | O::VMadF32 => {
                self.float_ternary(Op::FPMad32, false, true)
            }
            O::VFmacF32 => self.float_ternary(Op::FPFma32, true, true),
            O::VFmamkF32 | O::VFmaakF32 | O::VFmaF32 => {
                self.float_ternary(Op::FPFma32, false, true)
            }
            O::VMin3F32 => self.float_ternary(Op::FPMinTri32, false, false),
            O::VMax3F32 => self.float_ternary(Op::FPMaxTri32, false, false),
            O::VMed3F32 => self.float_ternary(Op::FPMedTri32, false, false),
            O::VCubeidF32 => self.float_cube(0),
            O::VCubescF32 => self.float_cube(1),
            O::VCubetcF32 => self.float_cube(2),
            O::VCubemaF32 => self.float_cube(3),

            O::VMulLoU32 | O::VMulLoI32 => {
                self.simple_integer(Op::IMul32, Type::U32, false, false, false)
            }
            O::VMulHiU32 => self.simple_integer(Op::UMulHi, Type::U32, false, false, false),
            O::VMulHiI32 => self.simple_integer(Op::SMulHi, Type::U32, false, false, false),
            O::VAddNcU32 => self.simple_integer(Op::IAdd32, Type::U32, false, false, false),
            O::VSubNcU32 => self.simple_integer(Op::ISub32, Type::U32, false, false, false),
            O::VSubrevNcU32 => self.simple_integer(Op::ISub32, Type::U32, true, false, false),
            O::VMinI32 => self.simple_integer(Op::SMin32, Type::U32, false, false, false),
            O::VMaxI32 => self.simple_integer(Op::SMax32, Type::U32, false, false, false),
            O::VMinU32 => self.simple_integer(Op::UMin32, Type::U32, false, false, false),
            O::VMaxU32 => self.simple_integer(Op::UMax32, Type::U32, false, false, false),
            O::VMin3I32 => self.simple_integer(Op::SMinTri32, Type::U32, false, false, false),
            O::VMax3I32 => self.simple_integer(Op::SMaxTri32, Type::U32, false, false, false),
            O::VMed3I32 => self.simple_integer(Op::SMedTri32, Type::U32, false, false, false),
            O::VMin3U32 => self.simple_integer(Op::UMinTri32, Type::U32, false, false, false),
            O::VMax3U32 => self.simple_integer(Op::UMaxTri32, Type::U32, false, false, false),
            O::VMed3U32 => self.simple_integer(Op::UMedTri32, Type::U32, false, false, false),
            O::VAndB32 => self.simple_integer(Op::BitwiseAnd32, Type::U32, false, false, false),
            O::VOrB32 => self.simple_integer(Op::BitwiseOr32, Type::U32, false, false, false),
            O::VXorB32 => self.simple_integer(Op::BitwiseXor32, Type::U32, false, false, false),
            O::VNotB32 => self.simple_integer(Op::BitwiseNot32, Type::U32, false, false, false),
            O::VBfrevB32 => self.simple_integer(Op::BitReverse32, Type::U32, false, false, false),
            O::VFfblB32 => self.simple_integer(Op::FindILsb32, Type::U32, false, false, false),
            O::VLshlB32 => {
                self.simple_integer(Op::ShiftLeftLogical32, Type::U32, false, true, false)
            }
            O::VLshlrevB32 => {
                self.simple_integer(Op::ShiftLeftLogical32, Type::U32, true, true, false)
            }
            O::VLshrB32 => {
                self.simple_integer(Op::ShiftRightLogical32, Type::U32, false, true, false)
            }
            O::VLshrrevB32 => {
                self.simple_integer(Op::ShiftRightLogical32, Type::U32, true, true, false)
            }
            O::VAshrI32 => {
                self.simple_integer(Op::ShiftRightArithmetic32, Type::U32, false, true, false)
            }
            O::VAshrrevI32 => {
                self.simple_integer(Op::ShiftRightArithmetic32, Type::U32, true, true, false)
            }
            O::VLshlrevB64 => {
                self.simple_integer(Op::ShiftLeftLogical64, Type::U64, true, false, false)
            }
            O::VLshrrevB64 => {
                self.simple_integer(Op::ShiftRightLogical64, Type::U64, true, false, false)
            }
            O::VXnorB32 => self.composed_integer_binary(Op::BitwiseXor32, false, true, false),
            O::VAndOrB32 => self.ternary_bits(Op::BitwiseAnd32, Op::BitwiseOr32),
            O::VOr3B32 => self.ternary_bits(Op::BitwiseOr32, Op::BitwiseOr32),
            O::VXor3B32 => self.ternary_bits(Op::BitwiseXor32, Op::BitwiseXor32),
            O::VFfbhU32 => self.ffbh_32(false),
            O::VFfbhI32 => self.ffbh_32(true),
            O::VMadI32I24 => self.integer24(true, true),
            O::VMadU32U24 => self.integer24(false, true),
            O::VMulI32I24 => self.integer24(true, false),
            O::VMulU32U24 => self.integer24(false, false),
            O::VMadU64U32 => self.mad_u64_u32(),
            O::VSadU32 => {
                let lhs = self.read_u32(inst.src(0))?;
                let rhs = self.read_u32(inst.src(1))?;
                let lo = self.e.emit(Op::UMin32, &[lhs, rhs])?;
                let hi = self.e.emit(Op::UMax32, &[lhs, rhs])?;
                let difference = self.e.isub(hi, lo)?;
                let addend = self.read_u32(inst.src(2))?;
                let result = self.e.iadd(difference, addend)?;
                self.write(inst.dst, result)
            }
            O::VAdd3U32 => self.ternary_bits(Op::IAdd32, Op::IAdd32),
            O::VBcntU32B32 => {
                let source = self.read_u32(inst.src(0))?;
                let count = self.e.emit(Op::BitCount32, &[source])?;
                let addend = self.read_u32(inst.src(1))?;
                let result = self.e.iadd(count, addend)?;
                self.write(inst.dst, result)
            }
            O::VMbcntLoU32B32 => self.mbcnt(true),
            O::VMbcntHiU32B32 => self.mbcnt(false),
            O::VBfmB32 => self.bfm_b32(),
            O::VBfeU32 => self.v_bfe(false),
            O::VBfeI32 => self.v_bfe(true),
            O::VBfiB32 => {
                let bits = self.read_u32(inst.src(0))?;
                let insert = self.read_u32(inst.src(1))?;
                let base = self.read_u32(inst.src(2))?;
                let selected = self.e.and(bits, insert)?;
                let inverse = self.e.not(bits)?;
                let kept = self.e.and(inverse, base)?;
                let result = self.e.or(selected, kept)?;
                self.write(inst.dst, result)
            }
            O::VAlignbitB32 => self.align(false),
            O::VAlignbyteB32 => self.align(true),
            O::VLshlAddU32 => {
                let raw = self.read_u32(inst.src(1))?;
                let shift = self.e.and(raw, u32v(31))?;
                let value = self.read_u32(inst.src(0))?;
                let shifted = self.e.shl(value, shift)?;
                let addend = self.read_u32(inst.src(2))?;
                let result = self.e.iadd(shifted, addend)?;
                self.write(inst.dst, result)
            }
            O::VAddLshlU32 => {
                let raw = self.read_u32(inst.src(2))?;
                let shift = self.e.and(raw, u32v(31))?;
                let a = self.read_u32(inst.src(0))?;
                let b = self.read_u32(inst.src(1))?;
                let sum = self.e.iadd(a, b)?;
                let result = self.e.shl(sum, shift)?;
                self.write(inst.dst, result)
            }
            O::VXadU32 => self.ternary_bits(Op::BitwiseXor32, Op::IAdd32),
            O::VPermB32 => self.perm(),
            O::VLshlOrB32 => {
                let raw = self.read_u32(inst.src(1))?;
                let shift = self.e.and(raw, u32v(31))?;
                let value = self.read_u32(inst.src(0))?;
                let shifted = self.e.shl(value, shift)?;
                let other = self.read_u32(inst.src(2))?;
                let result = self.e.or(shifted, other)?;
                self.write(inst.dst, result)
            }
            O::VCndmaskB32 => self.cndmask(),
            _ => Err(self.unsupported(U::NotPorted)),
        }
    }

    // ------------------------------------------------------ Compare.cpp

    fn float_compare(&mut self, op: Op, cmpx: bool) -> R {
        let inst = self.inst;
        let lhs = self.read_f32(inst.src(0))?;
        let rhs = self.read_f32(inst.src(1))?;
        // Source FPCompareFlags::flush_input_denorms (compute FLOAT_MODE).
        let aux = u32::from(self.flush_f32_inputs);
        let value = self.e.emit_aux(op, aux, &[lhs, rhs])?;
        self.compare_result(value, false, cmpx)
    }

    fn float_ordered_compare(&mut self, ordered: bool, cmpx: bool) -> R {
        let inst = self.inst;
        let lhs = self.read_f32(inst.src(0))?;
        let rhs = self.read_f32(inst.src(1))?;
        let lhs_nan = self.e.emit(Op::FPIsNan32, &[lhs])?;
        let rhs_nan = self.e.emit(Op::FPIsNan32, &[rhs])?;
        let unordered = self.e.lor(lhs_nan, rhs_nan)?;
        let value = if ordered {
            self.e.lnot(unordered)?
        } else {
            unordered
        };
        self.compare_result(value, false, cmpx)
    }

    fn float_class_compare(&mut self, cmpx: bool) -> R {
        let inst = self.inst;
        let value = self.read_f32(inst.src(0))?;
        let mask = self.read_u32(inst.src(1))?;
        let result = self.e.emit(Op::FPCmpClass32, &[value, mask])?;
        self.compare_result(result, false, cmpx)
    }

    // ------------------------------------------------------ Convert.cpp

    /// Source `ConvertF32ToU32Saturated`.
    fn f32_to_u32_saturated(
        &mut self,
        value: Value,
        upper_bound: f32,
        safe_upper: f32,
        upper_result: u32,
    ) -> Result<Value, CompileError> {
        let zero = Value::f32(0.0);
        let nan = self.e.emit(Op::FPIsNan32, &[value])?;
        let low = self.e.emit(Op::FPOrdLessThanEqual32, &[value, zero])?;
        let high = self.e.emit(
            Op::FPOrdGreaterThanEqual32,
            &[value, Value::f32(upper_bound)],
        )?;
        let truncated = self.e.emit(Op::FPTrunc32, &[value])?;
        let low_or_nan = self.e.lor(nan, low)?;
        let safe_low = self.e.select(low_or_nan, zero, truncated)?;
        let safe = self.e.select(high, Value::f32(safe_upper), safe_low)?;
        let converted = self.e.emit(Op::ConvertU32F32, &[safe])?;
        self.e.select(high, u32v(upper_result), converted)
    }

    /// Source `ConvertF32ToI32Saturated` with the 32-bit bounds.
    fn f32_to_i32_full(&mut self, value: Value) -> Result<Value, CompileError> {
        self.f32_to_i32_saturated(
            value,
            -2_147_483_648.0,
            2_147_483_648.0,
            2_147_483_520.0,
            0x8000_0000,
            0x7fff_ffff,
        )
    }

    fn f32_to_i32_saturated(
        &mut self,
        value: Value,
        lower_bound: f32,
        upper_bound: f32,
        safe_upper: f32,
        lower_result: u32,
        upper_result: u32,
    ) -> Result<Value, CompileError> {
        let nan = self.e.emit(Op::FPIsNan32, &[value])?;
        let low = self
            .e
            .emit(Op::FPOrdLessThanEqual32, &[value, Value::f32(lower_bound)])?;
        let high = self.e.emit(
            Op::FPOrdGreaterThanEqual32,
            &[value, Value::f32(upper_bound)],
        )?;
        let truncated = self.e.emit(Op::FPTrunc32, &[value])?;
        let safe_low = self.e.select(low, Value::f32(lower_bound), truncated)?;
        let safe_high = self.e.select(high, Value::f32(safe_upper), safe_low)?;
        let safe = self.e.select(nan, Value::f32(0.0), safe_high)?;
        let converted = self.e.emit(Op::ConvertS32F32, &[safe])?;
        let clamped_high = self.e.select(high, u32v(upper_result), converted)?;
        let clamped = self.e.select(low, u32v(lower_result), clamped_high)?;
        self.e.select(nan, u32v(0), clamped)
    }

    fn cvt_f32_ubyte(&mut self, byte_index: u32) -> R {
        let inst = self.inst;
        let source = self.read_u32(inst.src(0))?;
        let shifted = self.e.shr(source, u32v(byte_index * 8))?;
        let byte = self.e.and(shifted, u32v(0xff))?;
        let result = self.e.emit(Op::ConvertF32U32, &[byte])?;
        self.write(inst.dst, result)
    }

    fn frexp_exp(&mut self) -> R {
        let inst = self.inst;
        let source = self.read_f32(inst.src(0))?;
        let bits = self.e.bitcast_u32(source)?;
        let exponent = self
            .e
            .emit(Op::BitFieldUExtract, &[bits, u32v(23), u32v(8)])?;
        let mantissa = self.e.and(bits, u32v(0x007f_ffff))?;
        let normal = self.e.isub(exponent, u32v(126))?;
        let msb = self.e.emit(Op::FindUMsb32, &[mantissa])?;
        let subnormal = self.e.isub(msb, u32v(148))?;
        let mantissa_nonzero = self.e.ine(mantissa, u32v(0))?;
        let denormal = self.e.select(mantissa_nonzero, subnormal, u32v(0))?;
        let exponent_nonzero = self.e.ine(exponent, u32v(0))?;
        let finite_value = self.e.select(exponent_nonzero, normal, denormal)?;
        let finite = self.e.ine(exponent, u32v(0xff))?;
        let result = self.e.select(finite, finite_value, u32v(0))?;
        self.write(inst.dst, result)
    }

    fn cvt_pknorm(&mut self, op: Op) -> R {
        let inst = self.inst;
        let lhs = self.read_f32(inst.src(0))?;
        let rhs = self.read_f32(inst.src(1))?;
        let pair = self.e.emit(Op::CompositeConstructF32x2, &[lhs, rhs])?;
        let result = self.e.emit(op, &[pair])?;
        self.write(inst.dst, result)
    }

    // -------------------------------------------------------- Float.cpp

    fn float_unary(&mut self, op: Op) -> R {
        let inst = self.inst;
        let value = self.read(inst.src(0), op.args()[0])?;
        let result = self.e.emit(op, &[value])?;
        self.write(inst.dst, result)
    }

    fn float_binary(&mut self, op: Op, reverse: bool) -> R {
        let inst = self.inst;
        let mut args = [u32v(0); 2];
        for (index, slot) in args.iter_mut().enumerate() {
            let source = if reverse { 1 - index } else { index };
            *slot = self.read(inst.src(source), op.args()[index])?;
        }
        let result = self.e.emit(op, &args)?;
        self.write(inst.dst, result)
    }

    /// Source `ReadMixF32`; the f16 (op_sel_hi) form is refused.
    fn read_mix_f32(&mut self, op: Opnd) -> Result<Value, CompileError> {
        if op.op_sel_hi {
            return Err(self.unsupported(UnsupportedReason::Float16Operation));
        }
        let mut plain = op;
        plain.op_sel = false;
        plain.neg_hi = false;
        self.read_f32(plain)
    }

    fn float_ternary(&mut self, op: Op, accumulator: bool, mix: bool) -> R {
        let inst = self.inst;
        let mut args = [u32v(0); 3];
        for (index, slot) in args.iter_mut().enumerate() {
            let operand = if accumulator && index == 2 {
                inst.dst
            } else {
                inst.src(index)
            };
            let ty = op.args()[index];
            *slot = if ty == Type::F32 && mix {
                self.read_mix_f32(operand)?
            } else {
                self.read(operand, ty)?
            };
        }
        let result = self.e.emit(op, &args)?;
        self.write(inst.dst, result)
    }

    fn frexp_mant(&mut self) -> R {
        let inst = self.inst;
        let bits = self.read_u32(inst.src(0))?;
        let exponent = self
            .e
            .emit(Op::BitFieldUExtract, &[bits, u32v(23), u32v(8)])?;
        let mantissa = self.e.and(bits, u32v(0x007f_ffff))?;
        let sign = self.e.and(bits, u32v(0x8000_0000))?;
        let base = self.e.or(sign, u32v(0x3f00_0000))?;
        let normal = self.e.or(base, mantissa)?;
        let msb = self.e.emit(Op::FindUMsb32, &[mantissa])?;
        let shift = self.e.isub(u32v(23), msb)?;
        let shifted = self.e.shl(mantissa, shift)?;
        let fraction = self.e.and(shifted, u32v(0x007f_ffff))?;
        let subnormal = self.e.or(base, fraction)?;
        let zero = self.e.ieq(mantissa, u32v(0))?;
        let exponent_nonzero = self.e.ine(exponent, u32v(0))?;
        let small = self.e.select(zero, bits, subnormal)?;
        let finite = self.e.select(exponent_nonzero, normal, small)?;
        let special = self.e.ieq(exponent, u32v(0xff))?;
        let result_bits = self.e.select(special, bits, finite)?;
        let result = self.e.bitcast_f32(result_bits)?;
        self.write(inst.dst, result)
    }

    fn float_cube(&mut self, kind: u32) -> R {
        let inst = self.inst;
        let x = self.read_mix_f32(inst.src(0))?;
        let y = self.read_mix_f32(inst.src(1))?;
        let z = self.read_mix_f32(inst.src(2))?;
        let nx = self.e.emit(Op::FPNeg32, &[x])?;
        let ny = self.e.emit(Op::FPNeg32, &[y])?;
        let nz = self.e.emit(Op::FPNeg32, &[z])?;
        let ax = self.e.emit(Op::FPAbs32, &[x])?;
        let ay = self.e.emit(Op::FPAbs32, &[y])?;
        let az = self.e.emit(Op::FPAbs32, &[z])?;
        let zx = self.e.emit(Op::FPOrdGreaterThanEqual32, &[az, ax])?;
        let zy = self.e.emit(Op::FPOrdGreaterThanEqual32, &[az, ay])?;
        let z_face = self.e.land(zx, zy)?;
        let y_face = self.e.emit(Op::FPOrdGreaterThanEqual32, &[ay, ax])?;
        let zero = Value::f32(0.0);
        let x_neg = self.e.emit(Op::FPOrdLessThan32, &[x, zero])?;
        let y_neg = self.e.emit(Op::FPOrdLessThan32, &[y, zero])?;
        let z_neg = self.e.emit(Op::FPOrdLessThan32, &[z, zero])?;
        let (xv, yv, zv) = match kind {
            0 => (
                self.e.select(x_neg, Value::f32(1.0), Value::f32(0.0))?,
                self.e.select(y_neg, Value::f32(3.0), Value::f32(2.0))?,
                self.e.select(z_neg, Value::f32(5.0), Value::f32(4.0))?,
            ),
            1 => (
                self.e.select(x_neg, z, nz)?,
                x,
                self.e.select(z_neg, nx, x)?,
            ),
            2 => (ny, self.e.select(y_neg, nz, z)?, ny),
            _ => {
                let two = Value::f32(2.0);
                (
                    self.e.emit(Op::FPMul32, &[x, two])?,
                    self.e.emit(Op::FPMul32, &[y, two])?,
                    self.e.emit(Op::FPMul32, &[z, two])?,
                )
            }
        };
        let yx = self.e.select(y_face, yv, xv)?;
        let result = self.e.select(z_face, zv, yx)?;
        self.write(inst.dst, result)
    }

    // ------------------------------------------------------ Integer.cpp

    fn ternary_bits(&mut self, first: Op, second: Op) -> R {
        let inst = self.inst;
        let a = self.read_u32(inst.src(0))?;
        let b = self.read_u32(inst.src(1))?;
        let ab = self.e.emit(first, &[a, b])?;
        let c = self.read_u32(inst.src(2))?;
        let result = self.e.emit(second, &[ab, c])?;
        self.write(inst.dst, result)
    }

    fn integer24(&mut self, sign: bool, addend: bool) -> R {
        let inst = self.inst;
        let op = if sign {
            Op::BitFieldSExtract
        } else {
            Op::BitFieldUExtract
        };
        let a = self.read_u32(inst.src(0))?;
        let lhs = self.e.emit(op, &[a, u32v(0), u32v(24)])?;
        let b = self.read_u32(inst.src(1))?;
        let rhs = self.e.emit(op, &[b, u32v(0), u32v(24)])?;
        let mut result = self.e.imul(lhs, rhs)?;
        if addend {
            let c = self.read_u32(inst.src(2))?;
            result = self.e.iadd(result, c)?;
        }
        self.write(inst.dst, result)
    }

    fn mad_u64_u32(&mut self) -> R {
        let inst = self.inst;
        let lhs = self.read_u32(inst.src(0))?;
        let rhs = self.read_u32(inst.src(1))?;
        let low = self.e.imul(lhs, rhs)?;
        let high = self.e.emit(Op::UMulHi, &[lhs, rhs])?;
        let product = self.e.construct_u64(low, high)?;
        let addend = self.read(inst.src(2), Type::U64)?;
        let result = self.e.emit(Op::IAdd64, &[product, addend])?;
        self.write(inst.dst, result)?;
        if !matches!(inst.dst2.kind, OperandKind::Null | OperandKind::Unknown) {
            let carry = self.e.emit(Op::ULessThan64, &[result, product])?;
            self.write_mask(inst.dst2, carry, false)?;
        }
        Ok(())
    }

    fn mbcnt(&mut self, low: bool) -> R {
        let inst = self.inst;
        let lane = self.e.get(Op::LaneId, 0)?;
        let local = self.e.and(lane, u32v(31))?;
        let bit = self.e.shl(u32v(1), local)?;
        let below = self.e.isub(bit, u32v(1))?;
        let high_lane = self.e.emit(Op::UGreaterThanEqual32, &[lane, u32v(32)])?;
        let thread_mask = if low {
            self.e.select(high_lane, u32v(u32::MAX), below)?
        } else {
            self.e.select(high_lane, below, u32v(0))?
        };
        let source = self.read_u32(inst.src(0))?;
        let active = self.e.and(source, thread_mask)?;
        let count = self.e.emit(Op::BitCount32, &[active])?;
        let addend = self.read_u32(inst.src(1))?;
        let result = self.e.iadd(count, addend)?;
        self.write(inst.dst, result)
    }

    fn v_bfe(&mut self, sign: bool) -> R {
        let inst = self.inst;
        let source = self.read_u32(inst.src(0))?;
        let raw_offset = self.read_u32(inst.src(1))?;
        let offset = self.e.and(raw_offset, u32v(31))?;
        let raw_count_value = self.read_u32(inst.src(2))?;
        let raw_count = self.e.and(raw_count_value, u32v(31))?;
        let available = self.e.isub(u32v(32), offset)?;
        let count = self.e.emit(Op::UMin32, &[raw_count, available])?;
        let op = if sign {
            Op::BitFieldSExtract
        } else {
            Op::BitFieldUExtract
        };
        let result = self.e.emit(op, &[source, offset, count])?;
        self.write(inst.dst, result)
    }

    fn align(&mut self, byte_offset: bool) -> R {
        let inst = self.inst;
        let hi = self.read_u32(inst.src(0))?;
        let lo = self.read_u32(inst.src(1))?;
        let raw = self.read_u32(inst.src(2))?;
        let mut shift = self.e.and(raw, u32v(if byte_offset { 3 } else { 31 }))?;
        if byte_offset {
            shift = self.e.shl(shift, u32v(3))?;
        }
        let lo_part = self.e.shr(lo, shift)?;
        let complement = self.e.isub(u32v(32), shift)?;
        let inverse = self.e.and(complement, u32v(31))?;
        let hi_raw = self.e.shl(hi, inverse)?;
        let nonzero = self.e.ine(shift, u32v(0))?;
        let hi_part = self.e.select(nonzero, hi_raw, u32v(0))?;
        let result = self.e.or(lo_part, hi_part)?;
        self.write(inst.dst, result)
    }

    fn perm(&mut self) -> R {
        let inst = self.inst;
        let hi = self.read_u32(inst.src(0))?;
        let lo = self.read_u32(inst.src(1))?;
        let selector = self.read_u32(inst.src(2))?;
        let mut result = u32v(0);
        for byte in 0..4u32 {
            let sel = self
                .e
                .emit(Op::BitFieldUExtract, &[selector, u32v(byte * 8), u32v(8)])?;
            let bit4 = self.e.and(sel, u32v(4))?;
            let from_hi = self.e.ine(bit4, u32v(0))?;
            let word = self.e.select(from_hi, hi, lo)?;
            let low_bits = self.e.and(sel, u32v(3))?;
            let shift = self.e.shl(low_bits, u32v(3))?;
            let shifted = self.e.shr(word, shift)?;
            let data = self.e.and(shifted, u32v(0xff))?;
            let bit2 = self.e.and(sel, u32v(2))?;
            let sign_hi = self.e.ine(bit2, u32v(0))?;
            let sign_word = self.e.select(sign_hi, hi, lo)?;
            let bit1 = self.e.and(sel, u32v(1))?;
            let upper = self.e.ine(bit1, u32v(0))?;
            let sign_bit = self.e.select(upper, u32v(31), u32v(15))?;
            let sign_raw = self
                .e
                .emit(Op::BitFieldSExtract, &[sign_word, sign_bit, u32v(1)])?;
            let sign = self.e.and(sign_raw, u32v(0xff))?;
            let copy = self.e.ult(sel, u32v(8))?;
            let mut value = self.e.select(copy, data, sign)?;
            let is_zero = self.e.ieq(sel, u32v(12))?;
            value = self.e.select(is_zero, u32v(0), value)?;
            let is_ones = self.e.ugt(sel, u32v(12))?;
            value = self.e.select(is_ones, u32v(0xff), value)?;
            let placed = self.e.shl(value, u32v(byte * 8))?;
            result = self.e.or(result, placed)?;
        }
        self.write(inst.dst, result)
    }

    fn cndmask(&mut self) -> R {
        let inst = self.inst;
        let mask_operand = if inst.src_count >= 3 {
            inst.src(2)
        } else {
            Opnd::special(OperandKind::VccLo)
        };
        let condition = self.read_mask(mask_operand)?;
        let (a, b) = (inst.src(0), inst.src(1));
        let result = if a.neg || a.abs || b.neg || b.abs {
            let t = self.read_f32(b)?;
            let f = self.read_f32(a)?;
            self.e.emit(Op::SelectF32, &[condition, t, f])?
        } else {
            let t = self.read_u32(b)?;
            let f = self.read_u32(a)?;
            self.e.select(condition, t, f)?
        };
        self.write(inst.dst, result)
    }
}
