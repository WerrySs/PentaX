// SPDX-License-Identifier: GPL-2.0-only
//! All supported VEX broadcast shapes cross actual memory, modeling and service.
use super::*;

#[derive(Clone, Copy)]
struct Form {
    opcode: u8,
    tuple: usize,
    wide: bool,
    memory: bool,
    avx2: bool,
}
fn forms() -> Vec<Form> {
    let mut forms = Vec::new();
    for (opcode, tuple) in [
        (0x78, 1),
        (0x79, 2),
        (0x58, 4),
        (0x59, 8),
        (0x18, 4),
        (0x19, 8),
    ] {
        for wide in [false, true] {
            if opcode == 0x19 && !wide {
                continue;
            }
            for memory in [false, true] {
                forms.push(Form {
                    opcode,
                    tuple,
                    wide,
                    memory,
                    avx2: !matches!(opcode, 0x18 | 0x19) || !memory,
                });
            }
        }
    }
    for opcode in [0x1a, 0x5a] {
        forms.push(Form {
            opcode,
            tuple: 16,
            wide: true,
            memory: true,
            avx2: opcode == 0x5a,
        });
    }
    assert_eq!(forms.len(), 24);
    forms
}
fn source() -> [u64; 4] {
    [
        0x7fa0_0001_8765_4321,
        0x8000_0000_0000_0000,
        0x0123_4567_89ab_cdef,
        0xfedc_ba98_7654_3210,
    ]
}
fn result(form: Form) -> [u64; 4] {
    let mut input = [0; 32];
    for (index, word) in source().into_iter().enumerate() {
        input[index * 8..index * 8 + 8].copy_from_slice(&word.to_le_bytes());
    }
    let mut output = [0; 32];
    let width = if form.wide { 32 } else { 16 };
    for lane in output[..width].chunks_exact_mut(form.tuple) {
        lane.copy_from_slice(&input[..form.tuple]);
    }
    std::array::from_fn(|index| {
        u64::from_le_bytes(output[index * 8..index * 8 + 8].try_into().unwrap())
    })
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
fn copy(p: &mut Program, load: bool, register: u8, offset: u32) {
    p.code.extend([
        0xc5,
        if register >= 8 { 0x7c } else { 0xfc },
        if load { 0x10 } else { 0x11 },
        (if offset <= 127 { 0x44 } else { 0x84 }) | ((register & 7) << 3),
        0x24,
    ]);
    if offset <= 127 {
        p.code.push(offset as u8);
    } else {
        p.code.extend(offset.to_le_bytes());
    }
}
fn snapshot(p: &mut Program, offset: u32, words: [u64; 4]) {
    for (index, value) in words.into_iter().enumerate() {
        p.code.extend([0x48, 0x8b, 0x84, 0x24]);
        p.code.extend((offset + index as u32 * 8).to_le_bytes());
        p.expect(value);
    }
}
fn broadcast(p: &mut Program, form: Form) {
    // XMM/YMM9 receives either exact-width [RSP+16] or low bits of XMM13.
    // VEX.vvvv is reserved1111; opcode/L select the original decoder Code.
    p.code.extend([
        0xc4,
        if form.memory { 0x62 } else { 0x42 },
        if form.wide { 0x7d } else { 0x79 },
        form.opcode,
        if form.memory { 0x4c } else { 0xcd },
    ]);
    if form.memory {
        p.code.extend([0x24, 16]);
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
    fill(&mut p, 16, source());
    fill(&mut p, 48, [u64::MAX; 4]);
    for form in forms() {
        p.reg(1, iterations);
        let repeat = p.code.len();
        copy(&mut p, true, 13, 16);
        copy(&mut p, true, 9, 48);
        p.reg(0, 1);
        p.reg(2, 2);
        p.code.extend([0x48, 0x39, 0xd0]);
        broadcast(&mut p, form);
        copy(&mut p, false, 9, 112);
        // A genuinely modeled store lies between the broadcast output and a
        // subsequent native memory consumer. It repeats the same low qword.
        p.code.extend([0xc5, 0x79, 0xd6, 0x4c, 0x24, 112]);
        copy(&mut p, true, 14, 112);
        copy(&mut p, false, 14, 144);
        for failure in [0x83, 0x8b, 0x84, 0x89, 0x80] {
            p.code.extend([0x0f, failure, 0, 0, 0, 0]);
            p.branches.push(p.code.len() - 4);
        }
        p.code.extend([0x48, 0xff, 0xc9]);
        let relative = repeat as i32 - (p.code.len() as i32 + 6);
        p.code.extend([0x0f, 0x85]);
        p.code.extend(relative.to_le_bytes());
        snapshot(&mut p, 144, result(form));
    }
    rip(&mut p.code, 0x1000, &[0xff, 0x15], 0xf00);
    p.expect(1);
    let last = *forms().last().unwrap();
    for register in 0..16 {
        copy(&mut p, false, register, 112);
        snapshot(
            &mut p,
            112,
            match register {
                9 | 14 => result(last),
                13 => source(),
                _ => pattern(register),
            },
        );
    }
    snapshot(&mut p, 16, source());
    snapshot(&mut p, 48, [u64::MAX; 4]);
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
fn native_vector_broadcasts_all_shapes_keep_exact_bits_memory_boundaries_service_state_and_counts()
{
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
    let mode = results[0]["native_ymm_mode"].as_str();
    assert!(matches!(mode, "sse2" | "avx" | "avx2"));
    let native = mode != "sse2";
    let supported = |form: &Form| native && (!form.avx2 || mode == "avx2");
    let native_shapes = forms().iter().filter(|form| supported(form)).count() as u64;
    let modeled_shapes = 24 - native_shapes;
    let modeled_bytes: u64 = forms()
        .iter()
        .filter(|form| form.memory && !supported(form))
        .map(|form| form.tuple as u64)
        .sum();
    let count = |index: usize, key: &str| results[index][key].parse::<u64>().unwrap();
    for (index, iterations) in [1, 3].into_iter().enumerate() {
        let copies = 32 + 120 * iterations;
        assert_eq!(
            count(index, "vector_ops"),
            2 + 24 * iterations + modeled_shapes * iterations + u64::from(!native) * copies
        );
        assert_eq!(
            count(index, "vector_bytes"),
            8 + 192 * iterations + modeled_bytes * iterations + u64::from(!native) * 32 * copies
        );
    }
    for key in ["native_instructions", "primary_native_instructions"] {
        assert_eq!(
            count(1, key) - count(0, key),
            192 + u64::from(native) * 240 + native_shapes * 2
        );
    }
    println!(
        "native_vector_broadcast_worker_available={native} native_vector_broadcast_worker_shapes=24 native_vector_broadcast_worker_programs=2 native_vector_broadcast_worker_native_shapes={native_shapes}"
    );
}
