// SPDX-License-Identifier: GPL-2.0-only
//! Original unchanged-entry consumers and an explicit, finite artifact comparison.
use super::*;

fn consumer(iterations: u64) -> Vec<u8> {
    let mut p = Program::new();
    p.stack(0x89, 4, 8);
    p.reg(12, 0x5678_1234_90ab_cdef);
    p.reg(13, 0);
    p.reg(1, iterations);
    let repeat = p.code.len();
    // This register-only loop is deliberately outside affine memory tracing.
    p.code.extend([0x49, 0x83, 0xc5, 3, 0x48, 0xff, 0xc9]);
    let relative = repeat as i32 - (p.code.len() as i32 + 6);
    p.code.extend([0x0f, 0x85]);
    p.code.extend(relative.to_le_bytes());
    for failure in [0x82, 0x85, 0x8b, 0x88, 0x80] {
        // Final ADD did not carry; DEC reached zero with even parity and no
        // sign/overflow. The bridge must preserve these flags across JNZ.
        p.code.extend([0x0f, failure, 0, 0, 0, 0]);
        p.branches.push(p.code.len() - 4);
    }
    p.code.extend([0x4c, 0x89, 0xe8]);
    p.expect(iterations * 3);
    p.call(0);
    p.expect(1);
    p.code.extend([0x48, 0x89, 0xe0]);
    p.load(10, 8);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x4c, 0x89, 0xe0]);
    p.expect(0x5678_1234_90ab_cdef);
    p.finish(None).0
}

fn validate(fields: &HashMap<String, String>, iterations: u64) {
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "1");
    assert_eq!(fields["affine_loop_entries"], "0");
    assert_eq!(fields["affine_loop_iterations"], "0");
    assert_eq!(fields["integer_ops"], "0");
    assert!(fields["native_entries"].parse::<u64>().unwrap() >= iterations);
}

#[test]
fn native_context_staging_preserves_register_loop_flags_stack_and_service_in_the_real_worker() {
    const ITERATIONS: u64 = 513;
    let input = image(&consumer(ITERATIONS), &["HoLVWNanBBc"]);
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    validate(&fields, ITERATIONS);
    println!(
        "native_context_staging_worker_available=true native_context_staging_worker_iterations={ITERATIONS}"
    );
}

#[test]
#[ignore = "finite comparison requires explicit independently verified baseline and candidate artifacts"]
fn original_worker_context_staging_baseline_measurement() {
    let baseline = std::env::var_os("PENTAX_CONTEXT_BASELINE_WORKER")
        .expect("explicit verified baseline required");
    let candidate = std::env::var_os("PENTAX_TEST_WORKER").expect("explicit candidate required");
    assert_ne!(baseline, candidate);
    const ITERATIONS: u64 = 250_000;
    let input = image(&consumer(ITERATIONS), &["HoLVWNanBBc"]);
    let mut baseline_ns = Vec::new();
    let mut candidate_ns = Vec::new();
    let mut reference: Option<HashMap<String, String>> = None;
    // Alternate artifact order; time remains observational, never a pass gate.
    for reverse in [false, true] {
        for use_baseline in if reverse {
            [false, true]
        } else {
            [true, false]
        } {
            let worker = if use_baseline { &baseline } else { &candidate };
            let (fields, elapsed) = super::native_packed_float::measured(worker, &input);
            validate(&fields, ITERATIONS);
            if let Some(reference) = &reference {
                for key in [
                    "native_ymm_mode",
                    "native_instructions",
                    "primary_native_instructions",
                    "native_entries",
                    "block_attempts",
                    "service_stops",
                    "vector_ops",
                    "vector_bytes",
                    "integer_ops",
                    "integer_bytes",
                    "dispatch_work",
                    "fetched_source_bytes",
                    "admission_work",
                    "plan_cache_hits",
                    "plan_cache_misses",
                    "cache_publications",
                    "cache_reuses",
                    "output_bytes",
                    "affine_loop_entries",
                    "affine_loop_iterations",
                ] {
                    assert_eq!(fields[key], reference[key], "{key}");
                }
            } else {
                reference = Some(fields);
            }
            if use_baseline {
                baseline_ns.push(elapsed);
            } else {
                candidate_ns.push(elapsed);
            }
        }
    }
    println!(
        "context_staging_worker_measurement_available=true iterations_per_run={ITERATIONS} baseline_runs=2 candidate_runs=2 baseline_elapsed_ns={}:{} candidate_elapsed_ns={}:{}",
        baseline_ns[0], baseline_ns[1], candidate_ns[0], candidate_ns[1]
    );
}
