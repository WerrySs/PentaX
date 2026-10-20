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
    let vex_vector = opcode.encoding() == EncodingKind::VEX
        && policy.requests_native_ymm_registers()
        && (native_ymm_registers(instruction) || vex_memory);
    if opcode.encoding() != EncodingKind::Legacy && !vex_gpr && !vex_vector {
        return Err(UnsupportedReason::InstructionClass);
    }
    if !vex_vector && !integer(instruction.mnemonic()) && !sse(instruction.mnemonic()) {
        return Err(UnsupportedReason::InstructionClass);
    }
    if vex_vector && !policy.features.contains(CpuidFeature::AVX) {
        return Err(UnsupportedReason::MissingFeature(CpuidFeature::AVX));
    }
    for feature in instruction.cpuid_features() {
        if !policy.features.contains(*feature) {
            return Err(UnsupportedReason::MissingFeature(*feature));
        }
    }
    if vex_vector {
        // Exact gates prove either same-width registers only, or two such
        // registers plus the separately requested ordinary64-bit read. No other
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

/// Thirty-eight exact decoded forms, all register-only. The twelve packed
/// ADD/SUB/MUL floating-point forms require the runtime's checked masked MXCSR
/// profile, just like already admitted legacy SSE arithmetic. W/L,
/// mandatory prefixes and reserved fields remain the pinned decoder's contract.
/// Memory encodings sharing a Code are explicitly excluded below.
fn native_ymm_registers(instruction: &Instruction) -> bool {
    use Code::*;
    let (width, operands) = match instruction.code() {
        VEX_Vmovdqa_xmm_xmmm128
        | VEX_Vmovdqa_xmmm128_xmm
        | VEX_Vmovdqu_xmm_xmmm128
        | VEX_Vmovdqu_xmmm128_xmm => (16, 2),
        VEX_Vmovdqa_ymm_ymmm256
        | VEX_Vmovdqa_ymmm256_ymm
        | VEX_Vmovdqu_ymm_ymmm256
        | VEX_Vmovdqu_ymmm256_ymm => (32, 2),
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
        | VEX_Vmulpd_xmm_xmm_xmmm128 => (16, 3),
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
        | VEX_Vmulpd_ymm_ymm_ymmm256 => (32, 3),
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
