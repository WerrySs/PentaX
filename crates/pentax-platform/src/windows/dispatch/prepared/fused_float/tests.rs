// SPDX-License-Identifier: GPL-2.0-only
use super::*;

fn host_mxcsr() -> u32 {
    let mut value = 0;
    // SAFETY: The live local buffer receives SSE2 control; no control changes.
    unsafe {
        std::arch::asm!("stmxcsr [{p}]", p=in(reg) &mut value, options(nostack,preserves_flags));
    }
    value
}

#[test]
fn exact_integer_fma_retains_fused_cancellation_zero_signs_and_host_control() {
    let host = host_mxcsr();
    for (double, x, y, z, expected) in [
        (
            true,
            0x3ff0_0000_0200_0000,
            0x3fef_ffff_fc00_0000,
            0xbff0_0000_0000_0000,
            0xbc90_0000_0000_0000,
        ),
        (false, 0x3f80_0400, 0x3f7f_f800, 0xbf80_0000, 0xb280_0000),
    ] {
        for round in 0..4 {
            let control = 0x1f84 | (round << 13);
            assert_eq!(
                calculate(double, x, y, z, false, false, control).unwrap(),
                (expected, control)
            );
        }
    }
    for double in [false, true] {
        let sign = if double { 1 << 63 } else { 1 << 31 };
        let one = if double {
            0x3ff0_0000_0000_0000
        } else {
            0x3f80_0000
        };
        for round in 0..4 {
            let control = 0x1f80 | (round << 13);
            for (x, y, z, expected) in [
                (0, one, 0, 0),
                (sign, one, sign, sign),
                (one, one, one | sign, if round == 1 { sign } else { 0 }),
            ] {
                assert_eq!(
                    calculate(double, x, y, z, false, false, control).unwrap(),
                    (expected, control)
                );
            }
        }
    }
    assert!(calculate(true, 0, 0, 0, false, false, 0).is_err());
    assert!(calculate(true, 0, 0, 0, false, false, 0x1_1f80).is_err());
    assert_eq!(host_mxcsr(), host);
}

fn reference(
    double: bool,
    x: u64,
    y: u64,
    z: u64,
    neg_product: bool,
    neg_addend: bool,
    control: u32,
) -> (u64, u32) {
    let mut host = 0_u32;
    let mut guest = control;
    let result: u64;
    macro_rules! run {
        ($op:literal) => {
            // SAFETY: Caller checked host AVX/FMA including OS support. Only
            // copied artificial bits/local masked control enter this reference.
            // XMM0=y,XMM1=x means213 computes algebraic x*y+z with documented
            // NaN priority. Host control is restored before any compiled Rust.
            unsafe { std::arch::asm!(
                "stmxcsr [{host}]", "ldmxcsr [{guest}]",
                "movq xmm0, {y}", "movq xmm1, {x}", "movq xmm2, {z}",
                concat!($op," xmm0, xmm1, xmm2"),
                "stmxcsr [{guest}]", "ldmxcsr [{host}]", "movq {result}, xmm0",
                host=in(reg) &mut host,guest=in(reg) &mut guest,
                x=in(reg) x,y=in(reg) y,z=in(reg) z,result=lateout(reg) result,
                out("xmm0") _,out("xmm1") _,out("xmm2") _,options(nostack,preserves_flags)
            ); }
        }
    }
    match (double, neg_product, neg_addend) {
        (true, false, false) => run!("vfmadd213sd"),
        (false, false, false) => run!("vfmadd213ss"),
        (true, false, true) => run!("vfmsub213sd"),
        (false, false, true) => run!("vfmsub213ss"),
        (true, true, false) => run!("vfnmadd213sd"),
        (false, true, false) => run!("vfnmadd213ss"),
        (true, true, true) => run!("vfnmsub213sd"),
        (false, true, true) => run!("vfnmsub213ss"),
    }
    (if double { result } else { result & 0xffff_ffff }, guest)
}

#[test]
fn software_fma_matches_hardware_for_rounding_denormals_special_values_and_exponent_extremes() {
    if !std::is_x86_feature_detected!("avx") || !std::is_x86_feature_detected!("fma") {
        eprintln!("Host reference unavailable; analytic software FMA checks remain active.");
        return;
    }
    let host = host_mxcsr();
    let mut cases = 0;
    for double in [false, true] {
        let (sign, one, infinity, quiet, payload) = if double {
            (
                1_u64 << 63,
                0x3ff0_0000_0000_0000,
                0x7ff0_0000_0000_0000,
                1_u64 << 51,
                0x123,
            )
        } else {
            (1_u64 << 31, 0x3f80_0000, 0x7f80_0000, 1_u64 << 22, 0x123)
        };
        let classes = [
            0,
            sign,
            1,
            sign | 1,
            one,
            one | sign,
            infinity - 1,
            infinity,
            sign | infinity,
            infinity | quiet | payload,
            sign | infinity | 0x456,
        ];
        let mut triples = Vec::new();
        for x in classes {
            for y in classes {
                for z in classes {
                    triples.push([x, y, z]);
                }
            }
        }
        // Raw original random bits cover both exponent extremes and long-gap
        // alignment/cancellation; no source-game data or host FP oracle is used.
        let mut random = 0x17b2_924d_7839_aebf_u64;
        for _ in 0..256 {
            triples.push(std::array::from_fn(|_| {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                if double { random } else { random & 0xffff_ffff }
            }));
        }
        // Values straddling minimum-normal and overflow rounding thresholds.
        let (min_normal, half, epsilon) = if double {
            (1_u64 << 52, one - (1_u64 << 52), 0x3ca0_0000_0000_0000)
        } else {
            (1_u64 << 23, one - (1_u64 << 23), 0x3400_0000)
        };
        triples.extend([
            [min_normal, half, 0],
            [min_normal, one, sign | 1],
            [min_normal, one, sign | (1 << if double { 49 } else { 20 })],
            [infinity - 1, one, epsilon],
            [one, one, epsilon],
            [min_normal, half, min_normal - 1],
        ]);
        let fraction_bits = if double { 52 } else { 23 };
        let bias = if double { 1023 } else { 127 };
        let quarter_max_ulp = ((2 * bias - fraction_bits - 2) as u64) << fraction_bits;
        let half_max_ulp = quarter_max_ulp + (1_u64 << fraction_bits);
        triples.extend([
            [min_normal, one - 1, 0],
            [min_normal, one - 1, 1],
            [infinity - 1, one, quarter_max_ulp],
            [infinity - 1, one, half_max_ulp],
            [infinity - 1, one, half_max_ulp + 1],
        ]);
        for controls in 0..16_u32 {
            let control =
                0x1f84 | ((controls & 3) << 13) | ((controls & 4) << 4) | ((controls & 8) << 12);
            if !super::super::mxcsr_control::valid(control) {
                continue;
            }
            for [x, y, z] in &triples {
                for (np, na) in [(false, false), (false, true), (true, false), (true, true)] {
                    let expected = reference(double, *x, *y, *z, np, na, control);
                    let actual = calculate(double, *x, *y, *z, np, na, control).unwrap();
                    assert_eq!(
                        actual, expected,
                        "double={double} x={x:016x} y={y:016x} z={z:016x} np={np} na={na} control={control:04x}"
                    );
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(host_mxcsr(), host);
    eprintln!("Original FMA hardware-reference cases passed: {cases}");
}
