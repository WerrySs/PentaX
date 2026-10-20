// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn half_vector_memory_transfers_use_exact_spans_and_publish_only_on_success() {
    for pp in [0, 1] {
        for high in [false, true] {
            for store in [false, true] {
                for alias in [false, true] {
                    for case in 0..5 {
                        let reg = if alias { 1 } else { 15 };
                        let prefix =
                            if alias { 0x80 } else { 0 } | if store { 0x78 } else { 0x70 } | pp;
                        let opcode = if high { 0x16 } else { 0x12 } + u8::from(store);
                        let code = [0xc5, prefix, opcode, (reg & 7) << 3, 0xc3];
                        with_source(Some(&code), |source, arena, stack, mut state| {
                            let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                            let end = page + PAGE;
                            arena.pause().unwrap().write(end - 16, &[0x87; 16]).unwrap();
                            // Exact page end, unaligned success, overrun, permissions, budget.
                            state.gpr[RAX] = end - 8 - u64::from(case == 1) + u64::from(case == 2);
                            for (i, byte) in state.xmm.iter_mut().enumerate() {
                                *byte = (i as u8).wrapping_mul(13).wrapping_add(5);
                            }
                            state.ymm_upper.fill(0xa5);
                            state.mxcsr |= 0x25;
                            state.rflags |= 0x8d5;
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
                            if case == 3 {
                                arena
                                    .protect(
                                        page,
                                        PAGE,
                                        if store {
                                            NativePermissions::READ
                                        } else {
                                            NativePermissions::new(false, false, false)
                                        },
                                    )
                                    .unwrap();
                            }
                            let initial_bytes = if case == 4 { u64::MAX - 7 } else { 0 };
                            engine.machine.usage.vector_bytes = initial_bytes;
                            engine.machine.phase = Phase::Terminal;
                            let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                            let mut expected = state;
                            let mut memory = [0x87; 16];
                            if case < 2 {
                                result.unwrap();
                                let offset = usize::from(high) * 8;
                                let slot = usize::from(reg) * 16;
                                if store {
                                    let at = (state.gpr[RAX] - (end - 16)) as usize;
                                    memory[at..at + 8].copy_from_slice(
                                        &state.xmm[slot + offset..slot + offset + 8],
                                    );
                                } else {
                                    expected.xmm[slot..slot + 16]
                                        .copy_from_slice(&state.xmm[16..32]);
                                    expected.xmm[slot + offset..slot + offset + 8].fill(0x87);
                                    expected.ymm_upper[slot..slot + 16].fill(0);
                                }
                                expected.guest_pc = planned.exit().next_guest_pc();
                                assert_eq!(engine.machine.usage.vector_operations, 1);
                                assert_eq!(engine.machine.usage.vector_bytes, 8);
                            } else {
                                assert!(result.is_err());
                                assert_eq!(engine.machine.usage.vector_operations, 0);
                                assert_eq!(engine.machine.usage.vector_bytes, initial_bytes);
                                assert!(engine.machine.phase == Phase::Terminal);
                            }
                            assert_eq!(engine.machine.state, expected);
                            arena
                                .protect(page, PAGE, NativePermissions::READ_WRITE)
                                .unwrap();
                            let mut actual = [0; 16];
                            arena.pause().unwrap().read(end - 16, &mut actual).unwrap();
                            assert_eq!(actual, memory);
                        });
                    }
                }
            }
        }
    }
}

#[test]
fn half_vector_register_merges_snapshot_aliases_and_clear_destination_upper() {
    for (opcode, to, from) in [(0x12, 0, 8), (0x16, 8, 0)] {
        for (destination, right) in [(0, 2), (1, 2), (2, 2), (1, 1), (15, 2)] {
            let prefix = if destination == 15 { 0x70 } else { 0xf0 };
            let code = [
                0xc5,
                prefix,
                opcode,
                0xc0 | (destination & 7) << 3 | right,
                0xc3,
            ];
            with_source(Some(&code), |source, arena, stack, mut state| {
                for (i, byte) in state.xmm.iter_mut().enumerate() {
                    *byte = (i as u8).wrapping_mul(17).wrapping_add(3);
                }
                state.ymm_upper.fill(0xa5);
                state.mxcsr |= 0x25;
                state.rflags |= 0x8d5;
                let mut expected = state;
                let destination = usize::from(destination) * 16;
                let right = usize::from(right) * 16;
                expected.xmm[destination..destination + 16].copy_from_slice(&state.xmm[16..32]);
                expected.xmm[destination + to..destination + to + 8]
                    .copy_from_slice(&state.xmm[right + from..right + from + 8]);
                expected.ymm_upper[destination..destination + 16].fill(0);
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
                assert_eq!(engine.machine.usage.vector_bytes, 0);
            });
        }
    }
}
