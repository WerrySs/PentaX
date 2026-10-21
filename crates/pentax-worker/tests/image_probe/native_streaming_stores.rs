// SPDX-License-Identifier: GPL-2.0-only
//! Original NT stores followed immediately by a Rust-mediated vector read.
//! No guest fence conceals the native-to-Rust ordering requirement.
use super::*;

const ENTRY: i32 = 0x1000;
const CANARY: u64 = 0xa5a5_5a5a_c3c3_3c3c;

fn pattern(register: u8) -> [u64; 4] {
    let tag = (u64::from(register) + 1) * 0x0101_0101_0101_0101;
    [
        0x7fa0_0031_8000_0000 ^ tag,
        0x7ff0_0000_0000_0031 ^ tag,
        0x8877_6655_4433_2211 ^ tag,
        0xfedc_ba98_7654_3210 ^ tag,
    ]
}

fn stack_vector(p: &mut Program, load: bool, register: u8) {
    p.code.extend([
        0xc5,
        if register >= 8 { 0x7c } else { 0xfc },
        if load { 0x10 } else { 0x11 },
        0x44 | ((register & 7) << 3),
        0x24,
        if load { 16 } else { 80 },
    ]);
}

fn buffer_word(p: &mut Program, offset: u8, value: u64) {
    p.code.extend([0x49, 0x8b, 0x45, offset]); // RAX <- [R13+disp8].
    p.expect(value);
}

fn consumer(iterations: u64, all_registers: bool) -> Vec<u8> {
    let mut p = Program::new();
    p.stack(0x89, 4, 8);
    p.reg(12, 0x1020_3040_5060_7080);
    p.ptr(13, 112);
    p.code.extend([0x49, 0x83, 0xe5, 0xe0]); // R13 is a private 32-byte-aligned buffer.
    p.reg(0, 0x5fa5);
    p.save(0);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]);
    for register in 0..16 {
        for (word, value) in pattern(register).into_iter().enumerate() {
            p.reg(0, value);
            p.save(16 + word as u8 * 8);
        }
        stack_vector(&mut p, true, register);
    }
    let registers: &[u8] = if all_registers {
        &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
    } else {
        &[13]
    };
    for &register in registers {
        p.reg(0, CANARY);
        for offset in [0xf8, 0, 8, 16, 24, 32, 40, 48, 56, 64] {
            p.code.extend([0x49, 0x89, 0x45, offset]);
        }
        p.reg(1, iterations);
        let repeat = p.code.len();
        p.reg(0, 1);
        p.reg(2, 2);
        p.code.extend([0x48, 0x39, 0xd0]);
        for (wide, offset) in [(false, 0), (true, 32)] {
            // VEX.128 must not clear the upper half of its SOURCE: the next
            // VEX.256 store consumes those same upper bits immediately.
            p.code.extend([
                0xc4,
                if register >= 8 { 0x41 } else { 0xc1 },
                if wide { 0x7d } else { 0x79 },
                0xe7,
                0x45 | ((register & 7) << 3),
                offset,
            ]);
        }
        // This memory VMOVUPS remains modeled. It is the first Rust guest-data
        // read after the native stores; the source is reloaded with identical
        // literal bits, without a guest fence or service in between.
        p.code.extend([
            0xc4,
            if register >= 8 { 0x41 } else { 0xc1 },
            0x7c,
            0x10,
            0x45 | ((register & 7) << 3),
            32,
        ]);
        for failure in [0x83, 0x8b, 0x84, 0x89, 0x80] {
            p.code.extend([0x0f, failure, 0, 0, 0, 0]);
            p.branches.push(p.code.len() - 4);
        }
        p.code.extend([0x48, 0xff, 0xc9]);
        let relative = repeat as i32 - (p.code.len() as i32 + 6);
        p.code.extend([0x0f, 0x85]);
        p.code.extend(relative.to_le_bytes());
        let words = pattern(register);
        for (offset, value) in [
            (0xf8, CANARY),
            (0, words[0]),
            (8, words[1]),
            (16, CANARY),
            (24, CANARY),
            (32, words[0]),
            (40, words[1]),
            (48, words[2]),
            (56, words[3]),
            (64, CANARY),
        ] {
            buffer_word(&mut p, offset, value);
        }
    }
    rip(&mut p.code, ENTRY, &[0xff, 0x15], 0xf00);
    p.calls += 1;
    p.expect(1);
    for register in 0..16 {
        stack_vector(&mut p, false, register);
        for (word, value) in pattern(register).into_iter().enumerate() {
            p.word(80 + word as u8 * 8, value);
        }
    }
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 0]);
    p.word(0, 0x5fa5);
    p.code.extend([0x48, 0x89, 0xe0]);
    p.load(10, 8);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x4c, 0x89, 0xe0]);
    p.expect(0x1020_3040_5060_7080);
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

fn check_counts(fields: &HashMap<String, String>, iterations: u64, profiles: u64, native: bool) {
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "1");
    assert_eq!(fields["fp_status"], "37");
    assert_eq!(count(fields, "integer_ops"), 0);
    assert_eq!(
        count(fields, "vector_ops"),
        34 + profiles * iterations * if native { 1 } else { 3 }
    );
    assert_eq!(
        count(fields, "vector_bytes"),
        1032 + profiles * iterations * if native { 32 } else { 80 }
    );
}

#[test]
fn native_streaming_stores_publish_before_rust_reads_and_preserve_all_source_bits() {
    let mut results = Vec::new();
    for iterations in [1, 3] {
        let image = super::native_packed_float::original(&consumer(iterations, true));
        let (status, fields, text) = probe(&image);
        assert_eq!(status, 0, "{text}");
        check_counts(&fields, iterations, 16, admitted(&fields));
        results.push(fields);
    }
    assert_eq!(results[0]["native_ymm_mode"], results[1]["native_ymm_mode"]);
    let native = admitted(&results[0]);
    for key in ["native_instructions", "primary_native_instructions"] {
        assert_eq!(
            count(&results[1], key) - count(&results[0], key),
            if native { 192 } else { 128 }
        );
    }
    assert_eq!(
        count(&results[1], "vector_ops") - count(&results[0], "vector_ops"),
        if native { 32 } else { 96 }
    );
    println!(
        "native_streaming_stores_worker_mode={} native_streaming_stores_worker_forms=2 native_streaming_stores_worker_programs=2",
        results[0]["native_ymm_mode"]
    );
}

#[test]
#[ignore = "finite original comparison; requires explicitly selected independently verified0326 baseline and candidate"]
fn native_streaming_stores_original_baseline_measurement() {
    let baseline = std::env::var_os("PENTAX_STREAMING_STORES_BASELINE_WORKER")
        .expect("explicit0326 baseline required");
    let candidate = std::env::var_os("PENTAX_TEST_WORKER").expect("explicit candidate required");
    assert!(baseline != candidate, "distinct worker artifacts required");
    const ITERATIONS: u64 = 50_000;
    let image = super::native_packed_float::original(&consumer(ITERATIONS, false));
    let (before, before_ns) = super::native_packed_float::measured(&baseline, &image);
    let (after, after_ns) = super::native_packed_float::measured(&candidate, &image);
    assert_eq!(before["native_ymm_mode"], after["native_ymm_mode"]);
    check_counts(&before, ITERATIONS, 1, false);
    check_counts(&after, ITERATIONS, 1, admitted(&after));
    let promoted = if admitted(&after) { 2 * ITERATIONS } else { 0 };
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
        promoted * 24
    );
    println!(
        "native_streaming_stores_measurement_iterations={ITERATIONS} native_streaming_stores_measurement_mode={} baseline_elapsed_ns={before_ns} candidate_elapsed_ns={after_ns} baseline_entries={} candidate_entries={} baseline_blocks={} candidate_blocks={} baseline_work={} candidate_work={} baseline_publications={} candidate_publications={} baseline_reuses={} candidate_reuses={}",
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
