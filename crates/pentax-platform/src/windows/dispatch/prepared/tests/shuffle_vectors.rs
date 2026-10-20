// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn modeled_duplicate_vectors_preserve_aliases_flags_and_exact_memory_widths() {
    for (pp, opcode, element, selected) in [
        (3, 0x12, 8, &[0, 0, 2, 2][..]),
        (2, 0x12, 4, &[0, 0, 2, 2, 4, 4, 6, 6][..]),
        (2, 0x16, 4, &[1, 1, 3, 3, 5, 5, 7, 7][..]),
    ] {
        for wide in [false, true] {
            let size = if wide { 32 } else { 16 };
            let read_size = if pp == 3 && !wide { 8 } else { size };
            for (modrm, destination, memory) in
                [(0xc1, 0, false), (0xc9, 1, false), (0x00, 0, true)]
            {
                let code = [
                    0xc5,
                    0xf8 | pp | if wide { 4 } else { 0 },
                    opcode,
                    modrm,
                    0xc3,
                ];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                        - read_size as u64;
                    let input: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(17));
                    arena
                        .pause()
                        .unwrap()
                        .write(data, &input[..read_size])
                        .unwrap();
                    state.gpr[RAX] = data;
                    state.xmm.fill(0xa5);
                    state.ymm_upper.fill(0x5a);
                    state.xmm[16..32].copy_from_slice(&input[..16]);
                    state.ymm_upper[16..32].copy_from_slice(&input[16..]);
                    state.rflags |= 0x8d5;
                    state.mxcsr |= 0x3f;
                    let mut output = [0; 32];
                    for (item, &from) in selected[..size / element].iter().enumerate() {
                        output[item * element..(item + 1) * element]
                            .copy_from_slice(&input[from * element..(from + 1) * element]);
                    }
                    let mut expected = state;
                    let slot = destination * 16;
                    expected.xmm[slot..slot + 16].copy_from_slice(&output[..16]);
                    expected.ymm_upper[slot..slot + 16].copy_from_slice(&output[16..]);
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
                        if memory { read_size as u64 } else { 0 }
                    );
                });
            }
        }
    }
}

#[test]
fn modeled_duplicate_vectors_refuse_partial_sources_and_exhausted_counters() {
    for (pp, opcode) in [(3, 0x12), (2, 0x12), (2, 0x16)] {
        for wide in [false, true] {
            let read_size = if wide {
                32
            } else if pp == 3 {
                8
            } else {
                16
            };
            for case in 0..5 {
                let code = [
                    0xc5,
                    0xf8 | pp | if wide { 4 } else { 0 },
                    opcode,
                    0x00,
                    0xc3,
                ];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    state.gpr[RAX] = if case == 0 {
                        data + PAGE - read_size + 1
                    } else {
                        data
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
                });
            }
        }
    }
}

#[test]
fn modeled_pair_shuffles_select_each_source_and_preserve_aliases_and_flags() {
    for (pp, width, cases) in [
        (
            0,
            4,
            &[
                (0x1b, [3, 2, 1, 0, 3, 2, 1, 0]),
                (0xe4, [0, 1, 2, 3, 0, 1, 2, 3]),
                (0, [0; 8]),
                (0xff, [3; 8]),
            ][..],
        ),
        (
            1,
            8,
            &[
                (0xfa, [0, 1, 0, 1, 0, 0, 0, 0]),
                (0xf9, [1, 0, 0, 1, 0, 0, 0, 0]),
                (0, [0; 8]),
                (0xff, [1; 8]),
            ][..],
        ),
    ] {
        for wide in [false, true] {
            for &(control, selection) in cases {
                for (modrm, destination, memory) in [
                    (0xc2, 0, false),
                    (0xca, 1, false),
                    (0xd2, 2, false),
                    (0x00, 0, true),
                ] {
                    let code = [
                        0xc5,
                        0xf0 | pp | if wide { 4 } else { 0 },
                        0xc6,
                        modrm,
                        control,
                        0xc3,
                    ];
                    with_source(Some(&code), |source, arena, stack, mut state| {
                        let size = if wide { 32 } else { 16 };
                        let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                            - size as u64;
                        let left: [u8; 32] = std::array::from_fn(|i| i as u8 + 1);
                        let right: [u8; 32] = std::array::from_fn(|i| 0xff - i as u8);
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
                        let mut bytes = [0; 32];
                        for (item, &selected) in selection[..size / width].iter().enumerate() {
                            let at = item * width;
                            let source = if at % 16 < 8 { &left } else { &right };
                            let from = at / 16 * 16 + selected * width;
                            bytes[at..at + width].copy_from_slice(&source[from..from + width]);
                        }
                        let mut expected = state;
                        let slot = destination * 16;
                        expected.xmm[slot..slot + 16].copy_from_slice(&bytes[..16]);
                        expected.ymm_upper[slot..slot + 16].copy_from_slice(&bytes[16..]);
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

#[test]
fn modeled_pair_shuffles_refuse_partial_unselected_tails_and_counter_exhaustion() {
    for pp in [0, 1] {
        for wide in [false, true] {
            for case in 0..5 {
                let code = [
                    0xc5,
                    0xf0 | pp | if wide { 4 } else { 0 },
                    0xc6,
                    0x00,
                    0,
                    0xc3,
                ];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let size = if wide { 32 } else { 16 };
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
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

#[test]
fn modeled_lane_insertions_preserve_other_lane_and_snapshot_aliased_sources() {
    for opcode in [0x18, 0x38] {
        for control in [0, 1, 0xfe, 0xff] {
            for (modrm, destination, memory) in [
                (0xc2, 0, false),
                (0xca, 1, false),
                (0xd2, 2, false),
                (0x00, 0, true),
            ] {
                let code = [0xc4, 0xe3, 0x75, opcode, modrm, control, 0xc3];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE - 16;
                    let input: [u8; 16] = std::array::from_fn(|i| i as u8 + 0x81);
                    arena.pause().unwrap().write(data, &input).unwrap();
                    state.gpr[RAX] = data;
                    state.xmm.fill(0xa5);
                    state.ymm_upper.fill(0x5a);
                    state.xmm[32..48].copy_from_slice(&input);
                    state.rflags |= 0x8d5;
                    state.mxcsr |= 0x3f;
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
                    let mut expected = state;
                    let slot = destination * 16;
                    expected.xmm[slot..slot + 16].copy_from_slice(if control & 1 == 0 {
                        &input
                    } else {
                        &[0xa5; 16]
                    });
                    expected.ymm_upper[slot..slot + 16].copy_from_slice(if control & 1 == 1 {
                        &input
                    } else {
                        &[0x5a; 16]
                    });
                    expected.guest_pc = planned.exit().next_guest_pc();
                    assert_eq!(engine.machine.state, expected);
                    assert_eq!(engine.machine.usage.vector_operations, 1);
                    assert_eq!(
                        engine.machine.usage.vector_bytes,
                        if memory { 16 } else { 0 }
                    );
                });
            }
        }
    }
}

#[test]
fn modeled_lane_insertions_refuse_partial_sources_and_counter_exhaustion_atomically() {
    for case in 0..5 {
        let code = [0xc4, 0xe3, 0x7d, 0x18, 0x00, 1, 0xc3];
        with_source(Some(&code), |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            state.gpr[RAX] = if case == 0 { data + PAGE - 8 } else { data };
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

#[test]
fn modeled_shuffle_vectors_select_within_each_lane_and_preserve_unselected_words() {
    for (prefix, element, offset) in [(1, 4, 0), (2, 2, 8), (3, 2, 0)] {
        for wide in [false, true] {
            for form in 0..3 {
                for (control, selection) in [
                    (0xe4, [0, 1, 2, 3]),
                    (0x1b, [3, 2, 1, 0]),
                    (0xaa, [2, 2, 2, 2]),
                    (0x71, [1, 0, 3, 1]),
                ] {
                    let code = [
                        0xc5,
                        (if form == 2 { 0xf8 } else { 0x78 }) | prefix | if wide { 4 } else { 0 },
                        0x70,
                        match form {
                            0 => 0xf9,
                            1 => 0x38,
                            _ => 0xc9,
                        },
                        control,
                        0xc3,
                    ];
                    with_source(Some(&code), |source, arena, stack, mut state| {
                        let size = if wide { 32 } else { 16 };
                        let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                            - size as u64;
                        let input: [u8; 32] = std::array::from_fn(|i| (i * 7 + 0x81) as u8);
                        arena.pause().unwrap().write(data, &input[..size]).unwrap();
                        state.gpr[RAX] = data;
                        state.xmm.fill(0xa5);
                        state.ymm_upper.fill(0x5a);
                        state.xmm[16..32].copy_from_slice(&input[..16]);
                        state.ymm_upper[16..32].copy_from_slice(&input[16..]);
                        state.rflags |= 0x8d5;
                        state.mxcsr |= 0x3f;
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
                        let mut expected_bytes = [0; 32];
                        expected_bytes[..size].copy_from_slice(&input[..size]);
                        for lane in (0..size).step_by(16) {
                            for (destination, selected) in selection.into_iter().enumerate() {
                                let from = lane + offset + selected * element;
                                let to = lane + offset + destination * element;
                                expected_bytes[to..to + element]
                                    .copy_from_slice(&input[from..from + element]);
                            }
                        }
                        let mut expected = state;
                        let slot = if form == 2 { 16 } else { 240 };
                        expected.xmm[slot..slot + 16].copy_from_slice(&expected_bytes[..16]);
                        expected.ymm_upper[slot..slot + 16].copy_from_slice(&expected_bytes[16..]);
                        expected.guest_pc = planned.exit().next_guest_pc();
                        assert_eq!(engine.machine.state, expected);
                        assert_eq!(engine.machine.usage.vector_operations, 1);
                        assert_eq!(
                            engine.machine.usage.vector_bytes,
                            if form == 1 { size as u64 } else { 0 }
                        );
                    });
                }
            }
        }
    }
}

#[test]
fn modeled_shuffle_vectors_require_full_sources_and_refuse_budget_failures_before_output() {
    for case in 0..5 {
        let code = [0xc5, 0xfd, 0x70, 0x00, 0, 0xc3]; // VPSHUFD ymm0,[rax],0
        with_source(Some(&code), |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            state.gpr[RAX] = if case == 0 { data + PAGE - 16 } else { data };
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
