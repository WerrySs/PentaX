// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn integer_conversions_merge_sources_round_and_read_exact_scalar_widths() {
    for wide in [false, true] {
        for double in [false, true] {
            for memory in [false, true] {
                for alias in [false, true] {
                    for rounding in 0..4 {
                        let size = if wide { 8 } else { 4 };
                        let output_size = if double { 8 } else { 4 };
                        let raw = if wide {
                            (if double {
                                -9_007_199_254_740_993_i64
                            } else {
                                -16_777_217_i64
                            }) as u64
                        } else {
                            0xdead_beef_0000_0000 | u64::from((-16_777_217_i32) as u32)
                        };
                        let bits: u64 = if double {
                            if !wide {
                                0xc170_0000_1000_0000
                            } else if rounding == 1 {
                                0xc340_0000_0000_0001
                            } else {
                                0xc340_0000_0000_0000
                            }
                        } else if rounding == 1 {
                            0xcb80_0001
                        } else {
                            0xcb80_0000
                        };
                        let destination = usize::from(alias);
                        let code = [
                            0xc4,
                            if memory { 0xe1 } else { 0xc1 },
                            0x70 | if wide { 0x80 } else { 0 } | if double { 3 } else { 2 },
                            0x2a,
                            (destination as u8) << 3 | if memory { 0 } else { 0xc1 },
                            0xc3,
                        ];
                        with_source(Some(&code), |source, arena, stack, mut state| {
                            let tail =
                                source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                                    - size as u64;
                            arena
                                .pause()
                                .unwrap()
                                .write(tail, &raw.to_le_bytes()[..size])
                                .unwrap();
                            state.gpr[RAX] = tail;
                            state.gpr[9] = raw;
                            state.xmm.fill(0xa5);
                            state.xmm[16..32].fill(0x5a);
                            state.ymm_upper.fill(0x96);
                            state.mxcsr |= 1 | (rounding << 13) | (1 << 15);
                            let mut expected = state;
                            let slot = destination * 16;
                            expected.xmm[slot..slot + 16].copy_from_slice(&state.xmm[16..32]);
                            expected.xmm[slot..slot + output_size]
                                .copy_from_slice(&bits.to_le_bytes()[..output_size]);
                            expected.ymm_upper[slot..slot + 16].fill(0);
                            if !double || wide {
                                expected.mxcsr |= 1 << 5;
                            }
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
                                if memory { size as u64 } else { 0 }
                            );
                        });
                    }
                }
            }
        }
    }
}

#[test]
fn integer_conversions_refuse_partial_memory_permissions_and_exhausted_counters() {
    for (prefix, size) in [(0x72, 4_usize), (0xf3, 8)] {
        for case in 0..5 {
            let code = [0xc4, 0xe1, prefix, 0x2a, 0x00, 0xc3];
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let tail = data + PAGE - size as u64;
                arena
                    .pause()
                    .unwrap()
                    .write(tail, &[0xff; 8][..size])
                    .unwrap();
                state.gpr[RAX] = tail + u64::from(case == 0);
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
                engine.machine.phase = Phase::Terminal;
                assert!(
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .is_err()
                );
                assert_eq!(engine.machine.state, state);
                assert_eq!(
                    engine.machine.usage.vector_operations,
                    usage.vector_operations
                );
                assert_eq!(engine.machine.usage.vector_bytes, usage.vector_bytes);
                assert!(engine.machine.phase == Phase::Terminal);
                arena
                    .protect(data, PAGE, NativePermissions::READ_WRITE)
                    .unwrap();
                let mut actual = [0; 8];
                arena
                    .pause()
                    .unwrap()
                    .read(tail, &mut actual[..size])
                    .unwrap();
                assert_eq!(&actual[..size], &[0xff; 8][..size]);
            });
        }
    }
}
