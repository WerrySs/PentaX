// SPDX-License-Identifier: GPL-2.0-only
//! Original consumers inspect real register packets, patches and grow bodies.
use super::*;

const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const SAVED: u64 = 0x6b94_38c1_d752_a0ef;
const CALLBACK: i32 = 0x1100;

fn stack(p: &mut Program, opcode: u8, reg: u8, offset: u8) {
    if offset <= 127 {
        p.stack(opcode, reg, offset);
    } else {
        p.code.extend([
            0x48 | if reg >= 8 { 4 } else { 0 },
            opcode,
            0x84 | ((reg & 7) << 3),
            0x24,
        ]);
        p.code.extend(u32::from(offset).to_le_bytes());
    }
}

fn put_stack(p: &mut Program, offset: u8, value: u64) {
    p.reg(0, value);
    stack(p, 0x89, 0, offset);
}

fn pointer(p: &mut Program, reg: u8, offset: u8) {
    stack(p, 0x8d, reg, offset);
}

fn same_pointer(p: &mut Program, offset: u8) {
    pointer(p, 10, offset);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
}

fn base() -> Program {
    let mut p = Program::new();
    stack(&mut p, 0x89, 4, 240);
    stack(&mut p, 0x89, 12, 248);
    p.reg(12, SAVED);
    for offset in [
        0, 64, 72, 80, 88, 96, 104, 112, 120, 128, 136, 144, 152, 224, 232,
    ] {
        put_stack(&mut p, offset, CANARY);
    }
    for (field, target) in [(8, 80), (16, 144), (24, 80), (32, 144)] {
        pointer(&mut p, 0, target);
        stack(&mut p, 0x89, 0, field);
    }
    for offset in [40, 48, 56] {
        put_stack(&mut p, offset, 0);
    }
    p
}

fn finish(mut p: Program) -> (Vec<u8>, u64) {
    for offset in [0, 64, 72, 144, 152, 224, 232] {
        stack(&mut p, 0x8b, 0, offset);
        p.expect(CANARY);
    }
    p.code.extend([0x48, 0x89, 0xe0]);
    stack(&mut p, 0x8b, 10, 240);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x4c, 0x89, 0xe0]);
    p.expect(SAVED);
    stack(&mut p, 0x8b, 12, 248);
    p.finish(None)
}

fn packet_words(p: &mut Program, words: &[u32]) {
    assert!(words.len() <= 12);
    let mut expected = [0xa5_u8; 64];
    for (bytes, value) in expected.chunks_exact_mut(4).zip(words) {
        bytes.copy_from_slice(&value.to_le_bytes());
    }
    for (index, bytes) in expected.chunks_exact(8).enumerate() {
        let offset = 80 + index as u8 * 8;
        stack(p, 0x8b, 0, offset);
        p.expect(u64::from_le_bytes(bytes.try_into().unwrap()));
    }
}

fn check(code: &[u8], nids: &[&str], scope: &str, calls: u64, callback: Option<&[u8]>) {
    let mut source = scoped_system_image(code, nids, scope, scope, 1);
    if let Some(callback) = callback {
        let mut bytes = source.bytes.clone();
        bytes.resize(0x1600, 0);
        put(&mut bytes, 96, &0x1600_u64.to_le_bytes());
        assert!(CALLBACK as usize + callback.len() <= bytes.len());
        put(&mut bytes, CALLBACK as usize, callback);
        source = create_input(bytes);
    }
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
    assert_eq!(fields["graphics_calls"], calls.to_string(), "{text}");
    assert_eq!(fields["initializer_calls"], "0", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
}

fn packet(nid: &str, kind: u8, opcode: u32, scope: &str) {
    let mut p = base();
    // Independent original register values and non-consecutive third offset.
    put_stack(&mut p, 160, (0x1111_2222_u64 << 32) | 10);
    put_stack(&mut p, 168, (0x3333_4444_u64 << 32) | 11);
    put_stack(&mut p, 176, (0x5555_6666_u64 << 32) | 20);
    pointer(&mut p, 7, 8);
    let expected = match kind {
        0 => {
            p.reg(6, 0x6a5b_c4d3_0001_02c5);
            vec![0xc001_0000 | (opcode << 8), 0x2c5, 0x6a5b_c4d3]
        }
        1 | 2 => {
            p.reg(6, 0x12345);
            if kind == 1 {
                pointer(&mut p, 2, 160);
            } else {
                p.reg(2, 0);
            }
            p.reg(1, 2);
            let values = if kind == 1 {
                [10, 0x1111_2222]
            } else {
                [0xa5a5_a5a5; 2]
            };
            vec![0xc002_7600, 0x2345, values[0], values[1]]
        }
        3 => {
            pointer(&mut p, 6, 160);
            p.reg(2, 3);
            vec![
                0xc002_0000 | (opcode << 8),
                10,
                0x1111_2222,
                0x3333_4444,
                0xc001_0000 | (opcode << 8),
                20,
                0x5555_6666,
            ]
        }
        4 | 5 => {
            p.reg(6, if kind == 4 { 0x0000_1234_5678_9abf } else { 0 });
            p.reg(2, if kind == 4 { 0x8000_4003 } else { 0 });
            vec![
                0xc003_0000 | (opcode << 8),
                if kind == 4 { 0x5678_9abc } else { 0 },
                if kind == 4 { 0x1234 } else { 0 },
                0x8000_0000,
                if kind == 4 { 3 } else { 0 },
            ]
        }
        _ => unreachable!(),
    };
    p.call(0);
    same_pointer(&mut p, 80);
    p.load(0, 24);
    same_pointer(&mut p, 80 + expected.len() as u8 * 4);
    packet_words(&mut p, &expected);
    let (code, calls) = finish(p);
    check(&code, &[nid], scope, calls, None);
}

#[test]
fn native_agc_register_builders_publish_actual_packets_in_both_exact_pairs() {
    for scope in ["Graphics5", "libSceAgc"] {
        for (nid, opcode) in [
            ("pFLArOT53+w", 0x76),
            ("LHFXRrlTPD8", 0x69),
            ("w4-d0n60hdo", 0x79),
        ] {
            packet(nid, 0, opcode, scope);
        }
        packet("n2fD4A+pb+g", 1, 0x76, scope);
        packet("n2fD4A+pb+g", 2, 0x76, scope);
        for (nid, opcode) in [("UZbQjYAwwXM", 0x76), ("03RZmELWWzw", 0x79)] {
            packet(nid, 3, opcode, scope);
        }
        for (nid, opcode) in [
            ("ZvwO9euwYzc", 0x9f),
            ("-HOOCn0JY48", 0x63),
            ("hvUfkUIQcOE", 0x64),
        ] {
            packet(nid, 4, opcode, scope);
            packet(nid, 5, opcode, scope);
        }
    }
    println!(
        "agc_register_packets_native_available=true agc_register_packets_native_programs=26 agc_register_packets_native_calls=26"
    );
}

const PATCHES: [(&str, u32, u8); 9] = [
    ("vcmNN+AAXnY", 0x9f, 0),
    ("Qrj4c+61z4A", 0x63, 0),
    ("6lNcCp+fxi4", 0x64, 0),
    ("whb1RL7K4Ss", 0x9f, 1),
    ("nCUgItdN2ms", 0x63, 1),
    ("fRG-JOH5+sI", 0x64, 1),
    ("d-6uF9sZDIU", 0x9f, 2),
    ("z2duB-hHQSM", 0x63, 2),
    ("vRoArM9zaIk", 0x64, 2),
];

#[test]
fn native_agc_register_patches_preserve_bits_and_reject_other_opcodes_in_both_pairs() {
    for scope in ["Graphics5", "libSceAgc"] {
        for (nid, opcode, kind) in PATCHES {
            for matches in [true, false] {
                let mut p = base();
                // Deliberately irregular header: the public patch matcher checks
                // only opcode bits, independently of the command decoder.
                let actual_op = if matches { opcode } else { 0x10 };
                let mut expected = [
                    0xdead_00be | (actual_op << 8),
                    0x89ab_cde3,
                    0xface_1234,
                    0xa5a5_a5a5,
                    0xcafe_3ffe,
                ];
                for (index, pair) in expected.chunks(2).enumerate() {
                    let value = u64::from(pair[0])
                        | (u64::from(*pair.get(1).unwrap_or(&0xa5a5_a5a5)) << 32);
                    put_stack(&mut p, 80 + index as u8 * 8, value);
                }
                pointer(&mut p, 7, 80);
                p.reg(
                    6,
                    if kind == 0 {
                        0x0000_1234_5678_9abc
                    } else if kind == 1 {
                        0x8000_4003
                    } else {
                        5
                    },
                );
                p.call(0);
                p.expect(if matches { 0 } else { 0x8a6c_000c });
                if matches {
                    if kind == 0 {
                        expected[1] = 0x5678_9abf;
                        expected[2] = 0x1234;
                    } else {
                        expected[4] = 0xcafe_0003;
                    }
                }
                packet_words(&mut p, &expected);
                p.load(0, 24);
                same_pointer(&mut p, 80); // Patches never allocate.
                let (code, calls) = finish(p);
                check(&code, &[nid], scope, calls, None);
            }
        }
    }
    println!(
        "agc_register_patches_native_available=true agc_register_patches_native_programs=36 agc_register_patches_native_calls=36"
    );
}

#[test]
fn native_agc_register_size_queries_keep_source_integer_widths_in_both_pairs() {
    for scope in ["Graphics5", "libSceAgc"] {
        for (nid, arg, expected) in [
            ("bxGoVxpdSPQ", 0x4000_0000, 8),
            ("1DeUNpRIDDA", u64::MAX, 12),
            ("GBCh3zCihoU", u64::MAX, 20),
            ("nNlUtdDDvZ0", u64::MAX, 20),
        ] {
            let mut p = base();
            p.reg(7, arg);
            p.call(0);
            p.expect(expected);
            let (code, calls) = finish(p);
            check(&code, &[nid], scope, calls, None);
        }
    }
    println!(
        "agc_register_sizes_native_available=true agc_register_sizes_native_programs=8 agc_register_sizes_native_calls=8"
    );
}

fn callback(fail_middle: bool) -> Vec<u8> {
    let mut code = vec![
        0x41, 0x54, 0x48, 0x83, 0xec, 16, // Save R12 and retain SysV alignment.
        0x48, 0x89, 0x3c, 0x24, // Save descriptor across the nested library call.
        0x49, 0x89, 0xd4, // R12 = user data.
        0x48, 0x8b, 0x42, 8, 0x48, 0x39, 0xc7, // Actual arg0 is our descriptor.
    ];
    let mut branches = Vec::new();
    checked(&mut code, &mut branches);
    code.extend([0x48, 0x83, 0xfe, 3]); // Each non-consecutive run needs 3 DW.
    checked(&mut code, &mut branches);
    code.extend([
        0x48, 0xff, 0x02, // Count genuine callbacks in current guest memory.
        0x48, 0x8b, 0x42, 16, // Read the original input-array pointer.
        0xc7, 0x40, 12, 0x5c, 0, 0, 0, // Mutate a later pair's value.
        0xc7, 0x40, 20, 0x6d, 0, 0, 0,
    ]);
    if fail_middle {
        // A failed middle grow leaves capacity unchanged. UC must continue
        // and really invoke a third callback; SH must end after the second.
        code.extend([0x49, 0x83, 0x3c, 0x24, 2, 0x74, 12]);
    }
    code.extend([
        0x48, 0x8b, 0x47, 16, // Current UP, not a saved/pre-callback cursor.
        0x48, 0x83, 0xc0, 12, 0x48, 0x89, 0x47, 24, 0xbf, 3, 0, 0, 0,
    ]);
    rip(&mut code, CALLBACK, &[0xff, 0x15], 0xf08); // Genuine nested NopGetSize.
    code.extend([0x48, 0x83, 0xf8, 12]);
    checked(&mut code, &mut branches);
    code.extend([0x48, 0x8b, 0x3c, 0x24, 0x4c, 0x89, 0xe2, 0x48, 0xb8]);
    code.extend(0x5a5a_5a5a_5a5a_0001_u64.to_le_bytes());
    if fail_middle {
        code.extend([0x49, 0x83, 0x3c, 0x24, 2, 0x75, 10, 0x48, 0xb8]);
        // False AL with poisoned upper bytes must remain false.
        code.extend(0x5a5a_5a5a_5a5a_0000_u64.to_le_bytes());
    }
    code.extend([0x48, 0x83, 0xc4, 16, 0x41, 0x5c, 0xc3]);
    finish_checks(&mut code, branches);
    code
}

fn grows(scope: &str, uc: bool, fail_middle: bool) -> u64 {
    let mut p = base();
    for (offset, selector, value) in [
        (160, 10, 0x1111_2222_u32),
        (168, 20, 0x3333_4444),
        (176, 30, 0x5555_6666),
    ] {
        put_stack(&mut p, offset, (u64::from(value) << 32) | selector);
    }
    pointer(&mut p, 0, 80);
    stack(&mut p, 0x89, 0, 32); // No initial capacity: first grow is unavoidable.
    p.path(0, CALLBACK);
    stack(&mut p, 0x89, 0, 40);
    pointer(&mut p, 0, 192);
    stack(&mut p, 0x89, 0, 48);
    put_stack(&mut p, 192, 0);
    pointer(&mut p, 0, 8);
    stack(&mut p, 0x89, 0, 200);
    pointer(&mut p, 0, 160);
    stack(&mut p, 0x89, 0, 208);
    pointer(&mut p, 7, 8);
    pointer(&mut p, 6, 160);
    p.reg(2, 3);
    p.call(0);
    same_pointer(&mut p, 80); // Grouped calls return their FIRST actual packet.
    let header = if uc { 0xc001_7900 } else { 0xc001_7600 };
    let mut expected = vec![header, 10, 0x1111_2222];
    if !fail_middle {
        expected.extend([header, 20, if uc { 0x5c } else { 0x3333_4444 }]);
    }
    if !fail_middle || uc {
        expected.extend([header, 30, if uc { 0x6d } else { 0x5555_6666 }]);
    }
    let callbacks = if fail_middle && !uc { 2 } else { 3 };
    stack(&mut p, 0x8b, 0, 192);
    p.expect(callbacks);
    p.load(0, 24);
    same_pointer(&mut p, 80 + expected.len() as u8 * 4);
    packet_words(&mut p, &expected);
    let (code, calls) = finish(p);
    let nid = if uc { "03RZmELWWzw" } else { "UZbQjYAwwXM" };
    check(
        &code,
        &[nid, "t7PlZ9nt5Lc"],
        scope,
        calls + callbacks,
        Some(&callback(fail_middle)),
    );
    calls + callbacks
}

#[test]
fn native_agc_register_runs_use_fresh_genuine_callbacks_and_source_snapshot_timing() {
    let mut calls = 0;
    for scope in ["Graphics5", "libSceAgc"] {
        for uc in [false, true] {
            for fail_middle in [false, true] {
                calls += grows(scope, uc, fail_middle);
            }
        }
    }
    assert_eq!(calls, 30);
    println!(
        "agc_register_callbacks_native_available=true agc_register_callbacks_native_programs=8 agc_register_callbacks_native_calls=30"
    );
}
