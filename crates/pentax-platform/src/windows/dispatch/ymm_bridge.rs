// SPDX-License-Identifier: GPL-2.0-only
//! Optional full-YMM bridge. Hardware facts select private immutable state;
//! guest feature declarations never authorize an entry point or an XGETBV.

use super::{
    CONTEXT, CpuidFeature, GuestContext, host_supports, pentax_block_enter, pentax_block_trailer,
};
use pentax_codegen::CpuFeatures;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VectorMode {
    Sse2,
    Avx { avx2: bool, fma: bool },
}

impl VectorMode {
    pub(super) fn detect() -> Self {
        static MODE: OnceLock<VectorMode> = OnceLock::new();
        *MODE.get_or_init(|| {
            Self::detect_with(
                |leaf, subleaf| {
                    let result = std::arch::x86_64::__cpuid_count(leaf, subleaf);
                    [result.eax, result.ebx, result.ecx, result.edx]
                },
                || {
                    // SAFETY: detect_with calls this fixed XCR0 read only after
                    // CPUID reports XSAVE, OSXSAVE and AVX. OSXSAVE admits XGETBV
                    // in user mode. No control register or host policy is changed.
                    unsafe { std::arch::x86_64::_xgetbv(0) }
                },
            )
        })
    }

    fn detect_with(
        mut cpuid: impl FnMut(u32, u32) -> [u32; 4],
        xcr0: impl FnOnce() -> u64,
    ) -> Self {
        let identity = cpuid(0, 0);
        let maximum = identity[0];
        const REQUIRED: u32 = (1 << 26) | (1 << 27) | (1 << 28);
        if maximum < 1 {
            return Self::Sse2;
        }
        let capabilities = cpuid(1, 0)[2];
        if capabilities & REQUIRED != REQUIRED {
            return Self::Sse2;
        }
        if xcr0() & 6 != 6 {
            return Self::Sse2;
        }
        let avx2 = maximum >= 7 && cpuid(7, 0)[1] & (1 << 5) != 0;
        // The scalar FMA profile has Intel/AMD primary contracts plus original
        // hardware differentials. Unknown vendors keep software mediation
        // until their complete contract has independent equivalence evidence.
        // These fixed host words never become a guest CPUID response.
        let intel =
            identity[1] == 0x756e_6547 && identity[3] == 0x4965_6e69 && identity[2] == 0x6c65_746e;
        let amd =
            identity[1] == 0x6874_7541 && identity[3] == 0x6974_6e65 && identity[2] == 0x444d_4163;
        let fma = (intel || amd) && capabilities & (1 << 12) != 0;
        Self::Avx { avx2, fma }
    }

    pub(super) fn features(self) -> Option<CpuFeatures> {
        match self {
            Self::Sse2 => None,
            Self::Avx { avx2, fma } => {
                let mut features = CpuFeatures::empty().with(CpuidFeature::AVX);
                if avx2 {
                    features = features.with(CpuidFeature::AVX2);
                }
                if fma {
                    features = features.with(CpuidFeature::FMA);
                }
                Some(features)
            }
        }
    }

    pub(super) fn supports(self, feature: CpuidFeature) -> bool {
        match feature {
            CpuidFeature::AVX => matches!(self, Self::Avx { .. }),
            CpuidFeature::AVX2 => matches!(self, Self::Avx { avx2: true, .. }),
            CpuidFeature::FMA => matches!(self, Self::Avx { fma: true, .. }),
            _ => host_supports(feature),
        }
    }

    /// Same closed capability predicate as supports(), memoized independently
    /// for each immutable entry/trailer mode. No guest CPUID words participate.
    pub(super) fn supports_admission(
        self,
        summary: &pentax_codegen::NativeAdmissionSummary,
    ) -> bool {
        static SSE2: OnceLock<[u64; 4]> = OnceLock::new();
        static AVX: OnceLock<[u64; 4]> = OnceLock::new();
        static AVX_FMA: OnceLock<[u64; 4]> = OnceLock::new();
        static AVX2: OnceLock<[u64; 4]> = OnceLock::new();
        static AVX2_FMA: OnceLock<[u64; 4]> = OnceLock::new();
        let cache = match self {
            Self::Sse2 => &SSE2,
            Self::Avx {
                avx2: false,
                fma: false,
            } => &AVX,
            Self::Avx {
                avx2: false,
                fma: true,
            } => &AVX_FMA,
            Self::Avx {
                avx2: true,
                fma: false,
            } => &AVX2,
            Self::Avx {
                avx2: true,
                fma: true,
            } => &AVX2_FMA,
        };
        let available = cache.get_or_init(|| {
            let mut words = [0; 4];
            for feature in CpuidFeature::values() {
                let bit = feature as usize;
                if self.supports(feature)
                    && let Some(word) = words.get_mut(bit / 64)
                {
                    *word |= 1_u64 << (bit % 64);
                }
            }
            words
        });
        summary.features_supported_by(available)
    }

    pub(super) fn trailer(self) -> u64 {
        match self {
            Self::Sse2 => pentax_block_trailer as *const () as u64,
            Self::Avx { .. } => pentax_block_trailer_ymm as *const () as u64,
        }
    }

    pub(super) fn entry(self) -> unsafe extern "C" fn() {
        match self {
            Self::Sse2 => pentax_block_enter,
            Self::Avx { .. } => pentax_block_enter_ymm,
        }
    }

    pub(super) fn load(self, context: &mut GuestContext, low: &[u8; 256], upper: &[u8; 256]) {
        context.xmm = *low;
        if matches!(self, Self::Avx { .. }) {
            for register in 0..16 {
                context.ymm[register * 32..register * 32 + 16]
                    .copy_from_slice(&low[register * 16..register * 16 + 16]);
                context.ymm[register * 32 + 16..register * 32 + 32]
                    .copy_from_slice(&upper[register * 16..register * 16 + 16]);
            }
        }
    }

    pub(super) fn save(self, context: &GuestContext, low: &mut [u8; 256], upper: &mut [u8; 256]) {
        match self {
            Self::Sse2 => *low = context.xmm,
            Self::Avx { .. } => {
                for register in 0..16 {
                    low[register * 16..register * 16 + 16]
                        .copy_from_slice(&context.ymm[register * 32..register * 32 + 16]);
                    upper[register * 16..register * 16 + 16]
                        .copy_from_slice(&context.ymm[register * 32 + 16..register * 32 + 32]);
                }
            }
        }
    }

    /// # Safety
    /// The caller owns the actual paused guest, admitted RX cache and CONTEXT
    /// guard, with the tail published for exactly this immutable mode.
    pub(super) unsafe fn enter(self) {
        // SAFETY: Same source/owner/context obligation as the legacy bridge.
        // Avx is constructed only after checked CPU+OS XMM/YMM admission.
        unsafe {
            (self.entry())();
        }
    }
}

std::arch::global_asm!(r#"
    .text
    .globl pentax_block_enter_ymm
    .p2align 4
pentax_block_enter_ymm:
    push rbx
    push rbp
    push rsi
    push rdi
    push r12
    push r13
    push r14
    push r15
    sub rsp, 192
    stmxcsr [rsp]
    fnstcw [rsp + 4]
    movdqu [rsp + 16], xmm6
    movdqu [rsp + 32], xmm7
    movdqu [rsp + 48], xmm8
    movdqu [rsp + 64], xmm9
    movdqu [rsp + 80], xmm10
    movdqu [rsp + 96], xmm11
    movdqu [rsp + 112], xmm12
    movdqu [rsp + 128], xmm13
    movdqu [rsp + 144], xmm14
    movdqu [rsp + 160], xmm15
    push 0
    pop rax
    mov rax, gs:[8]
    mov [rsp + 176], rax
    mov rax, gs:[0x10]
    mov [rsp + 184], rax
    mov [rip + {ctx} + 160], rsp
    mov rax, [rip + {ctx} + 144]
    mov gs:[8], rax
    mov rax, [rip + {ctx} + 152]
    mov gs:[0x10], rax
    ldmxcsr [rip + {ctx} + 168]
    vmovdqu ymm0, [rip + {ctx} + 432]
    vmovdqu ymm1, [rip + {ctx} + 464]
    vmovdqu ymm2, [rip + {ctx} + 496]
    vmovdqu ymm3, [rip + {ctx} + 528]
    vmovdqu ymm4, [rip + {ctx} + 560]
    vmovdqu ymm5, [rip + {ctx} + 592]
    vmovdqu ymm6, [rip + {ctx} + 624]
    vmovdqu ymm7, [rip + {ctx} + 656]
    vmovdqu ymm8, [rip + {ctx} + 688]
    vmovdqu ymm9, [rip + {ctx} + 720]
    vmovdqu ymm10, [rip + {ctx} + 752]
    vmovdqu ymm11, [rip + {ctx} + 784]
    vmovdqu ymm12, [rip + {ctx} + 816]
    vmovdqu ymm13, [rip + {ctx} + 848]
    vmovdqu ymm14, [rip + {ctx} + 880]
    vmovdqu ymm15, [rip + {ctx} + 912]
    push qword ptr [rip + {ctx} + 136]
    popfq
    mov rax, [rip + {ctx}]
    mov rcx, [rip + {ctx} + 8]
    mov rdx, [rip + {ctx} + 16]
    mov rbx, [rip + {ctx} + 24]
    mov rbp, [rip + {ctx} + 40]
    mov rsi, [rip + {ctx} + 48]
    mov rdi, [rip + {ctx} + 56]
    mov r8, [rip + {ctx} + 64]
    mov r9, [rip + {ctx} + 72]
    mov r10, [rip + {ctx} + 80]
    mov r11, [rip + {ctx} + 88]
    mov r12, [rip + {ctx} + 96]
    mov r13, [rip + {ctx} + 104]
    mov r14, [rip + {ctx} + 112]
    mov r15, [rip + {ctx} + 120]
    mov rsp, [rip + {ctx} + 32]
    jmp qword ptr [rip + {ctx} + 128]

    .globl pentax_block_trailer_ymm
    .p2align 4
pentax_block_trailer_ymm:
    mov [rip + {ctx}], rax
    mov [rip + {ctx} + 8], rcx
    mov [rip + {ctx} + 16], rdx
    mov [rip + {ctx} + 24], rbx
    mov [rip + {ctx} + 40], rbp
    mov [rip + {ctx} + 48], rsi
    mov [rip + {ctx} + 56], rdi
    mov [rip + {ctx} + 64], r8
    mov [rip + {ctx} + 72], r9
    mov [rip + {ctx} + 80], r10
    mov [rip + {ctx} + 88], r11
    mov [rip + {ctx} + 96], r12
    mov [rip + {ctx} + 104], r13
    mov [rip + {ctx} + 112], r14
    mov [rip + {ctx} + 120], r15
    mov [rip + {ctx} + 32], rsp
    mov rsp, [rip + {ctx} + 160]
    pushfq
    pop rax
    mov [rip + {ctx} + 136], rax
    cld
    stmxcsr [rip + {ctx} + 168]
    # Capture every guest upper half BEFORE any call or VZEROUPPER.
    vmovdqu [rip + {ctx} + 432], ymm0
    vmovdqu [rip + {ctx} + 464], ymm1
    vmovdqu [rip + {ctx} + 496], ymm2
    vmovdqu [rip + {ctx} + 528], ymm3
    vmovdqu [rip + {ctx} + 560], ymm4
    vmovdqu [rip + {ctx} + 592], ymm5
    vmovdqu [rip + {ctx} + 624], ymm6
    vmovdqu [rip + {ctx} + 656], ymm7
    vmovdqu [rip + {ctx} + 688], ymm8
    vmovdqu [rip + {ctx} + 720], ymm9
    vmovdqu [rip + {ctx} + 752], ymm10
    vmovdqu [rip + {ctx} + 784], ymm11
    vmovdqu [rip + {ctx} + 816], ymm12
    vmovdqu [rip + {ctx} + 848], ymm13
    vmovdqu [rip + {ctx} + 880], ymm14
    vmovdqu [rip + {ctx} + 912], ymm15
    # Windows upper YMM halves are volatile; clear them only after capture.
    vzeroupper
    ldmxcsr [rsp]
    fldcw [rsp + 4]
    movdqu xmm6, [rsp + 16]
    movdqu xmm7, [rsp + 32]
    movdqu xmm8, [rsp + 48]
    movdqu xmm9, [rsp + 64]
    movdqu xmm10, [rsp + 80]
    movdqu xmm11, [rsp + 96]
    movdqu xmm12, [rsp + 112]
    movdqu xmm13, [rsp + 128]
    movdqu xmm14, [rsp + 144]
    movdqu xmm15, [rsp + 160]
    mov rax, [rsp + 176]
    mov gs:[8], rax
    mov rax, [rsp + 184]
    mov gs:[0x10], rax
    add rsp, 192
    pop r15
    pop r14
    pop r13
    pop r12
    pop rdi
    pop rsi
    pop rbp
    pop rbx
    ret
"#, ctx = sym CONTEXT);

unsafe extern "C" {
    fn pentax_block_enter_ymm();
    fn pentax_block_trailer_ymm();
}

#[cfg(test)]
#[path = "ymm_bridge/tests.rs"]
mod tests;
