// SPDX-License-Identifier: GPL-2.0-only
//! Closed host numeric scratch transaction. No guest address or saved guest
//! environment is ever installed on the host. FXSAVE64/FXRSTOR64 cover defined
//! host state; reserved padding and undefined MMX upper bits have no contract.
//! Intel SDM Vol1 8.1.8 and Vol2A FXSAVE/FXRSTOR (REX.W):
//! https://cdrdv2-public.intel.com/868137/325462-089-sdm-vol-1-2abcd-3abcd-4.pdf
//! AMD XSaveErPtr CPUID contract: AMD OSRR 56255:
//! https://www.amd.com/content/dam/amd/en/documents/processor-tech-docs/programmer-references/56255_OSRR.pdf
use super::{Arithmetic, NativeDispatchError};
use std::sync::OnceLock;

#[repr(C, align(16))]
struct Image([u8; 512]);

// The test-only literal prefix/suffix shares this exact transaction, keeping
// seeded host state and its restoration inside a single assembly boundary.
macro_rules! transaction {
    ($instruction:literal, $original:expr, $post:expr, $left:expr, $right:expr, $control:expr,
     [$($prefix:literal),*], [$($suffix:literal),*], [$($extra:ident = $value:expr),*]) => {
        // SAFETY: callers establish the host gate, aligned owned images and
        // canonical local operands. All exceptions are masked; only the actual
        // original host image is restored. The entire x87 stack, defined
        // environment, MXCSR and XMM are restored inside this one block. No
        // instruction changes EFLAGS, no GPR is written, and memory writes are
        // declared by the ordinary asm memory effects. No Rust runs in between.
        unsafe {
            std::arch::asm!(
                $($prefix,)*
                "fxsave64 [{original}]", "fninit",
                "fld tbyte ptr [{right}]", "fld tbyte ptr [{left}]",
                "fldcw [{control}]", "fnclex", $instruction,
                "fxsave64 [{post}]", "fxrstor64 [{original}]",
                $($suffix,)*
                original = in(reg) $original, post = in(reg) $post,
                left = in(reg) $left, right = in(reg) $right,
                control = in(reg) $control,
                $($extra = in(reg) $value,)*
                options(preserves_flags),
            );
        }
    };
}

pub(super) fn available_with(mut cpuid: impl FnMut(u32) -> [u32; 4]) -> bool {
    let identity = cpuid(0);
    if identity[0] < 1 {
        return false;
    }
    let features = cpuid(1)[3];
    if features & ((1 << 0) | (1 << 24)) != ((1 << 0) | (1 << 24)) {
        return false;
    }
    let intel =
        identity[1] == 0x756e_6547 && identity[3] == 0x4965_6e69 && identity[2] == 0x6c65_746e;
    let amd =
        identity[1] == 0x6874_7541 && identity[3] == 0x6974_6e65 && identity[2] == 0x444d_4163;
    // Intel SDM Vol1 8.1.8 defines unconditional full pointer save/restore with
    // REX.W. AMD requires XSaveErPtr; FXSR alone does not preserve error pointers
    // when ES is clear. Guest CPUID declarations never select this capability.
    intel || (amd && cpuid(0x8000_0000)[0] >= 0x8000_0008 && cpuid(0x8000_0008)[1] & 4 != 0)
}

pub(super) fn available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        available_with(|leaf| {
            let value = std::arch::x86_64::__cpuid(leaf);
            [value.eax, value.ebx, value.ecx, value.edx]
        })
    })
}

pub(super) fn compute(
    operation: Arithmetic,
    left: &[u8; 10],
    right: &[u8; 10],
    control: u16,
) -> Result<([u8; 10], u16), NativeDispatchError> {
    if !available() {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    let mut original = Image([0; 512]);
    let mut post = Image([0; 512]);
    let control = super::super::x87_control::control_word(control) | 0x3f;
    match operation {
        Arithmetic::Add => transaction!(
            "fadd st(0), st(1)",
            original.0.as_mut_ptr(),
            post.0.as_mut_ptr(),
            left.as_ptr(),
            right.as_ptr(),
            &control as *const u16,
            [],
            [],
            []
        ),
        Arithmetic::Subtract => transaction!(
            "fsub st(0), st(1)",
            original.0.as_mut_ptr(),
            post.0.as_mut_ptr(),
            left.as_ptr(),
            right.as_ptr(),
            &control as *const u16,
            [],
            [],
            []
        ),
        Arithmetic::Multiply => transaction!(
            "fmul st(0), st(1)",
            original.0.as_mut_ptr(),
            post.0.as_mut_ptr(),
            left.as_ptr(),
            right.as_ptr(),
            &control as *const u16,
            [],
            [],
            []
        ),
        Arithmetic::Divide => transaction!(
            "fdiv st(0), st(1)",
            original.0.as_mut_ptr(),
            post.0.as_mut_ptr(),
            left.as_ptr(),
            right.as_ptr(),
            &control as *const u16,
            [],
            [],
            []
        ),
    }
    let status = u16::from_le_bytes([post.0[2], post.0[3]]);
    // FNINIT plus exactly two loads leaves TOP6 and physical R6/R7 occupied.
    // Observe logical ST0 directly, before any store can change C1 or round.
    if status & 0x3840 != 0x3000 || post.0[4] != 0xc0 {
        return Err(NativeDispatchError::UnsupportedVectorInstruction);
    }
    let mut result = [0; 10];
    result.copy_from_slice(&post.0[32..42]);
    Ok((result, status))
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
