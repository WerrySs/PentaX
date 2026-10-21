// SPDX-License-Identifier: GPL-2.0-only
//! Original consumers prove actual trace retirement and checked memory effects.
use super::*;

fn branch_to(code: &mut Vec<u8>, target: usize) {
    code.extend([0x0f, 0x82]); // JB: use the comparison after the real induction.
    let displacement = target as i32 - (code.len() as i32 + 4);
    code.extend(displacement.to_le_bytes());
}

fn consumer(iterations: u8) -> Vec<u8> {
    let mut p = Program::new();
    p.stack(0x89, 4, 0); // Preserve the exact live RSP for the final comparison.
    p.reg(12, 0x1234_5678_90ab_cdef);
    for (offset, value) in [(8, 0x77), (16, 11), (24, 22), (32, 33), (40, 0x99)] {
        p.set(offset, value);
    }
    p.ptr(3, 16);
    p.ptr(15, 16 + 8 * iterations);
    // End setup before the self-target block. The trace contains no RSP use.
    p.code.extend([0xeb, 0]);
    let repeat = p.code.len();
    p.code.extend([
        0x48, 0x8b, 0x03, // MOV RAX,[RBX]
        0x48, 0x83, 0xc0, 7, // ADD RAX,7
        0x48, 0x89, 0x03, // MOV [RBX],RAX
        0x48, 0x83, 0xc3, 8, // ADD RBX,8: only EA induction, after memory
        0x4c, 0x39, 0xfb, // CMP RBX,R15
    ]);
    branch_to(&mut p.code, repeat);
    for (index, value) in [11_u64, 22, 33].into_iter().enumerate() {
        p.word(
            16 + index as u8 * 8,
            value
                + if index < usize::from(iterations) {
                    7
                } else {
                    0
                },
        );
    }
    p.word(8, 0x77);
    p.word(40, 0x99);
    p.code.extend([0x4c, 0x39, 0xfb]);
    checked(&mut p.code, &mut p.branches);
    p.call(0); // Real kernel boundary after actual updates.
    p.expect(1);
    p.code.extend([0x48, 0x89, 0xe0]);
    p.load(10, 0);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x4c, 0x89, 0xe0]);
    p.expect(0x1234_5678_90ab_cdef);
    p.finish(None).0
}

fn count(fields: &HashMap<String, String>, key: &str) -> u64 {
    fields[key].parse().unwrap()
}

#[test]
fn native_affine_loop_trace_retires_one_or_two_real_bodies_with_exact_memory_and_bridge_counts() {
    let mut observations = Vec::new();
    for iterations in [1, 2, 3] {
        let source = image(&consumer(iterations), &["HoLVWNanBBc"]); // getpid
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["service_stops"], "1");
        assert_eq!(
            count(&fields, "affine_loop_entries"),
            u64::from(iterations).div_ceil(2)
        );
        assert_eq!(
            count(&fields, "affine_loop_iterations"),
            u64::from(iterations)
        );
        observations.push(fields);
    }
    // Three executed guest iterations have fifteen native body instructions.
    // The private branch, selectors and cache tails receive no guest credit.
    assert_eq!(
        count(&observations[1], "native_instructions")
            - count(&observations[0], "native_instructions"),
        5
    );
    assert_eq!(
        count(&observations[2], "native_instructions")
            - count(&observations[0], "native_instructions"),
        10
    );
    assert_eq!(
        count(&observations[1], "block_attempts") - count(&observations[0], "block_attempts"),
        1
    );
    assert_eq!(
        count(&observations[2], "block_attempts") - count(&observations[0], "block_attempts"),
        2
    );
    assert_eq!(
        count(&observations[1], "native_entries"),
        count(&observations[0], "native_entries")
    );
    assert_eq!(
        count(&observations[2], "native_entries") - count(&observations[0], "native_entries"),
        1
    );
    println!(
        "affine_loop_worker_available=true affine_loop_worker_programs=3 affine_loop_worker_retired_iterations=6"
    );
}

fn long_consumer(iterations: u32) -> Input {
    const DATA: i32 = 0x4000;
    let mut p = Program::new();
    p.reg(14, 0);
    rip(&mut p.code, 0x1000, &[0x48, 0x8d, 0x1d], DATA);
    rip(
        &mut p.code,
        0x1000,
        &[0x4c, 0x8d, 0x3d],
        DATA + iterations as i32,
    );
    p.code.extend([0xeb, 0]);
    let repeat = p.code.len();
    p.code.extend([
        0x0f, 0xb6, 0x03, // MOVZX EAX,byte[RBX]: a fresh actual read each time
        0x49, 0x01, 0xc6, // ADD R14,RAX
        0x48, 0xff, 0xc3, // INC RBX: preserve the genuine architectural CF rule
        0x4c, 0x39, 0xfb, // CMP RBX,R15
    ]);
    branch_to(&mut p.code, repeat);
    p.code.extend([0x4c, 0x89, 0xf0]);
    p.expect(u64::from(iterations) * 3);
    rip(&mut p.code, 0x1000, &[0xff, 0x15], 0xf00);
    p.expect(1);
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    let template = super::native_packed_float::original(&p.code);
    let mut bytes = template.bytes.clone();
    bytes.resize(0x8000, 0);
    bytes[DATA as usize..DATA as usize + iterations as usize].fill(3);
    put(&mut bytes, 96, &0x8000_u64.to_le_bytes());
    put(&mut bytes, 104, &0x8000_u64.to_le_bytes());
    create_input(bytes)
}

#[test]
fn native_affine_loop_trace_keeps_long_actual_reads_across_turn_and_sample_positions() {
    const ITERATIONS: u32 = 4101;
    let source = long_consumer(ITERATIONS);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "1");
    let entries = count(&fields, "affine_loop_entries");
    let retired = count(&fields, "affine_loop_iterations");
    // Existing observation/scheduling boundaries may force ordinary single
    // blocks. The closed actual trace counters cannot credit those blocks.
    assert!(entries > 0 && entries < u64::from(ITERATIONS));
    assert!(entries <= retired && retired <= entries * 2);
    assert!(retired <= u64::from(ITERATIONS));
    assert!(count(&fields, "profile_samples") >= 2);
    let progress_blocks: Vec<u64> = text
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
    assert_eq!(progress_blocks, [2048, 4096]);
    assert_eq!(fields["plan_cache_observations_overflowed"], "false");
    assert_eq!(fields["plan_cache_byte_purges"], "0");
    println!("affine_loop_worker_long_available=true affine_loop_worker_actual_reads={ITERATIONS}");
}
