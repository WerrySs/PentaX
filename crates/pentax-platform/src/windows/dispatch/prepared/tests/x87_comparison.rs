// SPDX-License-Identifier: GPL-2.0-only
//! Original comparisons consume literal80 inputs and assert complete CPU state.
use super::*;
const J: u64 = 0x8000_0000_0000_0000;
fn bits(sig: u64, exp: u16) -> [u8; 10] {
    let mut v = [0; 10];
    v[..8].copy_from_slice(&sig.to_le_bytes());
    v[8..].copy_from_slice(&exp.to_le_bytes());
    v
}
fn tag(s: &mut NativeCpuState, slot: usize, value: u16) {
    s.x87.tag_word = (s.x87.tag_word & !(3 << (slot * 2))) | (value << (slot * 2));
}
fn value(s: &mut NativeCpuState, slot: usize, v: [u8; 10]) {
    s.x87.registers[slot] = v;
    let sig = u64::from_le_bytes(v[..8].try_into().unwrap());
    let exp = u16::from_le_bytes(v[8..].try_into().unwrap()) & 0x7fff;
    tag(
        s,
        slot,
        if exp == 0 && sig == 0 {
            1
        } else if exp == 0 || exp == 0x7fff {
            2
        } else {
            0
        },
    );
}
fn top(s: &mut NativeCpuState, t: usize) {
    s.x87.status_word = (s.x87.status_word & !0x3800) | ((t as u16) << 11);
}
fn outcome(s: &mut NativeCpuState, relation: u8, eflags: bool, pops: usize) {
    let (cpu, status) = match relation {
        0 => (0, 0),
        1 => (1, 0x100),
        4 => (0x40, 0x4000),
        7 => (0x45, 0x4500),
        _ => panic!("Invalid original relation"),
    };
    if eflags {
        s.rflags = (s.rflags & !0x8d5) | cpu;
    } else {
        s.x87.status_word = (s.x87.status_word & !0x4700) | status;
    }
    for _ in 0..pops {
        let old = usize::from((s.x87.status_word >> 11) & 7);
        tag(s, old, 3);
        top(s, (old + 1) & 7);
    }
}
fn run(
    bytes: &[u8],
    raw: Option<&[u8]>,
    setup: impl FnOnce(&mut NativeCpuState),
    expected: impl FnOnce(&mut NativeCpuState),
    error: Option<NativeDispatchError>,
) {
    let mut code = bytes.to_vec();
    code.push(0xc3);
    with_source(Some(&code), |source, arena, stack, mut state| {
        // OF/SF/AF and x87 C1 start at one: COMI clears the three CPU flags,
        // retains non-stack C1 and leaves all unrelated CPU state intact.
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
        value(&mut state, 0, bits(J, 0x4000));
        value(&mut state, 1, bits(J, 0x3fff));
        let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
            - raw.map_or(8, |v| v.len()) as u64;
        state.gpr[RAX] = data;
        if let Some(raw) = raw {
            arena.pause().unwrap().write(data, raw).unwrap();
        }
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
                "Original comparison context mismatch"
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
fn x87_comparison_register_relations_cpu_flags_aliases_and_pops_preserve_all_physical_payloads() {
    let forms = [
        (0xd8, 0xd0, false, 0),
        (0xd8, 0xd8, false, 1),
        (0xdd, 0xe0, false, 0),
        (0xdd, 0xe8, false, 1),
        (0xdb, 0xf0, true, 0),
        (0xdf, 0xf0, true, 1),
        (0xdb, 0xe8, true, 0),
        (0xdf, 0xe8, true, 1),
    ];
    let pairs = [
        (bits(J, 0x4000), bits(J, 0x3fff), 0),
        (bits(J, 0x3fff), bits(J, 0x4000), 1),
        (bits(J, 0x3fff), bits(J, 0x3fff), 4),
        (bits(J | 1, 0x3fff), bits(J, 0x3fff), 0),
        (bits(J, 0x3fff), bits(J | 1, 0x3fff), 1),
        (bits(J, 0xbfff), bits(J | 1, 0xbfff), 0),
        (bits(0, 0), bits(0, 0x8000), 4),
        (bits(0, 0x8000), bits(1, 0), 1),
        (bits(1, 0x8000), bits(0, 0), 1),
        (bits(J - 1, 0), bits(J, 1), 1),
        (bits(J, 1), bits(J - 1, 0), 0),
        (bits(J, 0x7fff), bits(u64::MAX, 0x7ffe), 0),
        (bits(J, 0xffff), bits(J, 0xbfff), 1),
        (bits(J, 0x7fff), bits(J, 0x7fff), 4),
        (bits(J, 0x3b00), bits(J, 0x4500), 1),
    ];
    for (first, second, eflags, pops) in forms {
        for t in [0, 3, 7] {
            for index in 0..8 {
                for (case, (left, right, relation)) in pairs.into_iter().enumerate() {
                    if (t != 0 || index != 1) && case >= 3 {
                        continue;
                    }
                    let source = (t + index) & 7;
                    let relation = if index == 0 { 4 } else { relation };
                    let de = (index != 0
                        && u16::from_le_bytes(left[8..].try_into().unwrap()) & 0x7fff == 0
                        && u64::from_le_bytes(left[..8].try_into().unwrap()) != 0)
                        || (u16::from_le_bytes(right[8..].try_into().unwrap()) & 0x7fff == 0
                            && u64::from_le_bytes(right[..8].try_into().unwrap()) != 0);
                    run(
                        &[first, second + index as u8],
                        None,
                        |s| {
                            top(s, t);
                            value(s, t, left);
                            value(s, source, right);
                        },
                        |s| {
                            outcome(s, relation, eflags, pops);
                            if de {
                                s.x87.status_word |= 2;
                            }
                        },
                        None,
                    );
                }
            }
        }
    }
    for (first, second) in [(0xde, 0xd9), (0xda, 0xe9)] {
        for t in [0, 3, 7] {
            run(
                &[first, second],
                None,
                |s| {
                    top(s, t);
                    value(s, t, bits(J, 0x3fff));
                    value(s, (t + 1) & 7, bits(J, 0x4000));
                },
                |s| outcome(s, 1, false, 2),
                None,
            );
        }
    }
}
#[test]
fn x87_comparison_ignores_precision_and_rounding_without_discarding_extended_low_bits() {
    for pc in [0_u16, 0x100, 0x200, 0x300] {
        for rc in 0..4_u16 {
            for code in [[0xd8, 0xd1], [0xdb, 0xe9]] {
                run(
                    &code,
                    None,
                    |s| {
                        s.x87.control_word = 0x007f | pc | (rc << 10);
                        value(s, 0, bits(J, 0x3fff));
                        value(s, 1, bits(J | 1, 0x3fff));
                    },
                    |s| outcome(s, 1, code[0] == 0xdb, 0),
                    None,
                );
            }
        }
    }
    for (value, relation) in [
        (bits(0, 0), 4),
        (bits(0, 0x8000), 4),
        (bits(J, 0x3fff), 0),
        (bits(J, 0xbfff), 1),
        (bits(1, 0), 0),
        (bits(1, 0x8000), 1),
        (bits(J, 0x7fff), 0),
        (bits(J, 0xffff), 1),
    ] {
        run(
            &[0xd9, 0xe4],
            None,
            |s| self::value(s, 0, value),
            |s| {
                outcome(s, relation, false, 0);
                if value[8] == 0 && value[9] & 0x7f == 0 && value[..8].iter().any(|b| *b != 0) {
                    s.x87.status_word |= 2;
                }
            },
            None,
        );
    }
}
#[test]
fn x87_comparison_memory_ieee_widths_nan_denormal_and_pop_results_keep_exact_input_bytes() {
    let single = [
        (0_u64, 0, 0),
        (0x8000_0000, 0, 0),
        (0x3f80_0000, 4, 0),
        (0x4000_0000, 1, 0),
        (0xbf80_0000, 0, 0),
        (0x7f80_0000, 1, 0),
        (0xff80_0000, 0, 0),
        (1, 0, 2),
        (0x7fc0_0001, 7, 1),
        (0x7f80_0001, 7, 1),
    ];
    let double = [
        (0_u64, 0, 0),
        (0x8000_0000_0000_0000, 0, 0),
        (0x3ff0_0000_0000_0000, 4, 0),
        (0x4000_0000_0000_0000, 1, 0),
        (0xbff0_0000_0000_0000, 0, 0),
        (0x7ff0_0000_0000_0000, 1, 0),
        (0xfff0_0000_0000_0000, 0, 0),
        (1, 0, 2),
        (0x7ff8_0000_0000_0001, 7, 1),
        (0x7ff0_0000_0000_0001, 7, 1),
    ];
    for (width, first, cases) in [(4, 0xd8, &single), (8, 0xdc, &double)] {
        for (second, pops) in [(0x10, 0), (0x18, 1)] {
            for &(raw, relation, flags) in cases {
                run(
                    &[0x66, first, second],
                    Some(&raw.to_le_bytes()[..width]),
                    |s| value(s, 0, bits(J, 0x3fff)),
                    |s| {
                        outcome(s, relation, false, pops);
                        s.x87.status_word |= flags;
                    },
                    None,
                );
            }
        }
    }
}
#[test]
fn x87_comparison_ordered_unordered_qnan_snan_and_denormal_exceptions_have_target_priority() {
    let qnan = bits(0xc000_0000_0000_0001, 0x7fff);
    let snan = bits(J | 1, 0xffff);
    for (first, second, unordered, eflags, pops) in [
        (0xd8, 0xd1, false, false, 0),
        (0xd8, 0xd9, false, false, 1),
        (0xdd, 0xe1, true, false, 0),
        (0xdd, 0xe9, true, false, 1),
        (0xdb, 0xf1, false, true, 0),
        (0xdf, 0xf1, false, true, 1),
        (0xdb, 0xe9, true, true, 0),
        (0xdf, 0xe9, true, true, 1),
    ] {
        for (operand, signaling) in [(qnan, false), (snan, true)] {
            for slot in [0, 1] {
                for masked in [false, true] {
                    let invalid = !unordered || signaling;
                    run(
                        &[first, second],
                        None,
                        |s| {
                            value(s, slot, operand);
                            if !masked {
                                s.x87.control_word &= !1;
                            }
                        },
                        |s| {
                            outcome(s, 7, eflags, pops);
                            if invalid {
                                s.x87.status_word |= 1;
                            }
                        },
                        if invalid && !masked {
                            Some(NativeDispatchError::UnsupportedX87Exception)
                        } else {
                            None
                        },
                    );
                }
            }
        }
        // QNaN decisions precede and suppress DE even when DM is clear.
        run(
            &[first, second],
            None,
            |s| {
                value(s, 0, qnan);
                value(s, 1, bits(1, 0));
                s.x87.control_word &= !2;
            },
            |s| {
                outcome(s, 7, eflags, pops);
                if !unordered {
                    s.x87.status_word |= 1;
                }
            },
            None,
        );
        run(
            &[first, second],
            None,
            |s| {
                value(s, 1, bits(1, 0));
                s.x87.control_word &= !2;
            },
            |_| {},
            Some(NativeDispatchError::UnsupportedX87Exception),
        );
    }
    for (width, first, snan) in [(4, 0xd8, 0x7f80_0001_u64), (8, 0xdc, 0x7ff0_0000_0000_0001)] {
        for raw in [snan, 1] {
            let mask = if raw == 1 { 2 } else { 1 };
            run(
                &[first, 0x18],
                Some(&raw.to_le_bytes()[..width]),
                |s| s.x87.control_word &= !mask,
                |_| {},
                Some(NativeDispatchError::UnsupportedX87Exception),
            );
        }
    }
    run(
        &[0xd9, 0xe4],
        None,
        |s| {
            value(s, 0, qnan);
            s.x87.control_word &= !1;
        },
        |_| {},
        Some(NativeDispatchError::UnsupportedX87Exception),
    );
}
#[test]
fn x87_comparison_stack_underflow_pending_and_unsupported_encodings_publish_no_partial_state() {
    for (bytes, eflags, pops) in [
        (vec![0xd8, 0xd1], false, 0),
        (vec![0xd8, 0xd9], false, 1),
        (vec![0xdb, 0xe9], true, 0),
        (vec![0xdf, 0xe9], true, 1),
        (vec![0xde, 0xd9], false, 2),
        (vec![0xda, 0xe9], false, 2),
    ] {
        for t in [0, 7] {
            for missing in [0, 1] {
                run(
                    &bytes,
                    None,
                    |s| {
                        top(s, t);
                        value(s, t, bits(J, 0x3fff));
                        value(s, (t + 1) & 7, bits(J, 0x4000));
                        tag(s, (t + missing) & 7, 3);
                    },
                    |s| {
                        outcome(s, 7, eflags, pops);
                        s.x87.status_word = (s.x87.status_word & !0x200) | 0x41;
                    },
                    None,
                );
                run(
                    &bytes,
                    None,
                    |s| {
                        top(s, t);
                        tag(s, (t + missing) & 7, 3);
                        s.x87.control_word &= !1;
                    },
                    |_| {},
                    Some(NativeDispatchError::UnsupportedX87Exception),
                );
            }
        }
    }
    // Stack priority avoids classifying an unsupported peer, and memory is
    // still fully read before a masked stack outcome can become visible.
    run(
        &[0xdb, 0xe9],
        None,
        |s| {
            tag(s, 0, 3);
            value(s, 1, bits(J, 0));
        },
        |s| {
            outcome(s, 7, true, 0);
            s.x87.status_word = (s.x87.status_word & !0x200) | 0x41;
        },
        None,
    );
    run(
        &[0xd8, 0x18],
        Some(&0x7f80_0001_u32.to_le_bytes()),
        |s| tag(s, 0, 3),
        |s| {
            outcome(s, 7, false, 1);
            s.x87.status_word = (s.x87.status_word & !0x200) | 0x41;
        },
        None,
    );
    run(
        &[0xd8, 0x10],
        Some(&0_u32.to_le_bytes()),
        |s| {
            s.x87.control_word &= !1;
            s.x87.status_word |= 0x8081;
            s.gpr[RAX] = 0x0000_8000_0000_0000;
        },
        |_| {},
        Some(NativeDispatchError::UnsupportedX87Exception),
    );
    for operand in [bits(J, 0), bits(1, 1), bits(0, 0x7fff)] {
        run(
            &[0xdb, 0xe9],
            None,
            |s| value(s, 1, operand),
            |_| {},
            Some(NativeDispatchError::UnsupportedVectorInstruction),
        );
    }
}

#[test]
fn x87_comparison_memory_owner_source_prefix_and_budget_refusals_keep_flags_top_and_payloads() {
    for (width, first, raw) in [(4, 0xd8, 0x3f80_0000_u64), (8, 0xdc, 0x3ff0_0000_0000_0000)] {
        for case in 0..8 {
            let code = [first, 0x18, 0xc3];
            with_source(Some(&code), |source, arena, stack, mut state| {
                state.x87.tag_word = 0xfffc;
                state.x87.registers[0] = bits(J, 0x4000);
                state.x87.status_word = 0x4700;
                state.rflags = 0xed7;
                state.xmm.fill(0x31);
                state.ymm_upper.fill(0xa9);
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
                let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                if case == 0 {
                    result.unwrap();
                    assert_eq!(engine.usage().vector_bytes, width as u64);
                    assert_eq!(engine.state().rflags, state.rflags);
                    assert_eq!(engine.state().x87.registers, state.x87.registers);
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
                assert_eq!(&actual[..width], &raw.to_le_bytes()[..width]);
            });
        }
    }
    for prefix in [0xf2, 0xf3, 0x64] {
        run(
            &[prefix, 0xdb, 0xe9],
            None,
            |_| {},
            |_| {},
            Some(NativeDispatchError::UnsupportedVectorInstruction),
        );
    }
    for bytes in [
        [0xdc, 0xd1, 0xc3],
        [0xdc, 0xd9, 0xc3],
        [0xde, 0xd1, 0xc3],
        [0xda, 0x10, 0xc3],
        [0xd9, 0xe5, 0xc3],
    ] {
        with_source(Some(&bytes), |source, arena, stack, state| {
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
            assert_eq!(
                engine.resolve(&mut arena.pause().unwrap(), &planned),
                Ok(NativeBlockStop::Unsupported {
                    guest_pc: state.guest_pc
                })
            );
            assert_eq!(*engine.state(), state);
            assert_eq!(engine.usage().vector_operations, 0);
        });
    }
    let bytes = [0xdb, 0xe9, 0xc3];
    with_source(Some(&bytes), |source, arena, stack, mut state| {
        state.x87.tag_word = 0xfff0;
        state.x87.registers[0] = bits(J, 0x3fff);
        state.x87.registers[1] = bits(J, 0x4000);
        state.rflags = 0xed7;
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
        with_source(Some(&bytes), |_, other, _, _| {
            assert_eq!(
                engine.preflight_block(&other.pause().unwrap(), &planned),
                Err(NativeDispatchError::WrongOwner)
            );
        });
        assert_eq!(*engine.state(), state);
        assert_eq!(engine.usage().vector_operations, 0);
        assert_eq!(engine.usage().vector_bytes, 0);
    });
}
