// SPDX-License-Identifier: GPL-2.0-only

//! Original full-vector context consumers. Successful data checks alone cannot
//! establish native vector execution: the paired loop also checks exact deltas
//! in real native-instruction and mediated-vector counters from the actual mode.

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

fn load(p: &mut Program, register: u8) {
    // The current temporal-memory profile can emit this checked full-width copy.
    p.code.extend([
        0xc5,
        if register >= 8 { 0x7e } else { 0xfe },
        0x6f,
        0x44 | ((register & 7) << 3),
        0x24,
        16,
    ]);
}

fn store(p: &mut Program, register: u8) {
    p.code.extend([
        0xc5,
        if register >= 8 { 0x7e } else { 0xfe },
        0x7f,
        0x44 | ((register & 7) << 3),
        0x24,
        80,
    ]);
}

fn move_register(p: &mut Program, destination: u8, source: u8, wide: bool) {
    // Reserved vvvv=1111, 0F/F3, W=0; explicit R/B inversion admits all16 regs.
    p.code.extend([
        0xc4,
        0x41 | if destination < 8 { 0x80 } else { 0 } | if source < 8 { 0x20 } else { 0 },
        if wide { 0x7e } else { 0x7a },
        0x6f,
        0xc0 | ((destination & 7) << 3) | (source & 7),
    ]);
}

fn seed(p: &mut Program, tag_offset: u8) {
    for register in 0..16 {
        for (word, value) in pattern(register + tag_offset).into_iter().enumerate() {
            p.reg(0, value);
            p.save(16 + word as u8 * 8);
        }
        load(p, register);
    }
}

fn check_vector(p: &mut Program, register: u8, words: [u64; 4]) {
    store(p, register);
    for (word, value) in words.into_iter().enumerate() {
        p.word(80 + word as u8 * 8, value);
    }
}

fn call(p: &mut Program, base: i32, slot: i32) {
    rip(&mut p.code, base, &[0xff, 0x15], 0xf00 + slot * 8);
    p.calls += 1;
}

fn finish(mut p: Program, value: u64) -> Vec<u8> {
    p.reg(0, value);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    p.code
}

fn large_image(code: &[u8], nids: &[&str]) -> Input {
    // Reuse the original import producer, with a separate larger initialized
    // executable area. Neither executable bytes nor metadata come from a game.
    let template = image(&[], nids);
    let mut bytes = template.bytes.clone();
    bytes.resize(0x4000, 0);
    assert!(code.len() <= 0x3000);
    put(&mut bytes, ENTRY as usize, code);
    put(&mut bytes, 24, &(ENTRY as u64).to_le_bytes());
    put(&mut bytes, 96, &0x4000_u64.to_le_bytes());
    put(&mut bytes, 104, &0x4000_u64.to_le_bytes());
    create_input(bytes)
}

fn registration(service: pentax_kernel::Service) -> &'static str {
    pentax_kernel::Service::registrations()
        .iter()
        .find(|registration| registration.service == service)
        .unwrap()
        .nid
}

fn counter(fields: &HashMap<String, String>, name: &str) -> u64 {
    fields[name].parse().unwrap()
}

fn bridge_mode(fields: &HashMap<String, String>) -> bool {
    match fields["native_ymm_mode"].as_str() {
        "avx" | "avx2" => true,
        "sse2" => false,
        other => panic!("unexpected closed native vector mode: {other}"),
    }
}

fn loop_consumer(iterations: u64) -> Vec<u8> {
    let mut p = Program::new();
    p.stack(0x89, 4, 8); // Actual caller stack identity.
    p.reg(0, 0x1f84);
    p.save(0);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]); // LDMXCSR, mediated.
    seed(&mut p, 0);
    p.reg(1, iterations);
    let repeat = p.code.len();
    for register in 0..16 {
        move_register(&mut p, register, register, true);
        if register == 7 {
            // An actual memory operation forces native -> mediated -> native
            // inside every loop without relying on planner window boundaries.
            // Excluded VMOVQ [RSP+80],XMM7 writes exactly eight bytes and leaves
            // its complete source unchanged. Full32 readbacks below may promote.
            p.code.extend([0xc5, 0xf9, 0xd6, 0x7c, 0x24, 80]);
        }
    }
    p.code.extend([0x48, 0xff, 0xc9]); // The only per-loop native scalar: DEC RCX.
    let displacement = repeat as i32 - (p.code.len() as i32 + 6);
    p.code.extend([0x0f, 0x85]);
    p.code.extend(displacement.to_le_bytes());

    // All16 live YMM values cross a genuine HLE stop and Rust service execution.
    call(&mut p, ENTRY, 0);
    p.expect(1);
    p.reg(0, 1);
    p.reg(2, 2);
    p.code.extend([0x48, 0x39, 0xd0]); // CF/PF/SF set, ZF/OF clear.
    p.code.extend([0x66, 0x45, 0x0f, 0xef, 0xc0]); // Legacy PXOR XMM8: upper preserved.
    move_register(&mut p, 9, 10, false); // XMM9 replace + zero upper YMM9.
    move_register(&mut p, 11, 12, true); // Full256-bit replacement.
    move_register(&mut p, 15, 0, true); // High destination / low source alias path.
    for failure in [0x83, 0x8b, 0x84, 0x89, 0x80] {
        p.code.extend([0x0f, failure, 0, 0, 0, 0]);
        p.branches.push(p.code.len() - 4);
    }
    for register in 0..16 {
        let words = match register {
            8 => [0, 0, pattern(8)[2], pattern(8)[3]],
            9 => [pattern(10)[0], pattern(10)[1], 0, 0],
            11 => pattern(12),
            15 => pattern(0),
            _ => pattern(register),
        };
        check_vector(&mut p, register, words);
    }
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 0]); // STMXCSR, mediated.
    p.word(0, 0x1f84);
    p.code.extend([0x48, 0x89, 0xe0]);
    p.load(10, 8);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    finish(p, 42)
}

#[test]
fn native_ymm_bridge_proves_all_registers_boundaries_upper_rules_and_real_native_counts() {
    let mut observations = Vec::new();
    for iterations in [1, 3] {
        let source = large_image(
            &loop_consumer(iterations),
            &[registration(pentax_kernel::Service::PthreadGetthreadid)],
        );
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert_eq!(fields["service_stops"], "1", "{text}");
        let native = bridge_mode(&fields);
        // The current artifact promotes its 32 fixed temporal copies. VMOVQ
        // and both MXCSR accesses remain modeled in either actual bridge mode.
        let modeled_moves = if native { 0 } else { iterations * 16 + 3 };
        assert_eq!(
            counter(&fields, "vector_ops"),
            if native {
                2 + iterations
            } else {
                34 + iterations + modeled_moves
            },
            "{text}"
        );
        assert_eq!(
            counter(&fields, "vector_bytes"),
            (if native { 8 } else { 1032 }) + iterations * 8,
            "{text}"
        );
        assert_eq!(counter(&fields, "integer_ops"), 0, "{text}");
        assert!(counter(&fields, "native_entries") > 0, "{text}");
        observations.push(fields);
    }
    assert_eq!(
        observations[0]["native_ymm_mode"],
        observations[1]["native_ymm_mode"]
    );
    let native = bridge_mode(&observations[0]);
    // Same setup/check/HLE paths: only two extra loop iterations differ. A
    // mediated implementation cannot satisfy the native34/vector2 delta pair.
    for name in ["native_instructions", "primary_native_instructions"] {
        assert_eq!(
            counter(&observations[1], name) - counter(&observations[0], name),
            if native { 34 } else { 2 },
        );
    }
    assert_eq!(
        counter(&observations[1], "vector_ops") - counter(&observations[0], "vector_ops"),
        if native { 2 } else { 34 },
    );
    println!(
        "native_ymm_mode={} native_ymm_register_cases=16 native_ymm_programs=2",
        observations[0]["native_ymm_mode"]
    );
}
