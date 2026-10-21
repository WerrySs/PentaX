// SPDX-License-Identifier: GPL-2.0-only
//! All twelve VEX MIN/MAX forms cross real native, memory and service boundaries.
use super::*;

#[derive(Clone, Copy)]
struct Form {
    opcode: u8,
    double: bool,
    wide: bool,
    scalar: bool,
}

fn forms() -> Vec<Form> {
    [0x5d, 0x5f]
        .into_iter()
        .flat_map(|opcode| {
            [
                (false, false, false),
                (false, true, false),
                (true, false, false),
                (true, true, false),
                (false, false, true),
                (true, false, true),
            ]
            .into_iter()
            .map(move |(double, wide, scalar)| Form {
                opcode,
                double,
                wide,
                scalar,
            })
        })
        .collect()
}
fn repeated(bits: u32) -> u64 {
    u64::from(bits) * 0x1_0000_0001
}
fn operand(form: Form, value: u32) -> [u64; 4] {
    let (single, double) = match value {
        2 => (0x4000_0000, 0x4000_0000_0000_0000),
        6 => (0x40c0_0000, 0x4018_0000_0000_0000),
        _ => unreachable!(),
    };
    let mut words = [if form.double {
        double
    } else {
        repeated(single)
    }; 4];
    if form.scalar {
        if !form.double {
            words[0] = (u64::from(0x7fa0_0100 | value) << 32) | u64::from(single);
        }
        words[1] = 0x7ff0_0000_0000_1000 | u64::from(value);
        words[2] = 0x0123_4567_89ab_cdef;
        words[3] = 0xfedc_ba98_7654_3210;
    }
    words
}
fn result(form: Form) -> [u64; 4] {
    let mut words = operand(form, 6);
    let active = operand(form, if form.opcode == 0x5d { 2 } else { 6 });
    if form.scalar {
        words[0] = if form.double {
            active[0]
        } else {
            (words[0] & !0xffff_ffff) | (active[0] & 0xffff_ffff)
        };
    } else {
        words = active;
    }
    if !form.wide {
        words[2..].fill(0);
    }
    words
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
fn fill(p: &mut Program, offset: u8, words: [u64; 4]) {
    for (index, value) in words.into_iter().enumerate() {
        p.reg(0, value);
        p.save(offset + index as u8 * 8);
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
fn snapshot(p: &mut Program, words: [u64; 4]) {
    for (index, value) in words.into_iter().enumerate() {
        p.code.extend([0x48, 0x8b, 0x84, 0x24]);
        p.code.extend((112 + index as u32 * 8).to_le_bytes());
        p.expect(value);
    }
}
fn check_vector(p: &mut Program, register: u8, words: [u64; 4]) {
    copy(p, false, register, 112);
    snapshot(p, words);
}
fn arithmetic(p: &mut Program, form: Form, memory: bool) {
    let (destination, left) = if memory { (14, 9) } else { (9, 13) };
    let prefix = if form.scalar {
        if form.double { 3 } else { 2 }
    } else {
        u8::from(form.double)
    };
    p.code.extend([
        0xc4,
        if memory { 0x61 } else { 0x41 },
        ((!left & 15) << 3) | (u8::from(form.wide) << 2) | prefix,
        form.opcode,
        ((destination & 7) << 3) | if memory { 0x44 } else { 0xc6 },
    ]);
    if memory {
        p.code.extend([0x24, 48]);
    }
}
fn consumer(iterations: u64) -> Vec<u8> {
    let mut p = Program::new();
    p.stack(0x89, 4, 8);
    p.reg(12, 0x5678_1234_90ab_cdef);
    p.reg(0, 0x1f84);
    p.save(0);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 0]);
    for register in 0..16 {
        fill(&mut p, 16, pattern(register));
        copy(&mut p, true, register, 16);
    }
    for form in forms() {
        fill(&mut p, 16, operand(form, 6));
        fill(&mut p, 48, operand(form, 2));
        p.reg(1, iterations);
        let repeat = p.code.len();
        copy(&mut p, true, 9, 16);
        copy(&mut p, true, 13, 16);
        copy(&mut p, true, 14, 48);
        p.reg(0, 1);
        p.reg(2, 2);
        p.code.extend([0x48, 0x39, 0xd0]);
        arithmetic(&mut p, form, false);
        copy(&mut p, false, 9, 112);
        // This excluded store retains a genuinely modeled boundary after the
        // register result snapshot. The native memory form then consumes XMM9.
        p.code.extend([0xc5, 0x79, 0xd6, 0x4c, 0x24, 112]);
        arithmetic(&mut p, form, true);
        for failure in [0x83, 0x8b, 0x84, 0x89, 0x80] {
            p.code.extend([0x0f, failure, 0, 0, 0, 0]);
            p.branches.push(p.code.len() - 4);
        }
        p.code.extend([0x48, 0xff, 0xc9]);
        let relative = repeat as i32 - (p.code.len() as i32 + 6);
        p.code.extend([0x0f, 0x85]);
        p.code.extend(relative.to_le_bytes());
        snapshot(&mut p, result(form));
        check_vector(&mut p, 9, result(form));
        check_vector(&mut p, 14, result(form));
        check_vector(&mut p, 13, operand(form, 6));
    }
    rip(&mut p.code, 0x1000, &[0xff, 0x15], 0xf00);
    p.expect(1);
    let last = *forms().last().unwrap();
    for register in 0..16 {
        check_vector(
            &mut p,
            register,
            match register {
                9 | 14 => result(last),
                13 => operand(last, 6),
                _ => pattern(register),
            },
        );
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
fn native_float_min_max_all_forms_keep_real_memory_modeling_service_state_and_exact_counts() {
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
    assert_eq!(results[0]["native_ymm_mode"], results[1]["native_ymm_mode"]);
    let native = match results[0]["native_ymm_mode"].as_str() {
        "avx" | "avx2" => true,
        "sse2" => false,
        _ => panic!("unknown closed vector mode"),
    };
    let count = |index: usize, key: &str| results[index][key].parse::<u64>().unwrap();
    for (index, iterations) in [1, 3].into_iter().enumerate() {
        let copies = 68 + 48 * iterations;
        assert_eq!(
            count(index, "vector_ops"),
            2 + 12 * iterations + u64::from(!native) * (24 * iterations + copies)
        );
        assert_eq!(
            count(index, "vector_bytes"),
            8 + 96 * iterations + u64::from(!native) * (216 * iterations + 32 * copies)
        );
    }
    for key in ["native_instructions", "primary_native_instructions"] {
        assert_eq!(count(1, key) - count(0, key), 96 + u64::from(native) * 144);
    }
    println!(
        "native_float_min_max_worker_available={native} native_float_min_max_worker_codes=12 native_float_min_max_worker_shapes=24 native_float_min_max_worker_programs=2"
    );
}
