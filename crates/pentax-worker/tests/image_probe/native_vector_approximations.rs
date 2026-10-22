// SPDX-License-Identifier: GPL-2.0-only
//! Original ISA consumers for reciprocal and reciprocal-square-root estimates.
//! Native register forms use an independent same-form hardware oracle. Checked
//! memory/fallback finite results use the ISA error bound, never exact DIV/SQRT
//! as a production substitute. No commercial input or timing claim is used.
use super::*;

const ENTRY: i32 = 0x1000;
const ERROR: f64 = 1.5 / 4096.0;
#[derive(Clone, Copy)]
struct Form {
    opcode: u8,
    wide: bool,
    scalar: bool,
}
fn forms() -> [Form; 6] {
    // The first operation is the actual fault's opcode/width/register shape,
    // with independently authored operands rather than an imported sequence.
    [
        Form {
            opcode: 0x52,
            wide: false,
            scalar: false,
        },
        Form {
            opcode: 0x52,
            wide: true,
            scalar: false,
        },
        Form {
            opcode: 0x52,
            wide: false,
            scalar: true,
        },
        Form {
            opcode: 0x53,
            wide: false,
            scalar: false,
        },
        Form {
            opcode: 0x53,
            wide: true,
            scalar: false,
        },
        Form {
            opcode: 0x53,
            wide: false,
            scalar: true,
        },
    ]
}
// This independent, test-only hardware oracle is the sole unsafe boundary.
// The worker production crate's deny policy is unchanged; inputs are original
// local arrays, AVX is OS/CPU gated, and each assembly block restores MXCSR.
#[allow(unsafe_code)]
mod hardware_oracle {
    use super::Form;
    use std::arch::asm;
    pub(super) fn mxcsr() -> u32 {
        let mut value = 0u32;
        // SAFETY: x86-64 guarantees SSE; this exact four-byte local is writable.
        unsafe {
            asm!("stmxcsr [{pointer}]", pointer = in(reg) &mut value, options(nostack, preserves_flags));
        }
        value
    }
    fn set_mxcsr(value: u32) {
        // SAFETY: callers use either the exact saved host state or a masked value
        // admitted by the independent FXSAVE-reported hardware control mask.
        unsafe {
            asm!("ldmxcsr [{pointer}]", pointer = in(reg) &value, options(nostack, preserves_flags));
        }
    }
    pub(super) struct RestoreMxcsr(pub(super) u32);
    impl Drop for RestoreMxcsr {
        fn drop(&mut self) {
            set_mxcsr(self.0);
        }
    }
    pub(super) fn controls() -> [u32; 2] {
        #[repr(C, align(16))]
        struct Save([u8; 512]);
        let mut saved = Save([0; 512]);
        // SAFETY: the complete 512-byte destination has FXSAVE's 16-byte alignment.
        unsafe {
            asm!("fxsave64 [{pointer}]", pointer = in(reg) saved.0.as_mut_ptr(), options(nostack, preserves_flags));
        }
        let mask = u32::from_le_bytes(saved.0[28..32].try_into().unwrap());
        let mask = if mask == 0 { 0xffbf } else { mask };
        let second = 0xffc4 & mask; // all masks, sticky ZE, alternate RC/FTZ/DAZ when supported.
        assert_eq!(second & 0x1f84, 0x1f84);
        [0x1f84, second]
    }
    #[inline(never)]
    #[target_feature(enable = "avx")]
    unsafe fn avx_oracle(input: [u32; 8], form: Form, control: u32) -> ([u32; 8], u32, u32) {
        let mut output = [0u32; 8];
        let mut before = 0u32;
        let mut observed = 0u32;
        macro_rules! estimate {
            ($load:literal, $operation:literal, $store:literal) => {{
                // SAFETY: AVX was OS/CPU gated. All pointers name disjoint,
                // complete local arrays/words, and valid masked MXCSR is used.
                // The single block restores control before any Rust instruction,
                // call or unwind. YMM0 is the sole vector clobber; GPR pointer
                // inputs are unchanged, with no lateout register aliasing.
                unsafe { asm!(
                    "stmxcsr [{saved}]", "ldmxcsr [{control}]",
                    $load, $operation, $store,
                    "stmxcsr [{observed}]", "ldmxcsr [{saved}]",
                    saved = in(reg) &mut before, control = in(reg) &control,
                    source = in(reg) input.as_ptr(), destination = in(reg) output.as_mut_ptr(),
                    observed = in(reg) &mut observed, out("ymm0") _,
                    options(nostack, preserves_flags)); }
            }};
        }
        match (form.opcode, form.wide, form.scalar) {
            (0x52, true, false) => estimate!(
                "vmovups ymm0, [{source}]",
                "vrsqrtps ymm0, ymm0",
                "vmovups [{destination}], ymm0"
            ),
            (0x53, true, false) => estimate!(
                "vmovups ymm0, [{source}]",
                "vrcpps ymm0, ymm0",
                "vmovups [{destination}], ymm0"
            ),
            (0x52, false, false) => estimate!(
                "vmovups xmm0, [{source}]",
                "vrsqrtps xmm0, xmm0",
                "vmovups [{destination}], xmm0"
            ),
            (0x53, false, false) => estimate!(
                "vmovups xmm0, [{source}]",
                "vrcpps xmm0, xmm0",
                "vmovups [{destination}], xmm0"
            ),
            (0x52, false, true) => estimate!(
                "vmovups xmm0, [{source}]",
                "vrsqrtss xmm0, xmm0, xmm0",
                "vmovups [{destination}], xmm0"
            ),
            (0x53, false, true) => estimate!(
                "vmovups xmm0, [{source}]",
                "vrcpss xmm0, xmm0, xmm0",
                "vmovups [{destination}], xmm0"
            ),
            _ => unreachable!(),
        }
        (output, before, observed)
    }
    fn sse_oracle(input: [u32; 8], form: Form, control: u32) -> ([u32; 8], u32, u32) {
        let mut output = [0u32; 8];
        let mut before = 0u32;
        let mut observed = 0u32;
        macro_rules! estimate {
            ($operation:literal, $second_half:literal) => {{
                // SAFETY: SSE is guaranteed on x86-64. These are complete local
                // arrays/words; only fixed ISA text and valid masked control
                // enter this block. No call/unwind/stack instruction executes
                // with guest control installed. Only XMM0 and the declared
                // output memory are changed; pointer input GPRs stay intact.
                unsafe { asm!(
                    "stmxcsr [{saved}]", "ldmxcsr [{control}]",
                    "movups xmm0, [{source}]", $operation, "movups [{destination}], xmm0",
                    $second_half,
                    "stmxcsr [{observed}]", "ldmxcsr [{saved}]",
                    saved = in(reg) &mut before, control = in(reg) &control,
                    source = in(reg) input.as_ptr(), destination = in(reg) output.as_mut_ptr(),
                    observed = in(reg) &mut observed, out("xmm0") _,
                    options(nostack, preserves_flags)); }
            }};
        }
        match (form.opcode, form.wide, form.scalar) {
            (0x52, true, false) => estimate!(
                "rsqrtps xmm0, xmm0",
                "movups xmm0, [{source} + 16]\nrsqrtps xmm0, xmm0\nmovups [{destination} + 16], xmm0"
            ),
            (0x53, true, false) => estimate!(
                "rcpps xmm0, xmm0",
                "movups xmm0, [{source} + 16]\nrcpps xmm0, xmm0\nmovups [{destination} + 16], xmm0"
            ),
            (0x52, false, false) => estimate!("rsqrtps xmm0, xmm0", ""),
            (0x53, false, false) => estimate!("rcpps xmm0, xmm0", ""),
            (0x52, false, true) => estimate!("rsqrtss xmm0, xmm0", ""),
            (0x53, false, true) => estimate!("rcpss xmm0, xmm0", ""),
            _ => unreachable!(),
        }
        (output, before, observed)
    }
    pub(super) fn hardware(input: [u32; 8], form: Form, control: u32) -> [u32; 8] {
        let input = std::hint::black_box(input);
        let before = mxcsr();
        assert!(controls().contains(&control));
        let (output, saved, observed) = if std::is_x86_feature_detected!("avx") {
            // SAFETY: OS/CPU AVX support is checked at this exact call site.
            unsafe { avx_oracle(input, form, control) }
        } else {
            sse_oracle(input, form, control)
        };
        assert_eq!(saved, before);
        assert_eq!(
            observed, control,
            "the ISA estimate changes no MXCSR flags/control"
        );
        assert_eq!(
            mxcsr(),
            before,
            "the single assembly block restores host MXCSR"
        );
        output
    }
}
use hardware_oracle::{RestoreMxcsr, controls, hardware, mxcsr};
#[derive(Clone, Copy)]
enum Expected {
    Exact(u32),
    Bounded(u32, u32),
    QuietNaN,
}
fn architectural(opcode: u8, input: u32) -> Expected {
    let sign = input & 0x8000_0000;
    let magnitude = input & 0x7fff_ffff;
    let exponent = magnitude >> 23;
    let fraction = magnitude & 0x007f_ffff;
    if exponent == 0 {
        return Expected::Exact(sign | 0x7f80_0000);
    }
    if exponent == 255 {
        if fraction != 0 {
            return Expected::Exact(input | 0x0040_0000);
        }
        return if opcode == 0x52 && sign != 0 {
            Expected::QuietNaN
        } else {
            Expected::Exact(sign)
        };
    }
    if opcode == 0x52 && sign != 0 {
        return Expected::QuietNaN;
    }
    // This reference runs after hardware() has restored the host control.
    // It establishes only the documented relative error interval for finite
    // normal results. A conservative one-ULP outward rounding of its endpoints
    // avoids demanding cross-opcode packed/scalar bit identity.
    let input = f64::from(f32::from_bits(magnitude));
    let ideal = if opcode == 0x52 {
        1.0 / input.sqrt()
    } else {
        1.0 / input
    };
    if opcode == 0x53 && ideal < f64::from(f32::MIN_POSITIVE) {
        return Expected::Exact(sign);
    }
    let lower = ((ideal * (1.0 - ERROR)) as f32).to_bits().saturating_sub(1);
    let upper = ((ideal * (1.0 + ERROR)) as f32).to_bits().saturating_add(1);
    Expected::Bounded(lower | sign, upper | sign)
}
fn merge() -> [u32; 8] {
    [
        0x3f00_0000,
        0x7fa0_0123,
        0x8000_0000,
        0x7fc0_4567,
        0x0123_4567,
        0x89ab_cdef,
        0x7654_3210,
        0xfedc_ba98,
    ]
}
fn expected(input: [u32; 8], form: Form, control: u32, exact: bool) -> [Expected; 8] {
    let before = mxcsr();
    let restore = RestoreMxcsr(before);
    let oracle = hardware(input, form, control);
    let result = std::array::from_fn(|lane| {
        if form.scalar && lane > 0 && lane < 4 {
            Expected::Exact(merge()[lane])
        } else if !form.wide && lane >= 4 {
            Expected::Exact(0)
        } else if exact {
            Expected::Exact(oracle[lane])
        } else {
            architectural(form.opcode, input[lane])
        }
    });
    drop(restore);
    assert_eq!(
        mxcsr(),
        before,
        "all oracle/reference work restores host MXCSR"
    );
    result
}
fn words(input: [u32; 8]) -> [u64; 4] {
    std::array::from_fn(|index| u64::from(input[index * 2]) | u64::from(input[index * 2 + 1]) << 32)
}
fn pattern(register: u8) -> [u64; 4] {
    let tag = (u64::from(register) + 1) * 0x0101_0101_0101_0101;
    [
        0x0123_4567_89ab_cdef ^ tag,
        0xfedc_ba98_7654_3210 ^ tag,
        0x8877_6655_4433_2211 ^ tag,
        0x89ab_cdef_0123_4567 ^ tag,
    ]
}
fn fill(p: &mut Program, offset: u8, words: [u64; 4]) {
    for (index, value) in words.into_iter().enumerate() {
        p.reg(0, value);
        p.save(offset + index as u8 * 8);
    }
}
fn copy(p: &mut Program, load: bool, register: u8, offset: u8) {
    p.code.extend([
        0xc5,
        if register >= 8 { 0x7c } else { 0xfc },
        if load { 0x10 } else { 0x11 },
        0x44 | ((register & 7) << 3),
        0x24,
        offset,
    ]);
}
fn approximation(
    p: &mut Program,
    form: Form,
    destination: u8,
    source: u8,
    memory: bool,
    null: bool,
) {
    p.code.extend([
        0xc4,
        0xe1 ^ (u8::from(destination >= 8) << 7) ^ (u8::from(!memory && source >= 8) << 5),
        (if form.scalar {
            ((!13u8 & 15) << 3) | 2
        } else {
            0x78
        }) | (u8::from(form.wide) << 2),
        form.opcode,
        ((destination & 7) << 3)
            | if memory {
                if null { 0 } else { 0x44 }
            } else {
                0xc0 | (source & 7)
            },
    ]);
    if memory && !null {
        p.code.extend([0x24, 49]);
    }
}
fn fail(p: &mut Program, opcode: u8) {
    p.code.extend([0x0f, opcode, 0, 0, 0, 0]);
    p.branches.push(p.code.len() - 4);
}
fn check_result(p: &mut Program, expected: [Expected; 8]) {
    if expected
        .iter()
        .all(|value| matches!(value, Expected::Exact(_)))
    {
        for index in 0..4 {
            let Expected::Exact(low) = expected[index * 2] else {
                unreachable!()
            };
            let Expected::Exact(high) = expected[index * 2 + 1] else {
                unreachable!()
            };
            p.word(96 + index as u8 * 8, u64::from(low) | u64::from(high) << 32);
        }
    } else {
        for (lane, value) in expected.into_iter().enumerate() {
            p.code.extend([0x8b, 0x44, 0x24, 96 + lane as u8 * 4]);
            match value {
                Expected::Exact(bits) => p.expect(u64::from(bits)),
                Expected::Bounded(lower, upper) => {
                    p.code.push(0x3d);
                    p.code.extend(lower.to_le_bytes());
                    fail(p, 0x82);
                    p.code.push(0x3d);
                    p.code.extend(upper.to_le_bytes());
                    fail(p, 0x87);
                }
                Expected::QuietNaN => {
                    p.code.push(0x25);
                    p.code.extend(0x7fc0_0000u32.to_le_bytes());
                    p.expect(0x7fc0_0000);
                }
            }
        }
    }
}
fn finish(mut p: Program) -> Vec<u8> {
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0x3000);
    p.code
}
fn consumer(input: [u32; 8], control: u32) -> Vec<u8> {
    let native = std::is_x86_feature_detected!("avx");
    let mut p = Program::new();
    p.stack(0x89, 4, 8);
    p.reg(12, 0x5678_1234_90ab_cdef);
    p.reg(0, u64::from(control));
    p.save(0);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]);
    for register in 0..16 {
        fill(&mut p, 96, pattern(register));
        copy(&mut p, true, register, 96);
    }
    fill(&mut p, 16, words(merge()));
    fill(&mut p, 49, words(input));
    for form in forms() {
        let cases = if form.scalar {
            [(1, 0), (13, 14), (14, 14)]
        } else {
            [(1, 0), (14, 14), (9, 14)]
        };
        for (case, (destination, source)) in cases.into_iter().chain([(9, 0)]).enumerate() {
            let memory = case == 3;
            copy(&mut p, true, 13, 16);
            copy(&mut p, true, 14, 49);
            copy(&mut p, true, 0, 49);
            p.reg(0, 1);
            p.reg(2, 2);
            p.code.extend([0x48, 0x39, 0xd0]);
            approximation(&mut p, form, destination, source, memory, false);
            for opcode in [0x83, 0x8b, 0x84, 0x89, 0x80] {
                fail(&mut p, opcode);
            }
            copy(&mut p, false, destination, 96);
            check_result(&mut p, expected(input, form, control, native && !memory));
        }
    }
    // A genuine HLE service boundary then exposes the entire live vector bank.
    rip(&mut p.code, ENTRY, &[0xff, 0x15], 0xf00);
    p.expect(1);
    let last = forms()[5];
    for register in 0..16 {
        copy(&mut p, false, register, 96);
        if matches!(register, 1 | 9) {
            check_result(
                &mut p,
                expected(input, last, control, native && register == 1),
            );
        } else {
            let expected = match register {
                0 | 14 => words(input),
                13 => words(merge()),
                _ => pattern(register),
            };
            for (index, value) in expected.into_iter().enumerate() {
                p.word(96 + index as u8 * 8, value);
            }
        }
    }
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 0]);
    p.word(0, u64::from(control));
    p.code.extend([0x48, 0x89, 0xe0]);
    p.load(10, 8);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x4c, 0x89, 0xe0]);
    p.expect(0x5678_1234_90ab_cdef);
    finish(p)
}
fn assert_matrix(fields: &HashMap<String, String>) {
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "1");
    assert_eq!(fields["fp_status"], "4");
    assert_eq!(fields["integer_ops"], "0");
    let native = match fields["native_ymm_mode"].as_str() {
        "avx" | "avx2" => true,
        "sse2" => false,
        _ => panic!("unknown closed vector mode"),
    };
    assert_eq!(native, std::is_x86_feature_detected!("avx"));
    // Six memory estimates read16+32+4 bytes twice. Eighteen register
    // estimates are native only with the real AVX bridge. The128 temporal
    // YMM copies are the same original setup/readback operations in each case.
    assert_eq!(
        fields["vector_ops"].parse::<u64>().unwrap(),
        if native { 8 } else { 154 }
    );
    assert_eq!(
        fields["vector_bytes"].parse::<u64>().unwrap(),
        if native { 112 } else { 4208 }
    );
}
#[test]
fn actual_native_estimate_forms_preserve_aliases_merge_upper_lanes_and_service_state() {
    let input = [
        0x3f00_0000,
        0x4000_0000,
        0x4040_0000,
        0x4080_0000,
        0x4100_0000,
        0x4180_0000,
        0x41c8_0000,
        0x0080_0000,
    ];
    for control in controls() {
        let image = super::native_packed_float::original(&consumer(input, control));
        let (status, fields, text) = probe(&image);
        assert_eq!(status, 0, "{text}");
        assert_matrix(&fields);
    }
    println!(
        "vector_approximation_native_consumers_available=true vector_approximation_native_consumer_programs=2 vector_approximation_native_consumer_forms=6 vector_approximation_native_consumer_cases=48"
    );
}
#[test]
fn actual_estimate_edges_keep_signed_zero_denormals_nan_classes_and_mxcsr() {
    for input in [
        [
            0,
            0x8000_0000,
            0x7f80_0000,
            0xff80_0000,
            0xc080_0000,
            0x7fc1_2345,
            0x7fa2_3456,
            1,
        ],
        [
            0x8000_0001,
            0x7f7f_ffff,
            0xff7f_ffff,
            0x0080_0000,
            0x8080_0000,
            0x3f80_0000,
            0xbf80_0000,
            0xffa0_4567,
        ],
    ] {
        for control in controls() {
            let image = super::native_packed_float::original(&consumer(input, control));
            let (status, fields, text) = probe(&image);
            assert_eq!(status, 0, "{text}");
            assert_matrix(&fields);
        }
    }
    println!(
        "vector_approximation_edges_available=true vector_approximation_edge_programs=4 vector_approximation_edge_cases=96"
    );
}
fn boundary_consumer(form: Form, boundary: bool, control: u32) -> Vec<u8> {
    let mut p = Program::new();
    p.reg(0, u64::from(control));
    p.save(0);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]);
    fill(&mut p, 16, words(merge()));
    copy(&mut p, true, 13, 16);
    if boundary {
        rip(&mut p.code, ENTRY, &[0x48, 0x8d, 0x05], 0x3ffc);
    } else {
        p.reg(0, 0);
    }
    approximation(&mut p, form, 9, 0, true, true);
    if boundary && form.scalar {
        copy(&mut p, false, 9, 96);
        check_result(&mut p, expected([0x4000_0000; 8], form, control, false));
        p.code.extend([0x0f, 0xae, 0x5c, 0x24, 0]);
        p.word(0, u64::from(control));
    }
    finish(p)
}
#[test]
fn actual_estimate_memory_widths_refuse_null_and_beyond_initialized_page_before_retirement() {
    let mut successes = 0;
    let mut refusals = 0;
    for form in forms() {
        for boundary in [false, true] {
            let original = super::native_packed_float::original(&boundary_consumer(
                form,
                boundary,
                controls()[0],
            ));
            let mut bytes = original.bytes.clone();
            put(&mut bytes, 0x3ffc, &0x4000_0000u32.to_le_bytes());
            let image = create_input(bytes);
            let (status, fields, text) = probe(&image);
            if boundary && form.scalar {
                assert_eq!(status, 0, "{text}");
                assert_eq!(fields["value"], "42");
                assert_eq!(fields["fp_status"], "4");
                successes += 1;
            } else {
                assert_eq!(status, 2, "{text}");
                assert_eq!(fields["code"], "native-dispatch");
                assert_eq!(fields["runtime_failure"], "memory-unmapped");
                assert_eq!(fields["service_stops"], "0");
                assert!(!fields.contains_key("value"));
                refusals += 1;
            }
        }
    }
    assert_eq!((successes, refusals), (2, 10));
    println!(
        "vector_approximation_bounds_available=true vector_approximation_boundary_programs=12 vector_approximation_boundary_successes=2 vector_approximation_boundary_refusals=10"
    );
}
