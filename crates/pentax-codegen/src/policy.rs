// SPDX-License-Identifier: GPL-2.0-only

use crate::{Code, CpuidFeature, Mnemonic, Register, UnsupportedReason};
use iced_x86::{EncodingKind, Instruction, OpKind};

/// Declared feature policy, not hardware detection or an execution capability.
/// Future workers must verify real CPUID/OS context support independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuFeatures {
    bits: [u64; 4],
}
impl CpuFeatures {
    pub const fn empty() -> Self {
        Self { bits: [0; 4] }
    }
    pub fn with(mut self, feature: CpuidFeature) -> Self {
        let bit = feature as usize;
        if bit < 256 {
            self.bits[bit / 64] |= 1 << (bit % 64);
        }
        self
    }
    pub fn contains(self, feature: CpuidFeature) -> bool {
        let bit = feature as usize;
        bit < 256 && self.bits[bit / 64] & (1 << (bit % 64)) != 0
    }
    /// Widely available x86-64 integer/SSE2 profile. This declares requirements,
    /// rather than proving host support (notably CMOV/CX8/multibyte NOP).
    pub fn x64_sse2() -> Self {
        [
            CpuidFeature::INTEL8086,
            CpuidFeature::INTEL186,
            CpuidFeature::INTEL286,
            CpuidFeature::INTEL386,
            CpuidFeature::INTEL486,
            CpuidFeature::X64,
            CpuidFeature::CMOV,
            CpuidFeature::CX8,
            CpuidFeature::MULTIBYTENOP,
            // PAUSE is backward-compatible as NOP, including on x86-64 hosts.
            CpuidFeature::PAUSE,
            CpuidFeature::SSE,
            CpuidFeature::SSE2,
        ]
        .into_iter()
        .fold(Self::empty(), Self::with)
    }
}

/// Immutable positive instruction-family policy. Adding CPUID bits never admits
/// another semantic family, privileged instruction or extended register state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranslationPolicy {
    features: CpuFeatures,
    isolate_memory: bool,
    independent_memory: bool,
    split_partial_windows: bool,
    native_ymm_registers: bool,
    native_packed_float_memory: bool,
    native_streaming_stores: bool,
    native_vector_moves: bool,
    native_scalar_float: bool,
    native_scalar_fma: bool,
    native_fma_registers: bool,
    native_float_min_max: bool,
    native_vector_broadcasts: bool,
    affine_loop_trace: bool,
    register_loop_trace: bool,
}
impl TranslationPolicy {
    pub const fn new(features: CpuFeatures) -> Self {
        Self {
            features,
            isolate_memory: false,
            independent_memory: false,
            split_partial_windows: false,
            native_ymm_registers: false,
            native_packed_float_memory: false,
            native_streaming_stores: false,
            native_vector_moves: false,
            native_scalar_float: false,
            native_scalar_fma: false,
            native_fma_registers: false,
            native_float_min_max: false,
            native_vector_broadcasts: false,
            affine_loop_trace: false,
            register_loop_trace: false,
        }
    }
    pub const fn features(self) -> CpuFeatures {
        self.features
    }
    /// Request the closed register-only VEX profile, including twelve packed
    /// ADD/SUB/MUL floating-point forms under the runtime's masked MXCSR policy.
    /// This is not host
    /// detection or execution authority: the runtime must independently bind
    /// it to an admitted full-YMM entry/trailer mode and actual CPU/OS features.
    pub const fn with_native_ymm_registers(mut self) -> Self {
        self.native_ymm_registers = true;
        self
    }
    pub const fn requests_native_ymm_registers(self) -> bool {
        self.native_ymm_registers
    }
    /// Separately request the twelve exact packed VEX ADD/SUB/MUL reads.
    /// This does not imply the full-YMM request or prove live memory authority.
    pub const fn with_native_packed_float_memory(mut self) -> Self {
        self.native_packed_float_memory = true;
        self
    }
    pub const fn requests_native_packed_float_memory(self) -> bool {
        self.native_packed_float_memory
    }
    /// Separately request only exact VEX.128/256 MOVNTDQ stores. The runtime
    /// must prove full-YMM/AVX admission, live aligned writes and fence actual
    /// emitted non-temporal bodies before exposing their effects.
    pub const fn with_native_streaming_stores(mut self) -> Self {
        self.native_streaming_stores = true;
        self
    }
    pub const fn requests_native_streaming_stores(self) -> bool {
        self.native_streaming_stores
    }
    /// Separately request exact temporal VEX vector/scalar memory transfers.
    /// This grants no live backing, alignment or host vector-state authority.
    pub const fn with_native_vector_moves(mut self) -> Self {
        self.native_vector_moves = true;
        self
    }
    pub const fn requests_native_vector_moves(self) -> bool {
        self.native_vector_moves
    }
    /// Separately request exactly VEX scalar ADD/SUB/MUL SS/SD, with an XMM
    /// or ordinary4/8-byte right source. Runtime admission must bind full-YMM
    /// state, actual AVX, masked MXCSR and every live memory read independently.
    pub const fn with_native_scalar_float(mut self) -> Self {
        self.native_scalar_float = true;
        self
    }
    pub const fn requests_native_scalar_float(self) -> bool {
        self.native_scalar_float
    }
    /// Separately request only register-source VFMADD213SS/VFNMADD213SS.
    /// This does not imply a full-YMM request or host AVX/FMA/OS admission.
    /// The runtime must independently preserve the checked masked MXCSR
    /// profile and retain all destination bits used by destructive scalar FMA.
    pub const fn with_native_scalar_fma(mut self) -> Self {
        self.native_scalar_fma = true;
        self
    }
    pub const fn requests_native_scalar_fma(self) -> bool {
        self.native_scalar_fma
    }
    /// Separately request the ninety-six supported VEX FMA3 register forms.
    /// This does not imply full-YMM or actual host AVX/FMA/OS authority. The
    /// runtime must independently retain masked MXCSR and destructive inputs.
    pub const fn with_native_fma_registers(mut self) -> Self {
        self.native_fma_registers = true;
        self
    }
    pub const fn requests_native_fma_registers(self) -> bool {
        self.native_fma_registers
    }
    /// Separately request twelve exact VEX MIN/MAX Codes, with register or
    /// ordinary4/8/16/32-byte read sources. Full-YMM, actual AVX, masked MXCSR
    /// and every live source span remain independent runtime requirements.
    pub const fn with_native_float_min_max(mut self) -> Self {
        self.native_float_min_max = true;
        self
    }
    pub const fn requests_native_float_min_max(self) -> bool {
        self.native_float_min_max
    }
    /// Separately request the sixteen supported VEX broadcast Codes. Exact
    /// source shapes, full-YMM state and actual AVX/AVX2 remain independent
    /// requirements; this grants no live memory or host execution authority.
    pub const fn with_native_vector_broadcasts(mut self) -> Self {
        self.native_vector_broadcasts = true;
        self
    }
    pub const fn requests_native_vector_broadcasts(self) -> bool {
        self.native_vector_broadcasts
    }
    /// Request optional cold preparation of a finite two-iteration trace.
    /// Ordinary block control-flow admission is unchanged. This grants no live
    /// source, memory, CPU, scheduling or native execution authority.
    pub const fn with_affine_loop_trace(mut self) -> Self {
        self.affine_loop_trace = true;
        self
    }
    pub const fn requests_affine_loop_trace(self) -> bool {
        self.affine_loop_trace
    }
    /// Separately request a finite two-iteration register-only self-loop.
    /// This changes no ordinary opcode, source, feature or runtime authority.
    pub const fn with_register_loop_trace(mut self) -> Self {
        self.register_loop_trace = true;
        self
    }
    pub const fn requests_register_loop_trace(self) -> bool {
        self.register_loop_trace
    }
    pub fn x64_sse2() -> Self {
        Self::new(CpuFeatures::x64_sse2())
    }
    /// Split before and after each potential ordinary memory instruction.
    /// This preserves register-only batches. Actual access sizes, register
    /// values, backing and permissions still require a native admission check.
    pub const fn with_isolated_memory(mut self) -> Self {
        self.isolate_memory = true;
        self.independent_memory = false;
        self
    }
    pub const fn isolates_memory(self) -> bool {
        self.isolate_memory
    }
    /// Batch ordinary memory instructions while all address GPRs retain their
    /// entry values. Split before an address dependency; atomics stay isolated.
    /// The dispatcher must independently recheck dependencies and live accesses.
    pub const fn with_independent_memory(mut self) -> Self {
        self.independent_memory = true;
        self.isolate_memory = false;
        self
    }
    pub const fn batches_independent_memory(self) -> bool {
        self.independent_memory
    }
    /// Permit a shorter fetch to end in the middle of an instruction, but only
    /// after at least one complete instruction. Return a fallthrough to that
    /// unconsumed instruction so the next fetch starts there. Invalid encodings
    /// and a truncated first instruction still fail; no bytes are skipped.
    pub const fn with_partial_window_splits(mut self) -> Self {
        self.split_partial_windows = true;
        self
    }
    pub const fn splits_partial_windows(self) -> bool {
        self.split_partial_windows
    }
}

pub(crate) fn potential_memory(instruction: &Instruction) -> bool {
    // LEA's encoding contains an address expression, but it never accesses
    // memory. Its destination write still participates in dependency analysis.
    instruction.mnemonic() != Mnemonic::Lea
        && (instruction.op_kinds().any(|kind| kind == OpKind::Memory)
            || matches!(
                instruction.mnemonic(),
                Mnemonic::Push | Mnemonic::Pop | Mnemonic::Leave
            ))
}

pub(crate) fn prefixes(instruction: &Instruction) -> Result<(), UnsupportedReason> {
    if instruction.has_segment_prefix()
        && !matches!(instruction.mnemonic(), Mnemonic::Nop | Mnemonic::Lea)
    {
        return Err(UnsupportedReason::SegmentPrefix);
    }
    if instruction.has_rep_prefix()
        || instruction.has_repne_prefix()
        || (instruction.has_lock_prefix() && !supported_lock(instruction))
        || instruction.has_xacquire_prefix()
        || instruction.has_xrelease_prefix()
    {
        return Err(UnsupportedReason::RepeatOrLockPrefix);
    }
    Ok(())
}

// Only decoder-validated memory-destination RMW forms are admitted. Bit-string
// offsets and HLE hints need separate semantics and remain refused.
fn supported_lock(instruction: &Instruction) -> bool {
    instruction.op0_kind() == OpKind::Memory
        && matches!(
            instruction.mnemonic(),
            Mnemonic::Add
                | Mnemonic::Adc
                | Mnemonic::And
                | Mnemonic::Or
                | Mnemonic::Xor
                | Mnemonic::Sub
                | Mnemonic::Sbb
                | Mnemonic::Inc
                | Mnemonic::Dec
                | Mnemonic::Neg
                | Mnemonic::Not
                | Mnemonic::Xadd
                | Mnemonic::Xchg
                | Mnemonic::Cmpxchg
                | Mnemonic::Cmpxchg8b
                | Mnemonic::Cmpxchg16b
        )
}

pub(crate) fn atomic_memory(instruction: &Instruction) -> bool {
    instruction.has_lock_prefix()
        || (instruction.mnemonic() == Mnemonic::Xchg
            && instruction.op_kinds().any(|kind| kind == OpKind::Memory))
}

pub(crate) fn gpr(register: Register) -> bool {
    register.is_gpr()
}
fn operand_register(register: Register) -> bool {
    gpr(register) || register.is_xmm() && register <= Register::XMM15
}

pub(crate) fn ordinary(
    instruction: &Instruction,
    policy: TranslationPolicy,
) -> Result<(), UnsupportedReason> {
    prefixes(instruction)?;
    let opcode = instruction.op_code();
    if instruction.is_privileged() || !opcode.cpl3() {
        return Err(UnsupportedReason::Privileged);
    }
    if instruction.is_save_restore_instruction() || opcode.is_save_restore() {
        return Err(UnsupportedReason::SaveRestore);
    }
    // BMI's VEX encoding addresses GPRs only. It does not authorize AVX or
    // extended vector state; declared features and actual host checks still apply.
    let vex_gpr = opcode.encoding() == EncodingKind::VEX && bmi(instruction.mnemonic());
    let vex_memory = opcode.encoding() == EncodingKind::VEX
        && policy.requests_native_ymm_registers()
        && policy.requests_native_packed_float_memory()
        && native_packed_float_memory_width(instruction).is_some();
    let vex_streaming = opcode.encoding() == EncodingKind::VEX
        && policy.requests_native_ymm_registers()
        && policy.requests_native_streaming_stores()
        && native_streaming_store_width(instruction).is_some();
    let vex_moves = opcode.encoding() == EncodingKind::VEX
        && policy.requests_native_ymm_registers()
        && policy.requests_native_vector_moves()
        && native_vector_move_shape(instruction).is_some();
    let vex_scalar = opcode.encoding() == EncodingKind::VEX
        && policy.requests_native_ymm_registers()
        && policy.requests_native_scalar_float()
        && native_scalar_float_width(instruction).is_some();
    let vex_scalar_fma = opcode.encoding() == EncodingKind::VEX
        && policy.requests_native_ymm_registers()
        && policy.requests_native_scalar_fma()
        && native_scalar_fma(instruction);
    let vex_fma_registers = opcode.encoding() == EncodingKind::VEX
        && policy.requests_native_ymm_registers()
        && policy.requests_native_fma_registers()
        && native_fma_registers(instruction);
    let vex_min_max = opcode.encoding() == EncodingKind::VEX
        && policy.requests_native_ymm_registers()
        && policy.requests_native_float_min_max()
        && native_float_min_max_shape(instruction).is_some();
    let vex_broadcast = opcode.encoding() == EncodingKind::VEX
        && policy.requests_native_ymm_registers()
        && policy.requests_native_vector_broadcasts()
        && native_vector_broadcast_shape(instruction).is_some();
    let vex_vector = opcode.encoding() == EncodingKind::VEX
        && policy.requests_native_ymm_registers()
        && (native_ymm_registers(instruction)
            || vex_memory
            || vex_streaming
            || vex_moves
            || vex_scalar
            || vex_scalar_fma
            || vex_fma_registers
            || vex_min_max
            || vex_broadcast);
    if opcode.encoding() != EncodingKind::Legacy && !vex_gpr && !vex_vector {
        return Err(UnsupportedReason::InstructionClass);
    }
    // Admit only the observed F3 90 hint. The decoder consumes mandatory F3;
    // a longer prefixed PAUSE remains outside this exact native profile.
    let pause = instruction.code() == Code::Pause && instruction.len() == 2;
    if !vex_vector && !pause && !integer(instruction.mnemonic()) && !sse(instruction.mnemonic()) {
        return Err(UnsupportedReason::InstructionClass);
    }
    if vex_vector && !policy.features.contains(CpuidFeature::AVX) {
        return Err(UnsupportedReason::MissingFeature(CpuidFeature::AVX));
    }
    if (vex_scalar_fma || vex_fma_registers) && !policy.features.contains(CpuidFeature::FMA) {
        return Err(UnsupportedReason::MissingFeature(CpuidFeature::FMA));
    }
    for feature in instruction.cpuid_features() {
        if !policy.features.contains(*feature) {
            return Err(UnsupportedReason::MissingFeature(*feature));
        }
    }
    if vex_vector {
        // Exact gates prove either same-width registers only, or two such
        // registers plus the separately requested ordinary64-bit read, or one
        // vector source plus a separately requested exact streaming write, or
        // one exact requested temporal vector/scalar transfer, or the separate
        // scalar arithmetic register/exact4-or8-byte read shape, or one of
        // separately requested destructive FMA3 registers, or a closed MIN/MAX
        // register/exact-read shape, or a closed broadcast shape. No other
        // memory/immediate/implicit authority follows from this early return.
        return Ok(());
    }
    // Multi-byte NOP's apparent memory operand is only an encoding field:
    // it computes no address and performs no access, including FS/GS overrides.
    if instruction.mnemonic() == Mnemonic::Nop {
        return Ok(());
    }
    for operand in 0..instruction.op_count() {
        match instruction.op_kind(operand) {
            OpKind::Register
                if if vex_gpr {
                    matches!(instruction.op_register(operand).size(), 4 | 8)
                        && gpr(instruction.op_register(operand))
                } else {
                    operand_register(instruction.op_register(operand))
                } => {}
            OpKind::Memory => {
                let base = instruction.memory_base();
                let index = instruction.memory_index();
                let address_only = instruction.mnemonic() == Mnemonic::Lea;
                if !(base == Register::None
                    || base == Register::RIP
                    || base.is_gpr64()
                    || address_only && (base == Register::EIP || base.is_gpr32()))
                    || !(index == Register::None
                        || index.is_gpr64()
                        || address_only && index.is_gpr32())
                {
                    return Err(UnsupportedReason::AddressSize);
                }
                if !address_only
                    && matches!(instruction.memory_segment(), Register::FS | Register::GS)
                {
                    return Err(UnsupportedReason::SegmentPrefix);
                }
            }
            OpKind::Immediate8
            | OpKind::Immediate8_2nd
            | OpKind::Immediate16
            | OpKind::Immediate32
            | OpKind::Immediate64
            | OpKind::Immediate8to16
            | OpKind::Immediate8to32
            | OpKind::Immediate8to64
            | OpKind::Immediate32to64 => {}
            _ => return Err(UnsupportedReason::OperandClass),
        }
    }
    Ok(())
}

/// Six exact decoded scalar arithmetic forms. Registers remain XMM even for
/// a 4/8-byte memory source; a size here is immutable shape, not live authority.
pub(crate) fn native_scalar_float_width(instruction: &Instruction) -> Option<usize> {
    use Code::*;
    let width = match instruction.code() {
        VEX_Vaddss_xmm_xmm_xmmm32 | VEX_Vsubss_xmm_xmm_xmmm32 | VEX_Vmulss_xmm_xmm_xmmm32 => 4,
        VEX_Vaddsd_xmm_xmm_xmmm64 | VEX_Vsubsd_xmm_xmm_xmmm64 | VEX_Vmulsd_xmm_xmm_xmmm64 => 8,
        _ => return None,
    };
    if instruction.op_code().encoding() != EncodingKind::VEX
        || instruction.code_size() != iced_x86::CodeSize::Code64
        || instruction.op_count() != 3
        || prefixes(instruction).is_err()
        || instruction.has_segment_prefix()
    {
        return None;
    }
    for operand in 0..2 {
        let register = instruction.op_register(operand);
        if instruction.op_kind(operand) != OpKind::Register
            || !register.is_xmm()
            || register > Register::XMM15
        {
            return None;
        }
    }
    match instruction.op2_kind() {
        OpKind::Register => {
            let register = instruction.op2_register();
            if !register.is_xmm() || register > Register::XMM15 {
                return None;
            }
        }
        OpKind::Memory => {
            let base = instruction.memory_base();
            let index = instruction.memory_index();
            if instruction.memory_size().size() != width
                || instruction.is_vsib()
                || !(base == Register::None || base == Register::RIP || base.is_gpr64())
                || !(index == Register::None || index.is_gpr64())
                || !matches!(instruction.memory_index_scale(), 1 | 2 | 4 | 8)
                // Displacement-only addr32 SIB has no GPR to expose its width.
                || (base == Register::None
                    && index == Register::None
                    && instruction.memory_displ_size() != 8)
            {
                return None;
            }
        }
        _ => return None,
    }
    Some(width)
}

/// Closed MIN/MAX shape: (vector register bytes, ordinary read bytes).
/// Scalar merge sources remain XMM. This retains no evaluated address or right.
pub(crate) fn native_float_min_max_shape(instruction: &Instruction) -> Option<(usize, usize)> {
    use Code::*;
    let shape = match instruction.code() {
        VEX_Vminss_xmm_xmm_xmmm32 | VEX_Vmaxss_xmm_xmm_xmmm32 => (16, 4),
        VEX_Vminsd_xmm_xmm_xmmm64 | VEX_Vmaxsd_xmm_xmm_xmmm64 => (16, 8),
        VEX_Vminps_xmm_xmm_xmmm128
        | VEX_Vmaxps_xmm_xmm_xmmm128
        | VEX_Vminpd_xmm_xmm_xmmm128
        | VEX_Vmaxpd_xmm_xmm_xmmm128 => (16, 16),
        VEX_Vminps_ymm_ymm_ymmm256
        | VEX_Vmaxps_ymm_ymm_ymmm256
        | VEX_Vminpd_ymm_ymm_ymmm256
        | VEX_Vmaxpd_ymm_ymm_ymmm256 => (32, 32),
        _ => return None,
    };
    if instruction.op_code().encoding() != EncodingKind::VEX
        || instruction.code_size() != iced_x86::CodeSize::Code64
        || instruction.op_count() != 3
        || prefixes(instruction).is_err()
        || instruction.has_segment_prefix()
    {
        return None;
    }
    for operand in 0..3 {
        if operand == 2 && instruction.op2_kind() == OpKind::Memory {
            let base = instruction.memory_base();
            let index = instruction.memory_index();
            if instruction.memory_size().size() != shape.1
                || instruction.is_vsib()
                || !(base == Register::None || base == Register::RIP || base.is_gpr64())
                || !(index == Register::None || index.is_gpr64())
                || !matches!(instruction.memory_index_scale(), 1 | 2 | 4 | 8)
                || (base == Register::None
                    && index == Register::None
                    && instruction.memory_displ_size() != 8)
            {
                return None;
            }
        } else {
            let register = instruction.op_register(operand);
            if instruction.op_kind(operand) != OpKind::Register
                || if shape.0 == 16 {
                    !register.is_xmm() || register > Register::XMM15
                } else {
                    !register.is_ymm() || register > Register::YMM15
                }
            {
                return None;
            }
        }
    }
    Some(shape)
}

/// Closed broadcast shape: (destination vector bytes, source element bytes).
/// Packed-integer register sources are XMM even for a YMM destination. Floating
/// register and memory forms have distinct Codes and cannot exchange shapes.
pub(crate) fn native_vector_broadcast_shape(instruction: &Instruction) -> Option<(usize, usize)> {
    use Code::*;
    let (destination, element, register, memory) = match instruction.code() {
        VEX_Vpbroadcastb_xmm_xmmm8 => (16, 1, true, true),
        VEX_Vpbroadcastb_ymm_xmmm8 => (32, 1, true, true),
        VEX_Vpbroadcastw_xmm_xmmm16 => (16, 2, true, true),
        VEX_Vpbroadcastw_ymm_xmmm16 => (32, 2, true, true),
        VEX_Vpbroadcastd_xmm_xmmm32 => (16, 4, true, true),
        VEX_Vpbroadcastd_ymm_xmmm32 => (32, 4, true, true),
        VEX_Vpbroadcastq_xmm_xmmm64 => (16, 8, true, true),
        VEX_Vpbroadcastq_ymm_xmmm64 => (32, 8, true, true),
        VEX_Vbroadcastss_xmm_m32 => (16, 4, false, true),
        VEX_Vbroadcastss_ymm_m32 => (32, 4, false, true),
        VEX_Vbroadcastss_xmm_xmm => (16, 4, true, false),
        VEX_Vbroadcastss_ymm_xmm => (32, 4, true, false),
        VEX_Vbroadcastsd_ymm_m64 => (32, 8, false, true),
        VEX_Vbroadcastsd_ymm_xmm => (32, 8, true, false),
        VEX_Vbroadcastf128_ymm_m128 | VEX_Vbroadcasti128_ymm_m128 => (32, 16, false, true),
        _ => return None,
    };
    if instruction.op_code().encoding() != EncodingKind::VEX
        || instruction.code_size() != iced_x86::CodeSize::Code64
        || instruction.op_count() != 2
        || prefixes(instruction).is_err()
        || instruction.has_segment_prefix()
        || instruction.op0_kind() != OpKind::Register
        || if destination == 16 {
            !instruction.op0_register().is_xmm() || instruction.op0_register() > Register::XMM15
        } else {
            !instruction.op0_register().is_ymm() || instruction.op0_register() > Register::YMM15
        }
    {
        return None;
    }
    match instruction.op1_kind() {
        OpKind::Register if register => {
            if !instruction.op1_register().is_xmm() || instruction.op1_register() > Register::XMM15
            {
                return None;
            }
        }
        OpKind::Memory if memory => {
            let base = instruction.memory_base();
            let index = instruction.memory_index();
            if instruction.memory_size().size() != element
                || instruction.is_vsib()
                || !(base == Register::None || base == Register::RIP || base.is_gpr64())
                || !(index == Register::None || index.is_gpr64())
                || !matches!(instruction.memory_index_scale(), 1 | 2 | 4 | 8)
                || (base == Register::None
                    && index == Register::None
                    && instruction.memory_displ_size() != 8)
            {
                return None;
            }
        }
        _ => return None,
    }
    Some((destination, element))
}

/// Exact temporal memory shape: (memory bytes, write). Scalar memory widths
/// are4/8, but their sole register is still XMM. No live address is retained.
pub(crate) fn native_vector_move_shape(instruction: &Instruction) -> Option<(usize, bool)> {
    use Code::*;
    let (width, write) = match instruction.code() {
        VEX_Vmovdqa_xmm_xmmm128
        | VEX_Vmovdqu_xmm_xmmm128
        | VEX_Vmovaps_xmm_xmmm128
        | VEX_Vmovups_xmm_xmmm128
        | VEX_Vmovapd_xmm_xmmm128
        | VEX_Vmovupd_xmm_xmmm128 => (16, false),
        VEX_Vmovdqa_xmmm128_xmm
        | VEX_Vmovdqu_xmmm128_xmm
        | VEX_Vmovaps_xmmm128_xmm
        | VEX_Vmovups_xmmm128_xmm
        | VEX_Vmovapd_xmmm128_xmm
        | VEX_Vmovupd_xmmm128_xmm => (16, true),
        VEX_Vmovdqa_ymm_ymmm256
        | VEX_Vmovdqu_ymm_ymmm256
        | VEX_Vmovaps_ymm_ymmm256
        | VEX_Vmovups_ymm_ymmm256
        | VEX_Vmovapd_ymm_ymmm256
        | VEX_Vmovupd_ymm_ymmm256 => (32, false),
        VEX_Vmovdqa_ymmm256_ymm
        | VEX_Vmovdqu_ymmm256_ymm
        | VEX_Vmovaps_ymmm256_ymm
        | VEX_Vmovups_ymmm256_ymm
        | VEX_Vmovapd_ymmm256_ymm
        | VEX_Vmovupd_ymmm256_ymm => (32, true),
        VEX_Vmovss_xmm_m32 => (4, false),
        VEX_Vmovss_m32_xmm => (4, true),
        VEX_Vmovsd_xmm_m64 => (8, false),
        VEX_Vmovsd_m64_xmm => (8, true),
        _ => return None,
    };
    let register_operand = u32::from(write);
    let memory_operand = u32::from(!write);
    let register = instruction.op_register(register_operand);
    let register_width = if width == 32 { 32 } else { 16 };
    if instruction.op_code().encoding() != EncodingKind::VEX
        || instruction.code_size() != iced_x86::CodeSize::Code64
        || instruction.op_count() != 2
        || instruction.op_kind(memory_operand) != OpKind::Memory
        || instruction.op_kind(register_operand) != OpKind::Register
        || register.size() != register_width
        || !(if width == 32 {
            register.is_ymm() && register <= Register::YMM15
        } else {
            register.is_xmm() && register <= Register::XMM15
        })
        || instruction.memory_size().size() != width
        || instruction.is_vsib()
        || prefixes(instruction).is_err()
        || instruction.has_segment_prefix()
    {
        return None;
    }
    let base = instruction.memory_base();
    let index = instruction.memory_index();
    if !(base == Register::None || base == Register::RIP || base.is_gpr64())
        || !(index == Register::None || index.is_gpr64())
        || !matches!(instruction.memory_index_scale(), 1 | 2 | 4 | 8)
        || (base == Register::None
            && index == Register::None
            && instruction.memory_displ_size() != 8)
    {
        return None;
    }
    Some((width, write))
}

/// Architectural alignment of selected temporal memory transfers only. Shared
/// register encodings and all unaligned/scalar/NT families are excluded.
pub(crate) fn aligned_vector_move_width(instruction: &Instruction) -> Option<usize> {
    use Code::*;
    let width = match instruction.code() {
        Movdqa_xmm_xmmm128
        | Movdqa_xmmm128_xmm
        | Movaps_xmm_xmmm128
        | Movaps_xmmm128_xmm
        | Movapd_xmm_xmmm128
        | Movapd_xmmm128_xmm
        | VEX_Vmovdqa_xmm_xmmm128
        | VEX_Vmovdqa_xmmm128_xmm
        | VEX_Vmovaps_xmm_xmmm128
        | VEX_Vmovaps_xmmm128_xmm
        | VEX_Vmovapd_xmm_xmmm128
        | VEX_Vmovapd_xmmm128_xmm => 16,
        VEX_Vmovdqa_ymm_ymmm256
        | VEX_Vmovdqa_ymmm256_ymm
        | VEX_Vmovaps_ymm_ymmm256
        | VEX_Vmovaps_ymmm256_ymm
        | VEX_Vmovapd_ymm_ymmm256
        | VEX_Vmovapd_ymmm256_ymm => 32,
        _ => return None,
    };
    (instruction.op_count() == 2 && instruction.op_kinds().any(|kind| kind == OpKind::Memory))
        .then_some(width)
}

/// Emitted-only classification for mandatory manual-path span preflight. It
/// includes the new28 VEX forms and only the six aligned legacy counterparts.
pub(crate) fn vector_memory_move(instruction: &Instruction) -> bool {
    native_vector_move_shape(instruction).is_some()
        || aligned_vector_move_width(instruction).is_some()
}

/// Exact ordinary64-bit write shape; never a live address/permission proof.
pub(crate) fn native_streaming_store_width(instruction: &Instruction) -> Option<usize> {
    let width = match instruction.code() {
        Code::VEX_Vmovntdq_m128_xmm => 16,
        Code::VEX_Vmovntdq_m256_ymm => 32,
        _ => return None,
    };
    let source = instruction.op1_register();
    if instruction.op_code().encoding() != EncodingKind::VEX
        || instruction.code_size() != iced_x86::CodeSize::Code64
        || instruction.op_count() != 2
        || instruction.op0_kind() != OpKind::Memory
        || instruction.op1_kind() != OpKind::Register
        || source.size() != width
        || !(if width == 16 {
            source.is_xmm() && source <= Register::XMM15
        } else {
            source.is_ymm() && source <= Register::YMM15
        })
        || instruction.memory_size().size() != width
        || instruction.is_vsib()
        || prefixes(instruction).is_err()
        || instruction.has_segment_prefix()
    {
        return None;
    }
    let base = instruction.memory_base();
    let index = instruction.memory_index();
    if !(base == Register::None || base == Register::RIP || base.is_gpr64())
        || !(index == Register::None || index.is_gpr64())
        || !matches!(instruction.memory_index_scale(), 1 | 2 | 4 | 8)
        || (base == Register::None
            && index == Register::None
            && instruction.memory_displ_size() != 8)
    {
        return None;
    }
    Some(width)
}

/// Exact emitted NT stores need alignment and a runtime store-completion fence.
/// Legacy authority is unchanged; other streaming/vector families stay closed.
pub(crate) fn non_temporal_store_width(instruction: &Instruction) -> Option<usize> {
    match instruction.code() {
        Code::Movntdq_m128_xmm | Code::Movntps_m128_xmm | Code::Movntpd_m128_xmm => Some(16),
        _ => native_streaming_store_width(instruction),
    }
}

/// Exact decoded read shape shared with the immutable footprint evaluator.
/// Returning a size grants no live address, ownership or permission authority.
pub(crate) fn native_packed_float_memory_width(instruction: &Instruction) -> Option<usize> {
    use Code::*;
    let width = match instruction.code() {
        VEX_Vaddps_xmm_xmm_xmmm128
        | VEX_Vaddpd_xmm_xmm_xmmm128
        | VEX_Vsubps_xmm_xmm_xmmm128
        | VEX_Vsubpd_xmm_xmm_xmmm128
        | VEX_Vmulps_xmm_xmm_xmmm128
        | VEX_Vmulpd_xmm_xmm_xmmm128 => 16,
        VEX_Vaddps_ymm_ymm_ymmm256
        | VEX_Vaddpd_ymm_ymm_ymmm256
        | VEX_Vsubps_ymm_ymm_ymmm256
        | VEX_Vsubpd_ymm_ymm_ymmm256
        | VEX_Vmulps_ymm_ymm_ymmm256
        | VEX_Vmulpd_ymm_ymm_ymmm256 => 32,
        _ => return None,
    };
    if instruction.op_code().encoding() != EncodingKind::VEX
        || instruction.code_size() != iced_x86::CodeSize::Code64
        || instruction.op_count() != 3
        || instruction.op2_kind() != OpKind::Memory
        || instruction.memory_size().size() != width
        || instruction.is_vsib()
        || prefixes(instruction).is_err()
        || instruction.has_segment_prefix()
    {
        return None;
    }
    for operand in 0..2 {
        let register = instruction.op_register(operand);
        if instruction.op_kind(operand) != OpKind::Register
            || register.size() != width
            || !(if width == 16 {
                register.is_xmm() && register <= Register::XMM15
            } else {
                register.is_ymm() && register <= Register::YMM15
            })
        {
            return None;
        }
    }
    let base = instruction.memory_base();
    let index = instruction.memory_index();
    if !(base == Register::None || base == Register::RIP || base.is_gpr64())
        || !(index == Register::None || index.is_gpr64())
        || !matches!(instruction.memory_index_scale(), 1 | 2 | 4 | 8)
        // Iced represents a displacement-only64-bit address with displ_size8;
        // addr32 absolute SIB has no GPR to reveal its width and uses size4.
        || (base == Register::None && index == Register::None && instruction.memory_displ_size() != 8)
    {
        return None;
    }
    Some(width)
}

// Exact destructive scalar213SS forms, separately requested and feature-gated.
// DEST supplies both the multiply input and the preserved bits32..127.
// Scalar LIG is canonicalized by the encoder, never copied as raw source bytes.
fn native_scalar_fma(instruction: &Instruction) -> bool {
    matches!(
        instruction.code(),
        Code::VEX_Vfmadd213ss_xmm_xmm_xmmm32 | Code::VEX_Vfnmadd213ss_xmm_xmm_xmmm32
    ) && instruction.code_size() == iced_x86::CodeSize::Code64
        && instruction.op_count() == 3
        && (0..3).all(|operand| {
            let register = instruction.op_register(operand);
            instruction.op_kind(operand) == OpKind::Register
                && register.is_xmm()
                && register <= Register::XMM15
        })
}

// Exact supported VEX FMA3 Codes; no mnemonic-wide or memory-form admission.
// Scalar SS/SD use XMM destructive destinations; packed forms use their
// declared full register width. Scalar LIG is canonicalized by the encoder.
fn native_fma_registers(instruction: &Instruction) -> bool {
    use Code::*;
    let width = match instruction.code() {
        VEX_Vfmadd132ss_xmm_xmm_xmmm32
        | VEX_Vfmadd132sd_xmm_xmm_xmmm64
        | VEX_Vfmadd132ps_xmm_xmm_xmmm128
        | VEX_Vfmadd132pd_xmm_xmm_xmmm128
        | VEX_Vfmadd213ss_xmm_xmm_xmmm32
        | VEX_Vfmadd213sd_xmm_xmm_xmmm64
        | VEX_Vfmadd213ps_xmm_xmm_xmmm128
        | VEX_Vfmadd213pd_xmm_xmm_xmmm128
        | VEX_Vfmadd231ss_xmm_xmm_xmmm32
        | VEX_Vfmadd231sd_xmm_xmm_xmmm64
        | VEX_Vfmadd231ps_xmm_xmm_xmmm128
        | VEX_Vfmadd231pd_xmm_xmm_xmmm128
        | VEX_Vfmsub132ss_xmm_xmm_xmmm32
        | VEX_Vfmsub132sd_xmm_xmm_xmmm64
        | VEX_Vfmsub132ps_xmm_xmm_xmmm128
        | VEX_Vfmsub132pd_xmm_xmm_xmmm128
        | VEX_Vfmsub213ss_xmm_xmm_xmmm32
        | VEX_Vfmsub213sd_xmm_xmm_xmmm64
        | VEX_Vfmsub213ps_xmm_xmm_xmmm128
        | VEX_Vfmsub213pd_xmm_xmm_xmmm128
        | VEX_Vfmsub231ss_xmm_xmm_xmmm32
        | VEX_Vfmsub231sd_xmm_xmm_xmmm64
        | VEX_Vfmsub231ps_xmm_xmm_xmmm128
        | VEX_Vfmsub231pd_xmm_xmm_xmmm128
        | VEX_Vfnmadd132ss_xmm_xmm_xmmm32
        | VEX_Vfnmadd132sd_xmm_xmm_xmmm64
        | VEX_Vfnmadd132ps_xmm_xmm_xmmm128
        | VEX_Vfnmadd132pd_xmm_xmm_xmmm128
        | VEX_Vfnmadd213ss_xmm_xmm_xmmm32
        | VEX_Vfnmadd213sd_xmm_xmm_xmmm64
        | VEX_Vfnmadd213ps_xmm_xmm_xmmm128
        | VEX_Vfnmadd213pd_xmm_xmm_xmmm128
        | VEX_Vfnmadd231ss_xmm_xmm_xmmm32
        | VEX_Vfnmadd231sd_xmm_xmm_xmmm64
        | VEX_Vfnmadd231ps_xmm_xmm_xmmm128
        | VEX_Vfnmadd231pd_xmm_xmm_xmmm128
        | VEX_Vfnmsub132ss_xmm_xmm_xmmm32
        | VEX_Vfnmsub132sd_xmm_xmm_xmmm64
        | VEX_Vfnmsub132ps_xmm_xmm_xmmm128
        | VEX_Vfnmsub132pd_xmm_xmm_xmmm128
        | VEX_Vfnmsub213ss_xmm_xmm_xmmm32
        | VEX_Vfnmsub213sd_xmm_xmm_xmmm64
        | VEX_Vfnmsub213ps_xmm_xmm_xmmm128
        | VEX_Vfnmsub213pd_xmm_xmm_xmmm128
        | VEX_Vfnmsub231ss_xmm_xmm_xmmm32
        | VEX_Vfnmsub231sd_xmm_xmm_xmmm64
        | VEX_Vfnmsub231ps_xmm_xmm_xmmm128
        | VEX_Vfnmsub231pd_xmm_xmm_xmmm128
        | VEX_Vfmaddsub132ps_xmm_xmm_xmmm128
        | VEX_Vfmaddsub132pd_xmm_xmm_xmmm128
        | VEX_Vfmaddsub213ps_xmm_xmm_xmmm128
        | VEX_Vfmaddsub213pd_xmm_xmm_xmmm128
        | VEX_Vfmaddsub231ps_xmm_xmm_xmmm128
        | VEX_Vfmaddsub231pd_xmm_xmm_xmmm128
        | VEX_Vfmsubadd132ps_xmm_xmm_xmmm128
        | VEX_Vfmsubadd132pd_xmm_xmm_xmmm128
        | VEX_Vfmsubadd213ps_xmm_xmm_xmmm128
        | VEX_Vfmsubadd213pd_xmm_xmm_xmmm128
        | VEX_Vfmsubadd231ps_xmm_xmm_xmmm128
        | VEX_Vfmsubadd231pd_xmm_xmm_xmmm128 => 16,
        VEX_Vfmadd132ps_ymm_ymm_ymmm256
        | VEX_Vfmadd132pd_ymm_ymm_ymmm256
        | VEX_Vfmadd213ps_ymm_ymm_ymmm256
        | VEX_Vfmadd213pd_ymm_ymm_ymmm256
        | VEX_Vfmadd231ps_ymm_ymm_ymmm256
        | VEX_Vfmadd231pd_ymm_ymm_ymmm256
        | VEX_Vfmsub132ps_ymm_ymm_ymmm256
        | VEX_Vfmsub132pd_ymm_ymm_ymmm256
        | VEX_Vfmsub213ps_ymm_ymm_ymmm256
        | VEX_Vfmsub213pd_ymm_ymm_ymmm256
        | VEX_Vfmsub231ps_ymm_ymm_ymmm256
        | VEX_Vfmsub231pd_ymm_ymm_ymmm256
        | VEX_Vfnmadd132ps_ymm_ymm_ymmm256
        | VEX_Vfnmadd132pd_ymm_ymm_ymmm256
        | VEX_Vfnmadd213ps_ymm_ymm_ymmm256
        | VEX_Vfnmadd213pd_ymm_ymm_ymmm256
        | VEX_Vfnmadd231ps_ymm_ymm_ymmm256
        | VEX_Vfnmadd231pd_ymm_ymm_ymmm256
        | VEX_Vfnmsub132ps_ymm_ymm_ymmm256
        | VEX_Vfnmsub132pd_ymm_ymm_ymmm256
        | VEX_Vfnmsub213ps_ymm_ymm_ymmm256
        | VEX_Vfnmsub213pd_ymm_ymm_ymmm256
        | VEX_Vfnmsub231ps_ymm_ymm_ymmm256
        | VEX_Vfnmsub231pd_ymm_ymm_ymmm256
        | VEX_Vfmaddsub132ps_ymm_ymm_ymmm256
        | VEX_Vfmaddsub132pd_ymm_ymm_ymmm256
        | VEX_Vfmaddsub213ps_ymm_ymm_ymmm256
        | VEX_Vfmaddsub213pd_ymm_ymm_ymmm256
        | VEX_Vfmaddsub231ps_ymm_ymm_ymmm256
        | VEX_Vfmaddsub231pd_ymm_ymm_ymmm256
        | VEX_Vfmsubadd132ps_ymm_ymm_ymmm256
        | VEX_Vfmsubadd132pd_ymm_ymm_ymmm256
        | VEX_Vfmsubadd213ps_ymm_ymm_ymmm256
        | VEX_Vfmsubadd213pd_ymm_ymm_ymmm256
        | VEX_Vfmsubadd231ps_ymm_ymm_ymmm256
        | VEX_Vfmsubadd231pd_ymm_ymm_ymmm256 => 32,
        _ => return false,
    };
    instruction.code_size() == iced_x86::CodeSize::Code64
        && instruction.op_count() == 3
        && (0..3).all(|operand| {
            let register = instruction.op_register(operand);
            instruction.op_kind(operand) == OpKind::Register
                && if width == 16 {
                    register.is_xmm() && register <= Register::XMM15
                } else {
                    register.is_ymm() && register <= Register::YMM15
                }
        })
}

/// Closed exact decoded forms, all register-only. The packed ADD/SUB/MUL
/// and reciprocal-estimate forms require the runtime's checked masked MXCSR
/// profile, just like already admitted legacy SSE arithmetic. W/L,
/// mandatory prefixes and reserved fields remain the pinned decoder's contract.
/// Memory encodings sharing a Code are explicitly excluded below.
fn native_ymm_registers(instruction: &Instruction) -> bool {
    use Code::*;
    // This single implicit-length index form reads two XMM registers and an
    // immediate, then writes ECX and arithmetic flags. Its cached dependency
    // template must retain the implicit full-RCX write just like any GPR alias.
    if instruction.code() == VEX_Vpcmpistri_xmm_xmmm128_imm8 {
        return instruction.code_size() == iced_x86::CodeSize::Code64
            && instruction.op_count() == 3
            && instruction.op2_kind() == OpKind::Immediate8
            && (0..2).all(|operand| {
                let register = instruction.op_register(operand);
                instruction.op_kind(operand) == OpKind::Register
                    && register.is_xmm()
                    && register <= Register::XMM15
            });
    }
    let (width, operands) = match instruction.code() {
        VEX_Vmovdqa_xmm_xmmm128
        | VEX_Vmovdqa_xmmm128_xmm
        | VEX_Vmovdqu_xmm_xmmm128
        | VEX_Vmovdqu_xmmm128_xmm
        | VEX_Vmovaps_xmm_xmmm128
        | VEX_Vmovaps_xmmm128_xmm
        | VEX_Vmovups_xmm_xmmm128
        | VEX_Vmovups_xmmm128_xmm
        | VEX_Vmovapd_xmm_xmmm128
        | VEX_Vmovapd_xmmm128_xmm
        | VEX_Vmovupd_xmm_xmmm128
        | VEX_Vmovupd_xmmm128_xmm
        | VEX_Vrcpps_xmm_xmmm128
        | VEX_Vrsqrtps_xmm_xmmm128 => (16, 2),
        VEX_Vmovdqa_ymm_ymmm256
        | VEX_Vmovdqa_ymmm256_ymm
        | VEX_Vmovdqu_ymm_ymmm256
        | VEX_Vmovdqu_ymmm256_ymm
        | VEX_Vmovaps_ymm_ymmm256
        | VEX_Vmovaps_ymmm256_ymm
        | VEX_Vmovups_ymm_ymmm256
        | VEX_Vmovups_ymmm256_ymm
        | VEX_Vmovapd_ymm_ymmm256
        | VEX_Vmovapd_ymmm256_ymm
        | VEX_Vmovupd_ymm_ymmm256
        | VEX_Vmovupd_ymmm256_ymm
        | VEX_Vrcpps_ymm_ymmm256
        | VEX_Vrsqrtps_ymm_ymmm256 => (32, 2),
        VEX_Vpand_xmm_xmm_xmmm128
        | VEX_Vpandn_xmm_xmm_xmmm128
        | VEX_Vpor_xmm_xmm_xmmm128
        | VEX_Vpxor_xmm_xmm_xmmm128
        | VEX_Vpcmpgtq_xmm_xmm_xmmm128
        | VEX_Vpacksswb_xmm_xmm_xmmm128
        | VEX_Vpackssdw_xmm_xmm_xmmm128
        | VEX_Vpackuswb_xmm_xmm_xmmm128
        | VEX_Vpackusdw_xmm_xmm_xmmm128
        | VEX_Vaddps_xmm_xmm_xmmm128
        | VEX_Vaddpd_xmm_xmm_xmmm128
        | VEX_Vsubps_xmm_xmm_xmmm128
        | VEX_Vsubpd_xmm_xmm_xmmm128
        | VEX_Vmulps_xmm_xmm_xmmm128
        | VEX_Vmulpd_xmm_xmm_xmmm128
        | VEX_Vandps_xmm_xmm_xmmm128
        | VEX_Vandpd_xmm_xmm_xmmm128
        | VEX_Vandnps_xmm_xmm_xmmm128
        | VEX_Vandnpd_xmm_xmm_xmmm128
        | VEX_Vorps_xmm_xmm_xmmm128
        | VEX_Vorpd_xmm_xmm_xmmm128
        | VEX_Vxorps_xmm_xmm_xmmm128
        | VEX_Vxorpd_xmm_xmm_xmmm128
        | VEX_Vrcpss_xmm_xmm_xmmm32
        | VEX_Vrsqrtss_xmm_xmm_xmmm32 => (16, 3),
        VEX_Vpand_ymm_ymm_ymmm256
        | VEX_Vpandn_ymm_ymm_ymmm256
        | VEX_Vpor_ymm_ymm_ymmm256
        | VEX_Vpxor_ymm_ymm_ymmm256
        | VEX_Vpcmpgtq_ymm_ymm_ymmm256
        | VEX_Vpacksswb_ymm_ymm_ymmm256
        | VEX_Vpackssdw_ymm_ymm_ymmm256
        | VEX_Vpackuswb_ymm_ymm_ymmm256
        | VEX_Vpackusdw_ymm_ymm_ymmm256
        | VEX_Vaddps_ymm_ymm_ymmm256
        | VEX_Vaddpd_ymm_ymm_ymmm256
        | VEX_Vsubps_ymm_ymm_ymmm256
        | VEX_Vsubpd_ymm_ymm_ymmm256
        | VEX_Vmulps_ymm_ymm_ymmm256
        | VEX_Vmulpd_ymm_ymm_ymmm256
        | VEX_Vandps_ymm_ymm_ymmm256
        | VEX_Vandpd_ymm_ymm_ymmm256
        | VEX_Vandnps_ymm_ymm_ymmm256
        | VEX_Vandnpd_ymm_ymm_ymmm256
        | VEX_Vorps_ymm_ymm_ymmm256
        | VEX_Vorpd_ymm_ymm_ymmm256
        | VEX_Vxorps_ymm_ymm_ymmm256
        | VEX_Vxorpd_ymm_ymm_ymmm256 => (32, 3),
        _ => return false,
    };
    instruction.op_count() == operands
        && (0..operands).all(|operand| {
            if instruction.op_kind(operand) != OpKind::Register {
                return false;
            }
            let register = instruction.op_register(operand);
            register.size() == width
                && if width == 16 {
                    register.is_xmm() && register <= Register::XMM15
                } else {
                    register.is_ymm() && register <= Register::YMM15
                }
        })
}

// Positive semantic families, covering all decoder-defined encodings of these
// mnemonics subject to the independent mode/operand/feature/prefix gates above.
// Implicit state is limited to GPRs/RFLAGS, SS:RSP for PUSH/POP/LEAVE, and the
// SSE arithmetic MXCSR status/exception semantics. No string/implicit-index
// memory operand kind is admitted; no environment/state-control mnemonic appears.
fn bmi(m: Mnemonic) -> bool {
    use Mnemonic::*;
    matches!(
        m,
        Andn | Bextr | Blsi | Blsmsk | Blsr | Bzhi | Mulx | Pdep | Pext | Rorx | Sarx | Shlx | Shrx
    )
}

fn integer(m: Mnemonic) -> bool {
    use Mnemonic::*;
    matches!(
        m,
        Adc | Add
            | And
            | Bsf
            | Bsr
            | Bswap
            | Bt
            | Btc
            | Btr
            | Bts
            | Cbw
            | Cdq
            | Cdqe
            | Cmovb
            | Cmovbe
            | Cmove
            | Cmovg
            | Cmovge
            | Cmovl
            | Cmovle
            | Cmovne
            | Cmovno
            | Cmovnp
            | Cmovns
            | Cmovo
            | Cmovp
            | Cmovs
            | Cmova
            | Cmovae
            | Cmp
            | Cmpxchg
            | Cmpxchg8b
            | Cmpxchg16b
            | Cqo
            | Cwd
            | Cwde
            | Dec
            | Div
            | Idiv
            | Imul
            | Inc
            | Lea
            | Leave
            | Mov
            | Movsx
            | Movsxd
            | Movzx
            | Mul
            | Neg
            | Nop
            | Not
            | Or
            | Pop
            | Push
            | Rcl
            | Rcr
            | Rol
            | Ror
            | Sal
            | Sar
            | Sbb
            | Seta
            | Setae
            | Setb
            | Setbe
            | Sete
            | Setg
            | Setge
            | Setl
            | Setle
            | Setne
            | Setno
            | Setnp
            | Setns
            | Seto
            | Setp
            | Sets
            | Shl
            | Shld
            | Shr
            | Shrd
            | Sub
            | Test
            | Xadd
            | Xchg
            | Xor
            | Clc
            | Stc
            | Cmc
            | Cld
            | Std
            | Popcnt
            | Lzcnt
            | Tzcnt
            | Andn
            | Bextr
            | Blsi
            | Blsmsk
            | Blsr
            | Bzhi
            | Mulx
            | Pdep
            | Pext
            | Rorx
            | Sarx
            | Shlx
            | Shrx
            | Adcx
            | Adox
    )
}
fn sse(m: Mnemonic) -> bool {
    use Mnemonic::*;
    matches!(
        m,
        Addpd
            | Addps
            | Addsd
            | Addss
            | Andnpd
            | Andnps
            | Andpd
            | Andps
            | Cmppd
            | Cmpps
            | Cmpsd
            | Cmpss
            | Comisd
            | Comiss
            | Cvtdq2pd
            | Cvtdq2ps
            | Cvtpd2dq
            | Cvtpd2ps
            | Cvtps2dq
            | Cvtps2pd
            | Cvtsd2si
            | Cvtsd2ss
            | Cvtsi2sd
            | Cvtsi2ss
            | Cvtss2sd
            | Cvtss2si
            | Cvttpd2dq
            | Cvttps2dq
            | Cvttsd2si
            | Cvttss2si
            | Divpd
            | Divps
            | Divsd
            | Divss
            | Lfence
            | Maxpd
            | Maxps
            | Maxsd
            | Maxss
            | Mfence
            | Minpd
            | Minps
            | Minsd
            | Minss
            | Movapd
            | Movaps
            | Movd
            | Movdqa
            | Movdqu
            | Movhpd
            | Movhps
            | Movlhps
            | Movhlps
            | Movlpd
            | Movlps
            | Movmskpd
            | Movmskps
            | Movntdq
            | Movntpd
            | Movntps
            | Movq
            | Movsd
            | Movss
            | Movupd
            | Movups
            | Mulpd
            | Mulps
            | Mulsd
            | Mulss
            | Orpd
            | Orps
            | Packssdw
            | Packsswb
            | Packuswb
            | Paddb
            | Paddd
            | Paddq
            | Paddsb
            | Paddsw
            | Paddusb
            | Paddusw
            | Paddw
            | Pand
            | Pandn
            | Pavgb
            | Pavgw
            | Pcmpeqb
            | Pcmpeqd
            | Pcmpeqw
            | Pcmpgtb
            | Pcmpgtd
            | Pcmpgtw
            | Pextrw
            | Pinsrw
            | Pmaddwd
            | Pmaxsw
            | Pmaxub
            | Pminsw
            | Pminub
            | Pmovmskb
            | Pmulhuw
            | Pmulhw
            | Pmullw
            | Pmuludq
            | Por
            | Psadbw
            | Pshufd
            | Pshufhw
            | Pshuflw
            | Pslld
            | Pslldq
            | Psllq
            | Psllw
            | Psrad
            | Psraw
            | Psrld
            | Psrldq
            | Psrlq
            | Psrlw
            | Psubb
            | Psubd
            | Psubq
            | Psubsb
            | Psubsw
            | Psubusb
            | Psubusw
            | Psubw
            | Punpckhbw
            | Punpckhdq
            | Punpckhqdq
            | Punpckhwd
            | Punpcklbw
            | Punpckldq
            | Punpcklqdq
            | Punpcklwd
            | Pxor
            | Rcpps
            | Rcpss
            | Rsqrtps
            | Rsqrtss
            | Shufpd
            | Shufps
            | Sfence
            | Sqrtpd
            | Sqrtps
            | Sqrtsd
            | Sqrtss
            | Subpd
            | Subps
            | Subsd
            | Subss
            | Ucomisd
            | Ucomiss
            | Unpckhpd
            | Unpckhps
            | Unpcklpd
            | Unpcklps
            | Xorpd
            | Xorps
    )
}
