// SPDX-License-Identifier: GPL-2.0-only
//! Original unaligned reads distinguish checked native memory arithmetic from
//! the retained software fallback using exact instruction/byte counters.
use super::*;

const ENTRY: i32 = 0x1000;
fn pattern(register: u8) -> [u64; 4] {
    let tag = (u64::from(register) + 1) * 0x0101_0101_0101_0101;
    [
        0x0123_4567_89ab_cdef ^ tag,
        0xfedc_ba98_7654_3210 ^ tag,
        0x8877_6655_4433_2211 ^ tag,
        0x89ab_cdef_0123_4567 ^ tag,
    ]
}
fn repeated(single: u32, double: u64, is_double: bool) -> [u64; 4] {
    [if is_double {
        double
    } else {
        u64::from(single) * 0x1_0000_0001
    }; 4]
}
fn answer(opcode: u8, double: bool, narrow: bool) -> [u64; 4] {
    let (single, bits) = match opcode {
        0x58 => (0x4100_0000, 0x4020_0000_0000_0000),
        0x5c => (0x4080_0000, 0x4010_0000_0000_0000),
        _ => (0x4140_0000, 0x4028_0000_0000_0000),
    };
    let mut result = repeated(single, bits, double);
    if narrow {
        result[2..].fill(0);
    }
    result
}
fn vector_memory(p: &mut Program, load: bool, register: u8, offset: u8) {
    p.code.extend([
        0xc5,
        if register >= 8 { 0x7c } else { 0xfc },
        if load { 0x10 } else { 0x11 },
        0x44 | ((register & 7) << 3),
        0x24,
        offset,
    ]);
}
fn arithmetic(p: &mut Program, opcode: u8, double: bool, wide: bool) {
    let destination = if wide { 9 } else { 13 };
    // Logical source13; operand2 is an unaligned but complete [RSP+49] read.
    p.code.extend([
        0xc4,
        0x61,
        0x10 | u8::from(double) | (u8::from(wide) << 2),
        opcode,
        0x44 | ((destination & 7) << 3),
        0x24,
        49,
    ]);
}
fn fill(p: &mut Program, offset: u8, words: [u64; 4]) {
    for (word, bits) in words.into_iter().enumerate() {
        p.reg(0, bits);
        p.save(offset + word as u8 * 8);
    }
}
fn check_vector(p: &mut Program, register: u8, words: [u64; 4]) {
    vector_memory(p, false, register, 80);
    for (word, bits) in words.into_iter().enumerate() {
        p.word(80 + word as u8 * 8, bits);
    }
}
fn snapshot_word(p: &mut Program, offset: u32, bits: u64) {
    p.code.extend([0x48, 0x8b, 0x84, 0x24]);
    p.code.extend(offset.to_le_bytes());
    p.expect(bits);
}
fn consumer(iterations: u64, all_forms: bool, snapshot_before_narrow: bool) -> Vec<u8> {
    let mut p = Program::new();
    p.stack(0x89, 4, 8);
    p.reg(12, 0x1020_3040_5060_7080);
    p.reg(0, 0x5f84);
    p.save(0);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]);
    for register in 0..16 {
        fill(&mut p, 16, pattern(register));
        vector_memory(&mut p, true, register, 16);
    }
    let forms = [
        (0x58, false),
        (0x58, true),
        (0x5c, false),
        (0x5c, true),
        (0x59, false),
        (0x59, true),
    ];
    let selected = if all_forms { &forms[..] } else { &forms[..1] };
    for &(opcode, double) in selected {
        fill(
            &mut p,
            16,
            repeated(0x40c0_0000, 0x4018_0000_0000_0000, double),
        );
        fill(
            &mut p,
            49,
            repeated(0x4000_0000, 0x4000_0000_0000_0000, double),
        );
        p.reg(1, iterations);
        let repeat = p.code.len();
        vector_memory(&mut p, true, 13, 16);
        if all_forms {
            p.reg(0, 1);
            p.reg(2, 2);
            p.code.extend([0x48, 0x39, 0xd0]);
        }
        arithmetic(&mut p, opcode, double, true);
        // This selects snapshot placement, not a modeled boundary: the current
        // temporal-copy profile may emit either snapshot natively.
        if snapshot_before_narrow {
            vector_memory(&mut p, false, 9, 112);
        }
        arithmetic(&mut p, opcode, double, false);
        if !snapshot_before_narrow {
            vector_memory(&mut p, false, 9, 112);
        }
        if all_forms {
            for failure in [0x83, 0x8b, 0x84, 0x89, 0x80] {
                p.code.extend([0x0f, failure, 0, 0, 0, 0]);
                p.branches.push(p.code.len() - 4);
            }
        }
        p.code.extend([0x48, 0xff, 0xc9]);
        let relative = repeat as i32 - (p.code.len() as i32 + 6);
        p.code.extend([0x0f, 0x85]);
        p.code.extend(relative.to_le_bytes());
        for (word, bits) in answer(opcode, double, false).into_iter().enumerate() {
            snapshot_word(&mut p, 112 + word as u32 * 8, bits);
        }
        // Check the whole caller source before readback overwrites the byte80
        // shared with the scratch readback span. No input is silently changed.
        for (word, bits) in repeated(0x4000_0000, 0x4000_0000_0000_0000, double)
            .into_iter()
            .enumerate()
        {
            p.word(49 + word as u8 * 8, bits);
        }
        check_vector(&mut p, 9, answer(opcode, double, false));
        check_vector(&mut p, 13, answer(opcode, double, true));
    }
    rip(&mut p.code, ENTRY, &[0xff, 0x15], 0xf00);
    p.calls += 1;
    p.expect(1);
    let &(opcode, double) = selected.last().unwrap();
    for register in 0..16 {
        check_vector(
            &mut p,
            register,
            match register {
                9 => answer(opcode, double, false),
                13 => answer(opcode, double, true),
                _ => pattern(register),
            },
        );
    }
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 0]);
    p.word(0, 0x5f84);
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
fn check_counts(
    fields: &HashMap<String, String>,
    iterations: u64,
    profiles: u64,
    native: bool,
    native_copies: bool,
) {
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "1");
    assert_eq!(fields["fp_status"], "4");
    let copies = if native_copies {
        32 + 2 * profiles + 2 * profiles * iterations
    } else {
        0
    };
    assert_eq!(
        count(fields, "vector_ops"),
        34 + 2 * profiles + profiles * iterations * (if native { 2 } else { 4 }) - copies
    );
    assert_eq!(
        count(fields, "vector_bytes"),
        1032 + 64 * profiles + profiles * iterations * (if native { 64 } else { 112 })
            - copies * 32
    );
    assert_eq!(count(fields, "integer_ops"), 0);
}

#[test]
fn native_packed_float_memory_family_proves_unaligned_spans_context_and_real_counts() {
    let mut results = Vec::new();
    for iterations in [1, 3] {
        let input = super::native_packed_float::original(&consumer(iterations, true, true));
        let (status, fields, text) = probe(&input);
        assert_eq!(status, 0, "{text}");
        check_counts(&fields, iterations, 6, admitted(&fields), admitted(&fields));
        results.push(fields);
    }
    assert_eq!(results[0]["native_ymm_mode"], results[1]["native_ymm_mode"]);
    let native = admitted(&results[0]);
    for key in ["native_instructions", "primary_native_instructions"] {
        assert_eq!(
            count(&results[1], key) - count(&results[0], key),
            if native { 96 } else { 48 }
        );
    }
    assert_eq!(
        count(&results[1], "vector_ops") - count(&results[0], "vector_ops"),
        if native { 0 } else { 48 }
    );
    println!(
        "native_packed_float_memory_worker_mode={} native_packed_float_memory_worker_forms=12 native_packed_float_memory_worker_programs=2",
        results[0]["native_ymm_mode"]
    );
}

#[test]
#[ignore = "finite original comparison; requires explicitly selected independently verified0324 baseline and candidate"]
fn native_packed_float_memory_original_baseline_measurement() {
    let baseline = std::env::var_os("PENTAX_PACKED_FLOAT_MEMORY_BASELINE_WORKER")
        .expect("explicit memory baseline is required");
    let candidate = std::env::var_os("PENTAX_TEST_WORKER").expect("explicit candidate is required");
    assert!(baseline != candidate, "distinct worker artifacts required");
    const ITERATIONS: u64 = 50_000;
    for snapshot_before_narrow in [true, false] {
        let input = super::native_packed_float::original(&consumer(
            ITERATIONS,
            false,
            snapshot_before_narrow,
        ));
        let (before, before_ns) = super::native_packed_float::measured(&baseline, &input);
        let (after, after_ns) = super::native_packed_float::measured(&candidate, &input);
        assert_eq!(before["native_ymm_mode"], after["native_ymm_mode"]);
        // This verified 0.32.4 baseline has neither memory-FP nor temporal-copy
        // promotion; its AVX bridge label alone cannot identify those features.
        check_counts(&before, ITERATIONS, 1, false, false);
        check_counts(&after, ITERATIONS, 1, admitted(&after), admitted(&after));
        let promoted = if admitted(&after) {
            // P=2N memory FP operations; C=34+2N temporal copies.
            2 * ITERATIONS + 34 + 2 * ITERATIONS
        } else {
            0
        };
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
            if admitted(&after) {
                1088 + 112 * ITERATIONS
            } else {
                0
            }
        );
        println!(
            "native_packed_float_memory_measurement_snapshot_before_narrow={snapshot_before_narrow} native_packed_float_memory_measurement_iterations={ITERATIONS} native_packed_float_memory_measurement_mode={} baseline_elapsed_ns={before_ns} candidate_elapsed_ns={after_ns} baseline_entries={} candidate_entries={} baseline_blocks={} candidate_blocks={} baseline_work={} candidate_work={} baseline_publications={} candidate_publications={} baseline_reuses={} candidate_reuses={}",
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
}
