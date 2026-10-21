// SPDX-License-Identifier: GPL-2.0-only
//! Original temporal memory transfers cross an actual modeled mask boundary.
//! The finite baseline comparison uses the same input and bounded drain helper.
use super::*;

const ENTRY: i32 = 0x1000;
const CANARY: u64 = 0xa5a5_5a5a_c3c3_3c3c;
const MASK: u64 = 0x61;

#[derive(Clone, Copy)]
struct Transfer {
    opcode: u8,
    pp: u8,
    width: usize,
    wide: bool,
}
impl Transfer {
    fn store(self) -> bool {
        matches!(self.opcode, 0x7f | 0x29 | 0x11)
    }
}
#[derive(Clone, Copy)]
enum Selection {
    First,
    Second,
    SampledStore,
}
impl Selection {
    fn range(self) -> std::ops::Range<usize> {
        match self {
            Self::First => 0..14,
            Self::Second => 14..28,
            Self::SampledStore => 2..3,
        }
    }
    // Independent literal partition counts, not the measured runtime's output.
    fn layout(self) -> (u64, u64, u64) {
        match self {
            Self::First => (8, 6, 336),
            Self::Second => (6, 8, 264),
            Self::SampledStore => (0, 1, 16),
        }
    }
}
fn transfers() -> Vec<Transfer> {
    let mut result = Vec::new();
    for (opcode, pp, scalar) in [
        (0x6f, 1, 0),
        (0x7f, 1, 0),
        (0x6f, 2, 0),
        (0x7f, 2, 0),
        (0x28, 0, 0),
        (0x29, 0, 0),
        (0x10, 0, 0),
        (0x11, 0, 0),
        (0x28, 1, 0),
        (0x29, 1, 0),
        (0x10, 1, 0),
        (0x11, 1, 0),
        (0x10, 2, 4),
        (0x11, 2, 4),
        (0x10, 3, 8),
        (0x11, 3, 8),
    ] {
        for wide in [false, true] {
            if scalar != 0 && wide {
                continue;
            }
            result.push(Transfer {
                opcode,
                pp,
                width: if scalar != 0 {
                    scalar
                } else if wide {
                    32
                } else {
                    16
                },
                wide,
            });
        }
    }
    assert_eq!(result.len(), 28);
    result
}
fn pattern(register: u8) -> [u64; 4] {
    if register == 13 {
        // Both the low32 and the low64 are signaling NaNs. The mask oracle
        // sees negative32-bit lanes0/5/6; no copy may quiet or classify them.
        return [
            0x7ff0_0001_ffa0_0031,
            0x7ff0_0000_0000_0031,
            0xffff_ffff_0000_0000,
            0x0123_4567_89ab_cdef,
        ];
    }
    let tag = (u64::from(register) + 1) * 0x0101_0101_0101_0101;
    [
        0x0123_4567_89ab_cdef ^ tag,
        0xfedc_ba98_7654_3210 ^ tag,
        0x8877_6655_4433_2211 ^ tag,
        0x89ab_cdef_0123_4567 ^ tag,
    ]
}
fn fill_stack(p: &mut Program, words: [u64; 4]) {
    for (word, value) in words.into_iter().enumerate() {
        p.reg(0, value);
        p.save(16 + word as u8 * 8);
    }
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
fn buffer_store(p: &mut Program, offset: u8, value: u64) {
    p.reg(0, value);
    p.code.extend([0x49, 0x89, 0x45, offset]);
}
fn buffer_word(p: &mut Program, offset: u8, value: u64) {
    p.code.extend([0x49, 0x8b, 0x45, offset]);
    p.expect(value);
}
fn check_vector(p: &mut Program, register: u8, words: [u64; 4]) {
    stack_vector(p, false, register);
    for (word, value) in words.into_iter().enumerate() {
        p.word(80 + word as u8 * 8, value);
    }
}
fn expected(width: usize, load: bool) -> [u64; 4] {
    let source = pattern(13);
    std::array::from_fn(|word| {
        if word * 8 >= width {
            if load { 0 } else { CANARY }
        } else if width == 4 {
            (if load { 0 } else { CANARY & !0xffff_ffff }) | (source[0] & 0xffff_ffff)
        } else {
            source[word]
        }
    })
}
fn transfer(p: &mut Program, form: Transfer) {
    let register = if form.store() { 13 } else { 9 };
    // ModRM.reg is the sole vector register in both opcode directions. R13
    // is an independent GPR base; vvvv stays reserved and L0 scalar is used.
    p.code.extend([
        0xc4,
        0x41,
        0x78 | form.pp | (u8::from(form.wide) << 2),
        form.opcode,
        0x45 | ((register & 7) << 3),
        0,
    ]);
}
fn consumer(iterations: u64, selection: Selection) -> Vec<u8> {
    let mut p = Program::new();
    p.stack(0x89, 4, 8);
    p.reg(12, 0x1020_3040_5060_7080);
    p.ptr(13, 112);
    p.code.extend([0x49, 0x83, 0xe5, 0xe0]);
    p.reg(0, 0x5fa5);
    p.save(0);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]);
    for register in 0..16 {
        fill_stack(&mut p, pattern(register));
        stack_vector(&mut p, true, register);
    }
    // The constant is disjoint from the aligned target and every snapshot.
    fill_stack(&mut p, pattern(9));
    let forms = transfers();
    let selected = &forms[selection.range()];
    let (loads, stores, bytes) = selection.layout();
    assert_eq!(
        selected.iter().filter(|form| !form.store()).count() as u64,
        loads
    );
    assert_eq!(
        selected.iter().filter(|form| form.store()).count() as u64,
        stores
    );
    assert_eq!(
        selected.iter().map(|form| form.width as u64).sum::<u64>(),
        bytes
    );
    for &form in selected {
        buffer_store(&mut p, 0xf8, CANARY);
        buffer_store(&mut p, 32, CANARY);
        for (word, value) in (if form.store() {
            [CANARY; 4]
        } else {
            pattern(13)
        })
        .into_iter()
        .enumerate()
        {
            buffer_store(&mut p, word as u8 * 8, value);
        }
        p.reg(1, iterations);
        let repeat = p.code.len();
        p.reg(0, 1);
        p.reg(2, 2);
        p.code.extend([0x48, 0x39, 0xd0]);
        transfer(&mut p, form);
        // The exact VMOVMSKPS GPR form remains modeled in both profiles.
        // It observes the unchanged complete source13 and forces return from
        // the preceding native memory body before the next native work.
        p.code.extend([0xc4, 0xc1, 0x7c, 0x50, 0xc5]);
        for failure in [0x83, 0x8b, 0x84, 0x89, 0x80] {
            p.code.extend([0x0f, failure, 0, 0, 0, 0]);
            p.branches.push(p.code.len() - 4);
        }
        p.code.extend([0x48, 0xff, 0xc9]);
        let relative = repeat as i32 - (p.code.len() as i32 + 6);
        p.code.extend([0x0f, 0x85]);
        p.code.extend(relative.to_le_bytes());
        p.expect(MASK);
        // The readback atRSP+80 can overlap this old target after its final use.
        // Check every input/output/canary byte BEFORE taking that snapshot.
        buffer_word(&mut p, 0xf8, CANARY);
        buffer_word(&mut p, 32, CANARY);
        let buffer = if form.store() {
            expected(form.width, false)
        } else {
            pattern(13)
        };
        for (word, value) in buffer.into_iter().enumerate() {
            buffer_word(&mut p, word as u8 * 8, value);
        }
        if form.store() {
            check_vector(&mut p, 13, pattern(13));
        } else {
            check_vector(&mut p, 9, expected(form.width, true));
            stack_vector(&mut p, true, 9);
        }
    }
    rip(&mut p.code, ENTRY, &[0xff, 0x15], 0xf00);
    p.calls += 1;
    p.expect(1);
    for register in 0..16 {
        check_vector(&mut p, register, pattern(register));
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
    selection: Selection,
    native: bool,
) {
    let (loads, stores, width_sum) = selection.layout();
    let profiles = loads + stores;
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "1");
    assert_eq!(fields["fp_status"], "37");
    assert_eq!(count(fields, "integer_ops"), 0);
    let (operations, bytes) = if native {
        (2 + profiles * iterations, 8)
    } else {
        (
            34 + 2 * loads + stores + 2 * profiles * iterations,
            1032 + 64 * loads + 32 * stores + width_sum * iterations,
        )
    };
    assert_eq!(count(fields, "vector_ops"), operations);
    assert_eq!(count(fields, "vector_bytes"), bytes);
}

#[test]
fn native_vector_memory_moves_preserve_raw_bits_and_real_counts() {
    let mut mode = None;
    for selection in [Selection::First, Selection::Second] {
        let mut results = Vec::new();
        for iterations in [1, 3] {
            let image = super::native_packed_float::original(&consumer(iterations, selection));
            let (status, fields, text) = probe(&image);
            assert_eq!(status, 0, "{text}");
            check_counts(&fields, iterations, selection, admitted(&fields));
            results.push(fields);
        }
        assert_eq!(results[0]["native_ymm_mode"], results[1]["native_ymm_mode"]);
        if let Some(previous) = &mode {
            assert_eq!(previous, &results[0]["native_ymm_mode"]);
        } else {
            mode = Some(results[0]["native_ymm_mode"].clone());
        }
        let native = admitted(&results[0]);
        for key in ["native_instructions", "primary_native_instructions"] {
            assert_eq!(
                count(&results[1], key) - count(&results[0], key),
                if native { 140 } else { 112 }
            );
        }
        assert_eq!(
            count(&results[1], "vector_ops") - count(&results[0], "vector_ops"),
            if native { 28 } else { 56 }
        );
    }
    println!(
        "native_vector_memory_worker_mode={} native_vector_memory_worker_forms=28 native_vector_memory_worker_programs=4",
        mode.unwrap()
    );
}

#[test]
#[ignore = "finite original comparison; requires explicitly selected independently verified0327 baseline and candidate"]
fn native_vector_memory_original_baseline_measurement() {
    let baseline = std::env::var_os("PENTAX_VECTOR_MEMORY_BASELINE_WORKER")
        .expect("explicit0327 baseline required");
    let candidate = std::env::var_os("PENTAX_TEST_WORKER").expect("explicit candidate required");
    assert!(baseline != candidate, "distinct worker artifacts required");
    const ITERATIONS: u64 = 50_000;
    let image =
        super::native_packed_float::original(&consumer(ITERATIONS, Selection::SampledStore));
    let (before, before_ns) = super::native_packed_float::measured(&baseline, &image);
    let (after, after_ns) = super::native_packed_float::measured(&candidate, &image);
    assert_eq!(before["native_ymm_mode"], after["native_ymm_mode"]);
    check_counts(&before, ITERATIONS, Selection::SampledStore, false);
    check_counts(
        &after,
        ITERATIONS,
        Selection::SampledStore,
        admitted(&after),
    );
    let promoted = if admitted(&after) { 33 + ITERATIONS } else { 0 };
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
            1056 + 16 * ITERATIONS
        } else {
            0
        }
    );
    println!(
        "native_vector_memory_measurement_iterations={ITERATIONS} native_vector_memory_measurement_mode={} baseline_elapsed_ns={before_ns} candidate_elapsed_ns={after_ns} baseline_entries={} candidate_entries={} baseline_blocks={} candidate_blocks={} baseline_work={} candidate_work={} baseline_publications={} candidate_publications={} baseline_reuses={} candidate_reuses={}",
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
