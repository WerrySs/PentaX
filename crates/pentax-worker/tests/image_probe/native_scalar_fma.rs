// SPDX-License-Identifier: GPL-2.0-only
//! Original fused scalar consumer through actual worker and kernel boundaries.
use super::*;

const X: [u64; 4] = [
    0xdead_beef_3f7f_f800,
    0x1122_3344_5566_7788,
    0x8877_6655_4433_2211,
    0xaabb_ccdd_eeff_1234,
];
const Y: [u64; 4] = [0x7fa0_1234_3f80_0400, 0x9988_7766_5544_3322, 31, 47];

fn fill(p: &mut Program, offset: u8, words: [u64; 4]) {
    for (word, value) in words.into_iter().enumerate() {
        p.reg(0, value);
        p.save(offset + word as u8 * 8);
    }
}

fn copy(p: &mut Program, load: bool, register: u8, offset: u8) {
    p.code.extend([
        0xc5,
        0xfc,
        if load { 0x10 } else { 0x11 },
        0x44 | (register << 3),
        0x24,
        offset,
    ]);
}

fn check_vector(p: &mut Program, register: u8, words: [u64; 4]) {
    copy(p, false, register, 112);
    for (word, value) in words.into_iter().enumerate() {
        // Snapshot crosses signed disp8; use an explicit disp32 throughout.
        p.code.extend([0x48, 0x8b, 0x84, 0x24]);
        p.code.extend((112 + word as u32 * 8).to_le_bytes());
        p.expect(value);
    }
}

fn result(bits: u64) -> [u64; 4] {
    [X[0] & !0xffff_ffff | bits, X[1], 0, 0]
}

fn consumer(iterations: u64) -> Vec<u8> {
    let mut p = Program::new();
    p.stack(0x89, 4, 8);
    p.reg(12, 0x5678_1234_90ab_cdef);
    p.reg(0, 0x1f84); // Masked exceptions, sticky ZE; both fused results exact.
    p.save(0);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]);
    for (opcode, addend, expected) in [
        (0xa9, 0xbf80_0000, 0xb280_0000),
        (0xad, 0x3f80_0000, 0x3280_0000),
    ] {
        let z = [
            0x7fa0_5678_0000_0000 | addend,
            0x1234_5678_90ab_cdef,
            53,
            71,
        ];
        fill(&mut p, 16, X);
        fill(&mut p, 48, Y);
        fill(&mut p, 80, z);
        p.reg(1, iterations);
        let repeat = p.code.len();
        copy(&mut p, true, 0, 16);
        copy(&mut p, true, 1, 48);
        copy(&mut p, true, 2, 80);
        p.reg(0, 1);
        p.reg(2, 2);
        p.code.extend([0x48, 0x39, 0xd0]);
        // Exact FMA213SS: signed (old XMM0 * XMM1) + XMM2; upper YMM0 clears.
        p.code.extend([0xc4, 0xe2, 0x71, opcode, 0xc2]);
        for failure in [0x83, 0x8b, 0x84, 0x89, 0x80] {
            p.code.extend([0x0f, failure, 0, 0, 0, 0]);
            p.branches.push(p.code.len() - 4);
        }
        p.code.extend([0x48, 0xff, 0xc9]);
        let relative = repeat as i32 - (p.code.len() as i32 + 6);
        p.code.extend([0x0f, 0x85]);
        p.code.extend(relative.to_le_bytes());
        check_vector(&mut p, 0, result(expected));
        check_vector(&mut p, 1, Y);
        check_vector(&mut p, 2, z);
    }
    rip(&mut p.code, 0x1000, &[0xff, 0x15], 0xf00);
    p.expect(1);
    check_vector(&mut p, 0, result(0x3280_0000));
    check_vector(&mut p, 1, Y);
    check_vector(
        &mut p,
        2,
        [0x7fa0_5678_3f80_0000, 0x1234_5678_90ab_cdef, 53, 71],
    );
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 0]);
    p.word(0, 0x1f84);
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

#[test]
fn native_scalar_fma_preserves_fusion_merge_flags_service_state_and_real_counts() {
    let mut results = Vec::new();
    for iterations in [1, 3] {
        let input = super::native_packed_float::original(&consumer(iterations));
        let (status, fields, text) = probe(&input);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["service_stops"], "1");
        assert_eq!(fields["fp_status"], "4");
        assert_eq!(fields["integer_ops"], "0");
        results.push(fields);
    }
    assert_eq!(
        results[0]["native_scalar_fma"],
        results[1]["native_scalar_fma"]
    );
    assert_eq!(results[0]["native_ymm_mode"], results[1]["native_ymm_mode"]);
    let native = match results[0]["native_scalar_fma"].as_str() {
        "true" => true,
        "false" => false,
        _ => panic!("unknown closed FMA capability"),
    };
    let copies = match results[0]["native_ymm_mode"].as_str() {
        "avx" | "avx2" => true,
        "sse2" => false,
        _ => panic!("unknown closed vector mode"),
    };
    assert!(!native || copies);
    let count = |index: usize, key: &str| results[index][key].parse::<u64>().unwrap();
    // Two profiles times two extra iterations: four GPR instructions, three
    // eligible copies and one FMA. Capability alone cannot satisfy these deltas.
    for key in ["native_instructions", "primary_native_instructions"] {
        assert_eq!(
            count(1, key) - count(0, key),
            16 + u64::from(copies) * 12 + u64::from(native) * 4
        );
    }
    assert_eq!(
        count(1, "vector_ops") - count(0, "vector_ops"),
        u64::from(!copies) * 12 + u64::from(!native) * 4
    );
    assert_eq!(
        count(1, "vector_bytes") - count(0, "vector_bytes"),
        u64::from(!copies) * 384
    );
    println!(
        "native_scalar_fma_worker_available={native} native_scalar_fma_worker_forms=2 native_scalar_fma_worker_programs=2"
    );
}
