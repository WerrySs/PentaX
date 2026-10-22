// SPDX-License-Identifier: GPL-2.0-only
//! Original conditional transfers consume every CPU predicate and retained80 class.
use super::*;
const J: u64 = 1 << 63;
// Truth-table columns encode CF,PF,ZF as bits0,1,2 independently of production.
const FORMS: [(u8, u8, u8); 8] = [
    (0xda, 0xc0, 0xaa),
    (0xda, 0xc8, 0xf0),
    (0xda, 0xd0, 0xfa),
    (0xda, 0xd8, 0xcc),
    (0xdb, 0xc0, 0x55),
    (0xdb, 0xc8, 0x0f),
    (0xdb, 0xd0, 0x05),
    (0xdb, 0xd8, 0x33),
];
fn bits(sig: u64, exp: u16) -> [u8; 10] {
    let mut v = [0; 10];
    v[..8].copy_from_slice(&sig.to_le_bytes());
    v[8..].copy_from_slice(&exp.to_le_bytes());
    v
}
fn tag(s: &mut NativeCpuState, t: usize, v: u16) {
    s.x87.tag_word = (s.x87.tag_word & !(3 << (t * 2))) | (v << (t * 2));
}
fn top(s: &mut NativeCpuState, t: usize) {
    s.x87.status_word = (s.x87.status_word & !0x3800) | ((t as u16) << 11);
}
fn flags(s: &mut NativeCpuState, column: u8) {
    s.rflags =
        (s.rflags & !0x45) | u64::from((column & 1) | ((column & 2) << 1) | ((column & 4) << 4));
}
fn run(
    bytes: &[u8],
    setup: impl FnOnce(&mut NativeCpuState),
    expected: impl FnOnce(&mut NativeCpuState),
    error: Option<NativeDispatchError>,
) {
    let mut code = bytes.to_vec();
    code.push(0xc3);
    with_source(Some(&code), |source, arena, stack, mut state| {
        state.rflags = 0xed7;
        state.mxcsr = 0x1fa5;
        state.xmm.fill(0x6b);
        state.ymm_upper.fill(0xbd);
        state.x87.control_word = 0x037f;
        state.x87.status_word = 0x4700;
        state.x87.tag_word = 0;
        state.x87.instruction_pointer = 0x1234;
        state.x87.data_pointer = 0x5678;
        state.x87.instruction_opcode = 0x456;
        for (i, v) in state.x87.registers.iter_mut().enumerate() {
            *v = bits(J | i as u64, 0x3ff0 + i as u16);
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
            assert_eq!(
                *engine.state(),
                after,
                "Original conditional-transfer context mismatch"
            );
            assert_eq!(engine.usage().vector_operations, 1);
            assert_eq!(engine.usage().vector_bytes, 0);
            assert_eq!(engine.usage().native_entries, 0);
        }
    });
}
#[test]
fn x87_conditional_move_all_eight_predicates_truth_bits_top_positions_and_self_aliases_preserve_flags()
 {
    for (first, second, table) in FORMS {
        for column in 0..8 {
            for t in [0, 3, 7] {
                for i in 0..8 {
                    let selected = table & (1 << column) != 0;
                    run(
                        &[0x66, first, second + i as u8],
                        |s| {
                            top(s, t);
                            flags(s, column);
                        },
                        |s| {
                            if selected {
                                s.x87.registers[t] = s.x87.registers[(t + i) & 7];
                                tag(s, t, 0);
                            }
                        },
                        None,
                    );
                }
            }
        }
    }
}
#[test]
fn x87_conditional_move_opaque_payloads_nan_unsupported_and_tag_classes_transfer_without_numeric_exceptions()
 {
    let payloads = [
        (bits(J | 1, 0x3fff), 0),
        (bits(0, 0), 1),
        (bits(0, 0x8000), 1),
        (bits(J, 0x7fff), 2),
        (bits(0xc000_0000_0000_0001, 0xffff), 2),
        (bits(J | 1, 0x7fff), 2),
        (bits(1, 0), 2),
        (bits(J, 0), 2),
        (bits(1, 1), 2),
        (bits(0, 0x7fff), 2),
    ];
    for (first, second, table) in FORMS {
        for selected in [false, true] {
            let column = (0..8)
                .find(|c| (table & (1 << c) != 0) == selected)
                .unwrap();
            for t in [0, 7] {
                for i in [0, 1, 7] {
                    for (value, class) in payloads {
                        let source = (t + i) & 7;
                        run(
                            &[first, second + i as u8],
                            |s| {
                                top(s, t);
                                flags(s, column);
                                s.x87.control_word = 0x0140;
                                s.x87.registers[source] = value;
                                tag(s, source, class);
                            },
                            |s| {
                                if selected {
                                    s.x87.registers[t] = value;
                                    tag(s, t, class);
                                }
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
fn x87_conditional_move_false_conditions_still_fault_on_either_empty_operand_and_masked_fault_never_pops()
 {
    for (first, second, table) in FORMS {
        for selected in [false, true] {
            let column = (0..8)
                .find(|c| (table & (1 << c) != 0) == selected)
                .unwrap();
            for t in [0, 7] {
                for i in [0, 1, 7] {
                    for missing_source in [false, true] {
                        let missing = if missing_source { (t + i) & 7 } else { t };
                        run(
                            &[first, second + i as u8],
                            |s| {
                                top(s, t);
                                flags(s, column);
                                tag(s, missing, 3);
                            },
                            |s| {
                                s.x87.status_word = (s.x87.status_word & !0x200) | 0x41;
                                s.x87.registers[t] = bits(0xc000_0000_0000_0000, 0xffff);
                                tag(s, t, 2);
                            },
                            None,
                        );
                        run(
                            &[first, second + i as u8],
                            |s| {
                                top(s, t);
                                flags(s, column);
                                tag(s, missing, 3);
                                s.x87.control_word &= !1;
                            },
                            |_| {},
                            Some(NativeDispatchError::UnsupportedX87Exception),
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn x87_conditional_move_pending_and_explicit_prefix_refusals_do_not_publish_even_a_false_move() {
    for (first, second, table) in FORMS {
        let column = (0..8).find(|c| table & (1 << c) == 0).unwrap();
        run(
            &[first, second + 1],
            |s| {
                flags(s, column);
                s.x87.control_word &= !0x20;
                s.x87.status_word |= 0x80a0;
            },
            |_| {},
            Some(NativeDispatchError::UnsupportedX87Exception),
        );
        for prefix in [0xf2, 0xf3, 0x64] {
            run(
                &[prefix, first, second + 1],
                |s| flags(s, column),
                |_| {},
                Some(NativeDispatchError::UnsupportedVectorInstruction),
            );
        }
    }
}
#[test]
fn x87_conditional_move_source_owner_work_and_operation_guards_apply_to_false_and_true_paths() {
    for column in [0, 4] {
        for case in 0..4 {
            let code = [0xdb, 0xc9, 0xc3];
            with_source(Some(&code), |source, arena, stack, mut state| {
                state.rflags = 0xed7;
                flags(&mut state, column);
                state.x87.tag_word = 0;
                state.x87.status_word = 0x4700;
                state.x87.registers[0] = bits(J, 0x3fff);
                state.x87.registers[1] = bits(J | 1, 0x4000);
                state.xmm.fill(0x51);
                state.ymm_upper.fill(0xba);
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
                    0 => arena
                        .protect(
                            source.source_plan().modules()[0].load_bias(),
                            PAGE,
                            NativePermissions::READ_WRITE,
                        )
                        .unwrap(),
                    1 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                    2 => engine.machine.usage.vector_operations = u64::MAX,
                    _ => engine.machine.usage.vector_bytes = u64::MAX,
                }
                let usage = engine.usage();
                engine.machine.phase = Phase::Terminal;
                let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                if case == 3 {
                    result.unwrap();
                    assert_eq!(engine.usage().vector_bytes, u64::MAX);
                    assert_eq!(engine.usage().vector_operations, 1);
                    assert_eq!(engine.state().rflags, state.rflags);
                    assert_eq!(engine.state().x87.status_word, state.x87.status_word);
                } else {
                    if case == 0 {
                        assert!(matches!(result, Err(NativeDispatchError::Memory(_))));
                    } else {
                        assert!(matches!(result, Err(NativeDispatchError::Budget(_))));
                    }
                    assert_eq!(*engine.state(), state);
                    assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                    assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
                    assert!(engine.machine.phase == Phase::Terminal);
                }
            });
        }
    }
    with_source(Some(&[0xdb, 0xc9, 0xc3]), |source, arena, stack, state| {
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
        with_source(Some(&[0xdb, 0xc9, 0xc3]), |_, other, _, _| {
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
