// SPDX-License-Identifier: MIT AND GPL-2.0-only
// Numeric SPIR-V and GLSL.std.450 enumerants transcribed from the official
// Khronos SPIRV-Headers at 01e0577914a75a2569c846778c2f93aa8e6feddd
// (include/spirv/unified1/spirv.hpp and GLSL.std.450.h), the submodule pin of
// KytyPS5-Performance 209dffe9032d72749257ce18680d506b891284de. Only the
// enumerants this crate emits are listed; names keep the header spelling.
//
// Original notice of the source headers:
//   Copyright: 2014-2024 The Khronos Group Inc.
//   License: MIT
//   MODIFICATIONS TO THIS FILE MAY MEAN IT NO LONGER ACCURATELY REFLECTS
//   KHRONOS STANDARDS. THE UNMODIFIED, NORMATIVE VERSIONS OF KHRONOS
//   SPECIFICATIONS AND HEADER INFORMATION ARE LOCATED AT
//   https://www.khronos.org/registry/
// The complete MIT permission text is retained in Khronos-MIT.txt.
// PentaX modification: selected enumerants rewritten as Rust constants.

#![allow(non_upper_case_globals, dead_code)]

pub const MagicNumber: u32 = 0x0723_0203;
pub const WordCountShift: u32 = 16;
pub const OpCodeMask: u32 = 0xffff;

// Op
pub const OpName: u32 = 5;
pub const OpExtension: u32 = 10;
pub const OpExtInstImport: u32 = 11;
pub const OpExtInst: u32 = 12;
pub const OpMemoryModel: u32 = 14;
pub const OpEntryPoint: u32 = 15;
pub const OpExecutionMode: u32 = 16;
pub const OpCapability: u32 = 17;
pub const OpTypeVoid: u32 = 19;
pub const OpTypeBool: u32 = 20;
pub const OpTypeInt: u32 = 21;
pub const OpTypeFloat: u32 = 22;
pub const OpTypeVector: u32 = 23;
pub const OpTypeImage: u32 = 25;
pub const OpTypeSampler: u32 = 26;
pub const OpTypeSampledImage: u32 = 27;
pub const OpTypeArray: u32 = 28;
pub const OpTypeRuntimeArray: u32 = 29;
pub const OpTypeStruct: u32 = 30;
pub const OpTypePointer: u32 = 32;
pub const OpTypeFunction: u32 = 33;
pub const OpConstantTrue: u32 = 41;
pub const OpConstantFalse: u32 = 42;
pub const OpConstant: u32 = 43;
pub const OpConstantComposite: u32 = 44;
pub const OpConstantNull: u32 = 46;
pub const OpFunction: u32 = 54;
pub const OpFunctionEnd: u32 = 56;
pub const OpVariable: u32 = 59;
pub const OpLoad: u32 = 61;
pub const OpStore: u32 = 62;
pub const OpAccessChain: u32 = 65;
pub const OpDecorate: u32 = 71;
pub const OpMemberDecorate: u32 = 72;
pub const OpUndef: u32 = 1;
pub const OpCompositeConstruct: u32 = 80;
pub const OpCompositeExtract: u32 = 81;
pub const OpCompositeInsert: u32 = 82;
pub const OpVectorShuffle: u32 = 79;
pub const OpSampledImage: u32 = 86;
pub const OpImageSampleImplicitLod: u32 = 87;
pub const OpImageSampleExplicitLod: u32 = 88;
pub const OpImageQueryLod: u32 = 105;
pub const OpConvertFToU: u32 = 109;
pub const OpConvertFToS: u32 = 110;
pub const OpConvertSToF: u32 = 111;
pub const OpConvertUToF: u32 = 112;
pub const OpFConvert: u32 = 115;
pub const OpBitcast: u32 = 124;
pub const OpSNegate: u32 = 126;
pub const OpFNegate: u32 = 127;
pub const OpIAdd: u32 = 128;
pub const OpFAdd: u32 = 129;
pub const OpISub: u32 = 130;
pub const OpFSub: u32 = 131;
pub const OpIMul: u32 = 132;
pub const OpFMul: u32 = 133;
pub const OpUDiv: u32 = 134;
pub const OpSDiv: u32 = 135;
pub const OpFDiv: u32 = 136;
pub const OpIAddCarry: u32 = 149;
pub const OpISubBorrow: u32 = 150;
pub const OpUMulExtended: u32 = 151;
pub const OpSMulExtended: u32 = 152;
pub const OpArrayLength: u32 = 68;
pub const OpIsNan: u32 = 156;
pub const OpIsInf: u32 = 157;
pub const OpLogicalEqual: u32 = 164;
pub const OpLogicalNotEqual: u32 = 165;
pub const OpLogicalOr: u32 = 166;
pub const OpLogicalAnd: u32 = 167;
pub const OpLogicalNot: u32 = 168;
pub const OpSelect: u32 = 169;
pub const OpIEqual: u32 = 170;
pub const OpINotEqual: u32 = 171;
pub const OpUGreaterThan: u32 = 172;
pub const OpSGreaterThan: u32 = 173;
pub const OpUGreaterThanEqual: u32 = 174;
pub const OpSGreaterThanEqual: u32 = 175;
pub const OpULessThan: u32 = 176;
pub const OpSLessThan: u32 = 177;
pub const OpULessThanEqual: u32 = 178;
pub const OpSLessThanEqual: u32 = 179;
pub const OpFOrdEqual: u32 = 180;
pub const OpFUnordEqual: u32 = 181;
pub const OpFOrdNotEqual: u32 = 182;
pub const OpFUnordNotEqual: u32 = 183;
pub const OpFOrdLessThan: u32 = 184;
pub const OpFUnordLessThan: u32 = 185;
pub const OpFOrdGreaterThan: u32 = 186;
pub const OpFUnordGreaterThan: u32 = 187;
pub const OpFOrdLessThanEqual: u32 = 188;
pub const OpFUnordLessThanEqual: u32 = 189;
pub const OpFOrdGreaterThanEqual: u32 = 190;
pub const OpFUnordGreaterThanEqual: u32 = 191;
pub const OpShiftRightLogical: u32 = 194;
pub const OpShiftRightArithmetic: u32 = 195;
pub const OpShiftLeftLogical: u32 = 196;
pub const OpBitwiseOr: u32 = 197;
pub const OpBitwiseXor: u32 = 198;
pub const OpBitwiseAnd: u32 = 199;
pub const OpNot: u32 = 200;
pub const OpBitFieldInsert: u32 = 201;
pub const OpBitFieldSExtract: u32 = 202;
pub const OpBitFieldUExtract: u32 = 203;
pub const OpBitReverse: u32 = 204;
pub const OpBitCount: u32 = 205;
pub const OpControlBarrier: u32 = 224;
pub const OpMemoryBarrier: u32 = 225;
pub const OpPhi: u32 = 245;
pub const OpLoopMerge: u32 = 246;
pub const OpSelectionMerge: u32 = 247;
pub const OpLabel: u32 = 248;
pub const OpBranch: u32 = 249;
pub const OpBranchConditional: u32 = 250;
pub const OpReturn: u32 = 253;
pub const OpUnreachable: u32 = 255;
pub const OpGroupNonUniformAny: u32 = 335;
pub const OpGroupNonUniformBallot: u32 = 339;
pub const OpGroupNonUniformBroadcastFirst: u32 = 338;

// Capability
pub const CapabilityShader: u32 = 1;
pub const CapabilityInt64: u32 = 11;
pub const CapabilityFloat64: u32 = 10;
pub const CapabilitySampled1D: u32 = 43;
pub const CapabilityImage1D: u32 = 44;
pub const CapabilityImageQuery: u32 = 50;
pub const CapabilityDrawParameters: u32 = 4427;
pub const CapabilityGroupNonUniform: u32 = 61;
pub const CapabilityGroupNonUniformVote: u32 = 62;
pub const CapabilityGroupNonUniformBallot: u32 = 64;

// Decoration
pub const DecorationBlock: u32 = 2;
pub const DecorationArrayStride: u32 = 6;
pub const DecorationBuiltIn: u32 = 11;
pub const DecorationNoPerspective: u32 = 13;
pub const DecorationFlat: u32 = 14;
pub const DecorationCentroid: u32 = 16;
pub const DecorationSample: u32 = 17;
pub const DecorationNonWritable: u32 = 24;
pub const DecorationCoherent: u32 = 23;
pub const DecorationAliased: u32 = 20;
pub const DecorationLocation: u32 = 30;
pub const DecorationIndex: u32 = 32;
pub const DecorationBinding: u32 = 33;
pub const DecorationDescriptorSet: u32 = 34;
pub const DecorationOffset: u32 = 35;
pub const DecorationNoContraction: u32 = 42;

// BuiltIn
pub const BuiltInPosition: u32 = 0;
pub const BuiltInVertexIndex: u32 = 42;
pub const BuiltInInstanceIndex: u32 = 43;
pub const BuiltInBaseInstance: u32 = 4425;
pub const BuiltInFragCoord: u32 = 15;
pub const BuiltInFrontFacing: u32 = 17;
pub const BuiltInFragDepth: u32 = 22;
pub const BuiltInWorkgroupId: u32 = 26;
pub const BuiltInLocalInvocationId: u32 = 27;
pub const BuiltInLocalInvocationIndex: u32 = 29;
pub const BuiltInGlobalInvocationId: u32 = 28;
pub const BuiltInSubgroupLocalInvocationId: u32 = 41;

// StorageClass
pub const StorageClassUniformConstant: u32 = 0;
pub const StorageClassInput: u32 = 1;
pub const StorageClassUniform: u32 = 2;
pub const StorageClassOutput: u32 = 3;
pub const StorageClassWorkgroup: u32 = 4;
pub const StorageClassPrivate: u32 = 6;
pub const StorageClassFunction: u32 = 7;
pub const StorageClassPushConstant: u32 = 9;
pub const StorageClassStorageBuffer: u32 = 12;

// Execution
pub const ExecutionModelVertex: u32 = 0;
pub const ExecutionModelFragment: u32 = 4;
pub const ExecutionModelGLCompute: u32 = 5;
pub const ExecutionModeOriginUpperLeft: u32 = 7;
pub const ExecutionModeDepthReplacing: u32 = 12;
pub const ExecutionModeLocalSize: u32 = 17;
pub const AddressingModelLogical: u32 = 0;
pub const MemoryModelGLSL450: u32 = 1;

// Scope, semantics and masks
pub const ScopeDevice: u32 = 1;
pub const ScopeWorkgroup: u32 = 2;
pub const ScopeSubgroup: u32 = 3;
pub const ScopeInvocation: u32 = 4;
pub const MemorySemanticsMaskNone: u32 = 0;
pub const MemorySemanticsReleaseMask: u32 = 0x00000004;
pub const MemorySemanticsAcquireReleaseMask: u32 = 0x00000008;
pub const MemorySemanticsUniformMemoryMask: u32 = 0x00000040;
pub const MemorySemanticsWorkgroupMemoryMask: u32 = 0x00000100;
pub const MemorySemanticsImageMemoryMask: u32 = 0x00000800;
pub const FunctionControlMaskNone: u32 = 0;
pub const SelectionControlMaskNone: u32 = 0;
pub const LoopControlMaskNone: u32 = 0;
pub const MemoryAccessMaskNone: u32 = 0;
pub const MemoryAccessVolatileMask: u32 = 0x00000001;
pub const Dim1D: u32 = 0;
pub const Dim2D: u32 = 1;
pub const Dim3D: u32 = 2;
pub const ImageOperandsLodMask: u32 = 0x00000002;
pub const ImageFormatUnknown: u32 = 0;

// GLSL.std.450 (GLSL.std.450.h)
pub const GLSLstd450RoundEven: u32 = 2;
pub const GLSLstd450Trunc: u32 = 3;
pub const GLSLstd450FAbs: u32 = 4;
pub const GLSLstd450Floor: u32 = 8;
pub const GLSLstd450Ceil: u32 = 9;
pub const GLSLstd450Fract: u32 = 10;
pub const GLSLstd450Sin: u32 = 13;
pub const GLSLstd450Cos: u32 = 14;
pub const GLSLstd450Exp2: u32 = 29;
pub const GLSLstd450Log2: u32 = 30;
pub const GLSLstd450Sqrt: u32 = 31;
pub const GLSLstd450InverseSqrt: u32 = 32;
pub const GLSLstd450FMin: u32 = 37;
pub const GLSLstd450FMax: u32 = 40;
pub const GLSLstd450FClamp: u32 = 43;
pub const GLSLstd450Fma: u32 = 50;
pub const GLSLstd450Ldexp: u32 = 53;
pub const GLSLstd450PackSnorm2x16: u32 = 56;
pub const GLSLstd450PackUnorm2x16: u32 = 57;
pub const GLSLstd450PackHalf2x16: u32 = 58;
pub const GLSLstd450UnpackSnorm2x16: u32 = 60;
pub const GLSLstd450UnpackUnorm2x16: u32 = 61;
pub const GLSLstd450UnpackHalf2x16: u32 = 62;
pub const GLSLstd450FindILsb: u32 = 73;
pub const GLSLstd450FindUMsb: u32 = 75;
pub const GLSLstd450NMin: u32 = 79;
pub const GLSLstd450NMax: u32 = 80;

// Float controls (SPV_KHR_float_controls) and helpers used by the emitter.
pub const CapabilitySignedZeroInfNanPreserve: u32 = 4466;
pub const ExecutionModeSignedZeroInfNanPreserve: u32 = 4461;
pub const OpVectorExtractDynamic: u32 = 77;
pub const OpUMod: u32 = 137;
pub const OpAll: u32 = 155;
pub const OpAny: u32 = 154;
pub const DecorationInvariant: u32 = 18;
pub const OpUConvert: u32 = 113;
