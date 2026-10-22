// SPDX-License-Identifier: GPL-2.0-only

//! Closed SSE2 operations on copied scalar bits, never guest pointers or code.
//! Each assembly block restores host MXCSR before returning to compiled Rust.

use super::NativeDispatchError;
use pentax_codegen::{Code, Mnemonic};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Operation {
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
    Sqrt,
    Reciprocal,
    ReciprocalSqrt,
    Convert,
}

pub(super) struct Form {
    pub vector_bytes: usize,
    pub element_bytes: usize,
    pub scalar: bool,
    pub operation: Operation,
}

// Architectural scalar width and whether quiet NaNs also signal invalid.
pub(super) fn comparison_form(code: Code) -> Option<(usize, bool)> {
    Some(match code {
        Code::VEX_Vucomiss_xmm_xmmm32 => (4, false),
        Code::VEX_Vucomisd_xmm_xmmm64 => (8, false),
        Code::VEX_Vcomiss_xmm_xmmm32 => (4, true),
        Code::VEX_Vcomisd_xmm_xmmm64 => (8, true),
        _ => return None,
    })
}

pub(super) fn compare(
    ordered: bool,
    double: bool,
    left: u64,
    right: u64,
    mxcsr: u32,
) -> Result<(u64, u32), NativeDispatchError> {
    if !super::mxcsr_control::valid(mxcsr) {
        return Err(NativeDispatchError::InvalidState);
    }
    let mut host_control = 0_u32;
    let mut guest_control = mxcsr;
    let carry: u8;
    let parity: u8;
    let zero: u8;
    macro_rules! comparison {
        ($instruction:literal) => {
            // SAFETY: SSE2 is mandatory on this backend. Only copied numeric
            // bits and live local u32 control buffers enter this closed block.
            // All exceptions remain masked; every control bit is host-supported.
            // There are no calls or Rust operations before restoring host MXCSR.
            // STMXCSR/LDMXCSR preserve the comparison flags. All late outputs
            // are written after every input is consumed; XMM0/XMM1 and flags
            // are declared clobbered. No guest pointer or instruction is used.
            unsafe {
                std::arch::asm!(
                    "stmxcsr [{host}]", "ldmxcsr [{guest}]",
                    "movq xmm0, {left}", "movq xmm1, {right}",
                    concat!($instruction, " xmm0, xmm1"),
                    "stmxcsr [{guest}]", "ldmxcsr [{host}]",
                    "setc {carry}", "setp {parity}", "setz {zero}",
                    host = in(reg) &mut host_control,
                    guest = in(reg) &mut guest_control,
                    left = in(reg) left, right = in(reg) right,
                    carry = lateout(reg_byte) carry,
                    parity = lateout(reg_byte) parity,
                    zero = lateout(reg_byte) zero,
                    out("xmm0") _, out("xmm1") _,
                    options(nostack),
                );
            }
        };
    }
    match (ordered, double) {
        (false, false) => comparison!("ucomiss"),
        (false, true) => comparison!("ucomisd"),
        (true, false) => comparison!("comiss"),
        (true, true) => comparison!("comisd"),
    }
    Ok((
        u64::from(carry) | (u64::from(parity) << 2) | (u64::from(zero) << 6),
        guest_control,
    ))
}

pub(super) fn form(code: Code) -> Option<Form> {
    use Code::*;
    let (vector_bytes, element_bytes, scalar) = match code {
        VEX_Vaddss_xmm_xmm_xmmm32
        | VEX_Vsubss_xmm_xmm_xmmm32
        | VEX_Vmulss_xmm_xmm_xmmm32
        | VEX_Vdivss_xmm_xmm_xmmm32
        | VEX_Vminss_xmm_xmm_xmmm32
        | VEX_Vmaxss_xmm_xmm_xmmm32
        | VEX_Vsqrtss_xmm_xmm_xmmm32
        | VEX_Vrcpss_xmm_xmm_xmmm32
        | VEX_Vrsqrtss_xmm_xmm_xmmm32 => (16, 4, true),
        VEX_Vaddsd_xmm_xmm_xmmm64
        | VEX_Vsubsd_xmm_xmm_xmmm64
        | VEX_Vmulsd_xmm_xmm_xmmm64
        | VEX_Vdivsd_xmm_xmm_xmmm64
        | VEX_Vminsd_xmm_xmm_xmmm64
        | VEX_Vmaxsd_xmm_xmm_xmmm64
        | VEX_Vsqrtsd_xmm_xmm_xmmm64 => (16, 8, true),
        VEX_Vaddps_xmm_xmm_xmmm128
        | VEX_Vsubps_xmm_xmm_xmmm128
        | VEX_Vmulps_xmm_xmm_xmmm128
        | VEX_Vdivps_xmm_xmm_xmmm128
        | VEX_Vminps_xmm_xmm_xmmm128
        | VEX_Vmaxps_xmm_xmm_xmmm128
        | VEX_Vsqrtps_xmm_xmmm128
        | VEX_Vrcpps_xmm_xmmm128
        | VEX_Vrsqrtps_xmm_xmmm128 => (16, 4, false),
        VEX_Vaddps_ymm_ymm_ymmm256
        | VEX_Vsubps_ymm_ymm_ymmm256
        | VEX_Vmulps_ymm_ymm_ymmm256
        | VEX_Vdivps_ymm_ymm_ymmm256
        | VEX_Vminps_ymm_ymm_ymmm256
        | VEX_Vmaxps_ymm_ymm_ymmm256
        | VEX_Vsqrtps_ymm_ymmm256
        | VEX_Vrcpps_ymm_ymmm256
        | VEX_Vrsqrtps_ymm_ymmm256 => (32, 4, false),
        VEX_Vaddpd_xmm_xmm_xmmm128
        | VEX_Vsubpd_xmm_xmm_xmmm128
        | VEX_Vmulpd_xmm_xmm_xmmm128
        | VEX_Vdivpd_xmm_xmm_xmmm128
        | VEX_Vminpd_xmm_xmm_xmmm128
        | VEX_Vmaxpd_xmm_xmm_xmmm128
        | VEX_Vsqrtpd_xmm_xmmm128 => (16, 8, false),
        VEX_Vaddpd_ymm_ymm_ymmm256
        | VEX_Vsubpd_ymm_ymm_ymmm256
        | VEX_Vmulpd_ymm_ymm_ymmm256
        | VEX_Vdivpd_ymm_ymm_ymmm256
        | VEX_Vminpd_ymm_ymm_ymmm256
        | VEX_Vmaxpd_ymm_ymm_ymmm256
        | VEX_Vsqrtpd_ymm_ymmm256 => (32, 8, false),
        _ => return None,
    };
    let operation = match code.mnemonic() {
        Mnemonic::Vaddss | Mnemonic::Vaddsd | Mnemonic::Vaddps | Mnemonic::Vaddpd => Operation::Add,
        Mnemonic::Vsubss | Mnemonic::Vsubsd | Mnemonic::Vsubps | Mnemonic::Vsubpd => Operation::Sub,
        Mnemonic::Vmulss | Mnemonic::Vmulsd | Mnemonic::Vmulps | Mnemonic::Vmulpd => Operation::Mul,
        Mnemonic::Vdivss | Mnemonic::Vdivsd | Mnemonic::Vdivps | Mnemonic::Vdivpd => Operation::Div,
        Mnemonic::Vminss | Mnemonic::Vminsd | Mnemonic::Vminps | Mnemonic::Vminpd => Operation::Min,
        Mnemonic::Vmaxss | Mnemonic::Vmaxsd | Mnemonic::Vmaxps | Mnemonic::Vmaxpd => Operation::Max,
        Mnemonic::Vsqrtss | Mnemonic::Vsqrtsd | Mnemonic::Vsqrtps | Mnemonic::Vsqrtpd => {
            Operation::Sqrt
        }
        Mnemonic::Vrcpss | Mnemonic::Vrcpps => Operation::Reciprocal,
        Mnemonic::Vrsqrtss | Mnemonic::Vrsqrtps => Operation::ReciprocalSqrt,
        _ => return None,
    };
    Some(Form {
        vector_bytes,
        element_bytes,
        scalar,
        operation,
    })
}

pub(super) fn calculate(
    operation: Operation,
    double: bool,
    left: u64,
    right: u64,
    mxcsr: u32,
) -> Result<(u64, u32), NativeDispatchError> {
    // Keep the same host-compatible masked-exception profile as the guest bridge.
    // Invalid callers cannot install reserved bits or unmask a host exception.
    if !super::mxcsr_control::valid(mxcsr) {
        return Err(NativeDispatchError::InvalidState);
    }
    let mut host_control = 0_u32;
    let mut guest_control = mxcsr;
    let result: u64;
    macro_rules! arithmetic {
        ($instruction:literal) => {
            // SAFETY: SSE2 is mandatory on this x86-64 backend. Both memory
            // operands are live local u32 storage; numeric inputs are copied
            // bits, never pointers. All guest SIMD exceptions are masked and
            // reserved controls refused above. One compiler-visible assembly block
            // saves and restores host MXCSR, with no calls, branches or Rust
            // code while guest controls are installed. Only XMM0/XMM1 and the
            // declared result are clobbered; these instructions preserve RFLAGS.
            unsafe {
                std::arch::asm!(
                    "stmxcsr [{host}]", "ldmxcsr [{guest}]",
                    "movq xmm0, {left}", "movq xmm1, {right}",
                    concat!($instruction, " xmm0, xmm1"),
                    "stmxcsr [{guest}]", "ldmxcsr [{host}]",
                    "movq {result}, xmm0",
                    host = in(reg) &mut host_control,
                    guest = in(reg) &mut guest_control,
                    left = in(reg) left, right = in(reg) right,
                    result = lateout(reg) result,
                    out("xmm0") _, out("xmm1") _,
                    options(nostack, preserves_flags),
                );
            }
        };
    }
    match (operation, double) {
        (Operation::Add, false) => arithmetic!("addss"),
        (Operation::Add, true) => arithmetic!("addsd"),
        (Operation::Sub, false) => arithmetic!("subss"),
        (Operation::Sub, true) => arithmetic!("subsd"),
        (Operation::Mul, false) => arithmetic!("mulss"),
        (Operation::Mul, true) => arithmetic!("mulsd"),
        (Operation::Div, false) => arithmetic!("divss"),
        (Operation::Div, true) => arithmetic!("divsd"),
        (Operation::Min, false) => arithmetic!("minss"),
        (Operation::Min, true) => arithmetic!("minsd"),
        (Operation::Max, false) => arithmetic!("maxss"),
        (Operation::Max, true) => arithmetic!("maxsd"),
        (Operation::Sqrt, false) => arithmetic!("sqrtss"),
        (Operation::Sqrt, true) => arithmetic!("sqrtsd"),
        // RCP/RSQRT are estimates, not DIV/SQRT compositions. The actual SSE
        // estimate instruction supplies each copied single-precision lane,
        // including its denormal/NaN/signed-zero semantics and lack of SIMD
        // exceptions. Scalar merge and VEX upper clearing are handled outside
        // this closed numeric operation. No double-precision estimate exists.
        (Operation::Reciprocal, false) => arithmetic!("rcpss"),
        (Operation::ReciprocalSqrt, false) => arithmetic!("rsqrtss"),
        (Operation::Reciprocal | Operation::ReciprocalSqrt, true) => {
            return Err(NativeDispatchError::UnsupportedVectorInstruction);
        }
        // `double` is the result precision. The other XMM operand holds the
        // copied scalar source; the existing boundary restores host MXCSR.
        (Operation::Convert, false) => arithmetic!("cvtsd2ss"),
        (Operation::Convert, true) => arithmetic!("cvtss2sd"),
    }
    Ok((
        if double { result } else { result & 0xffff_ffff },
        guest_control,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host_mxcsr() -> u32 {
        let mut control = 0_u32;
        // SAFETY: SSE2 is mandatory and the output is live local u32 storage.
        unsafe {
            std::arch::asm!("stmxcsr [{p}]", p = in(reg) &mut control, options(nostack, preserves_flags));
        }
        control
    }

    #[test]
    fn precision_conversions_match_ieee_encodings_flags_and_restore_host_control() {
        let host = host_mxcsr();
        // Result-precision selector, copied source, four rounding results, flags.
        // Expected encodings are independent of the production SSE2 operation.
        for (double, raw, results, status) in [
            (false, 0, [0; 4], 0),
            (false, 0x8000_0000_0000_0000, [0x8000_0000; 4], 0),
            (false, 0x3ff8_0000_0000_0000, [0x3fc0_0000; 4], 0),
            (
                false,
                0x3ff0_0000_1000_0000,
                [0x3f80_0000, 0x3f80_0000, 0x3f80_0001, 0x3f80_0000],
                32,
            ),
            (
                false,
                0xbff0_0000_1000_0000,
                [0xbf80_0000, 0xbf80_0001, 0xbf80_0000, 0xbf80_0000],
                32,
            ),
            (
                false,
                0x7fef_ffff_ffff_ffff,
                [0x7f80_0000, 0x7f7f_ffff, 0x7f80_0000, 0x7f7f_ffff],
                40,
            ),
            (
                false,
                0xffef_ffff_ffff_ffff,
                [0xff80_0000, 0xff80_0000, 0xff7f_ffff, 0xff7f_ffff],
                40,
            ),
            (false, 0x3690_0000_0000_0000, [0, 0, 1, 0], 48),
            (false, 0x36a0_0000_0000_0000, [1; 4], 0),
            (false, 1, [0, 0, 1, 0], 50),
            (
                false,
                0x8000_0000_0000_0001,
                [0x8000_0000, 0x8000_0001, 0x8000_0000, 0x8000_0000],
                50,
            ),
            (false, 0x7ff0_0000_0000_0000, [0x7f80_0000; 4], 0),
            (false, 0x7ff8_0000_0000_0001, [0x7fc0_0000; 4], 0),
            (false, 0x7ff0_0000_2000_0000, [0x7fc0_0001; 4], 1),
            (true, 0x3fc0_0000, [0x3ff8_0000_0000_0000; 4], 0),
            (true, 0x8000_0000, [0x8000_0000_0000_0000; 4], 0),
            (true, 1, [0x36a0_0000_0000_0000; 4], 2),
            (true, 0x8000_0001, [0xb6a0_0000_0000_0000; 4], 2),
            (true, 0x7f80_0000, [0x7ff0_0000_0000_0000; 4], 0),
            (true, 0x7fc0_0001, [0x7ff8_0000_2000_0000; 4], 0),
            (true, 0x7f80_0001, [0x7ff8_0000_2000_0000; 4], 1),
        ] {
            for (rounding, result) in results.into_iter().enumerate() {
                for daz in [false, true] {
                    for ftz in [false, true] {
                        let control = 0x1f84
                            | ((rounding as u32) << 13)
                            | (u32::from(daz) << 6)
                            | (u32::from(ftz) << 15);
                        if !super::super::mxcsr_control::valid(control) {
                            continue;
                        }
                        let (input_sign, input_subnormal) = if double {
                            (
                                (raw & 0x8000_0000) << 32,
                                raw & 0x7f80_0000 == 0 && raw & 0x7f_ffff != 0,
                            )
                        } else {
                            (
                                (raw >> 32) & 0x8000_0000,
                                raw & 0x7ff0_0000_0000_0000 == 0
                                    && raw & 0x000f_ffff_ffff_ffff != 0,
                            )
                        };
                        let (mut expected, mut flags) = (result, status);
                        if daz && input_subnormal {
                            expected = input_sign;
                            flags = 0;
                        } else if ftz
                            && !double
                            && expected & 0x7f80_0000 == 0
                            && expected & 0x7f_ffff != 0
                        {
                            expected &= 0x8000_0000;
                            flags |= 48;
                        }
                        let (actual, next) =
                            calculate(Operation::Convert, double, 0, raw, control).unwrap();
                        assert_eq!(
                            (actual, next),
                            (expected, control | flags),
                            "raw={raw:x}, double={double}, control={control:x}"
                        );
                        assert_eq!(host_mxcsr(), host);
                    }
                }
            }
        }
        assert_eq!(
            calculate(Operation::Convert, false, 0, 0, 0).err(),
            Some(NativeDispatchError::InvalidState)
        );
        assert_eq!(host_mxcsr(), host);
    }

    #[test]
    fn scalar_float_comparisons_preserve_host_control_and_distinguish_nan_exceptions() {
        let host = host_mxcsr();
        for (double, one, two, sign, infinity, quiet, signaling) in [
            (
                false,
                0x3f80_0000,
                0x4000_0000,
                0x8000_0000,
                0x7f80_0000,
                0x7fc0_0000,
                0x7f80_0001,
            ),
            (
                true,
                0x3ff0_0000_0000_0000,
                0x4000_0000_0000_0000,
                0x8000_0000_0000_0000,
                0x7ff0_0000_0000_0000,
                0x7ff8_0000_0000_0000,
                0x7ff0_0000_0000_0001,
            ),
        ] {
            for ordered in [false, true] {
                for (left, right, flags, status) in [
                    (one, two, 1, 0),
                    (two, one, 0, 0),
                    (one, one, 0x40, 0),
                    (0, sign, 0x40, 0),
                    (sign | two, sign | one, 1, 0),
                    (infinity, two, 0, 0),
                    (infinity, infinity, 0x40, 0),
                    (quiet, one, 0x45, u32::from(ordered)),
                    (one, quiet, 0x45, u32::from(ordered)),
                    (signaling, one, 0x45, 1),
                    (one, signaling, 0x45, 1),
                    (1, 0, 0, 2),
                    (0, 1, 1, 2),
                ] {
                    // Preexisting ZE, nondefault rounding and FTZ must survive.
                    let control = 0x1f84 | 0xc000;
                    assert_eq!(
                        compare(ordered, double, left, right, control).unwrap(),
                        (flags, control | status)
                    );
                    assert_eq!(host_mxcsr(), host);
                }
            }
        }
        let mut invalid = vec![0, 0x1f80 | (1 << 31)];
        if super::super::mxcsr_control::mask() & 0x40 == 0 {
            invalid.push(0x1fc0);
        }
        for control in invalid {
            assert!(matches!(
                compare(false, false, 0, 0, control),
                Err(NativeDispatchError::InvalidState)
            ));
            assert_eq!(host_mxcsr(), host);
        }
    }

    #[test]
    fn arithmetic_preserves_host_control_and_observes_guest_rounding_and_exceptions() {
        use Operation::*;
        let host = host_mxcsr();
        for (op, double, left, right, extra, bits, flags) in [
            (Div, false, 0x3f80_0000, 0x4040_0000, 0, 0x3eaa_aaab, 0x20),
            (
                Div,
                false,
                0x3f80_0000,
                0x4040_0000,
                0x2000,
                0x3eaa_aaaa,
                0x20,
            ),
            (
                Div,
                false,
                0x3f80_0000,
                0x4040_0000,
                0x4000,
                0x3eaa_aaab,
                0x20,
            ),
            (
                Div,
                false,
                0xbf80_0000,
                0x4040_0000,
                0x6000,
                0xbeaa_aaaa,
                0x20,
            ),
            (
                Div,
                true,
                0x3ff0_0000_0000_0000,
                0x4008_0000_0000_0000,
                0x4000,
                0x3fd5_5555_5555_5556,
                0x20,
            ),
            (Div, false, 0x3f80_0000, 0, 0, 0x7f80_0000, 4),
            (
                Div,
                true,
                0x3ff0_0000_0000_0000,
                0,
                0,
                0x7ff0_0000_0000_0000,
                4,
            ),
            (Div, false, 0, 0, 0, 0xffc0_0000, 1),
            (Add, false, 0x7f7f_ffff, 0x7f7f_ffff, 0, 0x7f80_0000, 0x28),
            (Mul, false, 0x0080_0000, 0x3f00_0000, 0, 0x0040_0000, 0),
            (Mul, false, 0x0080_0000, 0x3f00_0000, 0x8000, 0, 0x30),
            (Mul, false, 1, 0x3f80_0000, 0, 1, 2),
            (Min, false, 0, 0x8000_0000, 0, 0x8000_0000, 0),
            (Max, true, 0x8000_0000_0000_0000, 0, 0, 0, 0),
            (Sqrt, false, 0, 0x8000_0000, 0, 0x8000_0000, 0),
            (
                Sqrt,
                true,
                0,
                0xbff0_0000_0000_0000,
                0,
                0xfff8_0000_0000_0000,
                1,
            ),
        ] {
            assert_eq!(
                calculate(op, double, left, right, 0x1f80 | extra).unwrap(),
                (bits, 0x1f80 | extra | flags)
            );
            assert_eq!(host_mxcsr(), host);
        }
        let mut invalid = vec![0, 0x1f80 | (1 << 31)];
        if super::super::mxcsr_control::mask() & 0x40 == 0 {
            invalid.push(0x1fc0);
        }
        for control in invalid {
            assert!(matches!(
                calculate(Div, false, 0, 0, control),
                Err(NativeDispatchError::InvalidState)
            ));
            assert_eq!(host_mxcsr(), host);
        }
    }
}
