// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn precision_conversions_capture_aliases_merge_scalar_lanes_and_publish_exact_control() {
    for widening in [false, true] {
        for memory in [false, true] {
            for destination in [0, 1, 15] {
                for rounding in 0..4 {
                    for daz in [false, true] {
                        let size = if widening { 4 } else { 8 };
                        let raw = if widening {
                            1_u64
                        } else {
                            0x3ff0_0000_1000_0000
                        };
                        let code = [
                            0xc4,
                            (if destination >= 8 { 0 } else { 0x80 })
                                | 0x41
                                | if memory { 0x20 } else { 0 },
                            if widening { 0x72 } else { 0x73 },
                            0x5a,
                            ((destination & 7) << 3) | if memory { 0 } else { 0xc7 },
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
                            state.xmm.fill(0xa5);
                            state.xmm[16..32].fill(0x5a);
                            state.xmm[15 * 16..15 * 16 + 8].copy_from_slice(&raw.to_le_bytes());
                            state.ymm_upper.fill(0x96);
                            state.mxcsr |= 4 | (rounding << 13) | (u32::from(daz) << 6);
                            let mut expected = state;
                            let slot = destination as usize * 16;
                            expected.xmm[slot..slot + 16].copy_from_slice(&state.xmm[16..32]);
                            let bits: u64 = if widening {
                                if daz { 0 } else { 0x36a0_0000_0000_0000 }
                            } else if rounding == 2 {
                                0x3f80_0001
                            } else {
                                0x3f80_0000
                            };
                            let output = if widening { 8 } else { 4 };
                            expected.xmm[slot..slot + output]
                                .copy_from_slice(&bits.to_le_bytes()[..output]);
                            expected.ymm_upper[slot..slot + 16].fill(0);
                            expected.mxcsr |= if widening {
                                if daz { 0 } else { 2 }
                            } else {
                                32
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
fn precision_conversion_memory_and_counter_refusals_preserve_all_cpu_results() {
    for (prefix, size) in [(0x72, 4), (0x73, 8), (0x7c, 16), (0x7d, 32)] {
        for case in 0..5 {
            let code = [0xc4, 0xe1, prefix, 0x5a, 0x00, 0xc3];
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

#[test]
fn packed_precision_conversions_use_every_lane_and_zero_unused_destination_bits() {
    for widening in [false, true] {
        for wide in [false, true] {
            for memory in [false, true] {
                for alias in [false, true] {
                    for rounding in 0..4 {
                        let lanes = if wide { 4 } else { 2 };
                        let input_size = lanes * if widening { 4 } else { 8 };
                        let inputs: [u64; 4] = if widening {
                            [0x3fc0_0000, 0xc010_0000, 0x7f80_0001, 1]
                        } else {
                            [
                                0x3ff0_0000_1000_0000,
                                0xc002_0000_0000_0000,
                                0x7ff0_0000_2000_0000,
                                0x36a0_0000_0000_0000,
                            ]
                        };
                        let results: [u64; 4] = if widening {
                            [
                                0x3ff8_0000_0000_0000,
                                0xc002_0000_0000_0000,
                                0x7ff8_0000_2000_0000,
                                0x36a0_0000_0000_0000,
                            ]
                        } else {
                            [
                                if rounding == 2 {
                                    0x3f80_0001
                                } else {
                                    0x3f80_0000
                                },
                                0xc010_0000,
                                0x7fc0_0001,
                                1,
                            ]
                        };
                        let destination = if alias { 15 } else { 0 };
                        let code = [
                            0xc4,
                            (if alias { 0 } else { 0x80 }) | 0x41 | if memory { 0x20 } else { 0 },
                            0x78 | if wide { 4 } else { 0 } | u8::from(!widening),
                            0x5a,
                            (destination & 7) << 3 | if memory { 0 } else { 0xc7 },
                            0xc3,
                        ];
                        with_source(Some(&code), |source, arena, stack, mut state| {
                            let mut input = [0; 32];
                            let input_element = if widening { 4 } else { 8 };
                            for (i, value) in inputs.into_iter().take(lanes).enumerate() {
                                let at = i * input_element;
                                input[at..at + input_element]
                                    .copy_from_slice(&value.to_le_bytes()[..input_element]);
                            }
                            let tail =
                                source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                                    - input_size as u64;
                            arena
                                .pause()
                                .unwrap()
                                .write(tail, &input[..input_size])
                                .unwrap();
                            state.gpr[RAX] = tail;
                            state.xmm.fill(0xa5);
                            state.ymm_upper.fill(0x96);
                            state.xmm[240..256].copy_from_slice(&input[..16]);
                            state.ymm_upper[240..256].copy_from_slice(&input[16..]);
                            state.mxcsr |= 4 | (rounding << 13);
                            let mut output = [0; 32];
                            let output_element = if widening { 8 } else { 4 };
                            for (i, value) in results.into_iter().take(lanes).enumerate() {
                                let at = i * output_element;
                                output[at..at + output_element]
                                    .copy_from_slice(&value.to_le_bytes()[..output_element]);
                            }
                            let mut expected = state;
                            let slot = destination as usize * 16;
                            expected.xmm[slot..slot + 16].copy_from_slice(&output[..16]);
                            expected.ymm_upper[slot..slot + 16].copy_from_slice(&output[16..]);
                            expected.mxcsr |= if widening {
                                if wide { 3 } else { 0 }
                            } else {
                                32 | u32::from(wide)
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
