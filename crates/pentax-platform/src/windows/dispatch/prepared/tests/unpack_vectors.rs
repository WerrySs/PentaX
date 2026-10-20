// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn modeled_unpack_vectors_interleave_selected_halves_with_independent_lanes_and_aliases() {
    for (pp, opcode, width, high) in [
        (0, 0x14, 4, false),
        (0, 0x15, 4, true), // PS
        (1, 0x14, 8, false),
        (1, 0x15, 8, true), // PD
        (1, 0x60, 1, false),
        (1, 0x68, 1, true), // BW
        (1, 0x61, 2, false),
        (1, 0x69, 2, true), // WD
        (1, 0x62, 4, false),
        (1, 0x6a, 4, true), // DQ
        (1, 0x6c, 8, false),
        (1, 0x6d, 8, true), // QDQ
    ] {
        for wide in [false, true] {
            for (modrm, destination, memory, same_sources) in [
                (0xc2, 0, false, false),
                (0xca, 1, false, false),
                (0xd2, 2, false, false),
                (0xc9, 1, false, true),
                (0x00, 0, true, false),
            ] {
                let code = [
                    0xc5,
                    0xf0 | pp | if wide { 4 } else { 0 },
                    opcode,
                    modrm,
                    0xc3,
                ];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let size = if wide { 32 } else { 16 };
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                        - size as u64;
                    let left: [u8; 32] = std::array::from_fn(|i| i as u8 + 1);
                    let right: [u8; 32] = if same_sources {
                        left
                    } else {
                        std::array::from_fn(|i| 0xff - i as u8)
                    };
                    arena.pause().unwrap().write(data, &right[..size]).unwrap();
                    state.gpr[RAX] = data;
                    state.xmm.fill(0xa5);
                    state.ymm_upper.fill(0x5a);
                    state.xmm[16..32].copy_from_slice(&left[..16]);
                    state.ymm_upper[16..32].copy_from_slice(&left[16..]);
                    state.xmm[32..48].copy_from_slice(&right[..16]);
                    state.ymm_upper[32..48].copy_from_slice(&right[16..]);
                    state.rflags |= 0x8d5;
                    state.mxcsr |= 0x3f;
                    let mut expected = state;
                    let mut result = Vec::new();
                    for (a, b) in left[..size]
                        .chunks_exact(16)
                        .zip(right[..size].chunks_exact(16))
                    {
                        let selected = if high { 8..16 } else { 0..8 };
                        for (a, b) in a[selected.clone()]
                            .chunks_exact(width)
                            .zip(b[selected].chunks_exact(width))
                        {
                            result.extend_from_slice(a);
                            result.extend_from_slice(b);
                        }
                    }
                    result.resize(32, 0);
                    let slot = destination * 16;
                    expected.xmm[slot..slot + 16].copy_from_slice(&result[..16]);
                    expected.ymm_upper[slot..slot + 16].copy_from_slice(&result[16..]);
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

#[test]
fn modeled_unpack_vectors_require_complete_memory_sources_before_publication() {
    for wide in [false, true] {
        for (pp, opcode) in [(0, 0x14), (1, 0x14), (1, 0x60), (1, 0x6d)] {
            for case in 0..5 {
                let code = [
                    0xc5,
                    0xf0 | pp | if wide { 4 } else { 0 },
                    opcode,
                    0x00,
                    0xc3,
                ];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let size = if wide { 32 } else { 16 };
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    // Even low-half forms require the unselected tail to be readable.
                    state.gpr[RAX] = if case == 0 {
                        data + PAGE - size + 1
                    } else {
                        data
                    };
                    state.xmm.fill(0xa5);
                    state.ymm_upper.fill(0x5a);
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
                    let before = engine.machine.usage;
                    engine.machine.phase = Phase::Terminal;
                    assert!(
                        engine
                            .resolve(&mut arena.pause().unwrap(), &planned)
                            .is_err()
                    );
                    assert_eq!(engine.machine.state, state);
                    assert_eq!(
                        engine.machine.usage.vector_operations,
                        before.vector_operations
                    );
                    assert_eq!(engine.machine.usage.vector_bytes, before.vector_bytes);
                    assert!(engine.machine.phase == Phase::Terminal);
                });
            }
        }
    }
}
