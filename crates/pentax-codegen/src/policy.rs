// SPDX-License-Identifier: GPL-2.0-only

use crate::{CpuidFeature, Mnemonic, Register, UnsupportedReason};
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
}
impl TranslationPolicy {
    pub const fn new(features: CpuFeatures) -> Self {
        Self {
            features,
            isolate_memory: false,
            independent_memory: false,
            split_partial_windows: false,
        }
    }
    pub const fn features(self) -> CpuFeatures {
        self.features
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
    if opcode.encoding() != EncodingKind::Legacy && !vex_gpr {
        return Err(UnsupportedReason::InstructionClass);
    }
    if !integer(instruction.mnemonic()) && !sse(instruction.mnemonic()) {
        return Err(UnsupportedReason::InstructionClass);
    }
    for feature in instruction.cpuid_features() {
        if !policy.features.contains(*feature) {
            return Err(UnsupportedReason::MissingFeature(*feature));
        }
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
