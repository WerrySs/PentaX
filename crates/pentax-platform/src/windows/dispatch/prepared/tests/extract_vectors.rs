// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn integer_extractions_select_every_element_zero_extend_and_preserve_other_state() {
    for (prefix, opcode, size) in [
        (0x79, 0x14, 1),
        (0xf9, 0x14, 1),
        (0x79, 0x15, 2),
        (0xf9, 0x15, 2),
        (0x79, 0x16, 4),
        (0xf9, 0x16, 8),
        (0x79, 0xc5, 2),
        (0xf9, 0xc5, 2),
    ] {
        for memory in [false, true] {
            if memory && opcode == 0xc5 {
                continue;
            }
            for element in 0..16 / size {
                let mut code = if opcode == 0xc5 {
                    vec![0xc4, 0xc1, prefix, opcode, 0xc7]
                } else {
                    vec![0xc4, 0x63, prefix, opcode, if memory { 0x38 } else { 0xf8 }]
                };
                // High selector bits must be ignored in every source form.
                code.extend([0xf0 | element as u8, 0xc3]);
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let target = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                        - size as u64;
                    arena
                        .pause()
                        .unwrap()
                        .write(target - 8, &[0xa5; 16][..8 + size])
                        .unwrap();
                    state.gpr[RAX] = if memory { target } else { u64::MAX };
                    state.xmm =
                        std::array::from_fn(|i| (i as u8).wrapping_mul(17).wrapping_add(0x83));
                    state.ymm_upper.fill(0x69);
                    let mut value = [0; 8];
                    value[..size].copy_from_slice(
                        &state.xmm[240 + element * size..240 + (element + 1) * size],
                    );
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
                    expected.guest_pc = planned.exit().next_guest_pc();
                    let mut output = [0; 16];
                    arena
                        .pause()
                        .unwrap()
                        .read(target - 8, &mut output[..8 + size])
                        .unwrap();
                    assert_eq!(&output[..8], &[0xa5; 8]);
                    if memory {
                        assert_eq!(&output[8..8 + size], &value[..size]);
                    } else {
                        expected.gpr[RAX] = u64::from_le_bytes(value);
                        assert_eq!(&output[8..8 + size], &[0xa5; 8][..size]);
                    }
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
fn integer_extractions_refuse_partial_memory_and_invalid_stack_without_publication() {
    for (size, opcode, prefix) in [
        (1, 0x14, 0x79),
        (2, 0x15, 0x79),
        (4, 0x16, 0x79),
        (8, 0x16, 0xf9),
    ] {
        for refusal in 0..6 {
            // RSP register destination is only used for the last case.
            let code = [
                0xc4,
                0x63,
                prefix,
                opcode,
                if refusal == 5 { 0xfc } else { 0x38 },
                0xff,
                0xc3,
            ];
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                state.gpr[RAX] = if refusal == 0 {
                    data + PAGE - size as u64 + 1
                } else {
                    data
                };
                arena.pause().unwrap().write(data, &[0xa5; 32]).unwrap();
                arena
                    .pause()
                    .unwrap()
                    .write(data + PAGE - 8, &[0xa5; 8])
                    .unwrap();
                // Extracting zero into RSP must fail the actual stack check.
                state.xmm.fill(0);
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
                    1 => arena.protect(data, PAGE, NativePermissions::READ).unwrap(),
                    2 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                    3 => engine.machine.usage.vector_bytes = u64::MAX,
                    4 => engine.machine.usage.vector_operations = u64::MAX,
                    _ => {}
                }
                let operations = engine.machine.usage.vector_operations;
                let bytes = engine.machine.usage.vector_bytes;
                let error = engine
                    .resolve(&mut arena.pause().unwrap(), &planned)
                    .unwrap_err();
                match refusal {
                    0 | 1 => assert!(matches!(error, NativeDispatchError::Memory(_))),
                    2..=4 => assert!(matches!(error, NativeDispatchError::Budget(_))),
                    5 => assert_eq!(error, NativeDispatchError::InvalidStack),
                    _ => unreachable!(),
                }
                let mut output = [0; 32];
                arena.pause().unwrap().read(data, &mut output).unwrap();
                assert_eq!(output, [0xa5; 32]);
                let mut tail = [0; 8];
                arena
                    .pause()
                    .unwrap()
                    .read(data + PAGE - 8, &mut tail)
                    .unwrap();
                assert_eq!(tail, [0xa5; 8]);
                assert_eq!(engine.machine.state, state);
                assert_eq!(engine.machine.usage.vector_operations, operations);
                assert_eq!(engine.machine.usage.vector_bytes, bytes);
            });
        }
    }
}
