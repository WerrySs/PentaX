// SPDX-License-Identifier: GPL-2.0-only
//! Original colliding functions distinguish revisits from a new code frontier.
use super::*;

const ENTRY: i32 = 0x1000;
const FUNCTIONS: [(i32, u8); 3] = [(0x1800, 11), (0x2830, 22), (0x3820, 33)];

fn consumer(revisit: bool) -> Vec<u8> {
    let mut p = Program::new();
    // Under two-way retention A/B/A is a hit. A/B/C/B/A first evicts A,
    // refreshes B, then revisits A in the slot whose previous producer was A.
    let sequence: &[usize] = if revisit {
        &[0, 1, 2, 1, 0]
    } else {
        &[0, 1, 2]
    };
    for &index in sequence {
        let (target, value) = FUNCTIONS[index];
        rip(&mut p.code, ENTRY, &[0xe8], target);
        p.expect(u64::from(value));
    }
    let (mut code, _) = p.finish(None);
    for (target, value) in FUNCTIONS {
        let at = (target - ENTRY) as usize;
        assert!(code.len() <= at);
        code.resize(at, 0xcc);
        // Different literal results force different sealed executable bodies.
        code.extend([0xb8, value, 0, 0, 0, 0xc3]);
    }
    assert!(code.len() < 0x3000);
    code
}

#[test]
fn native_cache_observations_distinguish_returning_keys_from_unique_collisions() {
    let mut results = Vec::new();
    for revisit in [false, true] {
        let input = super::native_packed_float::original(&consumer(revisit));
        let (status, fields, text) = probe(&input);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["cache_history_saturated"], "false");
        assert_eq!(fields["plan_cache_observations_overflowed"], "false");
        let count = |key: &str| fields[key].parse::<u64>().unwrap();
        assert!(count("cache_replacements") >= if revisit { 2 } else { 1 });
        assert!(count("plan_cache_occupied_collisions") >= if revisit { 2 } else { 1 });
        assert_eq!(count("plan_cache_byte_purges"), 0);
        assert_eq!(count("plan_cache_purged_plans"), 0);
        assert!(count("plan_cache_retained_bytes") <= count("plan_cache_peak_retained_bytes"));
        assert!(count("plan_cache_peak_retained_bytes") <= 8 * 1024 * 1024);
        assert_eq!(count("cache_evicted_key_revisits"), u64::from(revisit));
        results.push(fields);
    }
    println!(
        "cache_causality_worker_revisits=true cache_causality_worker_programs={}",
        results.len()
    );
}

fn alternating_consumer(iterations: u64) -> Vec<u8> {
    let mut p = Program::new();
    p.reg(1, iterations);
    let repeat = p.code.len();
    for (target, value) in &FUNCTIONS[..2] {
        rip(&mut p.code, ENTRY, &[0xe8], *target);
        p.expect(u64::from(*value));
    }
    p.code.extend([0x48, 0xff, 0xc9]);
    let relative = repeat as i32 - (p.code.len() as i32 + 6);
    p.code.extend([0x0f, 0x85]);
    p.code.extend(relative.to_le_bytes());
    let (mut code, _) = p.finish(None);
    for (target, value) in &FUNCTIONS[..2] {
        let at = (*target - ENTRY) as usize;
        assert!(code.len() <= at);
        code.resize(at, 0xcc);
        code.extend([0xb8, *value, 0, 0, 0, 0xc3]);
    }
    code
}

#[test]
#[ignore = "finite original comparison; requires explicit verified direct-cache release and candidate"]
fn original_worker_two_way_cache_baseline_measurement() {
    let baseline = std::env::var_os("PENTAX_CACHE_BASELINE_WORKER")
        .expect("explicit verified direct-cache baseline required");
    let candidate = std::env::var_os("PENTAX_TEST_WORKER").expect("explicit candidate required");
    assert!(baseline != candidate);
    const ITERATIONS: u64 = 50_000;
    let input = super::native_packed_float::original(&alternating_consumer(ITERATIONS));
    let (before, before_ns) = super::native_packed_float::measured(&baseline, &input);
    let (after, after_ns) = super::native_packed_float::measured(&candidate, &input);
    for fields in [&before, &after] {
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["cache_history_saturated"], "false");
        assert_eq!(fields["plan_cache_observations_overflowed"], "false");
        assert_eq!(fields["plan_cache_byte_purges"], "0");
    }
    for key in [
        "native_instructions",
        "primary_native_instructions",
        "native_entries",
        "block_attempts",
        "service_stops",
        "vector_ops",
        "integer_ops",
    ] {
        assert_eq!(before[key], after[key], "{key}");
    }
    let count = |fields: &HashMap<String, String>, key: &str| fields[key].parse::<u64>().unwrap();
    let promoted = 2 * ITERATIONS - 2;
    assert_eq!(
        count(&before, "cache_publications") - count(&after, "cache_publications"),
        promoted
    );
    assert_eq!(
        count(&after, "cache_reuses") - count(&before, "cache_reuses"),
        promoted
    );
    assert_eq!(
        count(&before, "plan_cache_misses") - count(&after, "plan_cache_misses"),
        promoted
    );
    println!(
        "two_way_cache_worker_measurement_available=true iterations={ITERATIONS} baseline_elapsed_ns={before_ns} candidate_elapsed_ns={after_ns} baseline_publications={} candidate_publications={} baseline_plan_misses={} candidate_plan_misses={}",
        before["cache_publications"],
        after["cache_publications"],
        before["plan_cache_misses"],
        after["plan_cache_misses"]
    );
}
