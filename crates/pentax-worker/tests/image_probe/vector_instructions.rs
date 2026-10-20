// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn native_x87_controls_retain_guest_environment_across_service_calls() {
    let mut p = Program::new();
    for slot in [16, 24, 40, 48, 56] {
        p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
        p.save(slot);
    }
    p.code.extend([0xd9, 0x7c, 0x24, 16]); // FNSTCW: guest default, exact word.
    p.word(16, 0xa5a5_a5a5_a5a5_037f);
    p.code.extend([0xdd, 0x7c, 0x24, 24]); // FNSTSW: guest status, not host status.
    p.word(24, 0xa5a5_a5a5_a5a5_0000);
    p.code.push(0x9b); // WAIT checks retained guest exceptions.
    p.reg(0, u64::MAX);
    p.save(32);
    p.code.extend([0xd9, 0x6c, 0x24, 32]); // FLDCW normalizes reserved bits.
    p.call(0);
    p.expect(1);
    p.code.extend([0x9b, 0xd9, 0x7c, 0x24, 40]); // FSTCW, after real service return.
    p.word(40, 0xa5a5_a5a5_a5a5_1f7f);
    p.reg(0, 0x1234_5678_9abc_def0);
    p.code.extend([0xdf, 0xe0]); // FNSTSW AX preserves the upper 48 RAX bits.
    p.expect(0x1234_5678_9abc_0000);
    p.code.extend([0x9b, 0xdf, 0xe0]); // FSTSW AX.
    p.expect(0x1234_5678_9abc_0000);
    p.code.extend([0xdb, 0xe2, 0x9b, 0xdb, 0xe2]); // FNCLEX and FCLEX.
    p.code.extend([0xdb, 0xe3]); // FNINIT resets environment independently of MXCSR.
    p.code.extend([0xd9, 0x7c, 0x24, 56]);
    p.word(56, 0xa5a5_a5a5_a5a5_037f);
    p.code.extend([0x9b, 0xdb, 0xe3]); // FINIT.
    p.code.extend([0x9b, 0xdd, 0x7c, 0x24, 48]); // FSTSW m16.
    p.word(48, 0xa5a5_a5a5_a5a5_0000);
    let (code, calls) = p.finish(None);
    let getpid = pentax_kernel::Service::registrations()
        .iter()
        .find(|registration| registration.service == pentax_kernel::Service::Getpid)
        .unwrap();
    let (status, fields, text) = probe(&image(&code, &[getpid.nid]));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert_eq!(fields["fp_status"], "0");
    assert_eq!(fields["vector_bytes"], "12");
}

#[test]
fn native_mxcsr_transfers_control_rounding_and_preserve_store_width() {
    let mut p = Program::new();
    p.set(16, 0);
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 16]); // STMXCSR original guest control.
    p.word(16, 0x1f80);
    p.reg(0, 0x5f80); // Round toward positive infinity.
    p.save(24);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 24]);
    p.reg(0, 16_777_217);
    p.code.extend([0xc5, 0xf2, 0x2a, 0xc0]); // VCVTSI2SS XMM0,XMM1,EAX.
    p.code.extend([0xc5, 0xf9, 0x7e, 0xc0]); // VMOVD EAX,XMM0.
    p.expect(0x4b80_0001);
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(32);
    p.code.extend([0xc5, 0xf8, 0xae, 0x5c, 0x24, 32]);
    p.word(32, 0xa5a5_a5a5_0000_5fa0); // Sticky precision, only four bytes stored.
    p.reg(0, 0x9f80); // Flush-to-zero, nearest rounding and cleared status.
    p.save(24);
    p.code.extend([0xc5, 0xf8, 0xae, 0x54, 0x24, 24]);
    p.set(48, 0);
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 48]);
    p.word(48, 0x9f80);
    p.code.extend([0xc5, 0xf8, 0xae, 0x54, 0x24, 16]); // Restore guest control.
    p.set(56, 0);
    p.code.extend([0xc5, 0xf8, 0xae, 0x5c, 0x24, 56]);
    p.word(56, 0x1f80);
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["fp_status"], "0");
    assert_eq!(fields["vector_ops"], "9");
    assert_eq!(fields["vector_bytes"], "28");
}

#[test]
fn native_vector_shifts_keep_count_widths_signs_and_lane_boundaries() {
    let source_words = [
        0x8000_0001_ffff_ffff_u64,
        0x7fff_ffff_1234_5678,
        0x8000_0000_0000_0001,
        0xfedc_ba98_7654_3210,
    ];
    // Shared counts use only the low qword; element counts use each dword/qword.
    let counts = [1_u64, 0x0000_0020_0000_001f, 63, u64::MAX];
    let right = [
        0x4000_0000_7fff_ffff_u64,
        0x3fff_ffff_091a_2b3c,
        0x4000_0000_0000_0000,
        0x7f6e_5d4c_3b2a_1908,
    ];
    let expected = [
        right,
        right,
        [
            0x8000_0001_ffff_ffff,
            0,
            0x8000_0000_0000_0000,
            0xffff_ffff_0000_0000,
        ],
        [
            0xff00_0000_0000_0000,
            0x7880_0000_01ff_ffff,
            0x0100_0000_0000_0000,
            0x1080_0000_0000_0000,
        ],
        [0; 4],
        [0x0000_0003_ffff_fffe, 0, 0x8000_0000_0000_0000, 0],
    ];
    for wide in [false, true] {
        let mut p = Program::new();
        for (base, words) in [(16, source_words), (48, counts)] {
            for (lane, word) in words.into_iter().enumerate() {
                p.reg(0, word);
                p.save(base + lane as u8 * 8);
            }
        }
        p.code.extend([0xc5, 0xfc, 0x10, 0x4c, 0x24, 16]);
        p.code.extend([0xc5, 0xfc, 0x10, 0x54, 0x24, 48]);
        let l = if wide { 4 } else { 0 };
        let instructions: [&[u8]; 6] = [
            &[0xc4, 0xe1, 0x61 | l, 0x72, 0xd1, 1], // VPSRLD immediate.
            &[0xc4, 0xe1, 0x71 | l, 0xd2, 0x5c, 0x24, 48], // VPSRLD m128 count.
            &[0xc4, 0xe2, 0x71 | l, 0x46, 0x5c, 0x24, 48], // VPSRAVD element counts.
            &[0xc4, 0xe1, 0x61 | l, 0x73, 0xf9, 7], // VPSLLDQ within 128-bit lanes.
            &[0xc4, 0xe1, 0x61 | l, 0x73, 0xd9, 17], // VPSRLDQ saturates to zero.
            &[0xc4, 0xe2, 0xf1 | l, 0x47, 0xda],    // VPSLLVQ register counts.
        ];
        for (instruction, words) in instructions.into_iter().zip(expected) {
            p.code.extend_from_slice(instruction);
            p.code.extend([0xc5, 0xfc, 0x11, 0x5c, 0x24, 80]);
            for (lane, word) in words.into_iter().enumerate() {
                p.word(80 + lane as u8 * 8, if wide || lane < 2 { word } else { 0 });
            }
        }
        let (code, _) = p.finish(None);
        let (status, fields, text) = probe(&input(&code, false));
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["fp_status"], "0");
        assert_eq!(fields["vector_ops"], "14");
        assert_eq!(fields["vector_bytes"], if wide { "304" } else { "288" });
    }
}

#[test]
fn native_vector_blends_keep_element_masks_and_upper_lane_rules() {
    let left = [
        0x0706_0504_0302_0100_u64,
        0x7fc0_0001_7f80_0001,
        0x8000_0000_0000_0000,
        u64::MAX,
    ];
    let mut a = [0; 32];
    let mut b = [0; 32];
    let mut mask = [0x7f; 32];
    for (i, word) in left.into_iter().enumerate() {
        a[i * 8..i * 8 + 8].copy_from_slice(&word.to_le_bytes());
    }
    for i in 0..32 {
        b[i] = !a[i];
        if i % 3 == 0 {
            mask[i] = 0x80;
        }
    }
    let forms = [
        (0x02, 4, false, false),
        (0x0e, 2, true, false),
        (0x0c, 4, false, false),
        (0x0d, 8, false, false),
        (0x4c, 1, false, true),
        (0x4a, 4, false, true),
        (0x4b, 8, false, true),
    ];
    for wide in [false, true] {
        let mut p = Program::new();
        for (base, bytes) in [(16, a), (48, b), (80, mask)] {
            for (i, word) in bytes.chunks_exact(8).enumerate() {
                p.reg(0, u64::from_le_bytes(word.try_into().unwrap()));
                p.save(base + i as u8 * 8);
            }
        }
        for (register, offset) in [(1, 16), (2, 48), (3, 80)] {
            p.code
                .extend([0xc5, 0xfc, 0x10, 0x44 | register << 3, 0x24, offset]);
        }
        for (index, (opcode, width, repeat, variable)) in forms.into_iter().enumerate() {
            let memory = index % 2 == 0;
            p.code
                .extend([0xc4, 0xe3, 0x71 | if wide { 4 } else { 0 }, opcode]);
            if memory {
                p.code.extend([0x64, 0x24, 48]);
            } else {
                p.code.push(0xe2);
            }
            p.code.push(if variable { 0x3f } else { 0xa6 });
            // Use the owned SysV red zone so all four qword offsets fit disp8.
            p.code.extend([0xc5, 0xfc, 0x11, 0x64, 0x24, 0xa0]);
            let mut expected = [0; 32];
            for byte in 0..if wide { 32 } else { 16 } {
                let select = if variable {
                    mask[byte / width * width + width - 1] & 0x80 != 0
                } else {
                    0xa6_u8 & (1 << ((byte / width) % if repeat { 8 } else { 32 })) != 0
                };
                expected[byte] = if select { b[byte] } else { a[byte] };
            }
            for (lane, word) in expected.chunks_exact(8).enumerate() {
                p.word(
                    0xa0 + lane as u8 * 8,
                    u64::from_le_bytes(word.try_into().unwrap()),
                );
            }
        }
        let (code, _) = p.finish(None);
        let (status, fields, text) = probe(&input(&code, false));
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["fp_status"], "0");
        assert_eq!(fields["vector_ops"], "17");
        assert_eq!(fields["vector_bytes"], if wide { "448" } else { "384" });
    }
}

#[test]
fn native_unsigned_vector_products_keep_full_even_dwords_and_upper_results() {
    let left = [
        u32::MAX,
        0,
        0x8000_0000,
        u32::MAX,
        0x7fff_ffff,
        0,
        0xffff_8000,
        0x1234_5678,
    ];
    let right = [
        7_u32,
        u32::MAX,
        u32::MAX,
        0,
        0x8000_0000,
        u32::MAX,
        0x8000_0001,
        0,
    ];
    let mut p = Program::new();
    for (base, values) in [(16, left), (48, right)] {
        for (index, pair) in values.chunks_exact(2).enumerate() {
            p.reg(0, u64::from(pair[0]) | u64::from(pair[1]) << 32);
            p.save(base + index as u8 * 8);
        }
    }
    for wide in [false, true] {
        for memory in [false, true] {
            p.code.extend([0xc5, 0xfc, 0x10, 0x4c, 0x24, 16]);
            p.code.extend([0xc5, 0xfc, 0x10, 0x54, 0x24, 48]);
            p.code.extend([0xc5, 0xf1 | if wide { 4 } else { 0 }, 0xf4]);
            if memory {
                p.code.extend([0x4c, 0x24, 48]);
            } else {
                p.code.push(0xca);
            }
            p.code.extend([0xc5, 0xfc, 0x11, 0x4c, 0x24, 80]);
            for lane in 0..4 {
                let expected = if !wide && lane >= 2 {
                    0
                } else {
                    u64::from(left[lane * 2]) * u64::from(right[lane * 2])
                };
                p.word(80 + lane as u8 * 8, expected);
            }
        }
    }
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["fp_status"], "0");
    assert_eq!(fields["vector_ops"], "16");
    assert_eq!(fields["vector_bytes"], "432");
}

#[test]
fn native_scalar_insert_vectors_preserve_merge_bits_and_clear_upper() {
    let words = [
        0x0706_0504_0302_0100_u64,
        0x0f0e_0d0c_0b0a_0908,
        u64::MAX,
        u64::MAX,
    ];
    let forms = [
        (3, 0x20, 0, 1, 0xff),
        (3, 0x20, 1, 1, 0x83),
        (1, 0xc4, 0, 2, 0xff),
        (1, 0xc4, 1, 2, 0x82),
        (3, 0x22, 0, 4, 0xff),
        (3, 0x22, 1, 8, 0x81),
    ];
    let mut p = Program::new();
    for (index, word) in words.iter().enumerate() {
        p.reg(0, *word);
        p.save(16 + index as u8 * 8);
    }
    for memory in [false, true] {
        p.code.extend([0xc5, 0xfc, 0x10, 0x4c, 0x24, 16]);
        let value = 0xfedc_ba98_7654_3210_u64;
        p.reg(0, value);
        let mut expected = [0; 32];
        for (index, word) in words.iter().enumerate() {
            expected[index * 8..index * 8 + 8].copy_from_slice(&word.to_le_bytes());
        }
        for (map, opcode, w, size, immediate) in forms {
            p.code.extend([0xc4, 0xe0 | map, w << 7 | 0x71, opcode]);
            if memory {
                p.code.extend([0x4c, 0x24, 16]);
            } else {
                p.code.push(0xc8);
            }
            p.code.push(immediate);
            let at = (usize::from(immediate) & (16 / size - 1)) * size;
            let input = if memory { words[0] } else { value };
            expected[at..at + size].copy_from_slice(&input.to_le_bytes()[..size]);
            expected[16..].fill(0);
        }
        p.code.extend([0xc5, 0xfc, 0x11, 0x4c, 0x24, 80]);
        for (index, word) in expected.chunks_exact(8).enumerate() {
            p.word(
                80 + index as u8 * 8,
                u64::from_le_bytes(word.try_into().unwrap()),
            );
        }
    }
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["vector_ops"], "16");
    assert_eq!(fields["vector_bytes"], "146");
    assert_eq!(fields["fp_status"], "0");
}

#[test]
fn native_half_vectors_merge_and_store_selected_quadwords() {
    let mut p = Program::new();
    let words = [
        0x8000_0000_7f80_0001_u64,
        0x7fc0_1234_0123_4567,
        0x9876_5432_ffff_ffff,
        0xfedc_ba98_7654_3210,
    ];
    for (index, word) in words.into_iter().enumerate() {
        p.reg(0, word);
        p.save(16 + index as u8 * 8);
    }
    for pp in [0, 1] {
        // Seed YMM1, then merge high/low exact memory halves with aliasing.
        p.code.extend([0xc5, 0xfc, 0x10, 0x4c, 0x24, 16]);
        p.code.extend([0xc5, 0xf0 | pp, 0x12, 0x4c, 0x24, 32]);
        p.code.extend([0xc5, 0xf8 | pp, 0x13, 0x4c, 0x24, 48]);
        p.word(48, words[2]);
        p.code.extend([0xc5, 0xf0 | pp, 0x16, 0x4c, 0x24, 40]);
        p.code.extend([0xc5, 0xf8 | pp, 0x17, 0x4c, 0x24, 56]);
        p.word(56, words[3]);
        p.code.extend([0xc5, 0xfc, 0x11, 0x4c, 0x24, 80]);
        for (slot, word) in [(80, words[2]), (88, words[3]), (96, 0), (104, 0)] {
            p.word(slot, word);
        }
    }
    p.code.extend([0xc5, 0xfc, 0x10, 0x54, 0x24, 16]); // YMM2 = original bits.
    p.code.extend([0xc5, 0xf0, 0x12, 0xd2]); // XMM2 = high(XMM2), high(XMM1).
    p.code.extend([0xc5, 0xe8, 0x16, 0xc9]); // XMM1 = low(XMM2), low(XMM1).
    p.code.extend([0xc5, 0xfc, 0x11, 0x4c, 0x24, 80]);
    for (slot, word) in [(80, words[1]), (88, words[2]), (96, 0), (104, 0)] {
        p.word(slot, word);
    }
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["fp_status"], "0");
    assert_eq!(fields["vector_ops"], "16");
    assert_eq!(fields["vector_bytes"], "256");
}

#[test]
fn native_duplicate_vectors_keep_selected_bits_and_clear_vex128_upper_halves() {
    for (pp, opcode, selected) in [
        (3, 0x12, [0, 1, 0, 1, 4, 5, 4, 5]),
        (2, 0x12, [0, 0, 2, 2, 4, 4, 6, 6]),
        (2, 0x16, [1, 1, 3, 3, 5, 5, 7, 7]),
    ] {
        for wide in [false, true] {
            for memory in [false, true] {
                let words = [
                    0x8000_0000_u32,
                    0x7f80_0001,
                    0x7fc0_1234,
                    0x1234_5678,
                    0x8765_4321,
                    0xffff_ffff,
                    0,
                    0x3f80_0000,
                ];
                let mut p = Program::new();
                for (index, pair) in words.chunks_exact(2).enumerate() {
                    p.reg(0, u64::from(pair[0]) | (u64::from(pair[1]) << 32));
                    p.save(16 + index as u8 * 8);
                }
                p.code.extend([0xc5, 0xfc, 0x10, 0x4c, 0x24, 16]); // VMOVUPS YMM1,[RSP+16].
                p.code
                    .extend([0xc5, 0xf8 | pp | if wide { 4 } else { 0 }, opcode]);
                if memory {
                    p.code.extend([0x4c, 0x24, 16]);
                } else {
                    p.code.push(0xc9);
                }
                p.code.extend([0xc5, 0xfc, 0x11, 0x4c, 0x24, 80]);
                for (index, pair) in selected.chunks_exact(2).enumerate() {
                    let word = if !wide && index >= 2 {
                        0
                    } else {
                        u64::from(words[pair[0]]) | (u64::from(words[pair[1]]) << 32)
                    };
                    p.word(80 + index as u8 * 8, word);
                }
                let (code, _) = p.finish(None);
                let (status, fields, text) = probe(&input(&code, false));
                assert_eq!(status, 0, "{text}");
                assert_eq!(fields["value"], "42");
                assert_eq!(fields["fp_status"], "0");
                assert_eq!(fields["vector_ops"], "3");
                let read = if wide {
                    32
                } else if pp == 3 {
                    8
                } else {
                    16
                };
                assert_eq!(
                    fields["vector_bytes"],
                    (64 + if memory { read } else { 0 }).to_string()
                );
            }
        }
    }
}

#[test]
fn native_scalar_float_comparisons_drive_flags_and_preserve_vector_sources() {
    for (double, one, two, quiet) in [
        (false, 0x3f80_0000_u64, 0x4000_0000_u64, 0x7fc0_0000_u64),
        (
            true,
            0x3ff0_0000_0000_0000,
            0x4000_0000_0000_0000,
            0x7ff8_0000_0000_0000,
        ),
    ] {
        for ordered in [false, true] {
            let mut p = Program::new();
            let words = [
                one,
                0xa5a5_a5a5_a5a5_a5a5,
                0x5a5a_5a5a_5a5a_5a5a,
                0x1234_5678_9abc_def0,
            ];
            for (index, word) in words.into_iter().enumerate() {
                p.reg(0, word);
                p.save(16 + index as u8 * 8);
            }
            p.code.extend([0xc5, 0xfc, 0x10, 0x4c, 0x24, 16]); // VMOVUPS ymm1,[rsp+16].
            for (right, flags) in [(two, 1), (0, 0), (one, 0x40), (quiet, 0x45)] {
                p.reg(0, right);
                p.save(48);
                p.code.extend([
                    0xc5,
                    0xf8 | u8::from(double),
                    if ordered { 0x2f } else { 0x2e },
                    0x4c,
                    0x24,
                    48,
                ]); // Compare xmm1 with an exact scalar memory source.
                // Capture all three flags before subsequent integer operations modify them.
                p.code
                    .extend([0x0f, 0x92, 0xc0, 0x0f, 0x9a, 0xc2, 0x0f, 0x94, 0xc1]);
                p.code
                    .extend([0x0f, 0xb6, 0xc0, 0x0f, 0xb6, 0xd2, 0x0f, 0xb6, 0xc9]);
                p.code
                    .extend([0xc1, 0xe2, 2, 0xc1, 0xe1, 6, 0x09, 0xd0, 0x09, 0xc8]);
                p.expect(flags);
            }
            p.code.extend([0xc5, 0xfc, 0x11, 0x4c, 0x24, 80]);
            for (index, word) in words.into_iter().enumerate() {
                p.word(80 + index as u8 * 8, word);
            }
            let (code, _) = p.finish(None);
            let (status, fields, text) = probe(&input(&code, false));
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["fp_status"], if ordered { "1" } else { "0" });
            assert_eq!(fields["vector_ops"], "6");
            assert_eq!(fields["vector_bytes"], if double { "96" } else { "80" });
        }
    }
}

#[test]
fn native_float_arithmetic_keeps_status_across_scalar_and_packed_execution() {
    let mut p = Program::new();
    for (slot, value) in [
        (16, 0x40c0_0000_40c0_0000),
        (24, 0x40c0_0000_40c0_0000),
        (32, 0x4100_0000_4100_0000),
        (40, 0x4100_0000_4100_0000),
        (48, 0x4000_0000_4000_0000),
        (56, 0x4000_0000_4000_0000),
        (64, 0x4000_0000_4000_0000),
        (72, 0x4000_0000_4000_0000),
    ] {
        p.reg(0, value);
        p.save(slot);
    }
    p.code.extend([0xc5, 0xfc, 0x10, 0x4c, 0x24, 16]); // Load four6/two8 pairs into ymm1.
    p.code.extend([0xc5, 0xf4, 0x5e, 0x4c, 0x24, 48]); // VDIVPS ymm1,ymm1,[rsp+48].
    p.code.extend([0xc5, 0xfc, 0x11, 0x4c, 0x24, 80]);
    for (slot, value) in [
        (80, 0x4040_0000_4040_0000),
        (88, 0x4040_0000_4040_0000),
        (96, 0x4080_0000_4080_0000),
        (104, 0x4080_0000_4080_0000),
    ] {
        p.word(slot, value);
    }
    p.code.extend([0xc5, 0xe9, 0xef, 0xd2]); // Zero xmm2.
    p.code.extend([0xc5, 0xf2, 0x5e, 0xc2]); // VDIVSS xmm0,xmm1,xmm2: +infinity, ZE.
    p.code.extend([0xc5, 0xf9, 0x7e, 0xc0]);
    p.expect(0x7f80_0000);
    p.code.extend([0xc5, 0xea, 0x51, 0xd1]); // VSQRTSS xmm2,xmm2,xmm1: sqrt3, PE.
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["fp_status"], "36");
    assert_modeled_vector_operations(&fields, ["7", "6", "6"]);
    assert_eq!(fields["vector_bytes"], "96");
}

#[test]
fn native_integer_float_conversions_publish_guest_precision_and_merged_lanes() {
    let mut p = Program::new();
    p.reg(9, 16_777_217);
    p.code.extend([0xc5, 0xf1, 0xef, 0xc9]); // VPXOR xmm1,xmm1,xmm1.
    p.code.extend([0xc4, 0xc1, 0x72, 0x2a, 0xc1]); // VCVTSI2SS xmm0,xmm1,r9d.
    p.code.extend([0xc5, 0xf9, 0x7e, 0xc0]); // Observe the rounded result bits.
    p.expect(0x4b80_0000);
    p.reg(0, (-42_i64) as u64);
    p.save(16);
    p.code.extend([0xc4, 0xe1, 0xf3, 0x2a, 0x4c, 0x24, 16]); // VCVTSI2SD xmm1,xmm1,[rsp+16].
    p.code.extend([0xc4, 0xe1, 0xf9, 0x7e, 0xc8]);
    p.expect(0xc045_0000_0000_0000);
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["fp_status"], "32"); // Inexact remains sticky across the exact conversion.
    assert_modeled_vector_operations(&fields, ["5", "4", "4"]);
    assert_eq!(fields["vector_bytes"], "8");
}

#[test]
fn native_float_integer_conversions_round_truncate_and_publish_actual_invalid_precision_status() {
    let mut p = Program::new();
    for double in [false, true] {
        p.reg(
            0,
            if double {
                (-3.5_f64).to_bits()
            } else {
                u64::from((-3.5_f32).to_bits())
            },
        );
        p.save(16);
        for wide in [false, true] {
            for truncate in [false, true] {
                p.code.extend([
                    0xc4,
                    0xe1,
                    0x78 | if wide { 0x80 } else { 0 } | if double { 3 } else { 2 },
                    if truncate { 0x2c } else { 0x2d },
                    0x44,
                    0x24,
                    16,
                ]);
                let signed = if truncate { -3_i64 } else { -4_i64 };
                p.expect(if wide {
                    signed as u64
                } else {
                    u64::from(signed as u32)
                });
            }
        }
    }
    p.reg(0, 0x7ff8_0000_0000_0001);
    p.save(16);
    p.code.extend([0xc4, 0xe1, 0xfb, 0x2c, 0x44, 0x24, 16]);
    p.expect(1_u64 << 63);
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["fp_status"], "33");
    assert_eq!(fields["vector_ops"], "9");
    assert_eq!(fields["vector_bytes"], "56");
}

#[test]
fn native_scalar_elements_insert_mask_then_extract_exact_words() {
    let mut p = Program::new();
    for (slot, value) in [
        (16, 0x0000_0002_0000_0001),
        (24, 0x0000_0004_0000_0003),
        (32, 0x0000_0006_0000_0005),
        (40, 0x0000_0008_0000_0007),
    ] {
        p.reg(0, value);
        p.save(slot);
    }
    p.code.extend([0xc5, 0xfc, 0x10, 0x44, 0x24, 16]);
    p.code.extend([0xc5, 0xf8, 0x10, 0x4c, 0x24, 32]);
    p.code.extend([0xc4, 0xe3, 0x79, 0x21, 0xc1, 0xb2]); // [1,0,3,7]; upper YMM clears.
    p.code.extend([0xc5, 0xfc, 0x11, 0x44, 0x24, 48]);
    for (slot, value) in [(48, 1), (56, 0x0000_0007_0000_0003), (64, 0), (72, 0)] {
        p.word(slot, value);
    }
    p.reg(0, u64::MAX);
    p.code.extend([0xc4, 0xe3, 0xf9, 0x17, 0xc0, 0xff]); // RAX gets scalar7, zero extended.
    p.expect(7);
    p.code
        .extend([0xc4, 0xe3, 0x79, 0x21, 0x44, 0x24, 32, 0x70]); // Memory ignores source selector.
    p.set(80, 85);
    p.code
        .extend([0xc4, 0xe3, 0x79, 0x17, 0x44, 0x24, 84, 0xff]); // Four-byte store only.
    p.word(80, 0x0000_0005_0000_0055);
    p.code.extend([0xc4, 0xe3, 0x79, 0x21, 0xc0, 0xff]); // Aliased insertion, all zero mask.
    p.code.extend([0xc5, 0xfc, 0x11, 0x44, 0x24, 48]);
    for slot in [48, 56, 64, 72] {
        p.word(slot, 0);
    }
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["vector_ops"], "9");
    assert_eq!(fields["vector_bytes"], "120");
}

#[test]
fn native_pair_shuffles_select_distinct_lanes_and_clear_vex128_upper_halves() {
    let mut p = Program::new();
    for (slot, value) in [
        (16, 0x0000_0002_0000_0001),
        (24, 0x0000_0004_0000_0003),
        (32, 0x0000_0006_0000_0005),
        (40, 0x0000_0008_0000_0007),
    ] {
        p.reg(0, value);
        p.save(slot);
    }
    p.code.extend([0xc5, 0xfc, 0x10, 0x44, 0x24, 16]);
    p.code.extend([0xc5, 0xfc, 0x10, 0xc8]); // Both sources initially equal.
    p.code.extend([0xc5, 0xfc, 0xc6, 0xc1, 0x1b]); // Reverse each 128-bit PS lane.
    p.code.extend([0xc5, 0xfc, 0x11, 0x44, 0x24, 48]);
    for (slot, value) in [
        (48, 0x0000_0003_0000_0004),
        (56, 0x0000_0001_0000_0002),
        (64, 0x0000_0007_0000_0008),
        (72, 0x0000_0005_0000_0006),
    ] {
        p.word(slot, value);
    }
    p.code.extend([0xc5, 0xfd, 0xc6, 0x44, 0x24, 16, 0xf9]); // Different selectors for PD upper lane.
    p.code.extend([0xc5, 0xfc, 0x11, 0x44, 0x24, 48]);
    for (slot, value) in [
        (48, 0x0000_0001_0000_0002),
        (56, 0x0000_0002_0000_0001),
        (64, 0x0000_0007_0000_0008),
        (72, 0x0000_0008_0000_0007),
    ] {
        p.word(slot, value);
    }
    p.code.extend([0xc5, 0xf8, 0xc6, 0xc0, 0]); // Aliased 128-bit form clears YMM upper.
    p.code.extend([0xc5, 0xfc, 0x11, 0x44, 0x24, 48]);
    p.word(48, 0x0000_0002_0000_0002);
    p.word(56, 0x0000_0002_0000_0002);
    p.word(64, 0);
    p.word(72, 0);
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["vector_ops"], "8");
    assert_eq!(fields["vector_bytes"], "160");
}

#[test]
fn native_full_width_permutations_read_back_cross_lane_bits_and_aliased_sources() {
    let values = [
        0x7f80_0001_u32,
        0x8000_0000,
        0x0000_0001,
        0xff80_0000,
        0x7fc0_1234,
        0x3f80_0000,
        0xffc0_5678,
        0x0080_0000,
    ];
    let indices = [
        0xffff_fffc_u32,
        0x8000_0007,
        0xffff_fffe,
        0x4000_0000,
        0xffff_fff9,
        0x8000_0003,
        0x7fff_fffa,
        0x1234_567d,
    ];
    let selected = [4, 7, 6, 0, 1, 3, 2, 5];
    for opcode in [0x16, 0x36] {
        let mut p = Program::new();
        for (base, words) in [(16, values), (48, indices)] {
            for (index, pair) in words.chunks_exact(2).enumerate() {
                p.reg(0, u64::from(pair[0]) | (u64::from(pair[1]) << 32));
                p.save(base + index as u8 * 8);
            }
        }
        p.code.extend([0xc5, 0xfc, 0x10, 0x4c, 0x24, 48]); // Index YMM1.
        p.code.extend([0xc5, 0xfc, 0x10, 0x54, 0x24, 16]); // Data YMM2.
        // Destination aliases data first, then indices with a full memory source.
        for form in [false, true] {
            if form {
                p.code.extend([0xc4, 0xe2, 0x75, opcode, 0x4c, 0x24, 16]);
                p.code.extend([0xc5, 0xfc, 0x11, 0x4c, 0x24, 80]);
            } else {
                p.code.extend([0xc4, 0xe2, 0x75, opcode, 0xd2]);
                p.code.extend([0xc5, 0xfc, 0x11, 0x54, 0x24, 80]);
            }
            for (index, pair) in selected.chunks_exact(2).enumerate() {
                p.word(
                    80 + index as u8 * 8,
                    u64::from(values[pair[0]]) | (u64::from(values[pair[1]]) << 32),
                );
            }
        }
        let (code, _) = p.finish(None);
        let (status, fields, text) = probe(&input(&code, false));
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["vector_ops"], "6");
        assert_eq!(fields["vector_bytes"], "160");
    }
}

#[test]
fn native_unpack_vectors_interleave_memory_and_aliases_without_crossing_lanes() {
    let mut p = Program::new();
    for (slot, value) in [
        (16, 1),
        (24, 2),
        (32, 3),
        (40, 4),
        (48, 5),
        (56, 6),
        (64, 7),
        (72, 8),
    ] {
        p.set(slot, value);
    }
    p.code.extend([0xc5, 0xfc, 0x10, 0x44, 0x24, 16]);
    p.code.extend([0xc5, 0xfc, 0x10, 0x4c, 0x24, 48]);
    p.code.extend([0xc5, 0xfd, 0x14, 0xc1]); // VUNPCKLPD ymm0,ymm0,ymm1
    p.code.extend([0xc5, 0xfc, 0x11, 0x44, 0x24, 80]);
    for (slot, value) in [(80, 1), (88, 5), (96, 3), (104, 7)] {
        p.word(slot, value);
    }
    p.code.extend([0xc5, 0xf9, 0x15, 0x44, 0x24, 48]); // VUNPCKHPD xmm0,xmm0,[rsp+48]
    p.code.extend([0xc5, 0xfc, 0x11, 0x44, 0x24, 80]);
    for (slot, value) in [(80, 5), (88, 6), (96, 0), (104, 0)] {
        p.word(slot, value);
    }
    p.code.extend([0xc5, 0xf9, 0x62, 0xc1]); // VPUNPCKLDQ xmm0,xmm0,xmm1
    p.code.extend([0xc5, 0xfc, 0x11, 0x44, 0x24, 80]);
    p.word(80, 0x0000_0005_0000_0005);
    p.word(88, 0);
    p.word(96, 0);
    p.word(104, 0);
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["vector_ops"], "8");
    assert_eq!(fields["vector_bytes"], "176");
}

#[test]
fn native_lane_insertions_preserve_both_halves_across_native_bridge_calls() {
    let mut p = Program::new();
    for (slot, value) in [(16, 1), (24, 2), (32, 3), (40, 4)] {
        p.set(slot, value);
    }
    p.code.extend([0xc5, 0xfc, 0x10, 0x44, 0x24, 16]); // ymm0 = [1,2,3,4]
    p.code.extend([0xc5, 0xf8, 0x10, 0x4c, 0x24, 32]); // xmm1 = [3,4]
    p.code.extend([0xc4, 0xe3, 0x7d, 0x18, 0xc1, 0xfe]); // Insert low; upper immediate bits ignored.
    p.code.extend([0xc5, 0xfc, 0x11, 0x44, 0x24, 48]);
    for (slot, value) in [(48, 3), (56, 4), (64, 3), (72, 4)] {
        p.word(slot, value);
    }
    p.code
        .extend([0xc4, 0xe3, 0x7d, 0x38, 0x44, 0x24, 16, 0xff]); // Insert memory into high lane.
    p.code.extend([0xc5, 0xfc, 0x11, 0x44, 0x24, 48]);
    for (slot, value) in [(48, 3), (56, 4), (64, 1), (72, 2)] {
        p.word(slot, value);
    }
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["vector_ops"], "6");
    assert_eq!(fields["vector_bytes"], "128");
}

#[test]
fn native_scalar_float_moves_merge_registers_and_access_exact_memory_widths() {
    for (prefix, scalar) in [(0xfa, 4), (0xfb, 8)] {
        let mut p = Program::new();
        for (slot, value) in [
            (16, 0x1122_3344_5566_7788),
            (24, 0x99aa_bbcc_ddee_ff00),
            (32, 0x0123_4567_89ab_cdef),
            (40, 0xfedc_ba98_7654_3210),
        ] {
            p.reg(0, value);
            p.save(slot);
        }
        p.code.extend([0xc5, 0xfc, 0x10, 0x4c, 0x24, 16]); // Dirty ymm1.
        p.code.extend([0xc5, prefix, 0x10, 0x4c, 0x24, 32]); // Scalar load clears upper.
        p.code.extend([0xc5, 0xfc, 0x11, 0x4c, 0x24, 48]);
        p.word(
            48,
            if scalar == 4 {
                0x89ab_cdef
            } else {
                0x0123_4567_89ab_cdef
            },
        );
        for slot in [56, 64, 72] {
            p.word(slot, 0);
        }
        p.code.extend([0xc5, 0xfc, 0x10, 0x44, 0x24, 16]); // Upper-source xmm0.
        p.code.extend([0xc5, prefix, 0x10, 0xc1]); // xmm0 <- xmm0 upper + xmm1 low.
        p.code.extend([0xc5, 0xfc, 0x11, 0x44, 0x24, 48]);
        p.word(
            48,
            if scalar == 4 {
                0x1122_3344_89ab_cdef
            } else {
                0x0123_4567_89ab_cdef
            },
        );
        p.word(56, 0x99aa_bbcc_ddee_ff00);
        p.word(64, 0);
        p.word(72, 0);
        p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
        p.save(80);
        p.save(88);
        p.code.extend([0xc5, prefix, 0x11, 0x44, 0x24, 80]); // Exact scalar store.
        p.word(
            80,
            if scalar == 4 {
                0xa5a5_a5a5_89ab_cdef
            } else {
                0x0123_4567_89ab_cdef
            },
        );
        p.word(88, 0xa5a5_a5a5_a5a5_a5a5);
        let (code, _) = p.finish(None);
        let (status, fields, text) = probe(&input(&code, false));
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["vector_ops"], "7");
        assert_eq!(fields["vector_bytes"], (128 + scalar * 2).to_string());
    }
}

#[test]
fn native_broadcast_vectors_repeat_scalar_and_tuple_sources_in_guest_memory() {
    for wide in [false, true] {
        let mut p = Program::new();
        p.reg(0, 0x8877_6655_4433_2211);
        p.save(16);
        p.reg(0, 0xffee_ddcc_bbaa_9900);
        p.save(24);
        for (opcode, tuple) in [
            (0x78, 1),
            (0x79, 2),
            (0x58, 4),
            (0x59, 8),
            (0x18, 4),
            (0x19, 8),
            (0x1a, 16),
            (0x5a, 16),
        ] {
            if !wide && matches!(opcode, 0x19 | 0x1a | 0x5a) {
                continue;
            }
            if opcode & 1 == 1 {
                p.code.extend([0xc5, 0xfa, 0x6f, 0x4c, 0x24, 16]); // VMOVDQU xmm1,[rsp+16]
                p.code
                    .extend([0xc4, 0xe2, if wide { 0x7d } else { 0x79 }, opcode, 0xc9]);
            } else {
                p.code.extend([
                    0xc4,
                    0xe2,
                    if wide { 0x7d } else { 0x79 },
                    opcode,
                    0x4c,
                    0x24,
                    16,
                ]);
            }
            p.code.extend([0xc5, 0xfc, 0x11, 0x4c, 0x24, 48]); // VMOVUPS [rsp+48],ymm1
            let source = [
                0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x00, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff,
            ];
            let mut expected = [0; 32];
            for destination in expected[..if wide { 32 } else { 16 }].chunks_exact_mut(tuple) {
                destination.copy_from_slice(&source[..tuple]);
            }
            for (index, word) in expected.chunks_exact(8).enumerate() {
                p.word(
                    48 + index as u8 * 8,
                    u64::from_le_bytes(word.try_into().unwrap()),
                );
            }
        }
        let (code, _) = p.finish(None);
        let (status, fields, text) = probe(&input(&code, false));
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
    }
}

#[test]
fn native_logical_vectors_read_memory_shuffle_bytes_and_zero_aliased_registers() {
    let mut p = Program::new();
    for (slot, value) in [
        (16, 0x0f0f_0f0f_0f0f_0f0f),
        (24, 0x0f0f_0f0f_0f0f_0f0f),
        (32, 0xf0f0_f0f0_f0f0_f0f0),
        (40, 0xf0f0_f0f0_f0f0_f0f0),
        (48, 0x3333_3333_3333_3333),
        (56, 0x3333_3333_3333_3333),
        (64, 0x5555_5555_5555_5555),
        (72, 0x5555_5555_5555_5555),
    ] {
        p.reg(0, value);
        p.save(slot);
    }
    p.code.extend([0xc5, 0xfc, 0x10, 0x4c, 0x24, 16]);
    p.code.extend([0xc5, 0x75, 0xdf, 0x7c, 0x24, 48]); // VPANDN ymm15,ymm1,[rsp+48]
    p.code.extend([0xc5, 0x7c, 0x11, 0x7c, 0x24, 80]);
    for (slot, value) in [
        (80, 0x3030_3030_3030_3030),
        (88, 0x3030_3030_3030_3030),
        (96, 0x0505_0505_0505_0505),
        (104, 0x0505_0505_0505_0505),
    ] {
        p.word(slot, value);
    }
    p.code.extend([0xc4, 0x41, 0x01, 0xef, 0xff]); // VPXOR xmm15,xmm15,xmm15
    p.code.extend([0xc5, 0x7c, 0x11, 0x7c, 0x24, 80]);
    for slot in [80, 88, 96, 104] {
        p.word(slot, 0);
    }
    p.reg(0, 0x81);
    p.code.extend([0xc5, 0xf9, 0x6e, 0xc0]); // VMOVD xmm0,eax
    p.code.extend([0xc4, 0xc2, 0x79, 0x00, 0xc7]); // VPSHUFB xmm0,xmm0,xmm15: broadcast byte0.
    p.code.extend([0xc5, 0xfc, 0x11, 0x44, 0x24, 80]);
    for (slot, value) in [
        (80, 0x8181_8181_8181_8181),
        (88, 0x8181_8181_8181_8181),
        (96, 0),
        (104, 0),
    ] {
        p.word(slot, value);
    }
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_modeled_vector_operations(&fields, ["8", "7", "7"]);
    assert_eq!(fields["vector_bytes"], "160");
}

#[test]
fn native_scalar_vectors_transfer_gprs_and_exact_memory_words_with_upper_zeroing() {
    let mut p = Program::new();
    for offset in [16, 24, 32, 40] {
        p.set(offset, 85);
    }
    p.code.extend([0xc5, 0x7c, 0x10, 0x7c, 0x24, 16]); // Dirty ymm15.
    p.reg(9, 0x1234_5678_fedc_ba98);
    p.code.extend([0xc4, 0x41, 0x79, 0x6e, 0xf9]); // VMOVD xmm15,r9d
    p.code.extend([0xc5, 0x7c, 0x11, 0x7c, 0x24, 48]);
    for (slot, value) in [(48, 0xfedc_ba98), (56, 0), (64, 0), (72, 0)] {
        p.word(slot, value);
    }
    p.reg(0, u64::MAX);
    p.code.extend([0xc5, 0x79, 0x7e, 0xf8]); // VMOVD eax,xmm15
    p.expect(0xfedc_ba98);
    p.code.extend([0xc4, 0x41, 0xf9, 0x6e, 0xf9]); // VMOVQ xmm15,r9
    p.code.extend([0xc4, 0x61, 0xf9, 0x7e, 0xf8]); // VMOVQ rax,xmm15
    p.expect(0x1234_5678_fedc_ba98);
    p.code.extend([0xc5, 0x79, 0xd6, 0xf9]); // VMOVQ xmm1,xmm15 (D6 form)
    p.code.extend([0xc5, 0xfa, 0x7e, 0xc1]); // VMOVQ xmm0,xmm1 (7E form)
    p.code.extend([0xc5, 0xf9, 0xd6, 0x44, 0x24, 48]); // Store qword.
    p.word(48, 0x1234_5678_fedc_ba98);
    p.set(56, 85);
    p.code.extend([0xc5, 0xf9, 0x7e, 0x44, 0x24, 52]); // Store dword only.
    p.word(48, 0xfedc_ba98_fedc_ba98);
    p.word(56, 85);
    p.code.extend([0xc5, 0xfa, 0x7e, 0x4c, 0x24, 48]); // Load qword.
    p.code.extend([0xc5, 0xf9, 0x6e, 0x44, 0x24, 48]); // Load dword, clear upper.
    p.code.extend([0xc5, 0xfc, 0x11, 0x44, 0x24, 48]);
    for (slot, value) in [(48, 0xfedc_ba98), (56, 0), (64, 0), (72, 0)] {
        p.word(slot, value);
    }
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["vector_ops"], "13");
    assert_eq!(fields["vector_bytes"], "120");
}

#[test]
fn native_floating_bit_tests_set_sign_flags_and_preserve_upper_source_lanes() {
    for (opcode, element) in [(0x0e, 4), (0x0f, 8)] {
        for size in [16, 32] {
            let mut p = Program::new();
            let mut final_left = [0; 32];
            for (left_mask, right_mask, flags) in [(0, 0, 0x41), (1, 3, 0)] {
                let mut left = [0xff; 32];
                let mut right = [0xff; 32];
                for lane in 0..32 / element {
                    left[lane * element + element - 1] = if left_mask & (1 << lane) != 0 {
                        0xff
                    } else {
                        0x7f
                    };
                    right[lane * element + element - 1] = if right_mask & (1 << lane) != 0 {
                        0xff
                    } else {
                        0x7f
                    };
                }
                for (offset, values) in [(16, &left), (48, &right)] {
                    for (index, bytes) in values.chunks_exact(8).enumerate() {
                        p.reg(0, u64::from_le_bytes(bytes.try_into().unwrap()));
                        p.save(offset + index as u8 * 8);
                    }
                }
                p.code.extend([0xc5, 0x7c, 0x10, 0x7c, 0x24, 16]);
                p.code.extend([0xc5, 0xfc, 0x10, 0x4c, 0x24, 48]);
                for memory in [false, true] {
                    p.code
                        .extend([0xc4, 0x62, if size == 32 { 0x7d } else { 0x79 }, opcode]);
                    if memory {
                        p.code.extend([0x7c, 0x24, 48]);
                    } else {
                        p.code.push(0xf9);
                    }
                    // Capture both result flags without requiring the separate
                    // PUSHFQ instruction, which this caller does not exercise.
                    p.reg(0, 0);
                    p.reg(2, 0);
                    p.code.extend([
                        0x0f, 0x92, 0xc0, 0x0f, 0x94, 0xc2, 0x48, 0xc1, 0xe2, 6, 0x48, 0x09, 0xd0,
                    ]);
                    p.expect(flags);
                }
                final_left = left;
            }
            p.code.extend([0xc5, 0x7c, 0x11, 0x7c, 0x24, 80]);
            for (index, bytes) in final_left.chunks_exact(8).enumerate() {
                p.word(
                    80 + index as u8 * 8,
                    u64::from_le_bytes(bytes.try_into().unwrap()),
                );
            }
            let (code, _) = p.finish(None);
            let (status, fields, text) = probe(&input(&code, false));
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["vector_ops"], "9");
            assert_eq!(fields["vector_bytes"], (160 + 2 * size).to_string());
        }
    }
}

#[test]
fn native_integer_widening_uses_signed_elements_narrow_memory_and_aliased_sources() {
    let source: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(71).wrapping_add(0x80));
    for signed in [false, true] {
        let opcode = if signed { 0x21 } else { 0x31 };
        let mut p = Program::new();
        for (index, bytes) in source.chunks_exact(8).enumerate() {
            p.reg(0, u64::from_le_bytes(bytes.try_into().unwrap()));
            p.save(16 + index as u8 * 8);
        }
        p.code.extend([0xc5, 0x7c, 0x10, 0x7c, 0x24, 16]);
        for memory in [false, true] {
            if memory {
                p.code.extend([0xc4, 0x62, 0x7d, opcode, 0x7c, 0x24, 17]);
            } else {
                p.code.extend([0xc4, 0x42, 0x7d, opcode, 0xff]);
            }
            p.code.extend([0xc5, 0x7c, 0x11, 0x7c, 0x24, 48]);
            let start = usize::from(memory);
            for index in 0..4 {
                let element = |at: usize| {
                    if signed {
                        i32::from(source[at] as i8) as u32
                    } else {
                        u32::from(source[at])
                    }
                };
                let value = u64::from(element(start + index * 2))
                    | (u64::from(element(start + index * 2 + 1)) << 32);
                p.word(48 + index as u8 * 8, value);
            }
        }
        let (code, _) = p.finish(None);
        let (status, fields, text) = probe(&input(&code, false));
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["vector_ops"], "5");
        assert_eq!(fields["vector_bytes"], "104");
    }
}

#[test]
fn native_integer_extractions_zero_extend_and_store_only_the_selected_scalar() {
    let source: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(17).wrapping_add(0x83));
    for (size, opcode, prefix) in [
        (1, 0x14, 0x79),
        (2, 0x15, 0x79),
        (4, 0x16, 0x79),
        (8, 0x16, 0xf9),
    ] {
        let mut p = Program::new();
        for (index, bytes) in source.chunks_exact(8).enumerate() {
            p.reg(0, u64::from_le_bytes(bytes.try_into().unwrap()));
            p.save(16 + index as u8 * 8);
        }
        for offset in [64, 72] {
            p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
            p.save(offset);
        }
        p.code.extend([0xc5, 0x7c, 0x10, 0x7c, 0x24, 16]);
        let selected = &source[16 - size..16];
        let mut scalar = [0; 8];
        scalar[..size].copy_from_slice(selected);
        p.reg(0, u64::MAX);
        p.code.extend([0xc4, 0x63, prefix, opcode, 0xf8, 0xff]);
        p.expect(u64::from_le_bytes(scalar));
        p.ptr(0, 67); // A deliberately unaligned scalar memory destination.
        p.code.extend([0xc4, 0x63, prefix, opcode, 0x38, 0xff]);
        let mut expected = [0xa5; 16];
        expected[3..3 + size].copy_from_slice(selected);
        for (index, bytes) in expected.chunks_exact(8).enumerate() {
            p.word(
                64 + index as u8 * 8,
                u64::from_le_bytes(bytes.try_into().unwrap()),
            );
        }
        p.code.extend([0xc5, 0x7c, 0x11, 0x7c, 0x24, 80]);
        for (index, bytes) in source.chunks_exact(8).enumerate() {
            p.word(
                80 + index as u8 * 8,
                u64::from_le_bytes(bytes.try_into().unwrap()),
            );
        }
        let (code, _) = p.finish(None);
        let (status, fields, text) = probe(&input(&code, false));
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["vector_ops"], "4");
        assert_eq!(fields["vector_bytes"], (64 + size).to_string());
    }
}

#[test]
fn native_streaming_loads_copy_full_width_and_clear_only_vex128_upper_lanes() {
    for size in [16, 32] {
        let mut p = Program::new();
        for (offset, value) in [(16, 1), (24, 2), (32, 3), (40, 4)] {
            p.set(offset, value);
        }
        p.ptr(7, 95);
        p.code.extend([0x48, 0x83, 0xe7, 0xe0]); // Align source RDI to 32 bytes.
        for (offset, value) in [(0, 37), (8, 42), (16, 99), (24, 123)] {
            p.reg(0, value);
            p.code.extend([0x48, 0x89, 0x47, offset]);
        }
        p.code.extend([0xc5, 0x7c, 0x10, 0x7c, 0x24, 16]); // Initialize YMM15.
        p.code
            .extend([0xc4, 0x62, if size == 32 { 0x7d } else { 0x79 }, 0x2a, 0x3f]); // VMOVNTDQA XMM15/YMM15,[RDI].
        p.code.extend([0xc5, 0x7c, 0x11, 0x7c, 0x24, 112]);
        for (offset, value) in [
            (112, 37),
            (120, 42),
            (128, if size == 32 { 99 } else { 0 }),
            (136, if size == 32 { 123 } else { 0 }),
        ] {
            // Use a full disp32 above127, avoiding a signed disp8 wrap.
            p.code.extend([0x48, 0x8b, 0x84, 0x24]);
            p.code.extend((offset as u32).to_le_bytes());
            p.expect(value);
        }
        let (code, _) = p.finish(None);
        let (status, fields, text) = probe(&input(&code, false));
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["vector_ops"], "3");
        assert_eq!(fields["vector_bytes"], (64 + size).to_string());
    }
}

#[test]
fn native_non_temporal_vector_stores_write_exact_width_and_preserve_the_source() {
    for size in [16, 32] {
        let mut p = Program::new();
        for (offset, value) in [(16, 1), (24, 2), (32, 3), (40, 4)] {
            p.set(offset, value);
        }
        p.ptr(7, 95);
        p.code.extend([0x48, 0x83, 0xe7, 0xe0]); // Align RDI down to 32 bytes.
        p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
        for offset in [0xf8, 0, 8, 16, 24, 32, 40] {
            p.code.extend([0x48, 0x89, 0x47, offset]);
        }
        p.code.extend([0xc5, 0x7c, 0x10, 0x7c, 0x24, 16]); // Load YMM15.
        p.code
            .extend([0xc5, if size == 32 { 0x7d } else { 0x79 }, 0xe7, 0x3f]); // VMOVNTDQ [RDI], XMM15/YMM15.
        p.code.extend([0x0f, 0xae, 0xf8]); // SFENCE remains an ordinary native instruction.
        for (offset, value) in [
            (0xf8, 0xa5a5_a5a5_a5a5_a5a5),
            (0, 1),
            (8, 2),
            (16, if size == 32 { 3 } else { 0xa5a5_a5a5_a5a5_a5a5 }),
            (24, if size == 32 { 4 } else { 0xa5a5_a5a5_a5a5_a5a5 }),
            (32, 0xa5a5_a5a5_a5a5_a5a5),
            (40, 0xa5a5_a5a5_a5a5_a5a5),
        ] {
            p.code.extend([0x48, 0x8b, 0x47, offset]);
            p.expect(value);
        }
        // A VEX.128 memory destination must retain YMM15's upper half.
        p.code.extend([0xc5, 0x7c, 0x11, 0x7c, 0x24, 48]);
        for (offset, value) in [(48, 1), (56, 2), (64, 3), (72, 4)] {
            p.word(offset, value);
        }
        let (code, _) = p.finish(None);
        let (status, fields, text) = probe(&input(&code, false));
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["vector_ops"], "3");
        assert_eq!(fields["vector_bytes"], (64 + size).to_string());
    }
}

#[test]
fn native_vector_moves_preserve_upper_lanes_across_sse_and_clear_them_on_vex128() {
    let mut p = Program::new();
    for (offset, value) in [(16, 1), (24, 2), (32, 3), (40, 4)] {
        p.set(offset, value);
    }
    p.code.extend([0xc5, 0xfc, 0x10, 0x4c, 0x24, 16]); // VMOVUPS ymm1,[rsp+16]
    p.code.extend([0xc5, 0x7c, 0x10, 0xf9]); // VMOVUPS ymm15,ymm1
    p.code.extend([0x66, 0x0f, 0xef, 0xc9]); // Legacy PXOR xmm1,xmm1: retain upper.
    p.code.extend([0xc5, 0xfc, 0x11, 0x4c, 0x24, 48]);
    for (offset, value) in [(48, 0), (56, 0), (64, 3), (72, 4)] {
        p.word(offset, value);
    }
    p.code.extend([0xc5, 0xf8, 0x10, 0xc9]); // VMOVUPS xmm1,xmm1: clear upper.
    p.code.extend([0xc5, 0xfc, 0x11, 0x4c, 0x24, 48]);
    for offset in [48, 56, 64, 72] {
        p.word(offset, 0);
    }
    p.code.extend([0xc5, 0x7c, 0x11, 0x7c, 0x24, 48]); // Store untouched ymm15.
    for (offset, value) in [(48, 1), (56, 2), (64, 3), (72, 4)] {
        p.word(offset, value);
    }
    p.code.extend([0xc5, 0xf8, 0x77]); // VZEROUPPER, including ymm15.
    p.code.extend([0xc5, 0x7c, 0x11, 0x7c, 0x24, 48]);
    for (offset, value) in [(48, 1), (56, 2), (64, 0), (72, 0)] {
        p.word(offset, value);
    }
    p.code.extend([0xc5, 0xfc, 0x77]); // VZEROALL
    p.code.extend([0xc5, 0x7c, 0x11, 0x7c, 0x24, 48]);
    for offset in [48, 56, 64, 72] {
        p.word(offset, 0);
    }
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["vector_ops"], "10");
    assert_eq!(fields["vector_bytes"], "192");
}
