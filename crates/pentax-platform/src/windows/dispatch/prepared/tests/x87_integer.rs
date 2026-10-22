// SPDX-License-Identifier: GPL-2.0-only
//! Original integer conversion fixtures consume exact literal80 values.
use super::*;
const J: u64 = 1 << 63;
const STORES: [(u8, u8, usize, bool, bool); 8] = [
    (0xdf, 0x10, 2, false, false),
    (0xdb, 0x10, 4, false, false),
    (0xdf, 0x18, 2, true, false),
    (0xdb, 0x18, 4, true, false),
    (0xdf, 0x38, 8, true, false),
    (0xdf, 0x08, 2, true, true),
    (0xdb, 0x08, 4, true, true),
    (0xdd, 0x08, 8, true, true),
];
fn bits(sig: u64, exp: u16) -> [u8; 10] {
    let mut v = [0; 10];
    v[..8].copy_from_slice(&sig.to_le_bytes());
    v[8..].copy_from_slice(&exp.to_le_bytes());
    v
}
fn tag(s: &mut NativeCpuState, slot: usize, v: u16) {
    s.x87.tag_word = (s.x87.tag_word & !(3 << (slot * 2))) | (v << (slot * 2));
}
fn top(s: &mut NativeCpuState, t: usize) {
    s.x87.status_word = (s.x87.status_word & !0x3800) | ((t as u16) << 11);
}
fn operand(s: &mut NativeCpuState, v: [u8; 10]) {
    let t = usize::from((s.x87.status_word >> 11) & 7);
    s.x87.registers[t] = v;
    tag(s, t, 0);
}
fn status(s: &mut NativeCpuState, flags: u16, popping: bool) {
    s.x87.status_word = (s.x87.status_word & !0x200) | flags;
    if popping {
        let t = usize::from((s.x87.status_word >> 11) & 7);
        tag(s, t, 3);
        top(s, (t + 1) & 7);
    }
}
fn run(
    bytes: &[u8],
    input: &[u8],
    output: Option<&[u8]>,
    setup: impl FnOnce(&mut NativeCpuState),
    expected: impl FnOnce(&mut NativeCpuState),
    error: Option<NativeDispatchError>,
) {
    let mut code = bytes.to_vec();
    code.push(0xc3);
    with_source(Some(&code), |source, arena, stack, mut state| {
        state.rflags = 0xed7;
        state.mxcsr = 0x1fa5;
        state.xmm.fill(0x59);
        state.ymm_upper.fill(0xa3);
        state.x87.control_word = 0x037f;
        state.x87.status_word = 0x4700;
        state.x87.tag_word = 0xffff;
        state.x87.instruction_pointer = 0x1234;
        state.x87.instruction_opcode = 0x456;
        state.x87.data_pointer = 0x5678;
        for (i, v) in state.x87.registers.iter_mut().enumerate() {
            *v = bits(J | i as u64, 0x3ff0 + i as u16);
        }
        operand(&mut state, bits(J, 0x3fff));
        let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
            - input.len() as u64
            - 1;
        state.gpr[RAX] = data;
        let mut initial = vec![0xa5; input.len() + 2];
        initial[1..input.len() + 1].copy_from_slice(input);
        arena.pause().unwrap().write(data - 1, &initial).unwrap();
        setup(&mut state);
        let before = state;
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
        engine.machine.phase = Phase::Terminal;
        let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
        if let Some(error) = error {
            assert_eq!(result, Err(error));
            assert_eq!(*engine.state(), before);
            assert_eq!(engine.usage().vector_operations, 0);
            assert_eq!(engine.usage().vector_bytes, 0);
            assert!(engine.machine.phase == Phase::Terminal);
        } else {
            result.unwrap();
            let mut after = before;
            expected(&mut after);
            after.guest_pc = planned.exit().next_guest_pc();
            after.x87.instruction_pointer = before.guest_pc;
            after.x87.data_pointer = data;
            let first = bytes
                .iter()
                .position(|b| (0xd8..=0xdf).contains(b))
                .unwrap();
            after.x87.instruction_opcode =
                (u16::from(bytes[first] & 7) << 8) | u16::from(bytes[first + 1]);
            assert_eq!(
                *engine.state(),
                after,
                "Original integer-conversion context mismatch"
            );
            assert_eq!(engine.usage().vector_operations, 1);
            assert_eq!(engine.usage().vector_bytes, input.len() as u64);
            assert_eq!(engine.usage().native_entries, 0);
            if let Some(output) = output {
                initial[1..input.len() + 1].copy_from_slice(output);
            }
        }
        let mut actual = vec![0; initial.len()];
        arena.pause().unwrap().read(data - 1, &mut actual).unwrap();
        assert_eq!(actual, initial);
    });
}
#[test]
fn x87_integer_all_store_forms_round_once_with_every_rc_pc_sign_top_and_preserve_context() {
    for (first, second, size, popping, trunc) in STORES {
        for pc in 0..4 {
            for rc in 0..4 {
                for negative in [false, true] {
                    for t in [0, 3, 7] {
                        // +/-2.5: nearest-even=2, directed magnitude increment only towards sign.
                        let increment = !trunc && ((rc == 1 && negative) || (rc == 2 && !negative));
                        let magnitude = if increment { 3_u64 } else { 2 };
                        let integer = if negative {
                            magnitude.wrapping_neg()
                        } else {
                            magnitude
                        };
                        run(
                            &[0x66, first, second],
                            &[0xa5; 8][..size],
                            Some(&integer.to_le_bytes()[..size]),
                            |s| {
                                top(s, t);
                                operand(
                                    s,
                                    bits(
                                        0xa000_0000_0000_0000,
                                        0x4000 | if negative { 0x8000 } else { 0 },
                                    ),
                                );
                                s.x87.control_word = 0x007f | (pc << 8) | (rc << 10);
                            },
                            |s| status(s, 0x20 | if increment { 0x200 } else { 0 }, popping),
                            None,
                        );
                    }
                }
            }
        }
    }
    // Halfway odd rounds to even; a single retained bit above half changes rounding.
    for (sig, integer, c1) in [
        (0xe000_0000_0000_0000, 4_u64, true),
        (0xa000_0000_0000_0001, 3, false),
    ] {
        run(
            &[0xdd, 0x08],
            &[0; 8],
            Some(&(if integer == 4 { 3_u64 } else { 2 }).to_le_bytes()),
            |s| operand(s, bits(sig, 0x4000)),
            |s| status(s, 0x20, true),
            None,
        );
        run(
            &[0xdf, 0x38],
            &[0; 8],
            Some(&integer.to_le_bytes()),
            |s| operand(s, bits(sig, 0x4000)),
            |s| status(s, 0x20 | if c1 || integer == 3 { 0x200 } else { 0 }, true),
            None,
        );
    }
}
#[test]
fn x87_integer_signed_limits_range_after_rounding_invalid_priority_and_sticky_precision() {
    for (first, second, size, popping, trunc) in STORES {
        let width = size * 8;
        let indefinite = 1_u64 << (width - 1);
        // Exact negative minimum is valid, despite sharing the indefinite bits.
        for negative in [false, true] {
            run(
                &[first, second],
                &[0; 8][..size],
                Some(&indefinite.to_le_bytes()[..size]),
                |s| {
                    operand(
                        s,
                        bits(
                            J,
                            (16383 + width - 1) as u16 | if negative { 0x8000 } else { 0 },
                        ),
                    )
                },
                |s| status(s, if negative { 0 } else { 1 }, popping),
                None,
            );
        }
        if size < 8 {
            let magnitude = (indefinite as u128) * 4 + 3;
            let highest = 127 - magnitude.leading_zeros();
            let sig = (magnitude << (63 - highest)) as u64;
            let exponent = (16383 + highest - 2) as u16 | 0x8000;
            for rc in 0..4 {
                let invalid = !trunc && (rc == 0 || rc == 1);
                run(
                    &[first, second],
                    &[0; 8][..size],
                    Some(&indefinite.to_le_bytes()[..size]),
                    |s| {
                        operand(s, bits(sig, exponent));
                        s.x87.control_word |= rc << 10;
                    },
                    |s| status(s, if invalid { 1 } else { 0x20 }, popping),
                    None,
                );
            }
        }
        // Out-of-range fractional conversion must not raise PE even if PM=0.
        run(
            &[first, second],
            &[0; 8][..size],
            Some(&indefinite.to_le_bytes()[..size]),
            |s| {
                operand(s, bits(J | 1, (16383 + width) as u16));
                s.x87.control_word &= !0x20;
            },
            |s| status(s, 1, popping),
            None,
        );
        run(
            &[first, second],
            &[0; 8][..size],
            Some(&indefinite.to_le_bytes()[..size]),
            |s| {
                operand(s, bits(J, 0x7fff));
                s.x87.status_word |= 0x20;
            },
            |s| status(s, 1, popping),
            None,
        );
    }
}
#[test]
fn x87_integer_denormals_zero_nan_infinity_and_unmasked_flags_have_exact_atomic_outcomes() {
    for (first, second, size, popping, trunc) in STORES {
        for negative in [false, true] {
            for rc in 0..4 {
                let increment = !trunc && ((rc == 1 && negative) || (rc == 2 && !negative));
                let integer = if increment {
                    if negative { u64::MAX } else { 1 }
                } else {
                    0
                };
                run(
                    &[first, second],
                    &[0; 8][..size],
                    Some(&integer.to_le_bytes()[..size]),
                    |s| {
                        operand(s, bits(1, if negative { 0x8000 } else { 0 }));
                        s.x87.control_word = (s.x87.control_word & !2) | (rc << 10);
                    },
                    |s| status(s, 0x20 | if increment { 0x200 } else { 0 }, popping),
                    None,
                );
            }
        }
        for v in [bits(0, 0), bits(0, 0x8000)] {
            run(
                &[first, second],
                &[0; 8][..size],
                Some(&[0; 8][..size]),
                |s| operand(s, v),
                |s| status(s, 0, popping),
                None,
            );
        }
        for v in [
            bits(J, 0x7fff),
            bits(J, 0xffff),
            bits(0xc000_0000_0000_0001, 0x7fff),
            bits(J | 1, 0xffff),
        ] {
            let integer = 1_u64 << (size * 8 - 1);
            run(
                &[first, second],
                &[0; 8][..size],
                Some(&integer.to_le_bytes()[..size]),
                |s| operand(s, v),
                |s| status(s, 1, popping),
                None,
            );
            run(
                &[first, second],
                &[0; 8][..size],
                None,
                |s| {
                    operand(s, v);
                    s.x87.control_word &= !1;
                },
                |_| {},
                Some(NativeDispatchError::UnsupportedX87Exception),
            );
        }
        run(
            &[first, second],
            &[0; 8][..size],
            None,
            |s| {
                operand(s, bits(0xa000_0000_0000_0000, 0x4000));
                s.x87.control_word &= !0x20;
            },
            |_| {},
            Some(NativeDispatchError::UnsupportedX87Exception),
        );
    }
}
#[test]
fn x87_integer_loads_all_signed_widths_are_exact_for_every_pc_rc_and_rotate_physical_stack() {
    for (first, second, size) in [(0xdf, 0x00, 2), (0xdb, 0x00, 4), (0xdf, 0x28, 8)] {
        let values = match size {
            2 => [0, 1, -1, i64::from(i16::MIN), i64::from(i16::MAX)],
            4 => [0, 1, -1, i64::from(i32::MIN), i64::from(i32::MAX)],
            _ => [0, 1, -1, i64::MIN, i64::MAX],
        };
        for integer in values {
            for pc in 0..4 {
                for rc in 0..4 {
                    for t in [0, 7] {
                        let magnitude = integer.unsigned_abs();
                        let expected = if magnitude == 0 {
                            [0; 10]
                        } else {
                            let highest = 63 - magnitude.leading_zeros();
                            bits(
                                magnitude << (63 - highest),
                                (16383 + highest) as u16 | if integer < 0 { 0x8000 } else { 0 },
                            )
                        };
                        run(
                            &[first, second],
                            &integer.to_le_bytes()[..size],
                            None,
                            |s| {
                                top(s, t);
                                tag(s, (t + 7) & 7, 3);
                                s.x87.control_word = 0x007f | (pc << 8) | (rc << 10);
                            },
                            |s| {
                                status(s, 0, false);
                                let destination = (t + 7) & 7;
                                s.x87.registers[destination] = expected;
                                tag(s, destination, if integer == 0 { 1 } else { 0 });
                                top(s, destination);
                            },
                            None,
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn x87_integer_masked_stack_faults_pending_and_unsupported_values_do_not_leak_partial_effects() {
    for (first, second, size, popping, _) in STORES {
        let integer = 1_u64 << (size * 8 - 1);
        run(
            &[first, second],
            &[0; 8][..size],
            Some(&integer.to_le_bytes()[..size]),
            |s| tag(s, 0, 3),
            |s| status(s, 0x41, popping),
            None,
        );
        run(
            &[first, second],
            &[0; 8][..size],
            None,
            |s| {
                tag(s, 0, 3);
                s.x87.control_word &= !1;
            },
            |_| {},
            Some(NativeDispatchError::UnsupportedX87Exception),
        );
        for v in [bits(J, 0), bits(1, 1), bits(0, 0x7fff)] {
            run(
                &[first, second],
                &[0; 8][..size],
                None,
                |s| operand(s, v),
                |_| {},
                Some(NativeDispatchError::UnsupportedVectorInstruction),
            );
        }
        run(
            &[first, second],
            &[0; 8][..size],
            None,
            |s| {
                s.x87.control_word &= !1;
                s.x87.status_word |= 0x8081;
                s.gpr[RAX] = 0x0000_8000_0000_0000;
            },
            |_| {},
            Some(NativeDispatchError::UnsupportedX87Exception),
        );
    }
    for (first, second, size) in [(0xdf, 0, 2), (0xdb, 0, 4), (0xdf, 0x28, 8)] {
        run(
            &[first, second],
            &[0; 8][..size],
            None,
            |s| {
                for t in 0..8 {
                    tag(s, t, 0);
                }
            },
            |s| {
                status(s, 0x0241, false);
                s.x87.registers[7] = bits(0xc000_0000_0000_0000, 0xffff);
                tag(s, 7, 2);
                top(s, 7);
            },
            None,
        );
        run(
            &[first, second],
            &[0; 8][..size],
            None,
            |s| {
                tag(s, 7, 0);
                s.x87.control_word &= !1;
            },
            |_| {},
            Some(NativeDispatchError::UnsupportedX87Exception),
        );
    }
}

#[test]
fn x87_integer_low_residues_exact_maxima_and_boundary_ties_are_not_prerounded_to_pc() {
    for pc in 0..4 {
        // 2 + one full80 low bit: only upward rounds to 3, for every PC setting.
        for rc in 0..4 {
            let inc = rc == 2;
            let result = if inc { 3_u64 } else { 2 };
            run(
                &[0xdf, 0x38],
                &[0; 8],
                Some(&result.to_le_bytes()),
                |s| {
                    operand(s, bits(J | 1, 0x4000));
                    s.x87.control_word = 0x007f | (pc << 8) | (rc << 10);
                },
                |s| status(s, 0x20 | if inc { 0x200 } else { 0 }, true),
                None,
            );
        }
        run(
            &[0xdf, 0x38],
            &[0; 8],
            Some(&i64::MAX.to_le_bytes()),
            |s| {
                operand(s, bits(u64::MAX - 1, 0x403d));
                s.x87.control_word = 0x007f | (pc << 8);
            },
            |s| status(s, 0, true),
            None,
        );
    }
    for (first, second, size, _, _) in STORES {
        if size < 8 {
            let maximum = (1_u64 << (size * 8 - 1)) - 1;
            let highest = 63 - maximum.leading_zeros();
            run(
                &[first, second],
                &[0; 8][..size],
                Some(&maximum.to_le_bytes()[..size]),
                |s| operand(s, bits(maximum << (63 - highest), (16383 + highest) as u16)),
                |s| status(s, 0, second != 0x10),
                None,
            );
        }
    }
    // -32768.5 nearest-even is valid MIN; +32767.5 nearest-even is invalid.
    for (sig, exp, invalid) in [
        (0x8000_8000_0000_0000, 0xc00e, false),
        (0xffff_0000_0000_0000, 0x400d, true),
    ] {
        run(
            &[0xdf, 0x18],
            &[0; 2],
            Some(&i16::MIN.to_le_bytes()),
            |s| operand(s, bits(sig, exp)),
            |s| status(s, if invalid { 1 } else { 0x20 }, true),
            None,
        );
    }
}
#[test]
fn x87_integer_permissions_full_spans_source_owner_prefixes_and_budgets_guard_every_publication() {
    let forms = [
        (0xdf, 0x10, 2, false),
        (0xdb, 0x10, 4, false),
        (0xdf, 0x38, 8, false),
        (0xdd, 0x08, 8, false),
        (0xdf, 0x00, 2, true),
        (0xdb, 0x00, 4, true),
        (0xdf, 0x28, 8, true),
    ];
    for (first, second, width, load) in forms {
        for case in 0..8 {
            let code = [first, second, 0xc3];
            with_source(Some(&code), |source, arena, stack, mut state| {
                state.x87.tag_word = 0xfffc;
                state.x87.registers[0] = bits(0xa000_0000_0000_0000, 0x4000);
                state.x87.status_word = 0x4700;
                state.rflags = 0xed7;
                state.xmm.fill(0x51);
                state.ymm_upper.fill(0xab);
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let end = data + PAGE;
                let raw = [0xa5; 8];
                arena
                    .pause()
                    .unwrap()
                    .write(end - width as u64, &raw[..width])
                    .unwrap();
                state.gpr[RAX] = match case {
                    1 => end - width as u64 + 1,
                    2 => 0x0000_8000_0000_0000,
                    _ => end - width as u64,
                };
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
                    3 => arena
                        .protect(data, PAGE, NativePermissions::new(!load, load, false))
                        .unwrap(),
                    4 => arena
                        .protect(
                            source.source_plan().modules()[0].load_bias(),
                            PAGE,
                            NativePermissions::READ_WRITE,
                        )
                        .unwrap(),
                    5 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                    6 => engine.machine.usage.vector_bytes = u64::MAX - width as u64 + 1,
                    7 => engine.machine.usage.vector_operations = u64::MAX,
                    _ => {}
                }
                let usage = engine.usage();
                engine.machine.phase = Phase::Terminal;
                let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                if case == 0 {
                    result.unwrap();
                    assert_eq!(engine.usage().vector_bytes, width as u64);
                } else {
                    if case <= 4 {
                        assert!(matches!(result, Err(NativeDispatchError::Memory(_))));
                    } else {
                        assert!(matches!(result, Err(NativeDispatchError::Budget(_))));
                    }
                    assert_eq!(*engine.state(), state);
                    assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                    assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
                    assert!(engine.machine.phase == Phase::Terminal);
                }
                if case == 3 {
                    arena
                        .protect(data, PAGE, NativePermissions::READ_WRITE)
                        .unwrap();
                }
                let mut actual = [0; 8];
                arena
                    .pause()
                    .unwrap()
                    .read(end - width as u64, &mut actual[..width])
                    .unwrap();
                if case != 0 || load {
                    assert_eq!(&actual[..width], &raw[..width]);
                } else {
                    assert_eq!(&actual[..width], &2_u64.to_le_bytes()[..width]);
                }
            });
        }
    }
    for prefix in [0xf2, 0xf3, 0x64] {
        run(
            &[prefix, 0xdd, 0x08],
            &[0; 8],
            None,
            |_| {},
            |_| {},
            Some(NativeDispatchError::UnsupportedVectorInstruction),
        );
    }
    with_source(Some(&[0xdd, 0x08, 0xc3]), |source, arena, stack, state| {
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
        with_source(Some(&[0xdd, 0x08, 0xc3]), |_, other, _, _| {
            assert_eq!(
                engine.preflight_block(&other.pause().unwrap(), &planned),
                Err(NativeDispatchError::WrongOwner)
            )
        });
        assert_eq!(*engine.state(), state);
        assert_eq!(engine.usage().vector_operations, 0);
        assert_eq!(engine.usage().vector_bytes, 0);
    });
}
