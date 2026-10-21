// SPDX-License-Identifier: GPL-2.0-only
//! Original register FMA consumers retain a modeled memory operation and HLE.
use super::*;

#[derive(Clone, Copy)]
struct Form {
    opcode: u8,
    double: bool,
    wide: bool,
    scalar: bool,
    first: [u64; 4],
    second: [u64; 4],
}

fn single(bits: u32) -> u64 {
    u64::from(bits) * 0x1_0000_0001
}
fn packed(single_bits: u32, double_bits: u64, double: bool, wide: bool) -> [u64; 4] {
    let mut result = [if double {
        double_bits
    } else {
        single(single_bits)
    }; 4];
    if !wide {
        result[2..].fill(0);
    }
    result
}
fn forms() -> [Form; 6] {
    [
        Form {
            opcode: 0xb8,
            double: false,
            wide: false,
            scalar: false,
            first: packed(0x40e0_0000, 0, false, false), // 2*3+1=7.
            second: packed(0x4150_0000, 0, false, false),
        }, // 2*3+7=13.
        Form {
            opcode: 0xb8,
            double: false,
            wide: true,
            scalar: false,
            first: [single(0x40e0_0000); 4],
            second: [single(0x4150_0000); 4],
        },
        Form {
            opcode: 0x9e,
            double: true,
            wide: true,
            scalar: false,
            first: [0xc014_0000_0000_0000; 4], // -(1*3)-2=-5.
            second: [0x402a_0000_0000_0000; 4],
        }, // -(-5*3)-2=13.
        Form {
            opcode: 0xbd,
            double: true,
            wide: false,
            scalar: true,
            first: [0xc014_0000_0000_0000, 0x7ff0_0000_0000_1234, 0, 0],
            second: [0xc026_0000_0000_0000, 0x7ff0_0000_0000_1234, 0, 0],
        }, // -6-5=-11.
        Form {
            opcode: 0xa6,
            double: false,
            wide: true,
            scalar: false,
            first: [0x40a0_0000_bf80_0000; 4], // Even2-3=-1, odd2+3=5.
            second: [0x4150_0000_c0a0_0000; 4],
        }, // Even-2-3=-5, odd10+3=13.
        Form {
            opcode: 0x97,
            double: true,
            wide: false,
            scalar: false,
            first: [0x4014_0000_0000_0000, 0x3ff0_0000_0000_0000, 0, 0],
            second: [0x4031_0000_0000_0000, 0x3ff0_0000_0000_0000, 0, 0],
        }, // Even5*3+2=17, odd1*3-2=1.
    ]
}
fn pattern(register: u8) -> [u64; 4] {
    let tag = (u64::from(register) + 1) * 0x0101_0101_0101_0101;
    [
        0x0123_4567_89ab_cdef ^ tag,
        0xfedc_ba98_7654_3210 ^ tag,
        0x8877_6655_4433_2211 ^ tag,
        0x89ab_cdef_0123_4567 ^ tag,
    ]
}
fn input(form: Form, value: u32) -> [u64; 4] {
    let (f32_bits, f64_bits) = match value {
        1 => (0x3f80_0000, 0x3ff0_0000_0000_0000),
        2 => (0x4000_0000, 0x4000_0000_0000_0000),
        3 => (0x4040_0000, 0x4008_0000_0000_0000),
        _ => unreachable!(),
    };
    let mut words = [if form.double {
        f64_bits
    } else {
        single(f32_bits)
    }; 4];
    if form.scalar {
        // Inactive scalar sNaNs remain raw and cannot raise invalid status.
        words[1] = 0x7ff0_0000_0000_1234;
        words[2] = 0x1122_3344_5566_7788;
        words[3] = 0x8877_6655_4433_2211;
    }
    words
}
fn fill(p: &mut Program, offset: u8, words: [u64; 4]) {
    for (word, value) in words.into_iter().enumerate() {
        p.reg(0, value);
        p.save(offset + word as u8 * 8);
    }
}
fn copy(p: &mut Program, load: bool, register: u8, offset: u8) {
    p.code.extend([
        0xc5,
        if register >= 8 { 0x7c } else { 0xfc },
        if load { 0x10 } else { 0x11 },
        0x44 | ((register & 7) << 3),
        0x24,
        offset,
    ]);
}
fn word(p: &mut Program, offset: u32, expected: u64) {
    p.code.extend([0x48, 0x8b, 0x84, 0x24]);
    p.code.extend(offset.to_le_bytes());
    p.expect(expected);
}
fn snapshot(p: &mut Program, expected: [u64; 4]) {
    for (lane, value) in expected.into_iter().enumerate() {
        word(p, 112 + lane as u32 * 8, value);
    }
}
fn check_vector(p: &mut Program, register: u8, expected: [u64; 4]) {
    copy(p, false, register, 112);
    snapshot(p, expected);
}
fn fused(p: &mut Program, form: Form, memory: bool) {
    // DEST9 is destructive, logical source13, register source14 or [RSP+80].
    p.code.extend([
        0xc4,
        if memory { 0x62 } else { 0x42 },
        0x11 | (u8::from(form.double) << 7) | (u8::from(form.wide) << 2),
        form.opcode,
        if memory { 0x4c } else { 0xce },
    ]);
    if memory {
        p.code.extend([0x24, 80]);
    }
}

fn consumer(iterations: u64) -> Vec<u8> {
    let mut p = Program::new();
    p.stack(0x89, 4, 8);
    p.reg(12, 0x5678_1234_90ab_cdef);
    p.reg(0, 0x1f84); // Masked exceptions and preexisting sticky ZE.
    p.save(0);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]);
    for register in 0..16 {
        fill(&mut p, 16, pattern(register));
        copy(&mut p, true, register, 16);
    }
    for form in forms() {
        fill(&mut p, 16, input(form, 1));
        fill(&mut p, 48, input(form, 2));
        fill(&mut p, 80, input(form, 3));
        p.reg(1, iterations);
        let repeat = p.code.len();
        copy(&mut p, true, 9, 16);
        copy(&mut p, true, 13, 48);
        copy(&mut p, true, 14, 80);
        p.reg(0, 1);
        p.reg(2, 2);
        p.code.extend([0x48, 0x39, 0xd0]);
        fused(&mut p, form, false);
        copy(&mut p, false, 9, 112);
        // The same FMA Code's complete memory source remains genuinely
        // mediated, consuming the newly produced destructive destination.
        fused(&mut p, form, true);
        for failure in [0x83, 0x8b, 0x84, 0x89, 0x80] {
            p.code.extend([0x0f, failure, 0, 0, 0, 0]);
            p.branches.push(p.code.len() - 4);
        }
        p.code.extend([0x48, 0xff, 0xc9]);
        let relative = repeat as i32 - (p.code.len() as i32 + 6);
        p.code.extend([0x0f, 0x85]);
        p.code.extend(relative.to_le_bytes());
        snapshot(&mut p, form.first);
        check_vector(&mut p, 9, form.second);
        check_vector(&mut p, 13, input(form, 2));
        check_vector(&mut p, 14, input(form, 3));
    }
    rip(&mut p.code, 0x1000, &[0xff, 0x15], 0xf00);
    p.expect(1);
    let last = forms()[5];
    for register in 0..16 {
        let expected = match register {
            9 => last.second,
            13 => input(last, 2),
            14 => input(last, 3),
            _ => pattern(register),
        };
        check_vector(&mut p, register, expected);
    }
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
fn native_fma_registers_consume_real_memory_and_service_continuations_with_exact_counters() {
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
    for (index, iterations) in [1, 3].into_iter().enumerate() {
        let copies_count = 50 + 24 * iterations;
        // Four32B copies per loop, sixteen initial loads, eighteen immediate
        // checks and sixteen post-service checks. All memory FMA stays modeled.
        assert_eq!(
            count(index, "vector_ops"),
            2 + 6 * iterations
                + u64::from(!native) * 6 * iterations
                + u64::from(!copies) * copies_count
        );
        // Six memory reads total136B/iteration; MXCSR load/store total8B.
        assert_eq!(
            count(index, "vector_bytes"),
            8 + 136 * iterations + u64::from(!copies) * 32 * copies_count
        );
    }
    for key in ["native_instructions", "primary_native_instructions"] {
        assert_eq!(
            count(1, key) - count(0, key),
            48 + u64::from(copies) * 48 + u64::from(native) * 12
        );
    }
    println!(
        "native_fma_registers_worker_available={native} native_fma_registers_worker_forms=6 native_fma_registers_worker_programs=2"
    );
}
