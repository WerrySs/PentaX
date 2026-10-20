// SPDX-License-Identifier: GPL-2.0-only
use super::*;

// (opcode map, opcode, W, element bytes) covers every admitted exact VEX form.
const FORMS: [(u8, u8, u8, usize); 6] = [
    (3, 0x20, 0, 1),
    (3, 0x20, 1, 1),
    (1, 0xc4, 0, 2),
    (1, 0xc4, 1, 2),
    (3, 0x22, 0, 4),
    (3, 0x22, 1, 8),
];

#[test]
fn scalar_insert_register_forms_mask_indices_snapshot_aliases_and_clear_upper() {
    for (map, opcode, w, size) in FORMS {
        for (destination, left) in [(1_u8, 1_u8), (15, 1), (15, 15)] {
            for immediate in [0x80, 0xff] {
                let code = [
                    0xc4,
                    if destination < 8 { 0x80 } else { 0 } | 0x40 | map,
                    w << 7 | ((!left & 15) << 3) | 1,
                    opcode,
                    0xc0 | ((destination & 7) << 3) | 7,
                    immediate,
                    0xc3,
                ];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    for (i, byte) in state.xmm.iter_mut().enumerate() {
                        *byte = (i as u8).wrapping_mul(17).wrapping_add(3);
                    }
                    state.ymm_upper.fill(0xa5);
                    state.gpr[15] = 0xfedc_ba98_7654_3210;
                    state.mxcsr |= 0x25;
                    state.rflags |= 0x8d5;
                    let mut expected = state;
                    let to = usize::from(destination) * 16;
                    let from = usize::from(left) * 16;
                    expected.xmm[to..to + 16].copy_from_slice(&state.xmm[from..from + 16]);
                    let at = (usize::from(immediate) & (16 / size - 1)) * size;
                    expected.xmm[to + at..to + at + size]
                        .copy_from_slice(&state.gpr[15].to_le_bytes()[..size]);
                    expected.ymm_upper[to..to + 16].fill(0);
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
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .unwrap();
                    expected.guest_pc = planned.exit().next_guest_pc();
                    assert_eq!(engine.machine.state, expected);
                    assert_eq!(engine.machine.usage.vector_operations, 1);
                    assert_eq!(engine.machine.usage.vector_bytes, 0);
                });
            }
        }
    }
}

#[test]
fn scalar_insert_memory_reads_exact_spans_and_keeps_state_on_refusal() {
    for (map, opcode, w, size) in FORMS {
        for case in 0..5 {
            let code = [0xc4, 0x60 | map, w << 7 | 0x71, opcode, 0x38, 0xff, 0xc3];
            with_source(Some(&code), |source, arena, stack, mut state| {
                let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let end = page + PAGE;
                arena.pause().unwrap().write(end - 16, &[0x87; 16]).unwrap();
                state.gpr[RAX] = end - size as u64 - u64::from(case == 1) + u64::from(case == 2);
                state.xmm.fill(0x42);
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
                        .protect(page, PAGE, NativePermissions::new(false, false, false))
                        .unwrap();
                }
                let initial_bytes = if case == 4 {
                    u64::MAX - size as u64 + 1
                } else {
                    0
                };
                engine.machine.usage.vector_bytes = initial_bytes;
                engine.machine.phase = Phase::Terminal;
                let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                let mut expected = state;
                if case < 2 {
                    result.unwrap();
                    let to = 15 * 16;
                    expected.xmm[to..to + 16].copy_from_slice(&state.xmm[16..32]);
                    expected.xmm[to + 16 - size..to + 16].fill(0x87);
                    expected.ymm_upper[to..to + 16].fill(0);
                    expected.guest_pc = planned.exit().next_guest_pc();
                    assert_eq!(engine.machine.usage.vector_operations, 1);
                    assert_eq!(engine.machine.usage.vector_bytes, size as u64);
                } else {
                    assert!(result.is_err());
                    assert_eq!(engine.machine.usage.vector_operations, 0);
                    assert_eq!(engine.machine.usage.vector_bytes, initial_bytes);
                }
                assert_eq!(engine.machine.state, expected);
            });
        }
    }
}
