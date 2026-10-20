// SPDX-License-Identifier: GPL-2.0-only
//! Independent original predicate tables and optional, copied-value AVX oracle.

use super::*;

#[derive(Clone, Copy, Debug)]
enum Relation {
    Less,
    Equal,
    Greater,
    Unordered,
}

fn truth(predicate: u8, relation: Relation) -> bool {
    use Relation::*;
    match predicate & 15 {
        0 => matches!(relation, Equal),
        1 => matches!(relation, Less),
        2 => matches!(relation, Less | Equal),
        3 => matches!(relation, Unordered),
        4 => matches!(relation, Less | Greater | Unordered),
        5 => matches!(relation, Equal | Greater | Unordered),
        6 => matches!(relation, Greater | Unordered),
        7 => !matches!(relation, Unordered),
        8 => matches!(relation, Equal | Unordered),
        9 => matches!(relation, Less | Unordered),
        10 => matches!(relation, Less | Equal | Unordered),
        11 => false,
        12 => matches!(relation, Less | Greater),
        13 => matches!(relation, Equal | Greater),
        14 => matches!(relation, Greater),
        _ => true,
    }
}
fn signaling(predicate: u8) -> bool {
    matches!(
        predicate & 31,
        1 | 2 | 5 | 6 | 9 | 10 | 13 | 14 | 16 | 19 | 20 | 23 | 24 | 27 | 28 | 31
    )
}
fn forms() -> [Form; 6] {
    [
        Form {
            vector_bytes: 16,
            element_bytes: 4,
            scalar: false,
        },
        Form {
            vector_bytes: 32,
            element_bytes: 4,
            scalar: false,
        },
        Form {
            vector_bytes: 16,
            element_bytes: 8,
            scalar: false,
        },
        Form {
            vector_bytes: 32,
            element_bytes: 8,
            scalar: false,
        },
        Form {
            vector_bytes: 16,
            element_bytes: 4,
            scalar: true,
        },
        Form {
            vector_bytes: 16,
            element_bytes: 8,
            scalar: true,
        },
    ]
}
fn bits(form: Form) -> (u64, u64, u64, u64, u64) {
    if form.element_bytes == 8 {
        (
            1 << 63,
            0x3ff0_0000_0000_0000,
            0x4000_0000_0000_0000,
            0x7ff8_0000_0000_0123,
            0x7ff0_0000_0000_0456,
        )
    } else {
        (1 << 31, 0x3f80_0000, 0x4000_0000, 0x7fc0_0123, 0x7f80_0456)
    }
}
fn put(vector: &mut [u8; 32], width: usize, lane: usize, bits: u64) {
    vector[lane * width..(lane + 1) * width].copy_from_slice(&bits.to_le_bytes()[..width]);
}
fn repeated(form: Form, left: u64, right: u64) -> ([u8; 32], [u8; 32]) {
    let mut a = [0xa5; 32];
    let mut b = [0x5a; 32];
    let lanes = if form.scalar {
        1
    } else {
        form.vector_bytes / form.element_bytes
    };
    for lane in 0..lanes {
        put(&mut a, form.element_bytes, lane, left);
        put(&mut b, form.element_bytes, lane, right);
    }
    (a, b)
}
fn expected(form: Form, left: &[u8; 32], relations: &[Relation], predicate: u8) -> [u8; 32] {
    let mut output = [0; 32];
    if form.scalar {
        output[..16].copy_from_slice(&left[..16]);
    }
    for (lane, relation) in relations.iter().enumerate() {
        output[lane * form.element_bytes..(lane + 1) * form.element_bytes]
            .fill(if truth(predicate, *relation) { 0xff } else { 0 });
    }
    output
}
fn host_mxcsr() -> u32 {
    let mut value = 0;
    // SAFETY: SSE2 is mandatory here; only this local output is written.
    unsafe {
        std::arch::asm!("stmxcsr [{p}]", p=in(reg) &mut value, options(nostack,preserves_flags));
    }
    value
}

#[test]
fn all_mask_predicates_reserved_bits_special_values_and_lane_status_match_original_tables() {
    use Relation::*;
    for code in [
        pentax_codegen::Code::Cmpps_xmm_xmmm128_imm8,
        pentax_codegen::Code::EVEX_Vcmpps_kr_k1_xmm_xmmm128b32_imm8,
        pentax_codegen::Code::VEX_Vucomiss_xmm_xmmm32,
    ] {
        assert!(form(code).is_none());
    }
    let host = host_mxcsr();
    for form in forms() {
        let (sign, one, two, quiet, snan) = bits(form);
        let infinity = if form.element_bytes == 8 {
            0x7ff0_0000_0000_0000
        } else {
            0x7f80_0000
        };
        // Values and relations are explicit originals, rather than another
        // floating-point classifier duplicating the production implementation.
        for (a, b, relation, daz_relation, quiet_nan, signaling_nan, denormal) in [
            (one, two, Less, Less, false, false, false),
            (two, one, Greater, Greater, false, false, false),
            (one, one, Equal, Equal, false, false, false),
            (0, sign, Equal, Equal, false, false, false),
            (infinity, infinity, Equal, Equal, false, false, false),
            (sign | infinity, one, Less, Less, false, false, false),
            (quiet, one, Unordered, Unordered, true, false, false),
            (one, quiet, Unordered, Unordered, true, false, false),
            (snan, one, Unordered, Unordered, false, true, false),
            (one, snan, Unordered, Unordered, false, true, false),
            (1, 0, Greater, Equal, false, false, true),
            (sign | 1, 0, Less, Equal, false, false, true),
            (quiet, 1, Unordered, Unordered, true, false, false),
            (1, snan, Unordered, Unordered, false, true, false),
        ] {
            let (left, right) = repeated(form, a, b);
            let count = if form.scalar {
                1
            } else {
                form.vector_bytes / form.element_bytes
            };
            for daz in [false, true] {
                let control = 0xdf84 | if daz { 0x40 } else { 0 };
                if !super::super::mxcsr_control::valid(control) {
                    continue;
                }
                for immediate in 0..=255_u8 {
                    let status = u32::from(signaling_nan || (quiet_nan && signaling(immediate)))
                        | if denormal && !daz { 2 } else { 0 };
                    let relation = if daz { daz_relation } else { relation };
                    let relations = [relation; 8];
                    assert_eq!(
                        calculate(form, &left, &right, immediate, control).unwrap(),
                        (
                            expected(form, &left, &relations[..count], immediate),
                            control | status
                        ),
                        "form={form:?} a={a:x} b={b:x} immediate={immediate} daz={daz}"
                    );
                }
            }
        }
        // NaN suppresses DE in its own lane. A different packed denormal lane
        // contributes independently; inactive scalar lanes contribute nothing.
        for nan in [quiet, snan] {
            let (mut left, mut right) = repeated(form, one, one);
            put(&mut left, form.element_bytes, 0, nan);
            put(&mut left, form.element_bytes, 1, 1);
            put(&mut right, form.element_bytes, 1, 0);
            let count = if form.scalar {
                1
            } else {
                form.vector_bytes / form.element_bytes
            };
            for daz in [false, true] {
                let control = 0x1f84 | if daz { 0x40 } else { 0 };
                if !super::super::mxcsr_control::valid(control) {
                    continue;
                }
                for immediate in 0..32 {
                    let mut relations = [Equal; 8];
                    relations[0] = Unordered;
                    relations[1] = if daz { Equal } else { Greater };
                    let status = u32::from(nan == snan || signaling(immediate))
                        | if !form.scalar && !daz { 2 } else { 0 };
                    assert_eq!(
                        calculate(form, &left, &right, immediate, control).unwrap(),
                        (
                            expected(form, &left, &relations[..count], immediate),
                            control | status
                        )
                    );
                }
            }
        }
    }
    assert_eq!(host_mxcsr(), host);
}

fn hardware<const IMM: u8>(
    form: Form,
    left: &[u8; 32],
    right: &[u8; 32],
    control: u32,
) -> ([u8; 32], u32) {
    let mut host = 0_u32;
    let mut guest = control;
    let mut result = [0; 32];
    macro_rules! run {
        ($instruction:literal, $registers:literal) => {
            // SAFETY: The test caller checked AVX including OS support, and
            // host-supported masked control. Only copied initialized32-byte
            // arrays/local MXCSR buffers are used. Host control is restored
            // inside this single block before Rust resumes. All used YMM
            // registers are declared clobbered; no VZEROUPPER or XCR0 change
            // can disturb unrelated host upper registers/control state.
            unsafe { std::arch::asm!(
                "stmxcsr [{host}]", "ldmxcsr [{guest}]",
                "vmovdqu ymm1, [{left}]", "vmovdqu ymm2, [{right}]",
                concat!($instruction, " ", $registers, ", {imm}"),
                "vmovdqu [{result}], ymm0", "stmxcsr [{guest}]", "ldmxcsr [{host}]",
                host=in(reg) &mut host, guest=in(reg) &mut guest,
                left=in(reg) left.as_ptr(), right=in(reg) right.as_ptr(), result=in(reg) result.as_mut_ptr(),
                imm=const IMM, out("ymm0") _, out("ymm1") _, out("ymm2") _,
                options(nostack,preserves_flags)
            ); }
        };
    }
    match (form.scalar, form.element_bytes, form.vector_bytes) {
        (true, 4, 16) => run!("vcmpss", "xmm0, xmm1, xmm2"),
        (true, 8, 16) => run!("vcmpsd", "xmm0, xmm1, xmm2"),
        (false, 4, 16) => run!("vcmpps", "xmm0, xmm1, xmm2"),
        (false, 4, 32) => run!("vcmpps", "ymm0, ymm1, ymm2"),
        (false, 8, 16) => run!("vcmppd", "xmm0, xmm1, xmm2"),
        (false, 8, 32) => run!("vcmppd", "ymm0, ymm1, ymm2"),
        _ => unreachable!(),
    }
    (result, guest)
}
fn reference(
    form: Form,
    left: &[u8; 32],
    right: &[u8; 32],
    immediate: u8,
    control: u32,
) -> ([u8; 32], u32) {
    assert!(immediate & 0xe0 == 0 || immediate & 0xe0 == 0xe0);
    macro_rules! dispatch {
        ($($p:literal),*) => { match immediate & 31 {
            $($p => if immediate & 0xe0 == 0 { hardware::<$p>(form, left, right, control) }
                else { hardware::<{ $p | 0xe0 }>(form, left, right, control) },)*
            _ => unreachable!(),
        } };
    }
    dispatch!(
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31
    )
}

#[test]
fn mask_comparisons_match_available_avx_hardware_for_all_predicates_nan_denormal_priority() {
    if !std::is_x86_feature_detected!("avx") {
        eprintln!("mask_compare_oracle_available=false mask_compare_oracle_cases=0");
        return;
    }
    let host = host_mxcsr();
    let mut cases = 0;
    for form in forms() {
        let (sign, one, two, quiet, snan) = bits(form);
        let infinity = if form.element_bytes == 8 {
            0x7ff0_0000_0000_0000
        } else {
            0x7f80_0000
        };
        for (a, b) in [
            (one, two),
            (two, one),
            (one, one),
            (0, sign),
            (infinity, infinity),
            (quiet, one),
            (one, quiet),
            (snan, one),
            (one, snan),
            (1, 0),
            (sign | 1, 0),
            (quiet, 1),
            (1, snan),
        ] {
            let (left, right) = repeated(form, a, b);
            for daz in [false, true] {
                let control = 0xdf84 | if daz { 0x40 } else { 0 };
                if !super::super::mxcsr_control::valid(control) {
                    continue;
                }
                for predicate in 0..32 {
                    for immediate in [predicate, predicate | 0xe0] {
                        assert_eq!(
                            calculate(form, &left, &right, immediate, control).unwrap(),
                            reference(form, &left, &right, immediate, control),
                            "form={form:?} a={a:x} b={b:x} predicate={immediate} daz={daz}"
                        );
                        cases += 1;
                    }
                }
            }
        }
        for nan in [quiet, snan] {
            let (mut left, mut right) = repeated(form, one, one);
            put(&mut left, form.element_bytes, 0, nan);
            put(&mut left, form.element_bytes, 1, 1);
            put(&mut right, form.element_bytes, 1, 0);
            for daz in [false, true] {
                let control = 0x1f84 | if daz { 0x40 } else { 0 };
                if !super::super::mxcsr_control::valid(control) {
                    continue;
                }
                for immediate in 0..32 {
                    assert_eq!(
                        calculate(form, &left, &right, immediate, control).unwrap(),
                        reference(form, &left, &right, immediate, control),
                        "crosslane form={form:?} nan={nan:x} predicate={immediate} daz={daz}"
                    );
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(host_mxcsr(), host);
    eprintln!("mask_compare_oracle_available=true mask_compare_oracle_cases={cases}");
}
