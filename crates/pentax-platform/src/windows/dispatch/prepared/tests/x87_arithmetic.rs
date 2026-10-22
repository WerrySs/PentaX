// SPDX-License-Identifier: GPL-2.0-only
//! Original basic-arithmetic values, precision boundaries and atomic refusals.
use super::*;
const J: u64 = 0x8000_0000_0000_0000;
const INDEFINITE: [u8; 10] = [0, 0, 0, 0, 0, 0, 0, 0xc0, 0xff, 0xff];
fn bits(sig: u64, exp: u16) -> [u8; 10] {
    let mut v = [0; 10];
    v[..8].copy_from_slice(&sig.to_le_bytes());
    v[8..].copy_from_slice(&exp.to_le_bytes());
    v
}
fn tag(s: &mut NativeCpuState, slot: usize, value: u16) {
    s.x87.tag_word = (s.x87.tag_word & !(3 << (2 * slot))) | (value << (2 * slot));
}
fn value(s: &mut NativeCpuState, slot: usize, v: [u8; 10]) {
    s.x87.registers[slot] = v;
    let sig = u64::from_le_bytes(v[..8].try_into().unwrap());
    let exp = u16::from_le_bytes(v[8..].try_into().unwrap()) & 0x7fff;
    tag(
        s,
        slot,
        if sig == 0 && exp == 0 {
            1
        } else if exp == 0 || exp == 0x7fff {
            2
        } else {
            0
        },
    );
}
fn set_top(s: &mut NativeCpuState, t: usize) {
    s.x87.status_word = (s.x87.status_word & !0x3800) | ((t as u16) << 11);
}
fn run(
    bytes: &[u8],
    raw: Option<&[u8]>,
    setup: impl FnOnce(&mut NativeCpuState),
    expected: impl FnOnce(&mut NativeCpuState),
    mut error: Option<NativeDispatchError>,
) {
    let mut code = bytes.to_vec();
    code.push(0xc3);
    with_source(Some(&code), |source, arena, stack, mut state| {
        state.rflags = 0xed7;
        state.mxcsr = 0x1fa5;
        state.xmm.fill(0x56);
        state.ymm_upper.fill(0xa7);
        state.x87.control_word = 0x037f;
        state.x87.status_word = 0x4700;
        state.x87.tag_word = 0xffff;
        state.x87.instruction_pointer = 0x1234;
        state.x87.data_pointer = 0x5678;
        state.x87.instruction_opcode = 0x456;
        for (i, v) in state.x87.registers.iter_mut().enumerate() {
            *v = bits(J | i as u64, 0x3ff0 + i as u16);
        }
        value(&mut state, 0, bits(J, 0x4002));
        value(&mut state, 1, bits(J, 0x4000));
        let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
            - raw.map_or(8, |v| v.len()) as u64;
        state.gpr[RAX] = data;
        if let Some(raw) = raw {
            arena.pause().unwrap().write(data, raw).unwrap();
        }
        setup(&mut state);
        if !super::super::x87_arithmetic::host_available()
            && state.x87.status_word & !state.x87.control_word & 0x3f == 0
        {
            error = Some(NativeDispatchError::UnsupportedVectorInstruction);
        }
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
            let first = bytes
                .iter()
                .position(|b| (0xd8..=0xdf).contains(b))
                .unwrap();
            after.x87.instruction_opcode =
                (u16::from(bytes[first] & 7) << 8) | u16::from(bytes[first + 1]);
            if raw.is_some() {
                after.x87.data_pointer = data;
            }
            assert_eq!(
                *engine.state(),
                after,
                "Original arithmetic context mismatch"
            );
            assert_eq!(engine.usage().vector_operations, 1);
            assert_eq!(
                engine.usage().vector_bytes,
                raw.map_or(0, |v| v.len()) as u64
            );
            assert_eq!(engine.usage().native_entries, 0);
        }
        if let Some(raw) = raw {
            let mut actual = vec![0; raw.len()];
            arena.pause().unwrap().read(data, &mut actual).unwrap();
            assert_eq!(actual, raw);
        }
    });
}
#[test]
fn x87_arithmetic_register_destinations_reverse_order_pop_and_all_physical_aliases_are_exact() {
    // family, opcode, ModRM base, destination ST(i), popping, reverse.
    let rows = [
        (0, 0xd8, 0xc0, false, false, false),
        (0, 0xdc, 0xc0, true, false, false),
        (0, 0xde, 0xc0, true, true, false),
        (1, 0xd8, 0xe0, false, false, false),
        (1, 0xdc, 0xe8, true, false, false),
        (1, 0xde, 0xe8, true, true, false),
        (1, 0xd8, 0xe8, false, false, true),
        (1, 0xdc, 0xe0, true, false, true),
        (1, 0xde, 0xe0, true, true, true),
        (2, 0xd8, 0xc8, false, false, false),
        (2, 0xdc, 0xc8, true, false, false),
        (2, 0xde, 0xc8, true, true, false),
        (3, 0xd8, 0xf0, false, false, false),
        (3, 0xdc, 0xf8, true, false, false),
        (3, 0xde, 0xf8, true, true, false),
        (3, 0xd8, 0xf8, false, false, true),
        (3, 0xdc, 0xf0, true, false, true),
        (3, 0xde, 0xf0, true, true, true),
    ];
    for (family, first, second, sti, popping, reverse) in rows {
        for t in [0, 3, 7] {
            for i in 0..8 {
                let destination = if sti { (t + i) & 7 } else { t };
                let source = if sti { t } else { (t + i) & 7 };
                let result = if i == 0 {
                    match family {
                        0 | 2 => bits(J, 0x4001),
                        1 => bits(0, 0),
                        _ => bits(J, 0x3fff),
                    }
                } else {
                    match family {
                        0 => bits(0xa000_0000_0000_0000, 0x4002),
                        1 => bits(0xc000_0000_0000_0000, if reverse { 0xc001 } else { 0x4001 }),
                        2 => bits(J, 0x4003),
                        _ => bits(J, if reverse { 0x3ffd } else { 0x4001 }),
                    }
                };
                run(
                    &[first, second + i as u8],
                    None,
                    |s| {
                        set_top(s, t);
                        value(s, destination, bits(J, 0x4002));
                        value(s, source, bits(J, 0x4000));
                    },
                    |s| {
                        s.x87.status_word &= !0x200;
                        value(s, destination, result);
                        if popping {
                            tag(s, t, 3);
                            set_top(s, (t + 1) & 7);
                        }
                    },
                    None,
                );
            }
        }
    }
}
#[test]
fn x87_arithmetic_all_precision_rounding_modes_keep_wide_exponents_and_avoid_double_rounding() {
    for (pc, p) in [(0_u16, 24_u32), (0x200, 53), (0x300, 64)] {
        for rc in 0..4_u16 {
            for negative in [false, true] {
                let sign = if negative { 0x8000 } else { 0 };
                let increment = if negative { rc == 1 } else { rc == 2 };
                run(
                    &[0xd8, 0xc1],
                    None,
                    |s| {
                        s.x87.control_word = 0x007f | pc | (rc << 10);
                        value(s, 0, bits(J, 0x3fff | sign));
                        value(s, 1, bits(J, (0x3fff - p as u16) | sign));
                    },
                    |s| {
                        value(
                            s,
                            0,
                            bits(
                                J | if increment { 1_u64 << (64 - p) } else { 0 },
                                0x3fff | sign,
                            ),
                        );
                        s.x87.status_word =
                            (s.x87.status_word & !0x200) | 0x20 | if increment { 0x200 } else { 0 };
                    },
                    None,
                );
            }
        }
    }
    // A 64-bit intermediate would round away the extra half-LSB before the
    // lower-precision tie decision and incorrectly choose one instead of up.
    for (pc, p) in [(0_u16, 24_u32), (0x200, 53)] {
        run(
            &[0xd8, 0xc1],
            None,
            |s| {
                s.x87.control_word = 0x007f | pc;
                value(s, 0, bits(J | (1_u64 << (63 - p)), 0x3fff));
                value(s, 1, bits(J, 0x3fbf));
            },
            |s| {
                value(s, 0, bits(J | (1_u64 << (64 - p)), 0x3fff));
                s.x87.status_word |= 0x220;
            },
            None,
        );
        for exp in [0x3b00, 0x4500] {
            run(
                &[0xd8, 0xc1],
                None,
                |s| {
                    s.x87.control_word = 0x007f | pc;
                    value(s, 0, bits(J, exp));
                    value(s, 1, bits(J, exp));
                },
                |s| {
                    value(s, 0, bits(J, exp + 1));
                    s.x87.status_word &= !0x200;
                },
                None,
            );
        }
        run(
            &[0xd8, 0xc1],
            None,
            |s| {
                s.x87.control_word = 0x007f | pc;
                value(s, 0, bits(J | 3, 0x3fff));
                value(s, 1, bits(0, 0));
            },
            |s| {
                value(s, 0, bits(J, 0x3fff));
                s.x87.status_word = (s.x87.status_word & !0x200) | 0x20;
            },
            None,
        );
    }
}
#[test]
fn x87_arithmetic_ieee_memory_nan_selection_and_denormal_priority_preserve_original_sources() {
    for (width, first, raw_two) in [(4, 0xd8, 0x4000_0000_u64), (8, 0xdc, 0x4000_0000_0000_0000)] {
        for (extension, result) in [
            (0, bits(0xa000_0000_0000_0000, 0x4002)),
            (1, bits(J, 0x4003)),
            (4, bits(0xc000_0000_0000_0000, 0x4001)),
            (5, bits(0xc000_0000_0000_0000, 0xc001)),
            (6, bits(J, 0x4001)),
            (7, bits(J, 0x3ffd)),
        ] {
            run(
                &[0x66, first, extension << 3],
                Some(&raw_two.to_le_bytes()[..width]),
                |_| {},
                |s| {
                    value(s, 0, result);
                    s.x87.status_word &= !0x200;
                },
                None,
            );
        }
        let signaling = if width == 4 {
            0x7f80_0001_u64
        } else {
            0x7ff0_0000_0000_0001
        };
        let quiet = if width == 4 {
            bits(0xc000_0100_0000_0000, 0x7fff)
        } else {
            bits(0xc000_0000_0000_0800, 0x7fff)
        };
        run(
            &[first, 0],
            Some(&signaling.to_le_bytes()[..width]),
            |_| {},
            |s| {
                value(s, 0, quiet);
                s.x87.status_word = (s.x87.status_word & !0x200) | 1;
            },
            None,
        );
        let peer = bits(0xc000_0000_0000_0001, 0xffff);
        run(
            &[first, 0],
            Some(&signaling.to_le_bytes()[..width]),
            |s| value(s, 0, peer),
            |s| {
                value(s, 0, peer);
                s.x87.status_word = (s.x87.status_word & !0x200) | 1;
            },
            None,
        );
        // A quiet NaN suppresses raw-source DE even when that exception is
        // unmasked. Conversion to extended must not lose this source property.
        run(
            &[first, 0],
            Some(&1_u64.to_le_bytes()[..width]),
            |s| {
                value(s, 0, peer);
                s.x87.control_word &= !2;
            },
            |s| {
                value(s, 0, peer);
                s.x87.status_word &= !0x200;
            },
            None,
        );
    }
    let small = bits(0xc000_0000_0000_0001, 0x7fff);
    let large = bits(0xc000_0000_0000_0002, 0xffff);
    run(
        &[0xd8, 0xc1],
        None,
        |s| {
            value(s, 0, small);
            value(s, 1, large);
        },
        |s| {
            value(s, 0, large);
            s.x87.status_word &= !0x200;
        },
        None,
    );
    run(
        &[0xd8, 0xc1],
        None,
        |s| {
            value(s, 0, small);
            let mut opposite = small;
            opposite[9] ^= 0x80;
            value(s, 1, opposite);
        },
        |_| {},
        Some(NativeDispatchError::UnsupportedVectorInstruction),
    );
}
#[test]
fn x87_arithmetic_masked_defaults_tininess_signed_zero_and_unmasked_exceptions_are_transactional() {
    for (left, right, op, result, flags) in [
        (bits(0, 0), bits(0, 0), 0xf1, INDEFINITE, 1),
        (bits(J, 0x3fff), bits(0, 0), 0xf1, bits(J, 0x7fff), 4),
        (
            bits(J, 1),
            bits(J, 0x4000),
            0xf1,
            bits(0x4000_0000_0000_0000, 0),
            0,
        ),
        (bits(1, 0), bits(J, 0x4000), 0xf1, bits(0, 0), 0x32),
    ] {
        run(
            &[0xd8, op],
            None,
            |s| {
                value(s, 0, left);
                value(s, 1, right);
                s.x87.control_word = 0x0f7f;
            },
            |s| {
                value(s, 0, result);
                s.x87.status_word = (s.x87.status_word & !0x200) | flags;
            },
            None,
        );
        if flags != 0 {
            let bit = if flags & 1 != 0 {
                1
            } else if flags & 4 != 0 {
                4
            } else {
                0x10
            };
            run(
                &[0xd8, op],
                None,
                |s| {
                    value(s, 0, left);
                    value(s, 1, right);
                    s.x87.control_word = 0x0f7f & !bit;
                },
                |_| {},
                Some(NativeDispatchError::UnsupportedX87Exception),
            );
        }
    }
    // Exact tiny must refuse only for guest UM0; exact zero and a rounded
    // smallest normal are not inferred tiny from their operands.
    run(
        &[0xd8, 0xf1],
        None,
        |s| {
            value(s, 0, bits(J, 1));
            value(s, 1, bits(J, 0x4000));
            s.x87.control_word &= !0x10;
        },
        |_| {},
        Some(NativeDispatchError::UnsupportedX87Exception),
    );
    run(
        &[0xd8, 0xc1],
        None,
        |s| {
            value(s, 0, bits(0x4000_0000_0000_0000, 0));
            value(s, 1, bits(0x4000_0000_0000_0000, 0));
            s.x87.control_word &= !0x10;
        },
        |s| {
            value(s, 0, bits(J, 1));
            s.x87.status_word = (s.x87.status_word & !0x200) | 2;
        },
        None,
    );
    for rc in 0..4_u16 {
        run(
            &[0xd8, 0xe1],
            None,
            |s| {
                value(s, 0, bits(J, 1));
                value(s, 1, bits(J, 1));
                s.x87.control_word = 0x036f | (rc << 10);
            },
            |s| {
                value(s, 0, bits(0, if rc == 1 { 0x8000 } else { 0 }));
                s.x87.status_word &= !0x200;
            },
            None,
        );
    }
    run(
        &[0xd8, 0xc9],
        None,
        |s| {
            value(s, 0, bits(u64::MAX, 0x7ffe));
            value(s, 1, bits(J, 0x4000));
            s.x87.control_word = 0x0f7f;
        },
        |s| {
            value(s, 0, bits(u64::MAX, 0x7ffe));
            s.x87.status_word = (s.x87.status_word & !0x200) | 0x28;
        },
        None,
    );
    for mask in [8, 0x20] {
        run(
            &[0xd8, 0xc9],
            None,
            |s| {
                value(s, 0, bits(u64::MAX, 0x7ffe));
                value(s, 1, bits(J, 0x4000));
                s.x87.control_word = 0x0f7f & !mask;
            },
            |_| {},
            Some(NativeDispatchError::UnsupportedX87Exception),
        );
    }
    run(
        &[0xd8, 0xc1],
        None,
        |s| {
            value(s, 1, bits(1, 0));
            s.x87.control_word &= !2;
        },
        |_| {},
        Some(NativeDispatchError::UnsupportedX87Exception),
    );
}
#[test]
fn x87_arithmetic_stack_priority_pending_reserved_precision_and_disputed_inputs_refuse_atomically()
{
    for popping in [false, true] {
        for missing in [0, 1] {
            let code = if popping { [0xde, 0xe1] } else { [0xd8, 0xe1] };
            let destination = if popping { 1 } else { 0 };
            run(
                &code,
                None,
                |s| tag(s, missing, 3),
                |s| {
                    value(s, destination, INDEFINITE);
                    s.x87.status_word = (s.x87.status_word & !0x200) | 0x41;
                    if popping {
                        tag(s, 0, 3);
                        set_top(s, 1);
                    }
                },
                None,
            );
            run(
                &code,
                None,
                |s| {
                    tag(s, missing, 3);
                    s.x87.control_word &= !1;
                },
                |_| {},
                Some(NativeDispatchError::UnsupportedX87Exception),
            );
        }
    }
    run(
        &[0xd8, 0],
        Some(&0x7f80_0001_u32.to_le_bytes()),
        |s| tag(s, 0, 3),
        |s| {
            value(s, 0, INDEFINITE);
            s.x87.status_word = (s.x87.status_word & !0x200) | 0x41;
        },
        None,
    );
    run(
        &[0xd8, 0],
        Some(&0_u32.to_le_bytes()),
        |s| {
            s.x87.control_word &= !1;
            s.x87.status_word |= 0x8081;
            s.gpr[RAX] = 0x0000_8000_0000_0000;
        },
        |_| {},
        Some(NativeDispatchError::UnsupportedX87Exception),
    );
    run(
        &[0xd8, 0xc1],
        None,
        |s| s.x87.control_word = 0x017f,
        |_| {},
        Some(NativeDispatchError::UnsupportedVectorInstruction),
    );
    for v in [bits(J | 1, 0x7fff), bits(J, 0), bits(1, 1), bits(0, 0x7fff)] {
        run(
            &[0xd8, 0xc1],
            None,
            |s| value(s, 1, v),
            |_| {},
            Some(NativeDispatchError::UnsupportedVectorInstruction),
        );
    }
}

#[test]
fn x87_arithmetic_complete_memory_permissions_source_and_budget_failures_leave_all_state_unchanged()
{
    if !super::super::x87_arithmetic::host_available() {
        run(
            &[0xd8, 0],
            Some(&0x4000_0000_u32.to_le_bytes()),
            |_| {},
            |_| {},
            Some(NativeDispatchError::UnsupportedVectorInstruction),
        );
        return;
    }
    for (width, first, raw) in [(4, 0xd8, 0x4000_0000_u64), (8, 0xdc, 0x4000_0000_0000_0000)] {
        for case in 0..8 {
            let code = [first, 0, 0xc3];
            with_source(Some(&code), |source, arena, stack, mut state| {
                state.x87.tag_word = 0xfffc;
                state.x87.registers[0] = bits(J, 0x3fff);
                state.xmm.fill(0x31);
                state.ymm_upper.fill(0xa9);
                state.rflags = 0xed7;
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let end = data + PAGE;
                arena
                    .pause()
                    .unwrap()
                    .write(end - width as u64, &raw.to_le_bytes()[..width])
                    .unwrap();
                state.gpr[RAX] = if case == 1 {
                    end - width as u64 + 1
                } else if case == 2 {
                    0x0000_8000_0000_0000
                } else {
                    end - width as u64
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
                        .protect(data, PAGE, NativePermissions::new(false, true, false))
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
                let outcome = engine.resolve(&mut arena.pause().unwrap(), &planned);
                if case == 0 {
                    outcome.unwrap();
                    assert_eq!(
                        engine.state().x87.registers[0],
                        bits(0xc000_0000_0000_0000, 0x4000)
                    );
                    assert_eq!(engine.usage().vector_bytes, width as u64);
                } else {
                    if case <= 4 {
                        assert!(matches!(outcome, Err(NativeDispatchError::Memory(_))));
                    } else {
                        assert!(matches!(outcome, Err(NativeDispatchError::Budget(_))));
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
                assert_eq!(&actual[..width], &raw.to_le_bytes()[..width]);
            });
        }
    }
    for prefix in [0xf2, 0xf3, 0x64] {
        run(
            &[prefix, 0xd8, 0xc1],
            None,
            |_| {},
            |_| {},
            Some(NativeDispatchError::UnsupportedVectorInstruction),
        );
    }
}
#[test]
fn x87_arithmetic_precision_aware_tininess_distinguishes_rounded_normal_from_exact_denormal() {
    // (1-2^-25)*minnormal lies halfway below minnormal for PC24. Ties-even
    // rounds to normal, with PE but no UE, even when guest UM is clear.
    let factor = bits(0xffff_ff80_0000_0000, 0x3ffe);
    run(
        &[0xd8, 0xc9],
        None,
        |s| {
            value(s, 0, bits(J, 1));
            value(s, 1, factor);
            s.x87.control_word = 0x006f;
        },
        |s| {
            value(s, 0, bits(J, 1));
            s.x87.status_word |= 0x220;
        },
        None,
    );
    // The same exact product fits64 bits as a true denormal; masked hardware
    // has PE clear, so the explicit guest UM0 exact-tiny boundary must refuse.
    run(
        &[0xd8, 0xc9],
        None,
        |s| {
            value(s, 0, bits(J, 1));
            value(s, 1, factor);
            s.x87.control_word = 0x036f;
        },
        |_| {},
        Some(NativeDispatchError::UnsupportedX87Exception),
    );
}
