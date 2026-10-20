// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn modeled_scalar_vectors_preserve_flags_and_sources_with_exact_destination_widths() {
    // Source/destination: 0 = memory, 1 = r9, 2 = xmm15, 3 = xmm1.
    for (code, size, from, to) in [
        (&[0xc4, 0x41, 0x79, 0x6e, 0xf9][..], 4, 1, 2),
        (&[0xc4, 0x41, 0x79, 0x7e, 0xf9][..], 4, 2, 1),
        (&[0xc4, 0x41, 0xf9, 0x6e, 0xf9][..], 8, 1, 2),
        (&[0xc4, 0x41, 0xf9, 0x7e, 0xf9][..], 8, 2, 1),
        (&[0xc5, 0x79, 0x6e, 0x38][..], 4, 0, 2),
        (&[0xc5, 0x79, 0x7e, 0x38][..], 4, 2, 0),
        (&[0xc4, 0x61, 0xf9, 0x6e, 0x38][..], 8, 0, 2),
        (&[0xc4, 0x61, 0xf9, 0x7e, 0x38][..], 8, 2, 0),
        (&[0xc5, 0x7a, 0x7e, 0x38][..], 8, 0, 2),
        (&[0xc5, 0x79, 0xd6, 0x38][..], 8, 2, 0),
        (&[0xc5, 0x7a, 0x7e, 0xf9][..], 8, 3, 2),
        (&[0xc5, 0x79, 0xd6, 0xf9][..], 8, 2, 3),
        (&[0xc5, 0xfa, 0x7e, 0xc9][..], 8, 3, 3),
        (&[0xc5, 0x7a, 0x10, 0x38][..], 4, 0, 2), // VMOVSS xmm15,[rax]
        (&[0xc5, 0x7a, 0x11, 0x38][..], 4, 2, 0), // VMOVSS [rax],xmm15
        (&[0xc5, 0x7b, 0x10, 0x38][..], 8, 0, 2), // VMOVSD xmm15,[rax]
        (&[0xc5, 0x7b, 0x11, 0x38][..], 8, 2, 0), // VMOVSD [rax],xmm15
    ] {
        let mut bytes = code.to_vec();
        bytes.push(0xc3);
        with_source(Some(&bytes), |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE - 8;
            arena.pause().unwrap().write(data, &[0x87; 8]).unwrap();
            state.gpr[RAX] = data;
            state.gpr[9] = 0x1234_5678_9abc_def0;
            state.xmm.fill(0xa5);
            state.ymm_upper.fill(0x5a);
            state.xmm[16..32].fill(0x96);
            let value = match from {
                0 => [0x87; 8],
                1 => state.gpr[9].to_le_bytes(),
                2 => [0xa5; 8],
                3 => [0x96; 8],
                _ => unreachable!(),
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
            engine
                .resolve(&mut arena.pause().unwrap(), &planned)
                .unwrap();
            let mut expected = state;
            expected.guest_pc = planned.exit().next_guest_pc();
            let mut memory = [0x87; 8];
            match to {
                0 => memory[..size].copy_from_slice(&value[..size]),
                1 => {
                    let mut word = [0; 8];
                    word[..size].copy_from_slice(&value[..size]);
                    expected.gpr[9] = u64::from_le_bytes(word);
                }
                2 | 3 => {
                    let slot = if to == 2 { 240 } else { 16 };
                    expected.xmm[slot..slot + 16].fill(0);
                    expected.xmm[slot..slot + size].copy_from_slice(&value[..size]);
                    expected.ymm_upper[slot..slot + 16].fill(0);
                }
                _ => unreachable!(),
            }
            let mut actual = [0; 8];
            arena.pause().unwrap().read(data, &mut actual).unwrap();
            assert_eq!(actual, memory);
            assert_eq!(engine.machine.state, expected);
            assert_eq!(engine.machine.usage.vector_operations, 1);
            assert_eq!(
                engine.machine.usage.vector_bytes,
                if from == 0 || to == 0 { size as u64 } else { 0 }
            );
            assert_eq!(engine.machine.usage.native_instructions, 0);
        });
    }
}

#[test]
fn modeled_scalar_merges_snapshot_both_sources_and_clear_only_destination_upper() {
    for (prefix, size) in [(0xf2, 4), (0xf3, 8)] {
        for (opcode, modrm, destination, low) in [
            (0x10, 0xc2, 0, 2), // xmm0 <- xmm1 upper + xmm2 low
            (0x10, 0xca, 1, 2), // Destination aliases upper source.
            (0x10, 0xd2, 2, 2), // Destination aliases lower source.
            (0x10, 0xc9, 1, 1), // All three operands alias.
            (0x11, 0xd0, 0, 2), // Alternate register encoding.
        ] {
            let code = [0xc5, prefix, opcode, modrm, 0xc3];
            with_source(Some(&code), |source, arena, stack, mut state| {
                for (i, byte) in state.xmm.iter_mut().enumerate() {
                    *byte = (i as u8).wrapping_mul(17).wrapping_add(3);
                }
                state.ymm_upper.fill(0xa5);
                let mut expected = state;
                let destination = destination * 16;
                expected.xmm[destination..destination + size]
                    .copy_from_slice(&state.xmm[low * 16..low * 16 + size]);
                expected.xmm[destination + size..destination + 16]
                    .copy_from_slice(&state.xmm[16 + size..32]);
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

#[test]
fn modeled_scalar_float_moves_check_only_the_complete_scalar_memory_span() {
    for (prefix, size) in [(0xfa, 4), (0xfb, 8)] {
        for store in [false, true] {
            for case in 0..3 {
                let code = [0xc5, prefix, if store { 0x11 } else { 0x10 }, 0x00, 0xc3];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    let target = data + PAGE - size;
                    arena
                        .pause()
                        .unwrap()
                        .write(target, &[0x87; 8][..size as usize])
                        .unwrap();
                    state.gpr[RAX] = target + u64::from(case == 1);
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
                    if case == 2 {
                        arena
                            .protect(
                                data,
                                PAGE,
                                if store {
                                    NativePermissions::READ
                                } else {
                                    NativePermissions::new(false, false, false)
                                },
                            )
                            .unwrap();
                    }
                    engine.machine.phase = Phase::Terminal;
                    let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                    if case == 0 {
                        result.unwrap();
                        state.guest_pc = planned.exit().next_guest_pc();
                        if !store {
                            state.xmm[..16].fill(0);
                            state.xmm[..size as usize].fill(0x87);
                            state.ymm_upper[..16].fill(0);
                        }
                        assert_eq!(engine.machine.usage.vector_operations, 1);
                        assert_eq!(engine.machine.usage.vector_bytes, size);
                    } else {
                        assert!(result.is_err());
                        assert_eq!(engine.machine.usage.vector_operations, 0);
                        assert_eq!(engine.machine.usage.vector_bytes, 0);
                        assert!(engine.machine.phase == Phase::Terminal);
                    }
                    assert_eq!(engine.machine.state, state);
                    arena
                        .protect(data, PAGE, NativePermissions::READ_WRITE)
                        .unwrap();
                    let mut actual = [0; 8];
                    arena
                        .pause()
                        .unwrap()
                        .read(target, &mut actual[..size as usize])
                        .unwrap();
                    assert_eq!(
                        &actual[..size as usize],
                        &[if store && case == 0 { 0x12 } else { 0x87 }; 8][..size as usize]
                    );
                });
            }
        }
    }
}

#[test]
fn modeled_scalar_vectors_refuse_partial_memory_and_invalid_stack_without_effects() {
    for case in 0..5 {
        let code = if case >= 3 {
            &[0xc4, 0xe1, 0xf9, 0x7e, 0xc4, 0xc3][..] // VMOVQ rsp,xmm0
        } else {
            &[0xc5, 0xf9, if case == 0 { 0x6e } else { 0x7e }, 0x00, 0xc3][..]
        };
        with_source(Some(code), |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            let target = data + PAGE - 2;
            arena.pause().unwrap().write(target, &[0xa5; 2]).unwrap();
            state.gpr[RAX] = if case == 2 { data } else { target };
            state.xmm.fill(0x12);
            state.ymm_upper.fill(0x34);
            if case == 4 {
                state.xmm[..8].copy_from_slice(&state.gpr[RSP].to_le_bytes());
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
            if case == 2 {
                arena.protect(data, PAGE, NativePermissions::READ).unwrap();
            }
            engine.machine.phase = Phase::Terminal;
            let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
            if case == 4 {
                result.unwrap();
                state.guest_pc = planned.exit().next_guest_pc();
            } else {
                assert!(result.is_err());
                assert_eq!(engine.machine.usage.vector_operations, 0);
                assert!(engine.machine.phase == Phase::Terminal);
            }
            assert_eq!(engine.machine.state, state);
            let mut actual = [0; 2];
            arena.pause().unwrap().read(target, &mut actual).unwrap();
            assert_eq!(actual, [0xa5; 2]);
        });
    }
}
