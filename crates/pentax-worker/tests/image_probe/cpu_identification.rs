// SPDX-License-Identifier: GPL-2.0-only
//! Original guest queries for the conservative software CPU profile.
//! No host identity, hardware CPUID passthrough or external guest data is used.

use super::*;

fn set_word(program: &mut Program, offset: u8, value: u64) {
    program.reg(0, value);
    program.save(offset);
}

fn preserved_flags(program: &mut Program, pattern: usize, before: bool) {
    // Directly observable CF/PF/ZF/SF/OF patterns, including signed overflow.
    // Full RFLAGS (including AF) equality belongs to the platform context test;
    // this fixture does not introduce unsupported PUSHFQ/LAHF instructions.
    let (left, right, failures) = match pattern % 4 {
        0 => (1, 2, [0x83, 0x8b, 0x84, 0x89, 0x80]),
        1 => (0x8000_0000_0000_0000, 1, [0x82, 0x8b, 0x84, 0x88, 0x81]),
        2 => (1, 1, [0x82, 0x8b, 0x85, 0x88, 0x80]),
        _ => (3, 1, [0x82, 0x8a, 0x84, 0x88, 0x80]),
    };
    if before {
        program.reg(0, left);
        program.reg(2, right);
        program.code.extend([0x48, 0x39, 0xd0]); // CMP RAX,RDX
    } else {
        for condition in failures {
            program.code.extend([0x0f, condition, 0, 0, 0, 0]);
            program.branches.push(program.code.len() - 4);
        }
    }
}

#[test]
fn native_cpuid_consumes_software_leaves_subleaves_and_preserves_unrelated_context() {
    // Expected words are independent profile assertions, not values obtained
    // from the implementation or the host. EBX/ECX/EDX order is architectural.
    let queries: [(u32, u32, [u32; 4]); 10] = [
        (0, 0xdead_beef, [7, 0x746e_6550, 0x5550_4375, 0x6d45_5861]),
        (1, 0xdead_beef, [0, 0, 0x0080_0000, 0x0600_8100]),
        (7, 0, [0; 4]),
        (7, 1, [0; 4]),
        (7, u32::MAX, [0; 4]),
        (0x8000_0000, 0xdead_beef, [0x8000_0001, 0, 0, 0]),
        (0x8000_0001, 0xdead_beef, [0, 0, 0x20, 0x2000_0000]),
        (2, 0, [0; 4]),                  // Unimplemented in-range basic leaf.
        (8, 0x1234, [0; 4]),             // Out of range normalizes to basic leaf7.
        (0x8000_0002, u32::MAX, [0; 4]), // No brand-string claim.
    ];
    let vectors = [
        0x1122_3344_5566_7788,
        0x7ff0_0000_0000_0001,
        0x99aa_bbcc_ddee_ff00,
        0x8000_0000_7fc0_1234,
    ];
    let unrelated = [
        (6, 0x0606_a5a5_0000_0011),
        (7, 0x0707_a5a5_0000_0022),
        (8, 0x0808_a5a5_0000_0033),
        (9, 0x0909_a5a5_0000_0044),
        (11, 0x1111_a5a5_0000_0055),
        (12, 0x1212_a5a5_0000_0066),
        (13, 0x1313_a5a5_0000_0077),
        (14, 0x1414_a5a5_0000_0088),
        (15, 0x1515_a5a5_0000_0099),
    ];
    let mut program = Program::new();
    // The same real stack/frame pointers must survive every mediated query.
    program.stack(0x89, 4, 112);
    program.stack(0x89, 5, 120);
    set_word(&mut program, 56, 0x1f84);
    program.code.extend([0x0f, 0xae, 0x54, 0x24, 56]); // LDMXCSR
    for (index, value) in vectors.into_iter().enumerate() {
        set_word(&mut program, 64 + index as u8 * 8, value);
    }
    program.code.extend([0xc5, 0x7e, 0x6f, 0x7c, 0x24, 64]); // VMOVDQU YMM15
    for (register, value) in unrelated {
        program.reg(register, value);
    }
    for (index, (leaf, subleaf, expected)) in queries.into_iter().enumerate() {
        preserved_flags(&mut program, index, true);
        // CPUID consumes only low EAX/ECX and zero-extends all four results.
        program.reg(0, 0xa5a5_5a5a_0000_0000 | u64::from(leaf));
        program.reg(1, 0x5a5a_a5a5_0000_0000 | u64::from(subleaf));
        program.reg(2, u64::MAX);
        program.reg(3, u64::MAX);
        program.reg(10, 0x1010_a5a5_0000_00aa);
        program.code.extend([0x0f, 0xa2]);
        preserved_flags(&mut program, index, false);
        // MOV stores leave the result and flags intact until all are captured.
        for (register, offset) in [(0, 0), (3, 8), (1, 16), (2, 24), (10, 40)] {
            program.stack(0x89, register, offset);
        }
        program.word(40, 0x1010_a5a5_0000_00aa);
        for (word, expected) in expected.into_iter().enumerate() {
            let offset = word as u8 * 8;
            if word == 3 && matches!(leaf, 1 | 0x8000_0001) {
                // TSC/RDTSCP are the sole conditional advertised bits. They
                // must agree with each other; no host identity is consulted.
                let bit = if leaf == 1 { 4 } else { 27 };
                program.load(0, offset);
                program.reg(10, !(1_u64 << bit));
                program.code.extend([0x4c, 0x21, 0xd0]); // AND RAX,R10
                program.expect(u64::from(expected));
                program.load(0, offset);
                program
                    .code
                    .extend([0x48, 0xc1, 0xe8, bit, 0x48, 0x83, 0xe0, 1]);
                if leaf == 1 {
                    program.save(104);
                } else {
                    program.load(10, 104);
                    program.code.extend([0x4c, 0x39, 0xd0]);
                    checked(&mut program.code, &mut program.branches);
                }
            } else {
                program.word(offset, u64::from(expected));
            }
        }
    }
    for (register, value) in unrelated {
        program.code.extend([
            0x48 | if register >= 8 { 4 } else { 0 },
            0x89,
            0xc0 | ((register & 7) << 3),
        ]); // MOV RAX,register
        program.expect(value);
    }
    for (register, offset) in [(4, 112), (5, 120)] {
        program.code.extend([0x48, 0x89, 0xc0 | (register << 3)]);
        program.load(10, offset);
        program.code.extend([0x4c, 0x39, 0xd0]);
        checked(&mut program.code, &mut program.branches);
    }
    program.code.extend([0x0f, 0xae, 0x5c, 0x24, 56]); // STMXCSR
    program.word(56, 0x1f84);
    program.code.extend([0xc5, 0x7e, 0x7f, 0x7c, 0x24, 64]);
    for (index, value) in vectors.into_iter().enumerate() {
        program.word(64 + index as u8 * 8, value);
    }
    program.reg(0, 42);
    program
        .code
        .extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut program.code, program.branches);
    assert!(program.code.len() < 0xf00); // Bounded original raw ELF code segment.
    let original = input(&program.code, false);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["integer_ops"], queries.len().to_string(), "{text}");
    assert_eq!(fields["integer_bytes"], "0", "{text}");
    assert_eq!(fields["vector_ops"], "4", "{text}");
    assert_eq!(fields["vector_bytes"], "72", "{text}");
    assert_eq!(fields["fp_status"], "4", "{text}");
    assert_eq!(fields["service_stops"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert!(
        fields["native_instructions"].parse::<u64>().unwrap() < 1500,
        "{text}"
    );
    assert!(
        fields["block_attempts"].parse::<u64>().unwrap() < 1500,
        "{text}"
    );
}
