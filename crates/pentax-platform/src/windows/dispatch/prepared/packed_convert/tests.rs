// SPDX-License-Identifier: GPL-2.0-only
//! Original numeric tables and a separately gated copied-value AVX oracle.

use super::*;
use pentax_codegen::Code;

#[derive(Clone, Copy, Debug)]
struct Spec {
    code: Code,
    kind: u8,
    wide: bool,
}
const SPECS: [Spec; 12] = [
    Spec {
        code: Code::VEX_Vcvtps2dq_xmm_xmmm128,
        kind: 0,
        wide: false,
    },
    Spec {
        code: Code::VEX_Vcvtps2dq_ymm_ymmm256,
        kind: 0,
        wide: true,
    },
    Spec {
        code: Code::VEX_Vcvttps2dq_xmm_xmmm128,
        kind: 1,
        wide: false,
    },
    Spec {
        code: Code::VEX_Vcvttps2dq_ymm_ymmm256,
        kind: 1,
        wide: true,
    },
    Spec {
        code: Code::VEX_Vcvtpd2dq_xmm_xmmm128,
        kind: 2,
        wide: false,
    },
    Spec {
        code: Code::VEX_Vcvtpd2dq_xmm_ymmm256,
        kind: 2,
        wide: true,
    },
    Spec {
        code: Code::VEX_Vcvttpd2dq_xmm_xmmm128,
        kind: 3,
        wide: false,
    },
    Spec {
        code: Code::VEX_Vcvttpd2dq_xmm_ymmm256,
        kind: 3,
        wide: true,
    },
    Spec {
        code: Code::VEX_Vcvtdq2ps_xmm_xmmm128,
        kind: 4,
        wide: false,
    },
    Spec {
        code: Code::VEX_Vcvtdq2ps_ymm_ymmm256,
        kind: 4,
        wide: true,
    },
    Spec {
        code: Code::VEX_Vcvtdq2pd_xmm_xmmm64,
        kind: 5,
        wide: false,
    },
    Spec {
        code: Code::VEX_Vcvtdq2pd_ymm_xmmm128,
        kind: 5,
        wide: true,
    },
];
fn put(bytes: &mut [u8; 32], width: usize, lane: usize, value: u64) {
    bytes[lane * width..(lane + 1) * width].copy_from_slice(&value.to_le_bytes()[..width]);
}
fn host_mxcsr() -> u32 {
    let mut value = 0;
    // SAFETY: SSE2 is mandatory here and writes only this live local output.
    unsafe {
        std::arch::asm!("stmxcsr [{p}]", p=in(reg) &mut value, options(nostack,preserves_flags));
    }
    value
}

#[test]
fn packed_conversion_original_tables_cover_rounding_daz_zeroing_and_per_lane_exception_union() {
    assert!(form(Code::Cvtps2dq_xmm_xmmm128).is_none());
    assert!(form(Code::EVEX_Vcvtps2dq_xmm_k1z_xmmm128b32).is_none());
    let host = host_mxcsr();
    for spec in SPECS {
        let shape = form(spec.code).unwrap();
        let expected_shape = match spec.kind {
            2 | 3 => (8, 4, if spec.wide { 4 } else { 2 }),
            5 => (4, 8, if spec.wide { 4 } else { 2 }),
            _ => (4, 4, if spec.wide { 8 } else { 4 }),
        };
        assert_eq!(
            (shape.input_element, shape.output_element, shape.lanes),
            expected_shape
        );
        for rounding in 0..4 {
            for daz in [false, true] {
                let control = 0x9f84 | (rounding << 13) | if daz { 0x40 } else { 0 };
                if !super::super::mxcsr_control::valid(control) {
                    continue;
                }
                if spec.kind < 4 {
                    // Single/double raw bits, explicit RC results, sticky flag,
                    // and whether DAZ replaces this tiny input by signed zero.
                    for (single, double, outputs, status, tiny) in [
                        (0_u64, 0_u64, [0_u32; 4], 0, false),
                        (0x8000_0000, 0x8000_0000_0000_0000, [0; 4], 0, false),
                        (0x3fc0_0000, 0x3ff8_0000_0000_0000, [2, 1, 2, 1], 32, false),
                        (
                            0xc020_0000,
                            0xc004_0000_0000_0000,
                            [0xffff_fffe, 0xffff_fffd, 0xffff_fffe, 0xffff_fffe],
                            32,
                            false,
                        ),
                        (
                            0xcf00_0000,
                            0xc1e0_0000_0000_0000,
                            [0x8000_0000; 4],
                            0,
                            false,
                        ),
                        (
                            0x4f00_0000,
                            0x41e0_0000_0000_0000,
                            [0x8000_0000; 4],
                            1,
                            false,
                        ),
                        (
                            0x7f80_0000,
                            0x7ff0_0000_0000_0000,
                            [0x8000_0000; 4],
                            1,
                            false,
                        ),
                        (
                            0x7fc0_0123,
                            0x7ff8_0000_0000_0123,
                            [0x8000_0000; 4],
                            1,
                            false,
                        ),
                        (
                            0x7f80_0456,
                            0x7ff0_0000_0000_0456,
                            [0x8000_0000; 4],
                            1,
                            false,
                        ),
                        (1, 1, [0, 0, 1, 0], 32, true),
                        (
                            0x8000_0001,
                            0x8000_0000_0000_0001,
                            [0, u32::MAX, 0, 0],
                            32,
                            true,
                        ),
                    ] {
                        let raw = if shape.input_element == 8 {
                            double
                        } else {
                            single
                        };
                        let rc = if spec.kind & 1 != 0 {
                            3
                        } else {
                            rounding as usize
                        };
                        let value = if daz && tiny { 0 } else { outputs[rc] };
                        let status = if daz && tiny { 0 } else { status };
                        let mut input = [0xa5; 32];
                        let mut expected = [0; 32];
                        for lane in 0..shape.lanes {
                            put(&mut input, shape.input_element, lane, raw);
                            put(&mut expected, 4, lane, u64::from(value));
                        }
                        assert_eq!(
                            calculate(shape, &input, control).unwrap(),
                            (expected, control | status),
                            "spec={spec:?} raw={raw:x} rc={rounding} daz={daz}"
                        );
                    }
                    // Invalid suppresses its OWN precision exception. Another
                    // active1.5 lane must still contribute PE, in either order.
                    for nan_first in [false, true] {
                        let mut input = [0; 32];
                        let mut expected = [0; 32];
                        let (nan, half) = if shape.input_element == 8 {
                            (0x7ff8_0000_0000_0123, 0x3ff8_0000_0000_0000)
                        } else {
                            (0x7fc0_0123, 0x3fc0_0000)
                        };
                        let nan_lane = usize::from(!nan_first);
                        let half_lane = usize::from(nan_first);
                        put(&mut input, shape.input_element, nan_lane, nan);
                        put(&mut input, shape.input_element, half_lane, half);
                        put(&mut expected, 4, nan_lane, 0x8000_0000);
                        let value = if spec.kind & 1 != 0 || rounding == 1 || rounding == 3 {
                            1
                        } else {
                            2
                        };
                        put(&mut expected, 4, half_lane, value);
                        assert_eq!(
                            calculate(shape, &input, control).unwrap(),
                            (expected, control | 33)
                        );
                    }
                } else {
                    for (value, singles, double, inexact) in [
                        (0_i32, [0_u64; 4], 0_u64, false),
                        (1, [0x3f80_0000; 4], 0x3ff0_0000_0000_0000, false),
                        (-1, [0xbf80_0000; 4], 0xbff0_0000_0000_0000, false),
                        (
                            16_777_217,
                            [0x4b80_0000, 0x4b80_0000, 0x4b80_0001, 0x4b80_0000],
                            0x4170_0000_1000_0000,
                            true,
                        ),
                        (
                            -16_777_217,
                            [0xcb80_0000, 0xcb80_0001, 0xcb80_0000, 0xcb80_0000],
                            0xc170_0000_1000_0000,
                            true,
                        ),
                        (
                            i32::MAX,
                            [0x4f00_0000, 0x4eff_ffff, 0x4f00_0000, 0x4eff_ffff],
                            0x41df_ffff_ffc0_0000,
                            true,
                        ),
                        (i32::MIN, [0xcf00_0000; 4], 0xc1e0_0000_0000_0000, false),
                    ] {
                        let mut input = [0xa5; 32];
                        let mut expected = [0; 32];
                        let result = if spec.kind == 5 {
                            double
                        } else {
                            singles[rounding as usize]
                        };
                        for lane in 0..shape.lanes {
                            put(&mut input, 4, lane, u64::from(value as u32));
                            put(&mut expected, shape.output_element, lane, result);
                        }
                        assert_eq!(
                            calculate(shape, &input, control).unwrap(),
                            (
                                expected,
                                control | if spec.kind == 4 && inexact { 32 } else { 0 }
                            ),
                            "spec={spec:?} value={value} rc={rounding}"
                        );
                    }
                }
            }
        }
    }
    assert_eq!(host_mxcsr(), host);
}

fn hardware(spec: Spec, input: &[u8; 32], control: u32) -> ([u8; 32], u32) {
    let mut host = 0_u32;
    let mut guest = control;
    let mut output = [0; 32];
    macro_rules! run {
        ($op:literal, $registers:literal) => {
            // SAFETY: Caller checked AVX/OS support and masked host-supported
            // control. Only copied original local arrays enter this block.
            // MXCSR is restored before Rust resumes, full used YMM registers
            // are clobbered, and no VZEROUPPER/XCR0/global host policy is used.
            unsafe { std::arch::asm!(
                "stmxcsr [{host}]", "ldmxcsr [{guest}]", "vmovdqu ymm1, [{input}]",
                concat!($op, " ", $registers), "vmovdqu [{output}], ymm0",
                "stmxcsr [{guest}]", "ldmxcsr [{host}]",
                host=in(reg) &mut host, guest=in(reg) &mut guest,
                input=in(reg) input.as_ptr(), output=in(reg) output.as_mut_ptr(),
                out("ymm0") _, out("ymm1") _, options(nostack,preserves_flags)
            ); }
        };
    }
    match (spec.kind, spec.wide) {
        (0, false) => run!("vcvtps2dq", "xmm0, xmm1"),
        (0, true) => run!("vcvtps2dq", "ymm0, ymm1"),
        (1, false) => run!("vcvttps2dq", "xmm0, xmm1"),
        (1, true) => run!("vcvttps2dq", "ymm0, ymm1"),
        (2, false) => run!("vcvtpd2dq", "xmm0, xmm1"),
        (2, true) => run!("vcvtpd2dq", "xmm0, ymm1"),
        (3, false) => run!("vcvttpd2dq", "xmm0, xmm1"),
        (3, true) => run!("vcvttpd2dq", "xmm0, ymm1"),
        (4, false) => run!("vcvtdq2ps", "xmm0, xmm1"),
        (4, true) => run!("vcvtdq2ps", "ymm0, ymm1"),
        (5, false) => run!("vcvtdq2pd", "xmm0, xmm1"),
        (5, true) => run!("vcvtdq2pd", "ymm0, xmm1"),
        _ => unreachable!(),
    }
    (output, guest)
}
fn classes(spec: Spec) -> &'static [u64] {
    if spec.kind >= 4 {
        &[
            0,
            1,
            0xffff_ffff,
            0x0100_0001,
            0xfeff_ffff,
            0x7fff_ffff,
            0x8000_0000,
            0x0100_0000,
            0x7fff_ff80,
        ]
    } else if spec.kind < 2 {
        &[
            0,
            0x8000_0000,
            1,
            0x8000_0001,
            0x3fc0_0000,
            0xc020_0000,
            0x4eff_ffff,
            0x4f00_0000,
            0xcf00_0000,
            0x7f80_0000,
            0xff80_0000,
            0x7fc0_0123,
            0x7f80_0456,
        ]
    } else {
        &[
            0,
            0x8000_0000_0000_0000,
            1,
            0x8000_0000_0000_0001,
            0x3ff8_0000_0000_0000,
            0xc004_0000_0000_0000,
            0x41df_ffff_ffe0_0000,
            0x41e0_0000_0000_0000,
            0xc1e0_0000_0000_0000,
            0x7ff0_0000_0000_0000,
            0xfff0_0000_0000_0000,
            0x7ff8_0000_0000_0123,
            0x7ff0_0000_0000_0456,
        ]
    }
}

#[test]
fn packed_conversions_match_available_avx_for_rounding_special_values_and_mixed_lane_flags() {
    if !std::is_x86_feature_detected!("avx") {
        eprintln!("packed_convert_oracle_available=false packed_convert_oracle_cases=0");
        return;
    }
    let host = host_mxcsr();
    let mut count = 0;
    for spec in SPECS {
        let shape = form(spec.code).unwrap();
        let classes = classes(spec);
        for case in 0..classes.len() + 4 {
            let mut input = [0xa5; 32];
            for lane in 0..shape.lanes {
                let bits = if case < classes.len() {
                    classes[case]
                } else if spec.kind < 4 && lane < 2 {
                    let nan = if shape.input_element == 8 {
                        0x7ff8_0000_0000_0123
                    } else {
                        0x7fc0_0123
                    };
                    let half = if shape.input_element == 8 {
                        0x3ff8_0000_0000_0000
                    } else {
                        0x3fc0_0000
                    };
                    if (lane + case) & 1 == 0 { nan } else { half }
                } else {
                    classes[(case + lane) % classes.len()]
                };
                put(&mut input, shape.input_element, lane, bits);
            }
            for rounding in 0..4 {
                for daz in [false, true] {
                    for ftz in [false, true] {
                        let control = 0x1f84
                            | (rounding << 13)
                            | if daz { 0x40 } else { 0 }
                            | if ftz { 0x8000 } else { 0 };
                        if !super::super::mxcsr_control::valid(control) {
                            continue;
                        }
                        assert_eq!(
                            calculate(shape, &input, control).unwrap(),
                            hardware(spec, &input, control),
                            "spec={spec:?} originalcase={case} rc={rounding} daz={daz} ftz={ftz}"
                        );
                        count += 1;
                    }
                }
            }
        }
    }
    assert_eq!(host_mxcsr(), host);
    eprintln!("packed_convert_oracle_available=true packed_convert_oracle_cases={count}");
}
