// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn modeled_byte_shuffle_uses_lane_local_tables_and_reads_sources_before_aliasing() {
    for size in [16, 32] {
        for form in 0..4 {
            let code = [
                0xc4,
                if form == 0 || form == 3 { 0x62 } else { 0xe2 },
                0x71 | if size == 32 { 4 } else { 0 },
                0x00,
                match form {
                    0 => 0xfa,
                    1 => 0xca,
                    2 => 0xd2,
                    _ => 0x38,
                },
                0xc3,
            ];
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + 16;
                let control = [15, 0x80, 0x10, 0x71, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13];
                arena
                    .pause()
                    .unwrap()
                    .write(data, &control.repeat(2))
                    .unwrap();
                state.gpr[RAX] = data;
                state.xmm.fill(0xa5);
                state.ymm_upper.fill(0x5a);
                state.xmm[16..32].copy_from_slice(&[
                    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
                ]);
                state.ymm_upper[16..32].copy_from_slice(&[
                    32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,
                ]);
                state.xmm[32..48].copy_from_slice(&control);
                state.ymm_upper[32..48].copy_from_slice(&control);
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
                let slot = match form {
                    1 => 16,
                    2 => 32,
                    _ => 240,
                };
                expected.xmm[slot..slot + 16].copy_from_slice(&[
                    31, 0, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29,
                ]);
                expected.ymm_upper[slot..slot + 16].copy_from_slice(if size == 32 {
                    &[
                        47, 0, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45,
                    ]
                } else {
                    &[0; 16]
                });
                expected.guest_pc = planned.exit().next_guest_pc();
                assert_eq!(engine.machine.state, expected);
                assert_eq!(
                    engine.machine.usage.vector_bytes,
                    if form == 3 { size as u64 } else { 0 }
                );
            });
        }
    }
}

#[test]
fn modeled_logical_vectors_apply_three_operand_logic_and_preserve_other_state() {
    for (opcode, prefix, lower, upper) in [
        (0xdb, 1, 0x03, 0x50), // VPAND
        (0xdf, 1, 0x30, 0x05), // VPANDN
        (0xeb, 1, 0x3f, 0xf5), // VPOR
        (0xef, 1, 0x3c, 0xa5), // VPXOR
        (0x54, 0, 0x03, 0x50), // VANDPS
        (0x54, 1, 0x03, 0x50), // VANDPD
        (0x55, 0, 0x30, 0x05), // VANDNPS
        (0x55, 1, 0x30, 0x05), // VANDNPD
        (0x56, 0, 0x3f, 0xf5), // VORPS
        (0x56, 1, 0x3f, 0xf5), // VORPD
        (0x57, 0, 0x3c, 0xa5), // VXORPS
        (0x57, 1, 0x3c, 0xa5), // VXORPD
    ] {
        for size in [16, 32] {
            for form in 0..3 {
                let code = [
                    0xc5,
                    (if form == 2 { 0xf0 } else { 0x70 }) | prefix | if size == 32 { 4 } else { 0 },
                    opcode,
                    match form {
                        0 => 0xfa,
                        1 => 0x38,
                        _ => 0xca,
                    },
                    0xc3,
                ];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + 17;
                    let mut right = [0x33; 32];
                    right[16..].fill(0x55);
                    arena.pause().unwrap().write(data, &right).unwrap();
                    state.gpr[RAX] = data;
                    state.xmm.fill(0x87);
                    state.ymm_upper.fill(0x96);
                    state.xmm[16..32].fill(0x0f);
                    state.ymm_upper[16..32].fill(0xf0);
                    state.xmm[32..48].fill(0x33);
                    state.ymm_upper[32..48].fill(0x55);
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
                    let slot = if form == 2 { 16 } else { 240 };
                    expected.xmm[slot..slot + 16].fill(lower);
                    expected.ymm_upper[slot..slot + 16].fill(if size == 32 { upper } else { 0 });
                    expected.guest_pc = planned.exit().next_guest_pc();
                    assert_eq!(engine.machine.state, expected);
                    assert_eq!(engine.machine.usage.vector_operations, 1);
                    assert_eq!(
                        engine.machine.usage.vector_bytes,
                        if form == 1 { size as u64 } else { 0 }
                    );
                    let mut actual = [0; 32];
                    arena.pause().unwrap().read(data, &mut actual).unwrap();
                    assert_eq!(actual, right);
                });
            }
        }
    }
}

#[test]
fn modeled_logical_vectors_refuse_partial_reads_and_budget_exhaustion_before_outputs() {
    for case in 0..4 {
        with_source(
            Some(&[0xc5, 0xf5, 0xef, 0x00, 0xc3]),
            |source, arena, stack, mut state| {
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
                    1 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                    2 => engine.machine.usage.vector_bytes = u64::MAX,
                    3 => arena
                        .protect(data, PAGE, NativePermissions::new(false, false, false))
                        .unwrap(),
                    _ => {}
                }
                engine.machine.phase = Phase::Terminal;
                assert!(
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .is_err()
                );
                assert_eq!(engine.machine.state, state);
                assert_eq!(engine.machine.usage.vector_operations, 0);
                assert!(engine.machine.phase == Phase::Terminal);
            },
        );
    }
}
