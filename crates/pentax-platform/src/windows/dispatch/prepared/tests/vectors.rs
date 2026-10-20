// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn non_temporal_vector_stores_preserve_all_registers_flags_and_neighboring_bytes() {
    for size in [16, 32] {
        for high_register in [false, true] {
            let code = [
                0xc5,
                (if high_register { 0x79 } else { 0xf9 }) | if size == 32 { 4 } else { 0 },
                0xe7,
                if high_register { 0x38 } else { 0x00 },
                0xc3,
            ];
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                // An aligned store ending at the last writable byte must not
                // access the following unmapped page or touch its neighbors.
                let target = data + PAGE - size as u64;
                arena
                    .pause()
                    .unwrap()
                    .write(target - 16, &[0xa5; 48][..size + 16])
                    .unwrap();
                state.gpr[RAX] = target;
                state.xmm = std::array::from_fn(|i| i as u8 ^ 0x69);
                state.ymm_upper = std::array::from_fn(|i| i as u8 ^ 0xb4);
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
                let slot = if high_register { 15 * 16 } else { 0 };
                let mut output = [0; 48];
                arena
                    .pause()
                    .unwrap()
                    .read(target - 16, &mut output[..size + 16])
                    .unwrap();
                assert_eq!(&output[..16], &[0xa5; 16]);
                assert_eq!(&output[16..32], &state.xmm[slot..slot + 16]);
                if size == 32 {
                    assert_eq!(&output[32..48], &state.ymm_upper[slot..slot + 16]);
                }
                let mut expected = state;
                expected.guest_pc = planned.exit().next_guest_pc();
                assert_eq!(engine.machine.state, expected);
                assert_eq!(engine.machine.usage.vector_operations, 1);
                assert_eq!(engine.machine.usage.vector_bytes, size as u64);
                assert_eq!(engine.machine.usage.native_instructions, 0);
            });
        }
    }
}

#[test]
fn non_temporal_vector_stores_refuse_alignment_permissions_and_budgets_before_effects() {
    for size in [16, 32] {
        for refusal in 0..6 {
            let code = [0xc5, if size == 32 { 0xfd } else { 0xf9 }, 0xe7, 0x00, 0xc3];
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                state.gpr[RAX] = match refusal {
                    0 => data + 1,
                    1 => data + PAGE,
                    _ => data,
                };
                state.xmm.fill(0x12);
                state.ymm_upper.fill(0x34);
                arena.pause().unwrap().write(data, &[0xa5; 64]).unwrap();
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
                match refusal {
                    2 => arena.protect(data, PAGE, NativePermissions::READ).unwrap(),
                    3 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                    4 => engine.machine.usage.vector_bytes = u64::MAX,
                    5 => engine.machine.usage.vector_operations = u64::MAX,
                    _ => {}
                }
                let operations = engine.machine.usage.vector_operations;
                let bytes = engine.machine.usage.vector_bytes;
                let error = engine
                    .resolve(&mut arena.pause().unwrap(), &planned)
                    .unwrap_err();
                if refusal == 0 {
                    assert!(matches!(error, NativeDispatchError::VectorAlignment));
                }
                let mut output = [0; 64];
                arena.pause().unwrap().read(data, &mut output).unwrap();
                assert_eq!(output, [0xa5; 64]);
                assert_eq!(engine.machine.state, state);
                assert_eq!(engine.machine.usage.vector_operations, operations);
                assert_eq!(engine.machine.usage.vector_bytes, bytes);
            });
        }
    }
}

#[test]
fn modeled_vectors_move_all_lanes_and_zero_only_the_vex128_destination_upper_half() {
    for (prefix, load, store, aligned) in [
        (0xf8, 0x10, 0x11, false), // VMOVUPS
        (0xf9, 0x10, 0x11, false), // VMOVUPD
        (0xfa, 0x6f, 0x7f, false), // VMOVDQU
        (0xf8, 0x28, 0x29, true),  // VMOVAPS
        (0xf9, 0x28, 0x29, true),  // VMOVAPD
        (0xf9, 0x6f, 0x7f, true),  // VMOVDQA
    ] {
        for size in [16, 32] {
            for form in 0..3 {
                let code = [
                    0xc5,
                    prefix | if size == 32 { 4 } else { 0 },
                    if form == 2 { store } else { load },
                    if form == 0 { 0xc1 } else { 0x00 },
                    0xc3,
                ];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let data = source.source_plan().modules()[0].load_bias()
                        + DATA_OFF
                        + if aligned { 32 } else { 33 };
                    let bytes: [u8; 32] = std::array::from_fn(|i| i as u8 + 1);
                    arena.pause().unwrap().write(data, &bytes).unwrap();
                    state.gpr[RAX] = data;
                    state.xmm.fill(0xa5);
                    state.ymm_upper.fill(0x5a);
                    state.xmm[16..32].copy_from_slice(&bytes[..16]);
                    state.ymm_upper[16..32].copy_from_slice(&bytes[16..]);
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
                    assert!(matches!(planned.exit().kind(), ExitKind::Mediated { .. }));
                    engine.machine.phase = Phase::Terminal;
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .unwrap();
                    let mut expected = state;
                    expected.guest_pc = planned.exit().next_guest_pc();
                    if form == 2 {
                        let mut output = [0; 32];
                        arena.pause().unwrap().read(data, &mut output).unwrap();
                        assert_eq!(&output[..16], &[0xa5; 16]);
                        assert_eq!(
                            &output[16..],
                            if size == 32 {
                                &[0x5a; 16][..]
                            } else {
                                &bytes[16..]
                            }
                        );
                    } else {
                        expected.xmm[..16].copy_from_slice(&bytes[..16]);
                        expected.ymm_upper[..16].copy_from_slice(if size == 32 {
                            &bytes[16..]
                        } else {
                            &[0; 16]
                        });
                    }
                    assert_eq!(engine.machine.state, expected);
                    assert_eq!(engine.machine.usage.vector_operations, 1);
                    assert_eq!(
                        engine.machine.usage.vector_bytes,
                        if form == 0 { 0 } else { size as u64 }
                    );
                    assert_eq!(engine.machine.usage.native_instructions, 0);
                    assert!(!engine.machine.cache.executable);
                });
            }
        }
    }
}

#[test]
fn modeled_vectors_zero_all_sixteen_registers_without_changing_flags_or_memory() {
    for (vex, all) in [(0xf8, false), (0xfc, true)] {
        with_source(
            Some(&[0xc5, vex, 0x77, 0xc3]),
            |source, arena, stack, mut state| {
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
                engine
                    .resolve(&mut arena.pause().unwrap(), &planned)
                    .unwrap();
                state.guest_pc = planned.exit().next_guest_pc();
                state.ymm_upper.fill(0);
                if all {
                    state.xmm.fill(0);
                }
                assert_eq!(engine.machine.state, state);
                assert_eq!(engine.machine.usage.vector_bytes, 0);
            },
        );
    }
}

#[test]
fn modeled_vectors_refuse_partial_writes_alignment_readonly_and_budget_before_effects() {
    for refusal in 0..5 {
        let code = [
            0xc5,
            0xfc,
            if refusal == 1 { 0x29 } else { 0x11 },
            0x00,
            0xc3,
        ];
        with_source(Some(&code), |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            let target = match refusal {
                0 => data + PAGE - 16,
                1 => data + 1,
                _ => data,
            };
            let sentinel = [0xa5; 16];
            arena.pause().unwrap().write(target, &sentinel).unwrap();
            state.gpr[RAX] = target;
            state.xmm.fill(0x12);
            state.ymm_upper.fill(0x34);
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
            match refusal {
                2 => arena.protect(data, PAGE, NativePermissions::READ).unwrap(),
                3 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                4 => engine.machine.usage.vector_bytes = u64::MAX,
                _ => {}
            }
            engine.machine.phase = Phase::Terminal;
            assert!(
                engine
                    .resolve(&mut arena.pause().unwrap(), &planned)
                    .is_err()
            );
            let mut actual = [0; 16];
            arena.pause().unwrap().read(target, &mut actual).unwrap();
            assert_eq!(actual, sentinel);
            assert_eq!(engine.machine.state, state);
            assert_eq!(engine.machine.usage.vector_operations, 0);
            assert!(engine.machine.phase == Phase::Terminal);
        });
    }
}
