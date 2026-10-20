// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn modeled_broadcast_vectors_repeat_exact_tuples_and_preserve_aliases_flags_and_upper_state() {
    for (opcode, tuple, memory_only, wide_only) in [
        (0x78, 1, false, false),
        (0x79, 2, false, false),
        (0x58, 4, false, false),
        (0x59, 8, false, false),
        (0x18, 4, false, false),
        (0x19, 8, false, true),
        (0x1a, 16, true, true),
        (0x5a, 16, true, true),
    ] {
        for wide in [false, true] {
            if wide_only && !wide {
                continue;
            }
            for form in 0..3 {
                if memory_only && form != 0 {
                    continue;
                }
                // xmm/ymm15 <- memory, xmm1, or aliased xmm15.
                let code = [
                    0xc4,
                    if form == 2 { 0x42 } else { 0x62 },
                    if wide { 0x7d } else { 0x79 },
                    opcode,
                    match form {
                        0 => 0x38,
                        1 => 0xf9,
                        _ => 0xff,
                    },
                    0xc3,
                ];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                        - tuple as u64;
                    let pattern: [u8; 16] = [
                        0x81, 2, 0xfe, 4, 0xa5, 6, 7, 0x80, 9, 10, 11, 12, 13, 14, 15, 0xff,
                    ];
                    arena
                        .pause()
                        .unwrap()
                        .write(data, &pattern[..tuple])
                        .unwrap();
                    state.gpr[RAX] = data;
                    state.xmm.fill(0xa5);
                    state.ymm_upper.fill(0x5a);
                    state.xmm[16..32].copy_from_slice(&pattern);
                    state.xmm[240..].copy_from_slice(&pattern);
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
                    let expected_bytes =
                        pattern[..tuple].repeat(if wide { 32 } else { 16 } / tuple);
                    expected.xmm[240..].copy_from_slice(&expected_bytes[..16]);
                    expected.ymm_upper[240..].copy_from_slice(if wide {
                        &expected_bytes[16..]
                    } else {
                        &[0; 16]
                    });
                    expected.guest_pc = planned.exit().next_guest_pc();
                    assert_eq!(engine.machine.state, expected);
                    assert_eq!(engine.machine.usage.vector_operations, 1);
                    assert_eq!(
                        engine.machine.usage.vector_bytes,
                        if form == 0 { tuple as u64 } else { 0 }
                    );
                    assert_eq!(engine.machine.usage.native_instructions, 0);
                });
            }
        }
    }
}

#[test]
fn modeled_broadcast_vectors_refuse_partial_sources_permissions_and_budget_without_outputs() {
    for case in 0..5 {
        let code = [0xc4, 0xe2, 0x7d, 0x59, 0x00, 0xc3]; // VPBROADCASTQ ymm0,[rax]
        with_source(Some(&code), |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            state.gpr[RAX] = if case == 0 { data + PAGE - 7 } else { data };
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
