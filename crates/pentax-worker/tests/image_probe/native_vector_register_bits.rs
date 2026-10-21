// SPDX-License-Identifier: GPL-2.0-only
//! Original raw-bit consumers distinguish real native promotion from mediation.
//! All memory setup/readback remains checked; no floating arithmetic is used.
use super::*;

const ENTRY: i32 = 0x1000;
const FORMS: [(u8, u8); 16] = [
    (0x10, 0),
    (0x10, 1),
    (0x11, 0),
    (0x11, 1),
    (0x28, 0),
    (0x28, 1),
    (0x29, 0),
    (0x29, 1),
    (0x54, 0),
    (0x54, 1),
    (0x55, 0),
    (0x55, 1),
    (0x56, 0),
    (0x56, 1),
    (0x57, 0),
    (0x57, 1),
];
fn pattern(register: u8) -> [u64; 4] {
    match register {
        13 => [
            0x7fa0_0031_8000_0000,
            0x7ff0_0000_0000_0031,
            0xffff_ffff_0000_0000,
            0x0123_4567_89ab_cdef,
        ],
        14 => [
            0xffc0_0072_0000_0000,
            0xfff8_0000_0000_0072,
            0x5555_aaaa_ffff_0000,
            0xfedc_ba98_7654_3210,
        ],
        _ => {
            let tag = (u64::from(register) + 1) * 0x0101_0101_0101_0101;
            [
                0x0123_4567_89ab_cdef ^ tag,
                0xfedc_ba98_7654_3210 ^ tag,
                0x8877_6655_4433_2211 ^ tag,
                0x89ab_cdef_0123_4567 ^ tag,
            ]
        }
    }
}
fn answer(opcode: u8, wide: bool) -> [u64; 4] {
    let a = pattern(13);
    let b = pattern(14);
    std::array::from_fn(|word| {
        if !wide && word >= 2 {
            0
        } else {
            match opcode {
                0x54 => a[word] & b[word],
                0x55 => !a[word] & b[word],
                0x56 => a[word] | b[word],
                0x57 => a[word] ^ b[word],
                _ => a[word],
            }
        }
    })
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
fn bits(p: &mut Program, opcode: u8, pp: u8, wide: bool, destination: u8) {
    let copy = opcode < 0x50;
    let (reg, rm) = if matches!(opcode, 0x11 | 0x29) {
        (13, destination)
    } else {
        (destination, if copy { 13 } else { 14 })
    };
    p.code.extend([
        0xc4,
        0xe1 ^ (u8::from(reg >= 8) << 7) ^ (u8::from(rm >= 8) << 5),
        (if copy { 0x78 } else { 0x10 }) | (u8::from(wide) << 2) | pp,
        opcode,
        0xc0 | ((reg & 7) << 3) | (rm & 7),
    ]);
}
fn fill(p: &mut Program, offset: u8, words: [u64; 4]) {
    for (word, value) in words.into_iter().enumerate() {
        p.reg(0, value);
        p.save(offset + word as u8 * 8);
    }
}
fn check_result(p: &mut Program, words: [u64; 4]) {
    for (word, value) in words.into_iter().enumerate() {
        p.word(80 + word as u8 * 8, value);
    }
}
fn consumer(iterations: u64, all_forms: bool, snapshot_each_operation: bool) -> Vec<u8> {
    let mut p = Program::new();
    p.stack(0x89, 4, 8);
    p.reg(12, 0x1020_3040_5060_7080);
    p.reg(0, 0x5fa5);
    p.save(0);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]);
    let mut wanted = std::array::from_fn::<_, 16, _>(|register| pattern(register as u8));
    for register in 0..16 {
        fill(&mut p, 16, pattern(register));
        vector_memory(&mut p, true, register, 16);
    }
    fill(&mut p, 16, pattern(13));
    fill(&mut p, 48, pattern(14));
    let mut profile = 0;
    for &(opcode, pp) in if all_forms { &FORMS[..] } else { &FORMS[..1] } {
        for wide in if all_forms {
            &[false, true][..]
        } else {
            &[true][..]
        } {
            let destination = [9, 13, 14][profile % 3];
            let wide = *wide;
            p.reg(1, iterations);
            let repeat = p.code.len();
            vector_memory(&mut p, true, 13, 16);
            vector_memory(&mut p, true, 14, 48);
            if all_forms {
                p.reg(0, 1);
                p.reg(2, 2);
                p.code.extend([0x48, 0x39, 0xd0]);
            }
            for _ in 0..if all_forms { 1 } else { 4 } {
                bits(&mut p, opcode, pp, wide, destination);
                // Snapshot count is independent of mediation. Current checked
                // temporal stores can coalesce with the native raw-bit body.
                if snapshot_each_operation {
                    vector_memory(&mut p, false, destination, 80);
                }
            }
            if !snapshot_each_operation {
                vector_memory(&mut p, false, destination, 80);
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
            check_result(&mut p, answer(opcode, wide));
            wanted[13] = pattern(13);
            wanted[14] = pattern(14);
            wanted[destination as usize] = answer(opcode, wide);
            profile += 1;
        }
    }
    rip(&mut p.code, ENTRY, &[0xff, 0x15], 0xf00);
    p.calls += 1;
    p.expect(1);
    for (register, words) in wanted.into_iter().enumerate() {
        vector_memory(&mut p, false, register as u8, 80);
        check_result(&mut p, words);
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
fn check_counts(
    fields: &HashMap<String, String>,
    iterations: u64,
    all_forms: bool,
    snapshot_each_operation: bool,
    native: bool,
    native_copies: bool,
) {
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "1");
    assert_eq!(fields["fp_status"], "37");
    assert_eq!(fields["integer_ops"], "0");
    let profiles = if all_forms { 32 } else { 1 };
    let operations = if all_forms { 1 } else { 4 };
    let stores = if snapshot_each_operation {
        operations
    } else {
        1
    };
    let copies = if native_copies {
        32 + profiles * iterations * (2 + stores)
    } else {
        0
    };
    assert_eq!(
        count(fields, "vector_ops"),
        34 + profiles * iterations * (2 + stores + if native { 0 } else { operations }) - copies
    );
    assert_eq!(
        count(fields, "vector_bytes"),
        1032 + profiles * iterations * (64 + stores * 32) - copies * 32
    );
}

#[test]
fn native_vector_register_bits_family_proves_raw_bits_aliases_context_and_real_counts() {
    let mut results = Vec::new();
    for iterations in [1, 3] {
        let image = super::native_packed_float::original(&consumer(iterations, true, true));
        let (status, fields, text) = probe(&image);
        assert_eq!(status, 0, "{text}");
        check_counts(
            &fields,
            iterations,
            true,
            true,
            admitted(&fields),
            admitted(&fields),
        );
        results.push(fields);
    }
    assert_eq!(results[0]["native_ymm_mode"], results[1]["native_ymm_mode"]);
    let native = admitted(&results[0]);
    for key in ["native_instructions", "primary_native_instructions"] {
        assert_eq!(
            count(&results[1], key) - count(&results[0], key),
            if native { 512 } else { 256 }
        );
    }
    assert_eq!(
        count(&results[1], "vector_ops") - count(&results[0], "vector_ops"),
        if native { 0 } else { 256 }
    );
    println!(
        "native_vector_register_bits_worker_mode={} native_vector_register_bits_worker_forms=32 native_vector_register_bits_worker_programs=2",
        results[0]["native_ymm_mode"]
    );
}

#[test]
#[ignore = "finite original comparison; requires explicitly selected independently verified0325 baseline and candidate"]
fn native_vector_register_bits_original_baseline_measurement() {
    let baseline = std::env::var_os("PENTAX_VECTOR_REGISTER_BITS_BASELINE_WORKER")
        .expect("explicit0325 baseline required");
    let candidate = std::env::var_os("PENTAX_TEST_WORKER").expect("explicit candidate required");
    assert!(baseline != candidate, "distinct worker artifacts required");
    const ITERATIONS: u64 = 50_000;
    for snapshot_each_operation in [true, false] {
        let image = super::native_packed_float::original(&consumer(
            ITERATIONS,
            false,
            snapshot_each_operation,
        ));
        let (before, before_ns) = super::native_packed_float::measured(&baseline, &image);
        let (after, after_ns) = super::native_packed_float::measured(&candidate, &image);
        assert_eq!(before["native_ymm_mode"], after["native_ymm_mode"]);
        // This verified 0.32.5 baseline predates the 32 register-bit forms and
        // temporal copies, even when its immutable bridge mode is also AVX.
        check_counts(
            &before,
            ITERATIONS,
            false,
            snapshot_each_operation,
            false,
            false,
        );
        check_counts(
            &after,
            ITERATIONS,
            false,
            snapshot_each_operation,
            admitted(&after),
            admitted(&after),
        );
        let stores = if snapshot_each_operation { 4 } else { 1 };
        let copies = if admitted(&after) {
            32 + (2 + stores) * ITERATIONS
        } else {
            0
        };
        let promoted = if admitted(&after) {
            4 * ITERATIONS + copies
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
            copies * 32
        );
        println!(
            "native_vector_register_bits_measurement_snapshot_each_operation={snapshot_each_operation} native_vector_register_bits_measurement_iterations={ITERATIONS} native_vector_register_bits_measurement_mode={} baseline_elapsed_ns={before_ns} candidate_elapsed_ns={after_ns} baseline_entries={} candidate_entries={} baseline_blocks={} candidate_blocks={} baseline_work={} candidate_work={} baseline_publications={} candidate_publications={} baseline_reuses={} candidate_reuses={}",
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
