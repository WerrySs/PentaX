// SPDX-License-Identifier: GPL-2.0-only
use super::*;

// Opcode, element bytes, per-128-bit immediate repeat, explicit mask register.
const FORMS: [(u8, usize, bool, bool); 7] = [
    (0x02, 4, false, false),
    (0x0e, 2, true, false),
    (0x0c, 4, false, false),
    (0x0d, 8, false, false),
    (0x4c, 1, false, true),
    (0x4a, 4, false, true),
    (0x4b, 8, false, true),
];

#[test]
fn vector_blends_select_exact_elements_snapshot_all_aliases_and_clear_upper() {
    for (opcode, width, repeat, variable) in FORMS {
        for wide in [false, true] {
            for destination in [1_u8, 2, 3, 15] {
                for memory in [false, true] {
                    for control in [0_u8, 0x69, 0xff] {
                        let immediate = if variable {
                            0x30 | (control & 15)
                        } else {
                            control
                        };
                        let code = [
                            0xc4,
                            if destination == 15 { 0x63 } else { 0xe3 },
                            0x71 | if wide { 4 } else { 0 },
                            opcode,
                            (destination & 7) << 3 | if memory { 0 } else { 0xc2 },
                            immediate,
                            0xc3,
                        ];
                        with_source(Some(&code), |source, arena, stack, mut state| {
                            let size = if wide { 32 } else { 16 };
                            let mut left = [0; 32];
                            let mut right = [0; 32];
                            let mut mask = [0x7f; 32];
                            for i in 0..32 {
                                left[i] = (i as u8).wrapping_mul(13).wrapping_add(3);
                                right[i] = !left[i];
                            }
                            for element in 0..32 / width {
                                // Only the top bit of each complete element counts.
                                if control & (1 << (element % 8)) != 0 {
                                    mask[(element + 1) * width - 1] |= 0x80;
                                }
                            }
                            let mut expected_bytes = [0; 32];
                            for byte in 0..size {
                                let selected = if variable {
                                    mask[byte / width * width + width - 1] >> 7
                                } else {
                                    (control >> ((byte / width) % if repeat { 8 } else { 32 })) & 1
                                };
                                expected_bytes[byte] = if selected == 0 {
                                    left[byte]
                                } else {
                                    right[byte]
                                };
                            }
                            let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                            let address = page + PAGE - size as u64;
                            arena
                                .pause()
                                .unwrap()
                                .write(address, &right[..size])
                                .unwrap();
                            state.gpr[RAX] = address;
                            for (index, bytes) in [(1, left), (2, right), (3, mask)] {
                                state.xmm[index * 16..index * 16 + 16]
                                    .copy_from_slice(&bytes[..16]);
                                state.ymm_upper[index * 16..index * 16 + 16]
                                    .copy_from_slice(&bytes[16..]);
                            }
                            state.mxcsr |= 0x25;
                            state.rflags |= 0x8d5;
                            let mut expected = state;
                            let at = usize::from(destination) * 16;
                            expected.xmm[at..at + 16].copy_from_slice(&expected_bytes[..16]);
                            expected.ymm_upper[at..at + 16].copy_from_slice(&expected_bytes[16..]);
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
                            engine
                                .resolve(&mut arena.pause().unwrap(), &planned)
                                .unwrap();
                            expected.guest_pc = planned.exit().next_guest_pc();
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
fn vector_blends_require_complete_memory_even_when_mask_selects_no_bytes() {
    for (opcode, _, _, variable) in FORMS {
        for wide in [false, true] {
            for case in 0..4 {
                let code = [
                    0xc4,
                    0xe3,
                    0x71 | if wide { 4 } else { 0 },
                    opcode,
                    0x08,
                    if variable { 0x30 } else { 0 },
                    0xc3,
                ];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let size = if wide { 32 } else { 16 };
                    let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    let end = page + PAGE;
                    arena
                        .pause()
                        .unwrap()
                        .write(end - size, &[0x87; 32][..size as usize])
                        .unwrap();
                    state.gpr[RAX] = end - size + if case == 0 { 0 } else { 1 };
                    if case == 1 || case == 3 {
                        state.gpr[RAX] = end - size;
                    }
                    state.xmm[16..32].fill(0x42);
                    state.ymm_upper[16..32].fill(0xa5);
                    state.xmm[48..64].fill(0x7f);
                    state.ymm_upper[48..64].fill(0x7f);
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
                    if case == 1 {
                        arena
                            .protect(page, PAGE, NativePermissions::new(false, false, false))
                            .unwrap();
                    }
                    let initial = if case == 3 { u64::MAX - size + 1 } else { 0 };
                    engine.machine.usage.vector_bytes = initial;
                    engine.machine.phase = Phase::Terminal;
                    let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                    let mut expected = state;
                    if case == 0 {
                        result.unwrap();
                        if !wide {
                            expected.ymm_upper[16..32].fill(0);
                        }
                        expected.guest_pc = planned.exit().next_guest_pc();
                        assert_eq!(engine.machine.usage.vector_bytes, size);
                    } else {
                        assert!(result.is_err());
                        assert_eq!(engine.machine.usage.vector_operations, 0);
                        assert_eq!(engine.machine.usage.vector_bytes, initial);
                    }
                    assert_eq!(engine.machine.state, expected);
                });
            }
        }
    }
}
