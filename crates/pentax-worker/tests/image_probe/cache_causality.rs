// SPDX-License-Identifier: GPL-2.0-only
//! Original colliding functions distinguish revisits from a new code frontier.
use super::*;

const ENTRY: i32 = 0x1000;
const FUNCTIONS: [(i32, u8); 3] = [(0x1800, 11), (0x2830, 22), (0x3820, 33)];

fn consumer(revisit: bool) -> Vec<u8> {
    let mut p = Program::new();
    for index in [0, 1, if revisit { 0 } else { 2 }] {
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
        assert!(count("cache_replacements") >= 2);
        assert!(count("plan_cache_occupied_collisions") >= 2);
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
