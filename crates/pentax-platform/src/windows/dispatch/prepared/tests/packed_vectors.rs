// SPDX-License-Identifier: GPL-2.0-only

//! Hardware references execute fixed SSE4.2 instructions on original arrays.
//! No supplied executable bytes, host AVX state or guest pointers are executed.

use super::super::packed_vectors::{Comparison, compare};
use super::*;
use std::arch::asm;

fn hardware<const MODE: u8>(
    a: &[u8; 16],
    b: &[u8; 16],
    lengths: Option<(u64, u64, bool)>,
) -> Comparison {
    assert!(std::is_x86_feature_detected!("sse4.2"));
    let mut mask = [0; 16];
    let index: u64;
    let mask_flags: u64;
    let index_flags: u64;
    let (a_len, b_len, _) = lengths.unwrap_or((0, 0, false));
    macro_rules! run {
        ($mask:literal, $index:literal) => {
            // SAFETY: The instructions are fixed reference encodings, CPU
            // support is checked, and each pointer names a full owned array.
            // All clobbers are declared. Stack and host flags are restored.
            unsafe { asm!(
                "pushfq",
                "movdqu xmm1, [{a}]", "movdqu xmm2, [{b}]",
                $mask,
                "pushfq", "pop {mask_flags}", "movdqu [{output}], xmm0",
                $index,
                "pushfq", "pop {index_flags}", "popfq",
                a = in(reg) a.as_ptr(), b = in(reg) b.as_ptr(),
                output = in(reg) mask.as_mut_ptr(), mode = const MODE,
                mask_flags = out(reg) mask_flags, index_flags = out(reg) index_flags,
                in("rax") a_len, in("rdx") b_len, out("rcx") index,
                out("xmm0") _, out("xmm1") _, out("xmm2") _,
            ); }
        };
    }
    match lengths {
        None => run!(
            "pcmpistrm xmm1, xmm2, {mode}",
            "pcmpistri xmm1, xmm2, {mode}"
        ),
        Some((_, _, false)) => run!(
            "pcmpestrm xmm1, xmm2, {mode}",
            "pcmpestri xmm1, xmm2, {mode}"
        ),
        Some((_, _, true)) => run!(
            ".byte 0x66, 0x48, 0x0f, 0x3a, 0x60, 0xca, {mode}",
            ".byte 0x66, 0x48, 0x0f, 0x3a, 0x61, 0xca, {mode}"
        ),
    }
    assert_eq!(mask_flags & 0x8d5, index_flags & 0x8d5);
    Comparison {
        mask,
        index,
        flags: mask_flags & 0x8d5,
    }
}

fn inputs() -> Vec<([u8; 16], [u8; 16])> {
    let mut samples = vec![
        ([0; 16], [0; 16]),
        ([0; 16], [1; 16]),
        ([1; 16], [0; 16]),
        ([1; 16], [1; 16]),
        ([1; 16], [2; 16]),
        ([0x80; 16], [0x7f; 16]),
        (
            [1, 2, 3, 0, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
            [9, 1, 2, 3, 0, 1, 2, 3, 1, 2, 3, 9, 9, 9, 1, 2],
        ),
        (
            [0x80, 0xff, 0, 0x7f, 4, 9, 15, 12, 20, 30, 0, 0, 0, 0, 0, 0],
            [
                0x80, 0xff, 1, 2, 3, 4, 9, 10, 12, 13, 15, 20, 30, 31, 0, 0x7f,
            ],
        ),
        (
            [
                0, 1, 0, 0x80, 0xff, 0x7f, 0xff, 0xff, 0, 0, 5, 6, 7, 8, 9, 10,
            ],
            [
                0, 0x80, 0, 1, 0xff, 0xff, 0xff, 0x7f, 0, 0, 6, 5, 8, 7, 10, 9,
            ],
        ),
    ];
    for end in 0..16 {
        let mut a = std::array::from_fn(|i| 1 + ((i * 17 + end * 3) % 255) as u8);
        let mut b = a;
        a[end] = 0;
        b[15 - end] = 0;
        b[(end + 7) % 16] ^= 0x80;
        samples.push((a, b));
    }
    samples
}

#[test]
fn packed_comparison_all_controls_and_lengths_match_hardware() {
    assert!(
        std::is_x86_feature_detected!("sse4.2"),
        "reference requires SSE4.2"
    );
    let lengths = [
        None,
        Some((0, 0, false)),
        Some((3, 5, false)),
        Some((u64::MAX - 4, 7, false)),
        Some((0x8000_0000, 0x7fff_ffff, false)),
        Some((0xffff_ffff_0000_0001, u64::MAX - 2, false)),
        Some((0xffff_ffff_0000_0001, u64::MAX - 2, true)),
        Some((i64::MIN as u64, i64::MAX as u64, true)),
    ];
    macro_rules! controls {
        ($($mode:literal),*) => {
            for (mode, reference) in [$(($mode, hardware::<$mode> as fn(&[u8; 16], &[u8; 16], Option<(u64,u64,bool)>) -> Comparison)),*] {
                for (a, b) in inputs() {
                    for length in lengths {
                        let expected = reference(&a, &b, length);
                        assert_eq!(compare(&a, &b, length, mode), expected,
                            "control {mode:#x}, lengths {length:?}, a={a:?}, b={b:?}");
                        assert_eq!(compare(&a, &b, length, mode | 0x80), expected);
                    }
                }
            }
        };
    }
    controls!(
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,
        48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70,
        71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93,
        94, 95, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112,
        113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127
    );
}

#[test]
fn packed_comparison_encodings_route_outputs_and_preserve_other_state() {
    for vex in [false, true] {
        for kind in 0..3 {
            for mask in [false, true] {
                for modrm in [0xca, 0xc1, 0xc8, 0x0f] {
                    let opcode = if kind == 0 { 0x62 } else { 0x60 } + u8::from(!mask);
                    let mut code = if vex {
                        vec![0xc4, 0xe3, if kind == 2 { 0xf9 } else { 0x79 }]
                    } else if kind == 2 {
                        vec![0x66, 0x48, 0x0f, 0x3a]
                    } else {
                        vec![0x66, 0x0f, 0x3a]
                    };
                    code.extend_from_slice(&[opcode, modrm, 0x08, 0xc3]);
                    with_source(Some(&code), |source, arena, stack, mut state| {
                        let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + 17;
                        let a = [1; 16];
                        let mut b = [1; 16];
                        b[2] = 2;
                        b[4] = 0;
                        arena.pause().unwrap().write(data, &b).unwrap();
                        state.xmm.fill(0xa5);
                        state.ymm_upper.fill(0x5a);
                        let left_slot = if modrm == 0xc1 { 0 } else { 16 };
                        let right_slot = match modrm {
                            0xc1 => 16,
                            0xc8 => 0,
                            _ => 32,
                        };
                        state.xmm[left_slot..left_slot + 16].copy_from_slice(&a);
                        state.xmm[right_slot..right_slot + 16].copy_from_slice(&b);
                        state.gpr[RDI] = data;
                        state.gpr[RAX] = 0xffff_ffff_0000_0001;
                        state.gpr[RDX] = u64::MAX - 2;
                        state.gpr[RCX] = u64::MAX;
                        state.rflags = 0x602 | 0x8d5;
                        let lengths = if kind == 0 {
                            None
                        } else {
                            Some((state.gpr[RAX], state.gpr[RDX], kind == 2))
                        };
                        let result = hardware::<0x08>(&a, &b, lengths);
                        let mut engine = NativeBlockStepper::new(
                            source,
                            &mut arena.pause().unwrap(),
                            state,
                            stack,
                            &Default::default(),
                        )
                        .unwrap();
                        let planned = block(
                            source,
                            &arena.pause().unwrap(),
                            state.guest_pc,
                            engine.placement(),
                        );
                        engine
                            .resolve(&mut arena.pause().unwrap(), &planned)
                            .unwrap();
                        let mut expected = state;
                        expected.guest_pc = planned.exit().next_guest_pc();
                        expected.rflags = 0x602 | result.flags;
                        if mask {
                            expected.xmm[..16].copy_from_slice(&result.mask);
                            if vex {
                                expected.ymm_upper[..16].fill(0);
                            }
                        } else {
                            expected.gpr[RCX] = result.index;
                        }
                        assert_eq!(engine.machine.state, expected);
                        assert_eq!(engine.usage().vector_operations, 1);
                        assert_eq!(
                            engine.usage().vector_bytes,
                            if modrm == 0x0f { 16 } else { 0 }
                        );
                    });
                }
            }
        }
    }
}

#[test]
fn packed_comparison_refuses_full_span_and_budgets_before_outputs_even_at_zero_length() {
    for case in 0..5 {
        with_source(
            Some(&[0xc4, 0xe3, 0x79, 0x60, 0x0f, 0, 0xc3]),
            |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                state.gpr[RDI] = if case == 0 { data + PAGE - 8 } else { data };
                state.gpr[RAX] = 0;
                state.gpr[RDX] = 0;
                state.xmm.fill(0xa5);
                state.ymm_upper.fill(0x5a);
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    state,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                let planned = block(
                    source,
                    &arena.pause().unwrap(),
                    state.guest_pc,
                    engine.placement(),
                );
                match case {
                    1 => arena
                        .protect(data, PAGE, NativePermissions::new(false, false, false))
                        .unwrap(),
                    2 => engine.machine.limits.max_work_units = engine.usage().work_units,
                    3 => engine.machine.usage.vector_bytes = u64::MAX,
                    4 => engine.machine.usage.vector_operations = u64::MAX,
                    _ => {}
                }
                let operations = engine.usage().vector_operations;
                engine.machine.phase = Phase::Terminal;
                assert!(
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .is_err()
                );
                assert_eq!(engine.machine.state, state);
                assert_eq!(engine.usage().vector_operations, operations);
                assert!(engine.machine.phase == Phase::Terminal);
            },
        );
    }
}
