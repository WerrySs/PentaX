// SPDX-License-Identifier: GPL-2.0-only
use super::*;

fn encoding(opcode: u8, size: usize, memory: bool, alias: bool) -> Vec<u8> {
    let mut code = Vec::new();
    if size == 2 {
        code.push(0x66);
    }
    code.extend([
        0xf3,
        0x45 | if size == 8 { 8 } else { 0 },
        0x0f,
        opcode,
        if memory {
            0x09
        } else if alias {
            0xc9
        } else {
            0xcf
        },
        0xc3,
    ]);
    code
}

// Independent bit-by-bit oracle, rather than the production count intrinsics.
fn oracle(opcode: u8, value: u64, bits: usize) -> u64 {
    let mut count = 0;
    for at in 0..bits {
        let bit = if opcode == 0xbd { bits - 1 - at } else { at };
        if opcode == 0xb8 {
            count += (value >> bit) & 1;
        } else if value & (1 << bit) == 0 {
            count += 1;
        } else {
            break;
        }
    }
    count
}

#[test]
fn integer_bit_counts_match_independent_oracle_all_forms_aliases_and_complete_state() {
    for opcode in [0xbd, 0xbc, 0xb8] {
        for size in [2, 4, 8] {
            let bits = size * 8;
            let mask = u64::MAX >> (64 - bits);
            for value in [0, 1, 1 << (bits - 1), mask, 0x3510] {
                for memory in [false, true] {
                    for alias in [false, true] {
                        let code = encoding(opcode, size, memory, alias);
                        with_source(Some(&code), |source, arena, stack, mut state| {
                            let tail =
                                source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                                    - size as u64;
                            arena
                                .pause()
                                .unwrap()
                                .write(tail, &value.to_le_bytes()[..size])
                                .unwrap();
                            state.gpr[9] = if memory {
                                tail
                            } else {
                                0xdead_beef_aeae_0000 | value
                            };
                            state.gpr[15] = value;
                            let source_value = if !memory && alias {
                                state.gpr[9] & mask
                            } else {
                                value
                            };
                            state.rflags = 0xed7;
                            state.xmm.fill(0xa5);
                            state.ymm_upper.fill(0x96);
                            let mut expected = state;
                            let count = oracle(opcode, source_value, bits);
                            expected.gpr[9] = if size == 2 {
                                (state.gpr[9] & !0xffff) | count
                            } else {
                                count
                            };
                            expected.rflags = if opcode == 0xb8 {
                                0x602 | (u64::from(source_value == 0) << 6)
                            } else {
                                (state.rflags & !0x41)
                                    | u64::from(source_value == 0)
                                    | (u64::from(count == 0) << 6)
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
                            expected.guest_pc = planned.exit().next_guest_pc();
                            engine
                                .resolve(&mut arena.pause().unwrap(), &planned)
                                .unwrap();
                            assert_eq!(engine.machine.state, expected);
                            assert_eq!(engine.machine.usage.integer_operations, 1);
                            assert_eq!(
                                engine.machine.usage.integer_bytes,
                                if memory { size as u64 } else { 0 }
                            );
                            assert_eq!(engine.machine.usage.vector_operations, 0);
                        });
                    }
                }
            }
        }
    }
}

#[test]
fn integer_bit_count_refusals_preserve_cpu_and_success_counters() {
    for size in [2, 4, 8] {
        for case in 0..5 {
            let code = encoding(0xbd, size, true, false);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                state.gpr[9] = data + PAGE - size as u64 + u64::from(case == 0);
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
                    3 => engine.machine.usage.integer_bytes = u64::MAX,
                    4 => engine.machine.usage.integer_operations = u64::MAX,
                    _ => {}
                }
                let usage = engine.machine.usage;
                let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                match case {
                    0 | 1 => assert!(matches!(result, Err(NativeDispatchError::Memory(_)))),
                    2 => assert_eq!(result, Err(NativeDispatchError::Budget("integer work"))),
                    3 => assert_eq!(result, Err(NativeDispatchError::Budget("integer bytes"))),
                    4 => assert_eq!(
                        result,
                        Err(NativeDispatchError::Budget("integer operations"))
                    ),
                    _ => unreachable!(),
                }
                assert_eq!(engine.machine.state, state);
                assert_eq!(
                    engine.machine.usage.integer_operations,
                    usage.integer_operations
                );
                assert_eq!(engine.machine.usage.integer_bytes, usage.integer_bytes);
            });
        }
    }
    for size in [4, 8] {
        let code = [
            0xf3,
            if size == 8 { 0x48 } else { 0x40 },
            0x0f,
            0xbd,
            0xe0,
            0xc3,
        ];
        with_source(Some(&code), |source, arena, stack, state| {
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
                Err(NativeDispatchError::InvalidStack)
            );
            assert_eq!(engine.machine.state, state);
            assert_eq!(engine.machine.usage.integer_operations, 0);
        });
    }
}
