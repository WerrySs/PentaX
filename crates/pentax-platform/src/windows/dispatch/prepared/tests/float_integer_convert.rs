// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn float_integer_rsp_results_require_the_same_owned_stack_before_publication() {
    for invalid in [false, true] {
        let code = [0xc4, 0xe1, 0xfb, 0x2c, 0xe0, 0xc3];
        with_source(Some(&code), |source, arena, stack, mut state| {
            let raw = if invalid {
                0
            } else {
                (state.gpr[RSP] as f64).to_bits()
            };
            state.xmm[..8].copy_from_slice(&raw.to_le_bytes());
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
            let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
            if invalid {
                assert_eq!(result, Err(NativeDispatchError::InvalidStack));
                assert_eq!(engine.machine.state, state);
                assert_eq!(engine.machine.usage.vector_operations, 0);
            } else {
                result.unwrap();
                let mut expected = state;
                expected.guest_pc = planned.exit().next_guest_pc();
                assert_eq!(engine.machine.state, expected);
                assert_eq!(engine.machine.usage.vector_operations, 1);
            }
        });
    }
}

#[test]
fn float_integer_conversions_preserve_vectors_flags_and_round_or_truncate_exact_scalar_sources() {
    for double in [false, true] {
        for wide in [false, true] {
            for truncate in [false, true] {
                for rounding in 0..4 {
                    for memory in [false, true] {
                        for high in [false, true] {
                            let input_size = if double { 8 } else { 4 };
                            let destination = if high { 9 } else { RAX };
                            let raw = if double {
                                (-3.5_f64).to_bits()
                            } else {
                                u64::from((-3.5_f32).to_bits())
                            };
                            let code = [
                                0xc4,
                                (if high { 0 } else { 0x80 })
                                    | 0x41
                                    | if memory { 0x20 } else { 0 },
                                0x78 | if wide { 0x80 } else { 0 } | if double { 3 } else { 2 },
                                if truncate { 0x2c } else { 0x2d },
                                (if high { 8 } else { 0 }) | if memory { 0 } else { 0xc7 },
                                0xc3,
                            ];
                            with_source(Some(&code), |source, arena, stack, mut state| {
                                let tail =
                                    source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                                        - input_size as u64;
                                arena
                                    .pause()
                                    .unwrap()
                                    .write(tail, &raw.to_le_bytes()[..input_size])
                                    .unwrap();
                                state.gpr[RAX] = tail;
                                state.gpr[9] = 0xdead_beef_a5a5_a5a5;
                                state.xmm.fill(0xa5);
                                state.xmm[15 * 16..15 * 16 + 8].copy_from_slice(&raw.to_le_bytes());
                                state.ymm_upper.fill(0x96);
                                state.mxcsr |= 1 | (rounding << 13) | (1 << 15);
                                let mut expected = state;
                                let value = if truncate || rounding >= 2 {
                                    -3_i64
                                } else {
                                    -4_i64
                                };
                                expected.gpr[destination] = if wide {
                                    value as u64
                                } else {
                                    u64::from(value as u32)
                                };
                                expected.mxcsr |= 1 << 5;
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
                                expected.guest_pc = planned.exit().next_guest_pc();
                                engine
                                    .resolve(&mut arena.pause().unwrap(), &planned)
                                    .unwrap();
                                assert_eq!(engine.machine.state, expected);
                                assert_eq!(engine.machine.usage.vector_operations, 1);
                                assert_eq!(
                                    engine.machine.usage.vector_bytes,
                                    if memory { input_size as u64 } else { 0 }
                                );
                            });
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn float_integer_conversion_refuses_partial_memory_permissions_and_counter_exhaustion_before_results()
 {
    for double in [false, true] {
        let size = if double { 8 } else { 4 };
        for case in 0..5 {
            let code = [
                0xc4,
                0xe1,
                if double { 0xfb } else { 0xfa },
                0x2c,
                0x00,
                0xc3,
            ];
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                state.gpr[RAX] = data + PAGE - size + u64::from(case == 0);
                state.xmm.fill(0xa5);
                state.ymm_upper.fill(0x96);
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
                    2 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                    3 => engine.machine.usage.vector_bytes = u64::MAX,
                    4 => engine.machine.usage.vector_operations = u64::MAX,
                    _ => {}
                }
                let usage = engine.machine.usage;
                let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                match case {
                    0 | 1 => assert!(matches!(result, Err(NativeDispatchError::Memory(_)))),
                    2 => assert_eq!(result, Err(NativeDispatchError::Budget("vector work"))),
                    3 => assert_eq!(result, Err(NativeDispatchError::Budget("vector bytes"))),
                    4 => assert_eq!(
                        result,
                        Err(NativeDispatchError::Budget("vector operations"))
                    ),
                    _ => unreachable!(),
                }
                assert_eq!(engine.machine.state, state);
                assert_eq!(
                    engine.machine.usage.vector_operations,
                    usage.vector_operations
                );
                assert_eq!(engine.machine.usage.vector_bytes, usage.vector_bytes);
            });
        }
    }
}
