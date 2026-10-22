// SPDX-License-Identifier: GPL-2.0-only
//! Original literal80 narrowing stores assert raw IEEE bits and complete context.
use super::*;
const J: u64 = 1 << 63;
const FORMS: [(u8, u8, usize, bool); 4] = [
    (0xd9, 0x10, 4, false),
    (0xdd, 0x10, 8, false),
    (0xd9, 0x18, 4, true),
    (0xdd, 0x18, 8, true),
];
fn bits(sig: u64, exp: u16) -> [u8; 10] {
    let mut out = [0; 10];
    out[..8].copy_from_slice(&sig.to_le_bytes());
    out[8..].copy_from_slice(&exp.to_le_bytes());
    out
}
fn tag(s: &mut NativeCpuState, t: usize, v: u16) {
    s.x87.tag_word = (s.x87.tag_word & !(3 << (t * 2))) | (v << (t * 2));
}
fn top(s: &mut NativeCpuState, t: usize) {
    s.x87.status_word = (s.x87.status_word & !0x3800) | ((t as u16) << 11);
}
fn operand(s: &mut NativeCpuState, v: [u8; 10]) {
    let t = usize::from((s.x87.status_word >> 11) & 7);
    s.x87.registers[t] = v;
    let sig = u64::from_le_bytes(v[..8].try_into().unwrap());
    let exp = u16::from_le_bytes(v[8..].try_into().unwrap()) & 0x7fff;
    tag(
        s,
        t,
        if exp == 0 && sig == 0 {
            1
        } else if exp == 0 || exp == 0x7fff {
            2
        } else {
            0
        },
    );
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
    size: usize,
    output: u64,
    setup: impl FnOnce(&mut NativeCpuState),
    expected: impl FnOnce(&mut NativeCpuState),
    error: Option<NativeDispatchError>,
) {
    let mut code = bytes.to_vec();
    code.push(0xc3);
    with_source(Some(&code), |source, arena, stack, mut state| {
        state.rflags = 0xed7;
        state.mxcsr = 0x1fa5;
        state.xmm.fill(0x63);
        state.ymm_upper.fill(0xad);
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
        let data =
            source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE - size as u64 - 1;
        state.gpr[RAX] = data;
        let mut canaries = vec![0xa5; size + 2];
        arena.pause().unwrap().write(data - 1, &canaries).unwrap();
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
                "Original narrowing-store context mismatch"
            );
            assert_eq!(engine.usage().vector_operations, 1);
            assert_eq!(engine.usage().vector_bytes, size as u64);
            assert_eq!(engine.usage().native_entries, 0);
            canaries[1..size + 1].copy_from_slice(&output.to_le_bytes()[..size]);
        }
        let mut actual = vec![0; size + 2];
        arena.pause().unwrap().read(data - 1, &mut actual).unwrap();
        assert_eq!(actual, canaries);
    });
}
#[test]
fn x87_float_store_exact_finite_zero_and_infinity_ignore_pc_rc_preserve_source_and_pop_only_tags() {
    for (first, second, size, popping) in FORMS {
        let (p, bias) = if size == 4 { (24, 127) } else { (53, 1023) };
        for pc in 0..4 {
            for rc in 0..4 {
                for negative in [false, true] {
                    for t in [0, 3, 7] {
                        let sign = u64::from(negative) << (size * 8 - 1);
                        for (sig, exp, result) in [
                            (J, 0x3fff, (bias as u64) << (p - 1)),
                            (
                                0xc000_0000_0000_0000,
                                0x3fff,
                                ((bias as u64) << (p - 1)) | (1 << (p - 2)),
                            ),
                            (0, 0, 0),
                            (J, 0x7fff, ((bias as u64) * 2 + 1) << (p - 1)),
                        ] {
                            run(
                                &[0x66, first, second],
                                size,
                                sign | result,
                                |s| {
                                    top(s, t);
                                    operand(s, bits(sig, exp | if negative { 0x8000 } else { 0 }));
                                    s.x87.control_word = 0x007f | (pc << 8) | (rc << 10);
                                },
                                |s| status(s, 0, popping),
                                None,
                            );
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn x87_float_store_ties_low_bits_carry_and_all_rounding_modes_use_one_destination_round() {
    for (first, second, size, popping) in FORMS {
        let (p, bias) = if size == 4 { (24, 127) } else { (53, 1023) };
        let halfway = 1_u64 << (63 - p);
        for pc in 0..4 {
            for rc in 0..4 {
                for negative in [false, true] {
                    for delta in [halfway - 1, halfway, halfway + 1, 1] {
                        let increment = match rc {
                            0 => delta > halfway,
                            1 => negative,
                            2 => !negative,
                            _ => false,
                        };
                        let sign = u64::from(negative) << (size * 8 - 1);
                        let result = sign | (((bias as u64) << (p - 1)) + u64::from(increment));
                        run(
                            &[first, second],
                            size,
                            result,
                            |s| {
                                operand(
                                    s,
                                    bits(J | delta, 0x3fff | if negative { 0x8000 } else { 0 }),
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
        // Odd halfway rounds to the next even destination significand.
        run(
            &[first, second],
            size,
            ((bias as u64) << (p - 1)) + 2,
            |s| operand(s, bits(J | (3_u64 << (63 - p)), 0x3fff)),
            |s| status(s, 0x220, popping),
            None,
        );
        for rc in 0..4 {
            let increment = rc == 0 || rc == 2;
            let two = ((bias + 1) as u64) << (p - 1);
            let result = if increment { two } else { two - 1 };
            run(
                &[first, second],
                size,
                result,
                |s| {
                    operand(s, bits(u64::MAX, 0x3fff));
                    s.x87.control_word |= rc << 10;
                },
                |s| status(s, 0x20 | if increment { 0x200 } else { 0 }, popping),
                None,
            );
        }
        run(
            &[first, second],
            size,
            0,
            |s| {
                operand(s, bits(J | 1, 0x3fff));
                s.x87.control_word &= !0x20;
            },
            |_| {},
            Some(NativeDispatchError::UnsupportedX87Exception),
        );
    }
}

#[test]
fn x87_float_store_subnormals_and_precision_tininess_preserve_exact_and_inexact_exception_boundaries()
 {
    for (first, second, size, popping) in FORMS {
        let (p, bias) = if size == 4 { (24, 127) } else { (53, 1023) };
        let emin = 1 - bias;
        let minsub = (16383 + emin - (p - 1)) as u16;
        let minimum_normal = 1_u64 << (p - 1);
        for negative in [false, true] {
            let sign = u64::from(negative) << (size * 8 - 1);
            let sx = if negative { 0x8000 } else { 0 };
            for (sig, exp, result) in [
                (J, minsub, 1),
                (J, (16383 + emin) as u16, minimum_normal),
                (
                    ((1_u64 << (p - 1)) - 1) << (65 - p),
                    (16383 + emin - 1) as u16,
                    minimum_normal - 1,
                ),
            ] {
                run(
                    &[first, second],
                    size,
                    sign | result,
                    |s| {
                        operand(s, bits(sig, exp | sx));
                        s.x87.control_word &= !2;
                    },
                    |s| status(s, 0, popping),
                    None,
                );
            }
            for rc in 0..4 {
                let increment = rc == 1 && negative || rc == 2 && !negative;
                let result = sign | u64::from(increment);
                for value in [bits(J, (minsub - 1) | sx), bits(1, sx)] {
                    run(
                        &[first, second],
                        size,
                        result,
                        |s| {
                            operand(s, value);
                            s.x87.control_word = (s.x87.control_word & !2) | (rc << 10);
                        },
                        |s| status(s, 0x30 | if increment { 0x200 } else { 0 }, popping),
                        None,
                    );
                }
            }
            run(
                &[first, second],
                size,
                0,
                |s| {
                    operand(s, bits(J, minsub | sx));
                    s.x87.control_word &= !0x10;
                },
                |_| {},
                Some(NativeDispatchError::UnsupportedX87Exception),
            );
        }
        // Tininess uses destination precision with unbounded exponent. A result
        // stored as minnormal can still be tiny; no intermediate feeds its bits.
        for (gap, tiny) in [
            (1_u64 << (62 - p), false),
            (1_u64 << (63 - p), false),
            (3_u64 << (62 - p), true),
            (1_u64 << (64 - p), true),
        ] {
            let source = bits(0_u64.wrapping_sub(gap), (16383 + emin - 1) as u16);
            run(
                &[first, second],
                size,
                minimum_normal,
                |s| operand(s, source),
                |s| status(s, 0x220 | if tiny { 0x10 } else { 0 }, popping),
                None,
            );
            run(
                &[first, second],
                size,
                minimum_normal,
                |s| {
                    operand(s, source);
                    s.x87.control_word &= !0x10;
                },
                |s| status(s, 0x220, popping),
                if tiny {
                    Some(NativeDispatchError::UnsupportedX87Exception)
                } else {
                    None
                },
            );
        }
        run(
            &[first, second],
            size,
            minimum_normal,
            |s| {
                operand(s, bits(J, (16383 + emin) as u16));
                s.x87.control_word &= !0x10;
            },
            |s| status(s, 0, popping),
            None,
        );
        run(
            &[first, second],
            size,
            0,
            |s| {
                operand(s, bits(J, minsub - 1));
                s.x87.control_word &= !0x20;
            },
            |_| {},
            Some(NativeDispatchError::UnsupportedX87Exception),
        );
    }
}
#[test]
fn x87_float_store_overflow_defaults_follow_sign_rc_and_check_after_destination_precision_rounding()
{
    for (first, second, size, popping) in FORMS {
        let (p, bias) = if size == 4 { (24, 127) } else { (53, 1023) };
        let infinity = ((bias as u64) * 2 + 1) << (p - 1);
        let maximum = infinity - 1;
        for negative in [false, true] {
            let sign = u64::from(negative) << (size * 8 - 1);
            let sx = if negative { 0x8000 } else { 0 };
            for rc in 0..4 {
                let to_inf = rc == 0 || rc == 1 && negative || rc == 2 && !negative;
                let result = sign | if to_inf { infinity } else { maximum };
                run(
                    &[first, second],
                    size,
                    result,
                    |s| {
                        operand(s, bits(J, (16383 + bias + 1) as u16 | sx));
                        s.x87.control_word |= rc << 10;
                    },
                    |s| status(s, 0x28 | if to_inf { 0x200 } else { 0 }, popping),
                    None,
                );
                run(
                    &[first, second],
                    size,
                    0,
                    |s| {
                        operand(s, bits(J, (16383 + bias + 1) as u16 | sx));
                        s.x87.control_word = (s.x87.control_word & !8) | (rc << 10);
                    },
                    |_| {},
                    Some(NativeDispatchError::UnsupportedX87Exception),
                );
            }
            let exact_max = ((1_u64 << p) - 1) << (64 - p);
            run(
                &[first, second],
                size,
                sign | maximum,
                |s| operand(s, bits(exact_max, (16383 + bias) as u16 | sx)),
                |s| status(s, 0, popping),
                None,
            );
        }
        // Slightly above maxfinite remains finite under nearest, with only PE.
        let maximum_sig = ((1_u64 << p) - 1) << (64 - p);
        run(
            &[first, second],
            size,
            maximum,
            |s| {
                operand(
                    s,
                    bits(maximum_sig + (1_u64 << (62 - p)), (16383 + bias) as u16),
                )
            },
            |s| status(s, 0x20, popping),
            None,
        );
        // A halfway carry produces overflow under nearest, but chopping still
        // has an in-range unbounded rounded significand and raises only PE.
        for rc in [0, 3] {
            let overflow = rc == 0;
            run(
                &[first, second],
                size,
                if overflow { infinity } else { maximum },
                |s| {
                    operand(
                        s,
                        bits(maximum_sig + (1_u64 << (63 - p)), (16383 + bias) as u16),
                    );
                    s.x87.control_word |= rc << 10;
                },
                |s| status(s, if overflow { 0x228 } else { 0x20 }, popping),
                None,
            );
        }
    }
}
#[test]
fn x87_float_store_nan_payloads_stack_faults_and_pending_refusals_preserve_original_physical_bits()
{
    for (first, second, size, popping) in FORMS {
        let p = if size == 4 { 24 } else { 53 };
        let infinity = if size == 4 {
            0x7f80_0000
        } else {
            0x7ff0_0000_0000_0000
        };
        let quiet = 1_u64 << (p - 2);
        for negative in [false, true] {
            let sign = u64::from(negative) << (size * 8 - 1);
            let sx = if negative { 0x8000 } else { 0 };
            for (sig, payload, invalid) in [
                (
                    0xc000_0000_0000_0000 | (1_u64 << (64 - p)) | 1,
                    quiet | 1,
                    false,
                ),
                (J | 1, quiet, true),
                (J | (1_u64 << (64 - p)) | 1, quiet | 1, true),
            ] {
                run(
                    &[first, second],
                    size,
                    sign | infinity | payload,
                    |s| operand(s, bits(sig, 0x7fff | sx)),
                    |s| status(s, if invalid { 1 } else { 0 }, popping),
                    None,
                );
                if invalid {
                    run(
                        &[first, second],
                        size,
                        0,
                        |s| {
                            operand(s, bits(sig, 0x7fff | sx));
                            s.x87.control_word &= !1;
                        },
                        |_| {},
                        Some(NativeDispatchError::UnsupportedX87Exception),
                    );
                }
            }
        }
        let indefinite = if size == 4 {
            0xffc0_0000
        } else {
            0xfff8_0000_0000_0000
        };
        run(
            &[first, second],
            size,
            indefinite,
            |s| tag(s, 0, 3),
            |s| status(s, 0x41, popping),
            None,
        );
        run(
            &[first, second],
            size,
            0,
            |s| {
                tag(s, 0, 3);
                s.x87.control_word &= !1;
            },
            |_| {},
            Some(NativeDispatchError::UnsupportedX87Exception),
        );
        run(
            &[first, second],
            size,
            0,
            |s| {
                s.x87.control_word &= !1;
                s.x87.status_word |= 0x8081;
                s.gpr[RAX] = 0x0000_8000_0000_0000;
            },
            |_| {},
            Some(NativeDispatchError::UnsupportedX87Exception),
        );
        for v in [bits(J, 0), bits(1, 1), bits(0, 0x7fff)] {
            run(
                &[first, second],
                size,
                0,
                |s| operand(s, v),
                |_| {},
                Some(NativeDispatchError::UnsupportedVectorInstruction),
            );
        }
    }
}
#[test]
fn x87_float_store_full_write_spans_permissions_source_owner_prefix_and_budget_failures_are_atomic()
{
    for (first, second, width, _) in FORMS {
        for case in 0..8 {
            with_source(
                Some(&[first, second, 0xc3]),
                |source, arena, stack, mut state| {
                    state.x87.tag_word = 0xfffc;
                    state.x87.registers[0] = bits(0xa000_0000_0000_0000, 0x4000);
                    state.x87.status_word = 0x4700;
                    state.rflags = 0xed7;
                    state.xmm.fill(0x61);
                    state.ymm_upper.fill(0xb9);
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
                            .protect(data, PAGE, NativePermissions::new(true, false, false))
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
                    if case == 0 {
                        let expected = if width == 4 {
                            0x4020_0000_u64
                        } else {
                            0x4004_0000_0000_0000
                        };
                        assert_eq!(&actual[..width], &expected.to_le_bytes()[..width]);
                    } else {
                        assert_eq!(&actual[..width], &raw[..width]);
                    }
                },
            );
        }
    }
    for prefix in [0xf2, 0xf3, 0x64] {
        run(
            &[prefix, 0xd9, 0x18],
            4,
            0,
            |_| {},
            |_| {},
            Some(NativeDispatchError::UnsupportedVectorInstruction),
        );
    }
    with_source(Some(&[0xdd, 0x18, 0xc3]), |source, arena, stack, state| {
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
        with_source(Some(&[0xdd, 0x18, 0xc3]), |_, other, _, _| {
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
