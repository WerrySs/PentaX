// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn modeled_scalar_elements_insert_before_zero_mask_with_memory_selector_ignored() {
    for (control, register_result, memory_result) in [
        (0, [5_u32, 2, 3, 4], [9, 2, 3, 4]),
        (0x70, [1, 2, 3, 6], [1, 2, 3, 9]),
        (0xb2, [1, 0, 3, 7], [1, 0, 3, 9]),
        (0xa4, [1, 2, 0, 4], [1, 2, 0, 4]),
        (0x58, [1, 6, 3, 0], [1, 9, 3, 0]),
        (0xff, [0; 4], [0; 4]),
    ] {
        for (modrm, destination, memory) in [
            (0xc2, 0, false),
            (0xca, 1, false),
            (0xd2, 2, false),
            (0x00, 0, true),
        ] {
            let code = [0xc4, 0xe3, 0x71, 0x21, modrm, control, 0xc3];
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE - 4;
                arena
                    .pause()
                    .unwrap()
                    .write(data, &9_u32.to_le_bytes())
                    .unwrap();
                state.gpr[RAX] = data;
                state.xmm.fill(0xa5);
                state.ymm_upper.fill(0x5a);
                for (i, value) in (1_u32..=8).enumerate() {
                    state.xmm[16 + i * 4..20 + i * 4].copy_from_slice(&value.to_le_bytes());
                }
                state.rflags |= 0x8d5;
                state.mxcsr |= 0x3f;
                let mut expected = state;
                let slot = destination * 16;
                for (i, value) in if memory {
                    memory_result
                } else {
                    register_result
                }
                .iter()
                .enumerate()
                {
                    expected.xmm[slot + i * 4..slot + i * 4 + 4]
                        .copy_from_slice(&value.to_le_bytes());
                }
                expected.ymm_upper[slot..slot + 16].fill(0);
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
                    if memory { 4 } else { 0 }
                );
            });
        }
    }
}

#[test]
fn modeled_scalar_elements_extract_exact_words_zero_extended_and_preserve_vector_source() {
    let values = [0x89ab_cdef_u32, 0x7fc0_0001, 0x8000_0000, 0xffff_ffff];
    for (control, value) in [
        (0xfc, values[0]),
        (0xfd, values[1]),
        (0xfe, values[2]),
        (0xff, values[3]),
    ] {
        for (extension, width, modrm, destination) in [
            (0xe3, 0x79, 0xc8, Some(RAX)),
            (0xe3, 0xf9, 0xc8, Some(RAX)),
            (0xc3, 0x79, 0xc9, Some(9)),
            (0xc3, 0xf9, 0xc9, Some(9)),
            (0xe3, 0xf9, 0x08, None),
        ] {
            let code = [0xc4, extension, width, 0x17, modrm, control, 0xc3];
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE - 8;
                arena.pause().unwrap().write(data, &[0xa5; 8]).unwrap();
                state.gpr[RAX] = data + 4;
                state.gpr[9] = u64::MAX;
                state.xmm.fill(0xa5);
                state.ymm_upper.fill(0x5a);
                for (i, value) in values.iter().enumerate() {
                    state.xmm[16 + i * 4..20 + i * 4].copy_from_slice(&value.to_le_bytes());
                }
                let mut expected = state;
                if let Some(slot) = destination {
                    expected.gpr[slot] = u64::from(value);
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
                let mut bytes = [0; 8];
                arena.pause().unwrap().read(data, &mut bytes).unwrap();
                let mut expected_bytes = [0xa5; 8];
                if destination.is_none() {
                    expected_bytes[4..].copy_from_slice(&value.to_le_bytes());
                }
                assert_eq!(bytes, expected_bytes);
                assert_eq!(
                    engine.machine.usage.vector_bytes,
                    if destination.is_none() { 4 } else { 0 }
                );
            });
        }
    }
}

#[test]
fn modeled_scalar_elements_refuse_partial_memory_budget_and_invalid_stack_without_effects() {
    for insert in [false, true] {
        for case in 0..6 {
            if insert && case == 5 {
                continue;
            }
            let code = if insert {
                vec![0xc4, 0xe3, 0x71, 0x21, 0x00, 0xff, 0xc3] // All-zero mask still reads memory.
            } else {
                vec![
                    0xc4,
                    0xe3,
                    0xf9,
                    0x17,
                    if case == 5 { 0xcc } else { 0x08 },
                    0xff,
                    0xc3,
                ]
            };
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let tail = data + PAGE - 2;
                arena.pause().unwrap().write(tail, &[0xa5; 2]).unwrap();
                state.gpr[RAX] = if case == 0 { tail } else { data };
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
                match case {
                    1 => arena
                        .protect(
                            data,
                            PAGE,
                            if insert {
                                NativePermissions::new(false, false, false)
                            } else {
                                NativePermissions::READ
                            },
                        )
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
                arena
                    .protect(data, PAGE, NativePermissions::READ_WRITE)
                    .unwrap();
                let mut tail_bytes = [0; 2];
                arena.pause().unwrap().read(tail, &mut tail_bytes).unwrap();
                assert_eq!(tail_bytes, [0xa5; 2]);
            });
        }
    }
}
