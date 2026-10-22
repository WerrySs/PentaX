// SPDX-License-Identifier: GPL-2.0-only
//! Original register-loop consumers exercise retirement through the real worker.
use super::*;

fn consumer(iterations: u64) -> Vec<u8> {
    assert!(iterations > 0);
    let mut p = Program::new();
    p.stack(0x89, 4, 8);
    p.reg(12, 0x2b31_790d_ace0_4567);
    p.reg(13, 0);
    p.reg(1, iterations);
    // Separate setup from the self-targeted body, without changing guest flags.
    p.code.extend([0xeb, 0]);
    let repeat = p.code.len();
    p.code.extend([
        0x49, 0x83, 0xc5, 3, // ADD R13,3
        0x48, 0xff, 0xc9, // DEC RCX
        0x0f, 0x85, // JNZ repeat
    ]);
    let displacement = repeat as i32 - (p.code.len() as i32 + 4);
    p.code.extend(displacement.to_le_bytes());
    // ADD never carries; final DEC has ZF/PF set and SF/OF clear. Check before
    // any result comparison or HLE call can overwrite the real loop flags.
    for failure in [0x82, 0x85, 0x8b, 0x88, 0x80] {
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
    p.expect(0x2b31_790d_ace0_4567);
    p.finish(None).0
}

fn count(fields: &HashMap<String, String>, key: &str) -> u64 {
    fields[key].parse().unwrap()
}

fn validate_result(fields: &HashMap<String, String>) {
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "1");
    assert_eq!(fields["affine_loop_entries"], "0");
    assert_eq!(fields["affine_loop_iterations"], "0");
    assert_eq!(fields["integer_ops"], "0");
}

#[test]
fn actual_register_loop_retires_one_two_and_three_bodies_with_exact_bridge_deltas() {
    let mut observations = Vec::new();
    for iterations in [1_u64, 2, 3] {
        let source = image(&consumer(iterations), &["HoLVWNanBBc"]);
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 0, "{text}");
        validate_result(&fields);
        assert_eq!(
            count(&fields, "register_loop_entries"),
            iterations.div_ceil(2)
        );
        assert_eq!(count(&fields, "register_loop_iterations"), iterations);
        observations.push(fields);
    }
    for (index, extra) in [(1, 1), (2, 2)] {
        assert_eq!(
            count(&observations[index], "native_instructions")
                - count(&observations[0], "native_instructions"),
            extra * 2
        );
        assert_eq!(
            count(&observations[index], "block_attempts")
                - count(&observations[0], "block_attempts"),
            extra
        );
    }
    assert_eq!(
        count(&observations[1], "native_entries"),
        count(&observations[0], "native_entries")
    );
    assert_eq!(
        count(&observations[2], "native_entries") - count(&observations[0], "native_entries"),
        1
    );
    println!(
        "register_loop_native_available=true register_loop_native_programs=3 register_loop_native_retired_iterations=6"
    );
}

#[test]
fn actual_register_loop_preserves_state_across_progress_and_scheduling_boundaries() {
    const ITERATIONS: u64 = 4101;
    let source = image(&consumer(ITERATIONS), &["HoLVWNanBBc"]);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    validate_result(&fields);
    let entries = count(&fields, "register_loop_entries");
    let retired = count(&fields, "register_loop_iterations");
    assert!(entries > 0 && entries < ITERATIONS);
    assert!(entries <= retired && retired <= entries * 2 && retired <= ITERATIONS);
    assert!(count(&fields, "profile_samples") >= 2);
    let progress: Vec<u64> = text
        .lines()
        .filter(|line| line.starts_with("PENTAX_WORKER\t1\tPROGRESS\t"))
        .map(|line| {
            line.split('\t')
                .find_map(|field| field.strip_prefix("block_attempts="))
                .unwrap()
                .parse()
                .unwrap()
        })
        .collect();
    assert_eq!(progress, [2048, 4096]);
    assert_eq!(fields["plan_cache_observations_overflowed"], "false");
    assert_eq!(fields["plan_cache_byte_purges"], "0");
    println!(
        "register_loop_boundaries_available=true register_loop_boundary_iterations={ITERATIONS}"
    );
}

#[test]
#[ignore = "finite measurement requires explicit independently verified baseline and candidate artifacts"]
fn original_register_loop_baseline_measurement() {
    let baseline = std::env::var_os("PENTAX_REGISTER_LOOP_BASELINE_WORKER")
        .expect("explicit verified baseline required");
    let candidate = std::env::var_os("PENTAX_TEST_WORKER").expect("explicit candidate required");
    assert_ne!(baseline, candidate);
    const ITERATIONS: u64 = 250_000;
    let source = image(&consumer(ITERATIONS), &["HoLVWNanBBc"]);
    let mut baseline_times = Vec::new();
    let mut candidate_times = Vec::new();
    let mut reference: Option<HashMap<String, String>> = None;
    for reverse in [false, true] {
        for use_baseline in if reverse {
            [false, true]
        } else {
            [true, false]
        } {
            let worker = if use_baseline { &baseline } else { &candidate };
            let (fields, elapsed) = super::native_packed_float::measured(worker, &source);
            validate_result(&fields);
            if let Some(reference) = &reference {
                for key in [
                    "native_ymm_mode",
                    "native_instructions",
                    "primary_native_instructions",
                    "block_attempts",
                    "service_stops",
                    "vector_ops",
                    "vector_bytes",
                    "integer_ops",
                    "integer_bytes",
                ] {
                    assert_eq!(fields[key], reference[key], "{key}");
                }
                if !use_baseline {
                    assert!(count(&fields, "native_entries") < count(reference, "native_entries"));
                }
            } else {
                reference = Some(fields.clone());
            }
            if use_baseline {
                baseline_times.push(elapsed);
            } else {
                assert!(count(&fields, "register_loop_entries") > 0);
                let entries = count(&fields, "register_loop_entries");
                let retired = count(&fields, "register_loop_iterations");
                assert!(entries <= retired && retired <= entries * 2 && retired <= ITERATIONS);
                candidate_times.push(elapsed);
            }
        }
    }
    println!(
        "register_loop_measurement_available=true iterations_per_run={ITERATIONS} baseline_runs=2 candidate_runs=2 baseline_elapsed_ns={}:{} candidate_elapsed_ns={}:{}",
        baseline_times[0], baseline_times[1], candidate_times[0], candidate_times[1]
    );
}
