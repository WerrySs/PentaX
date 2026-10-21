// SPDX-License-Identifier: GPL-2.0-only
//! Original scalar consumers retain real native-to-modeled boundaries and a
//! complete service-boundary vector snapshot. Benchmark artifacts are explicit.
use super::*;

const ENTRY: i32 = 0x1000;
const FORMS: [(u8, bool); 6] = [
    (0x58, false),
    (0x58, true),
    (0x5c, false),
    (0x5c, true),
    (0x59, false),
    (0x59, true),
];
fn pattern(register: u8) -> [u64; 4] {
    let tag = (u64::from(register) + 1) * 0x0101_0101_0101_0101;
    [
        0x0123_4567_89ab_cdef ^ tag,
        0xfedc_ba98_7654_3210 ^ tag,
        0x8877_6655_4433_2211 ^ tag,
        0x89ab_cdef_0123_4567 ^ tag,
    ]
}
fn operand(double: bool, right: bool) -> [u64; 4] {
    let mut words = [
        0x7fa0_1234_40c0_0000,
        0x7ff0_0000_0000_0031,
        0x0123_4567_89ab_cdef,
        0xfedc_ba98_7654_3210,
    ];
    words[0] = match (double, right) {
        (false, false) => 0x7fa0_1234_40c0_0000,
        (false, true) => 0x7fa0_5678_4000_0000,
        (true, false) => 0x4018_0000_0000_0000,
        (true, true) => 0x4000_0000_0000_0000,
    };
    words
}
fn result(opcode: u8, double: bool) -> [u64; 4] {
    let mut words = operand(double, false);
    let (single, wide) = match opcode {
        0x58 => (0x4100_0000, 0x4020_0000_0000_0000), // 6+2=8
        0x5c => (0x4080_0000, 0x4010_0000_0000_0000), // 6-2=4
        _ => (0x4140_0000, 0x4028_0000_0000_0000),    // 6*2=12
    };
    words[0] = if double {
        wide
    } else {
        (words[0] & !0xffff_ffff) | single
    };
    words[2..].fill(0);
    words
}
fn fill(p: &mut Program, offset: u8, words: [u64; 4]) {
    for (word, value) in words.into_iter().enumerate() {
        p.reg(0, value);
        p.save(offset + word as u8 * 8);
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
fn arithmetic(p: &mut Program, opcode: u8, double: bool, memory: bool) {
    let destination = if memory { 14 } else { 9 };
    // Source1 XMM13; source2 XMM14 or exact scalar [RSP+48]. Both preserve
    // source1's inactive low128 signaling-NaN bits and zero the upper128.
    p.code.extend([
        0xc4,
        if memory { 0x61 } else { 0x41 },
        0x12 | u8::from(double),
        opcode,
        ((destination & 7) << 3) | if memory { 0x44 } else { 0xc6 },
    ]);
    if memory {
        p.code.extend([0x24, 48]);
    }
}
fn check_vector(p: &mut Program, register: u8, words: [u64; 4]) {
    copy(p, false, register, 80);
    for (word, value) in words.into_iter().enumerate() {
        p.word(80 + word as u8 * 8, value);
    }
}
fn consumer(iterations: u64, all: bool) -> Vec<u8> {
    let mut p = Program::new();
    p.stack(0x89, 4, 8);
    p.reg(12, 0x5678_1234_90ab_cdef);
    p.reg(0, 0x5f84); // All exceptions masked; sticky ZE, round toward +infinity.
    p.save(0);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]);
    for register in 0..16 {
        fill(&mut p, 16, pattern(register));
        copy(&mut p, true, register, 16);
    }
    let sampled = [(0x59, false)];
    let forms = if all { &FORMS[..] } else { &sampled[..] };
    for &(opcode, double) in forms {
        fill(&mut p, 16, operand(double, false));
        fill(&mut p, 48, operand(double, true));
        p.reg(1, iterations);
        let repeat = p.code.len();
        copy(&mut p, true, 13, 16);
        copy(&mut p, true, 14, 48);
        p.reg(0, 1);
        p.reg(2, 2);
        p.code.extend([0x48, 0x39, 0xd0]);
        arithmetic(&mut p, opcode, double, false);
        copy(&mut p, false, 9, 112);
        // Still-excluded VMOVQ is an actual mediated boundary in both worker
        // artifacts, immediately after the full scalar-result snapshot.
        p.code.extend([0xc5, 0x79, 0xd6, 0x4c, 0x24, 112]);
        arithmetic(&mut p, opcode, double, true);
        for failure in [0x83, 0x8b, 0x84, 0x89, 0x80] {
            p.code.extend([0x0f, failure, 0, 0, 0, 0]);
            p.branches.push(p.code.len() - 4);
        }
        p.code.extend([0x48, 0xff, 0xc9]);
        let relative = repeat as i32 - (p.code.len() as i32 + 6);
        p.code.extend([0x0f, 0x85]);
        p.code.extend(relative.to_le_bytes());
        for (word, value) in result(opcode, double).into_iter().enumerate() {
            // Snapshot spans112..143: use a real disp32 above signed disp8.
            p.code.extend([0x48, 0x8b, 0x84, 0x24]);
            p.code.extend((112 + word as u32 * 8).to_le_bytes());
            p.expect(value);
        }
        check_vector(&mut p, 9, result(opcode, double));
        check_vector(&mut p, 14, result(opcode, double));
        check_vector(&mut p, 13, operand(double, false));
    }
    rip(&mut p.code, ENTRY, &[0xff, 0x15], 0xf00);
    p.calls += 1;
    p.expect(1);
    let &(opcode, double) = forms.last().unwrap();
    for register in 0..16 {
        let words = match register {
            9 | 14 => result(opcode, double),
            13 => operand(double, false),
            _ => pattern(register),
        };
        check_vector(&mut p, register, words);
    }
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 0]);
    p.word(0, 0x5f84);
    p.code.extend([0x48, 0x89, 0xe0]);
    p.load(10, 8);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x4c, 0x89, 0xe0]);
    p.expect(0x5678_1234_90ab_cdef);
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0x3000);
    p.code
}
fn count(fields: &HashMap<String, String>, key: &str) -> u64 {
    fields[key].parse().unwrap()
}
fn admitted(fields: &HashMap<String, String>) -> bool {
    match fields["native_ymm_mode"].as_str() {
        "avx" | "avx2" => true,
        "sse2" => false,
        _ => panic!("unknown closed native mode"),
    }
}
fn check_counts(
    fields: &HashMap<String, String>,
    iterations: u64,
    all: bool,
    copies: bool,
    scalar: bool,
) {
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "1");
    assert_eq!(fields["fp_status"], "4");
    assert_eq!(count(fields, "integer_ops"), 0);
    let profiles = if all { 6 } else { 1 };
    let widths = if all { 36 } else { 4 };
    let operations = if copies {
        2 + profiles * iterations * if scalar { 1 } else { 3 }
    } else {
        34 + 3 * profiles + 6 * profiles * iterations
    };
    let bytes = if copies {
        8 + (8 * profiles + if scalar { 0 } else { widths }) * iterations
    } else {
        1032 + 96 * profiles + (104 * profiles + widths) * iterations
    };
    assert_eq!(count(fields, "vector_ops"), operations);
    assert_eq!(count(fields, "vector_bytes"), bytes);
}

#[test]
fn native_scalar_float_family_preserves_merge_flags_host_boundaries_and_real_counts() {
    let mut results = Vec::new();
    for iterations in [1, 3] {
        let input = super::native_packed_float::original(&consumer(iterations, true));
        let (status, fields, text) = probe(&input);
        assert_eq!(status, 0, "{text}");
        let native = admitted(&fields);
        check_counts(&fields, iterations, true, native, native);
        results.push(fields);
    }
    assert_eq!(results[0]["native_ymm_mode"], results[1]["native_ymm_mode"]);
    let native = admitted(&results[0]);
    for key in ["native_instructions", "primary_native_instructions"] {
        // Six profiles × two iterations: four GPR instructions plus three
        // copies and two scalar arithmetic bodies when actually admitted.
        assert_eq!(
            count(&results[1], key) - count(&results[0], key),
            if native { 108 } else { 48 }
        );
    }
    assert_eq!(
        count(&results[1], "vector_ops") - count(&results[0], "vector_ops"),
        if native { 12 } else { 72 }
    );
    println!(
        "native_scalar_float_worker_mode={} native_scalar_float_worker_forms=12 native_scalar_float_worker_programs=2",
        results[0]["native_ymm_mode"]
    );
}

#[test]
#[ignore = "finite original comparison; requires explicit independently verified0328 baseline and candidate"]
fn native_scalar_float_original_baseline_measurement() {
    let baseline = std::env::var_os("PENTAX_SCALAR_FLOAT_BASELINE_WORKER")
        .expect("explicit0328 baseline required");
    let candidate = std::env::var_os("PENTAX_TEST_WORKER").expect("explicit candidate required");
    assert!(baseline != candidate);
    const ITERATIONS: u64 = 50_000;
    let input = super::native_packed_float::original(&consumer(ITERATIONS, false));
    let (before, before_ns) = super::native_packed_float::measured(&baseline, &input);
    let (after, after_ns) = super::native_packed_float::measured(&candidate, &input);
    assert_eq!(before["native_ymm_mode"], after["native_ymm_mode"]);
    let native = admitted(&after);
    check_counts(&before, ITERATIONS, false, native, false);
    check_counts(&after, ITERATIONS, false, native, native);
    let promoted = if native { 2 * ITERATIONS } else { 0 };
    assert_eq!(
        count(&after, "native_instructions") - count(&before, "native_instructions"),
        promoted
    );
    assert_eq!(
        count(&before, "vector_ops") - count(&after, "vector_ops"),
        promoted
    );
    assert_eq!(
        count(&before, "vector_bytes") - count(&after, "vector_bytes"),
        if native { 4 * ITERATIONS } else { 0 }
    );
    println!(
        "native_scalar_float_measurement_iterations={ITERATIONS} native_scalar_float_measurement_mode={} baseline_elapsed_ns={before_ns} candidate_elapsed_ns={after_ns} baseline_entries={} candidate_entries={} baseline_blocks={} candidate_blocks={} baseline_work={} candidate_work={} baseline_publications={} candidate_publications={} baseline_reuses={} candidate_reuses={}",
        after["native_ymm_mode"],
        before["native_entries"],
        after["native_entries"],
        before["block_attempts"],
        after["block_attempts"],
        before["dispatch_work"],
        after["dispatch_work"],
        before["cache_publications"],
        after["cache_publications"],
        before["cache_reuses"],
        after["cache_reuses"]
    );
}
