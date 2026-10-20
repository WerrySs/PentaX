// SPDX-License-Identifier: GPL-2.0-only
//! Original full-context consumers distinguish native execution from mediation
//! using exact counter deltas. The ignored measurement needs two explicit,
//! externally verified worker artifacts and makes no timing inequality claim.
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
fn repeated(single: u32, double: u64, is_double: bool) -> [u64; 4] {
    [if is_double {
        double
    } else {
        u64::from(single) * 0x1_0000_0001
    }; 4]
}
fn result(opcode: u8, double: bool, narrow: bool) -> [u64; 4] {
    let (single, bits) = match (opcode, narrow) {
        (0x58, false) => (0x4100_0000, 0x4020_0000_0000_0000), //6+2=8
        (0x5c, false) => (0x4080_0000, 0x4010_0000_0000_0000), //6-2=4
        (0x59, false) => (0x4140_0000, 0x4028_0000_0000_0000), //6*2=12
        (0x58, true) => (0x4160_0000, 0x402c_0000_0000_0000),  //6+8=14
        (0x5c, true) => (0x4000_0000, 0x4000_0000_0000_0000),  //6-4=2
        _ => (0x4290_0000, 0x4052_0000_0000_0000),             //6*12=72
    };
    let mut words = repeated(single, bits, double);
    if narrow {
        words[2..].fill(0);
    }
    words
}
fn vector_memory(p: &mut Program, load: bool, register: u8, offset: u8) {
    // VMOVUPS memory always remains mediated, including on admitted AVX hosts.
    p.code.extend([
        0xc5,
        if register >= 8 { 0x7c } else { 0xfc },
        if load { 0x10 } else { 0x11 },
        0x44 | ((register & 7) << 3),
        0x24,
        offset,
    ]);
}
fn arithmetic(
    p: &mut Program,
    opcode: u8,
    double: bool,
    wide: bool,
    registers: [u8; 3],
    memory: bool,
) {
    let [destination, left, right] = registers;
    p.code.extend([
        0xc4,
        0xe1 ^ (u8::from(destination >= 8) << 7) ^ (u8::from(!memory && right >= 8) << 5),
        ((!left & 15) << 3) | (u8::from(wide) << 2) | u8::from(double),
        opcode,
        ((destination & 7) << 3) | if memory { 0x44 } else { 0xc0 | (right & 7) },
    ]);
    if memory {
        p.code.extend([0x24, 48]);
    }
}
fn check_vector(p: &mut Program, register: u8, words: [u64; 4]) {
    vector_memory(p, false, register, 80);
    for (word, value) in words.into_iter().enumerate() {
        p.word(80 + word as u8 * 8, value);
    }
}
fn fill(p: &mut Program, offset: u8, words: [u64; 4]) {
    for (word, value) in words.into_iter().enumerate() {
        p.reg(0, value);
        p.save(offset + word as u8 * 8);
    }
}
fn stack_word(p: &mut Program, offset: u32, value: u64) {
    // Program::word uses a signed disp8. Snapshot qwords at128/136 require a
    // genuine disp32; never reinterpret them as negative stack displacements.
    p.code.extend([0x48, 0x8b, 0x84, 0x24]);
    p.code.extend(offset.to_le_bytes());
    p.expect(value);
}

fn consumer(iterations: u64, dense: bool, all_forms: bool) -> Vec<u8> {
    let mut p = Program::new();
    p.stack(0x89, 4, 8);
    p.reg(12, 0x5678_1234_90ab_cdef);
    p.reg(0, 0x5f84);
    p.save(0);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]); //admitted all-masked MXCSR, stickyZE
    for register in 0..16 {
        fill(&mut p, 16, pattern(register));
        vector_memory(&mut p, true, register, 16);
    }
    let forms = if all_forms { &FORMS[..] } else { &FORMS[..1] };
    for &(opcode, double) in forms {
        fill(
            &mut p,
            16,
            repeated(0x40c0_0000, 0x4018_0000_0000_0000, double),
        );
        fill(
            &mut p,
            48,
            repeated(0x4000_0000, 0x4000_0000_0000_0000, double),
        );
        p.reg(1, iterations);
        let repeat = p.code.len();
        vector_memory(&mut p, true, 13, 16);
        vector_memory(&mut p, true, 14, 48);
        if all_forms {
            p.reg(0, 1);
            p.reg(2, 2);
            p.code.extend([0x48, 0x39, 0xd0]);
        }
        arithmetic(&mut p, opcode, double, true, [9, 13, 14], false);
        // Preserve this result before another instruction could overwrite it.
        // This exact memory store forces real native -> checked Rust -> native.
        vector_memory(&mut p, false, 9, 112);
        arithmetic(&mut p, opcode, double, true, [9, 13, 0], true);
        if dense {
            arithmetic(&mut p, opcode, double, true, [14, 13, 14], false);
            arithmetic(&mut p, opcode, double, false, [13, 13, 9], false);
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
        for (word, value) in result(opcode, double, false).into_iter().enumerate() {
            stack_word(&mut p, 112 + word as u32 * 8, value);
        }
        check_vector(&mut p, 9, result(opcode, double, false));
        check_vector(
            &mut p,
            13,
            if dense {
                result(opcode, double, true)
            } else {
                repeated(0x40c0_0000, 0x4018_0000_0000_0000, double)
            },
        );
        check_vector(
            &mut p,
            14,
            if dense {
                result(opcode, double, false)
            } else {
                repeated(0x4000_0000, 0x4000_0000_0000_0000, double)
            },
        );
    }
    // A genuine service boundary preserves every live vector before readback.
    rip(&mut p.code, ENTRY, &[0xff, 0x15], 0xf00);
    p.calls += 1;
    p.expect(1);
    let &(opcode, double) = forms.last().unwrap();
    for register in 0..16 {
        let words = match register {
            9 => result(opcode, double, false),
            13 if dense => result(opcode, double, true),
            13 => repeated(0x40c0_0000, 0x4018_0000_0000_0000, double),
            14 if dense => result(opcode, double, false),
            14 => repeated(0x4000_0000, 0x4000_0000_0000_0000, double),
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

fn original(code: &[u8]) -> Input {
    let nid = pentax_kernel::Service::registrations()
        .iter()
        .find(|r| r.service == pentax_kernel::Service::PthreadGetthreadid)
        .unwrap()
        .nid;
    let template = image(&[], &[nid]);
    let mut bytes = template.bytes.clone();
    bytes.resize(0x4000, 0);
    put(&mut bytes, ENTRY as usize, code);
    put(&mut bytes, 24, &(ENTRY as u64).to_le_bytes());
    put(&mut bytes, 96, &0x4000_u64.to_le_bytes());
    put(&mut bytes, 104, &0x4000_u64.to_le_bytes());
    create_input(bytes)
}
fn count(fields: &HashMap<String, String>, key: &str) -> u64 {
    fields[key].parse().unwrap()
}
fn admitted(fields: &HashMap<String, String>) -> bool {
    match fields["native_ymm_mode"].as_str() {
        "avx" | "avx2" => true,
        "sse2" => false,
        _ => panic!("unknown closed native vector mode"),
    }
}
fn check_counts(
    fields: &HashMap<String, String>,
    iterations: u64,
    profiles: u64,
    register_ops: u64,
    native: bool,
) {
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "1");
    assert_eq!(
        count(fields, "vector_ops"),
        34 + 3 * profiles + profiles * iterations * (4 + if native { 0 } else { register_ops })
    );
    assert_eq!(
        count(fields, "vector_bytes"),
        1032 + 96 * profiles + 128 * profiles * iterations
    );
    assert_eq!(count(fields, "integer_ops"), 0);
    assert_eq!(count(fields, "fp_status"), 4);
}

#[test]
fn native_packed_float_family_proves_memory_continuations_full_context_and_real_counts() {
    let mut observations = Vec::new();
    for iterations in [1, 3] {
        let source = original(&consumer(iterations, true, true));
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 0, "{text}");
        check_counts(&fields, iterations, 6, 3, admitted(&fields));
        observations.push(fields);
    }
    assert_eq!(
        observations[0]["native_ymm_mode"],
        observations[1]["native_ymm_mode"]
    );
    let native = admitted(&observations[0]);
    for key in ["native_instructions", "primary_native_instructions"] {
        // Six profiles, two extra iterations; each has MOV/MOV/CMP/DEC and,
        // only under actual admitted execution, three packed FP instructions.
        assert_eq!(
            count(&observations[1], key) - count(&observations[0], key),
            if native { 84 } else { 48 }
        );
    }
    assert_eq!(
        count(&observations[1], "vector_ops") - count(&observations[0], "vector_ops"),
        if native { 48 } else { 84 }
    );
    println!(
        "native_packed_float_worker_mode={} native_packed_float_worker_forms=12 native_packed_float_worker_programs=2",
        observations[0]["native_ymm_mode"]
    );
}

fn drain_diagnostics(mut reader: impl std::io::Read) -> (Vec<u8>, bool) {
    // Two readers share an aggregate64KiB ceiling. Keep draining/discarding on
    // overflow so a child cannot block on a full pipe before deadline cleanup.
    const LIMIT: usize = 32 * 1024;
    let mut output = Vec::new();
    output.try_reserve_exact(LIMIT).unwrap();
    assert!(output.capacity() <= LIMIT);
    let mut overflow = false;
    let mut chunk = [0; 4096];
    loop {
        let count = reader
            .read(&mut chunk)
            .expect("cannot drain worker diagnostic pipe");
        if count == 0 {
            break;
        }
        let retained = count.min(LIMIT - output.len());
        output.extend_from_slice(&chunk[..retained]);
        overflow |= retained != count;
    }
    (output, overflow)
}

fn measured(worker: &std::ffi::OsStr, source: &Input) -> (HashMap<String, String>, u128) {
    let started = Instant::now();
    let mut child = Command::new(worker)
        .args(["probe-entry", "--image"])
        .arg(&source.path)
        .args(["--execution-profile", "extended-startup"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x0800_0000)
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stdout_reader = std::thread::spawn(move || drain_diagnostics(stdout));
    let stderr_reader = std::thread::spawn(move || drain_diagnostics(stderr));
    let deadline = started + Duration::from_secs(30);
    let mut timed_out = false;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child
                .kill()
                .expect("cannot terminate timed-out original worker");
            timed_out = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let status = child.wait().unwrap();
    let (stdout, stdout_overflow) = stdout_reader.join().unwrap();
    let (stderr, stderr_overflow) = stderr_reader.join().unwrap();
    let elapsed = started.elapsed().as_nanos();
    assert!(
        !timed_out,
        "original packed-float measurement exceeded finite deadline"
    );
    assert!(
        !stdout_overflow && !stderr_overflow,
        "worker diagnostic ceiling exceeded"
    );
    assert!(status.success());
    assert!(stderr.is_empty());
    let text = String::from_utf8(stdout).unwrap();
    let results: Vec<_> = text
        .lines()
        .filter(|line| line.starts_with("PENTAX_WORKER\t1\tRESULT\t"))
        .collect();
    assert_eq!(results.len(), 1);
    let fields = results[0]
        .split('\t')
        .skip(4)
        .map(|field| field.split_once('=').unwrap())
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    (fields, elapsed)
}

#[test]
#[ignore = "bounded original measurement; requires explicit independently verified baseline and candidate artifacts"]
fn native_packed_float_original_baseline_measurement() {
    // The coordinating runner verifies the immutable baseline artifact hash.
    // Never change process-global environment variables inside this test.
    let baseline = std::env::var_os("PENTAX_PACKED_FLOAT_BASELINE_WORKER")
        .expect("explicit baseline worker is required");
    let candidate =
        std::env::var_os("PENTAX_TEST_WORKER").expect("explicit candidate worker is required");
    assert!(
        baseline != candidate,
        "distinct worker artifacts are required"
    );
    const ITERATIONS: u64 = 50_000;
    for dense in [false, true] {
        let source = original(&consumer(ITERATIONS, dense, false));
        let (before, before_ns) = measured(&baseline, &source);
        let (after, after_ns) = measured(&candidate, &source);
        assert_eq!(before["native_ymm_mode"], after["native_ymm_mode"]);
        let registers = if dense { 3 } else { 1 };
        check_counts(&before, ITERATIONS, 1, registers, false);
        check_counts(&after, ITERATIONS, 1, registers, admitted(&after));
        let promoted = if admitted(&after) {
            ITERATIONS * registers
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
        // End-to-end process elapsed includes startup and teardown. Warmed
        // repeated plans dominate the finite loop; there is no timing assertion.
        println!(
            "native_packed_float_measurement_dense={dense} native_packed_float_measurement_iterations={ITERATIONS} native_packed_float_measurement_mode={} baseline_elapsed_ns={before_ns} candidate_elapsed_ns={after_ns} baseline_entries={} candidate_entries={} baseline_blocks={} candidate_blocks={} baseline_work={} candidate_work={} baseline_publications={} candidate_publications={} baseline_cache_reuses={} candidate_cache_reuses={}",
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
