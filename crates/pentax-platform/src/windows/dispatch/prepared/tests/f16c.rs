// SPDX-License-Identifier: GPL-2.0-only
//! Original copied-state and owned-memory consumers of all four F16C forms.
use super::*;

fn code(narrow: bool, wide: bool, memory: bool, alias: bool, immediate: u8) -> Vec<u8> {
    // Source15; destination0 or15. Narrow's ModRM.reg is the SOURCE.
    let second = if narrow {
        0x43 | if memory || !alias { 0x20 } else { 0 }
    } else {
        0x42 | if !alias { 0x80 } else { 0 } | if memory { 0x20 } else { 0 }
    };
    let mut bytes = vec![
        0xc4,
        second,
        0x79 | if wide { 4 } else { 0 },
        if narrow { 0x1d } else { 0x13 },
    ];
    bytes.push(if narrow {
        0x38 | if memory {
            0
        } else if alias {
            0xc7
        } else {
            0xc0
        }
    } else {
        (if alias { 0x38 } else { 0 }) | if memory { 0 } else { 0xc7 }
    });
    if narrow {
        bytes.push(immediate);
    }
    bytes.push(0xc3);
    bytes
}

fn set_vector(state: &mut NativeCpuState, register: usize, bytes: &[u8; 32]) {
    let at = register * 16;
    state.xmm[at..at + 16].copy_from_slice(&bytes[..16]);
    state.ymm_upper[at..at + 16].copy_from_slice(&bytes[16..]);
}
fn input(narrow: bool) -> ([u8; 32], [u8; 32]) {
    let singles = [
        0x3f80_0000_u32,
        0x8000_0000,
        0x7f80_0000,
        0x3380_0000,
        0xc000_0000,
        0x477f_e000,
        0x3880_0000,
        0x7fc0_2000,
    ];
    let halves = [
        0x3c00_u16, 0x8000, 0x7c00, 1, 0xc000, 0x7bff, 0x0400, 0x7e01,
    ];
    let mut source = [0xa5; 32];
    let mut destination = [0; 32];
    for lane in 0..8 {
        if narrow {
            source[lane * 4..lane * 4 + 4].copy_from_slice(&singles[lane].to_le_bytes());
            destination[lane * 2..lane * 2 + 2].copy_from_slice(&halves[lane].to_le_bytes());
        } else {
            source[lane * 2..lane * 2 + 2].copy_from_slice(&halves[lane].to_le_bytes());
            destination[lane * 4..lane * 4 + 4].copy_from_slice(&singles[lane].to_le_bytes());
        }
    }
    (source, destination)
}

#[test]
fn f16c_all_forms_preserve_context_capture_aliases_and_touch_only_exact_tail_bytes() {
    for narrow in [false, true] {
        for wide in [false, true] {
            for (memory, alias) in [(false, false), (false, true), (true, false)] {
                let code = code(narrow, wide, memory, alias, 0);
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let lanes = if wide { 8 } else { 4 };
                    let input_size = lanes * if narrow { 4 } else { 2 };
                    let output_size = lanes * if narrow { 2 } else { 4 };
                    let size = if narrow { output_size } else { input_size };
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    let tail = data + PAGE - size as u64;
                    let (input, mut output) = input(narrow);
                    output[output_size..].fill(0);
                    state.xmm.fill(0x33);
                    state.ymm_upper.fill(0x77);
                    state.rflags = 0x602 | 0x8d5;
                    state.mxcsr = 0x9f84; // FTZ cannot flush exact half subnormals.
                    set_vector(&mut state, 15, &input);
                    state.gpr[RAX] = tail;
                    arena
                        .pause()
                        .unwrap()
                        .write(tail - 3, &[0x69; 35][..size + 3])
                        .unwrap();
                    if !narrow {
                        arena
                            .pause()
                            .unwrap()
                            .write(tail, &input[..input_size])
                            .unwrap();
                    }
                    let mut expected = state;
                    if !memory || !narrow {
                        set_vector(&mut expected, if alias { 15 } else { 0 }, &output);
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
                    assert_eq!(
                        engine.machine.state, expected,
                        "narrow={narrow} wide={wide} memory={memory} alias={alias}"
                    );
                    assert_eq!(engine.usage().vector_operations, 1);
                    assert_eq!(
                        engine.usage().vector_bytes,
                        if memory { size as u64 } else { 0 }
                    );
                    assert_eq!(engine.usage().native_instructions, 0);
                    let mut actual = [0; 35];
                    arena
                        .pause()
                        .unwrap()
                        .read(tail - 3, &mut actual[..size + 3])
                        .unwrap();
                    assert_eq!(actual[..3], [0x69; 3]);
                    assert_eq!(
                        &actual[3..size + 3],
                        if narrow && memory {
                            &output[..size]
                        } else if !narrow {
                            &input[..size]
                        } else {
                            &[0x69; 16][..size]
                        }
                    );
                });
            }
        }
    }
}

#[test]
fn f16c_full_span_memory_counter_work_and_permission_failures_leave_cpu_and_output_unchanged() {
    for narrow in [false, true] {
        for wide in [false, true] {
            for refusal in 0..6 {
                let code = code(narrow, wide, true, false, 0);
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    let size = if wide { 16 } else { 8 };
                    let tail = data + PAGE - size;
                    arena
                        .pause()
                        .unwrap()
                        .write(tail, &[0xa5; 16][..size as usize])
                        .unwrap();
                    let (mut input, _) = input(true);
                    if narrow {
                        // A successful conversion would raise both IE and PE.
                        // Failed stores must retain MXCSR as well as output.
                        input[..4].copy_from_slice(&0x7f80_0001_u32.to_le_bytes());
                        input[4..8].copy_from_slice(&0x3f80_1000_u32.to_le_bytes());
                    }
                    set_vector(&mut state, 15, &input);
                    state.gpr[RAX] = tail + u64::from(refusal == 0);
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
                        1 => arena
                            .protect(
                                data,
                                PAGE,
                                if narrow {
                                    NativePermissions::READ
                                } else {
                                    NativePermissions::new(false, false, false)
                                },
                            )
                            .unwrap(),
                        2 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                        3 => engine.machine.usage.vector_bytes = u64::MAX,
                        4 => engine.machine.usage.vector_operations = u64::MAX,
                        5 => {
                            engine.machine.state.mxcsr &= !0x1000;
                            state.mxcsr &= !0x1000;
                        }
                        _ => {}
                    }
                    let usage = engine.usage();
                    let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                    match refusal {
                        0 | 1 => assert!(matches!(result, Err(NativeDispatchError::Memory(_)))),
                        2 => assert_eq!(result, Err(NativeDispatchError::Budget("vector work"))),
                        3 => assert_eq!(result, Err(NativeDispatchError::Budget("vector bytes"))),
                        4 => assert_eq!(
                            result,
                            Err(NativeDispatchError::Budget("vector operations"))
                        ),
                        5 => assert_eq!(result, Err(NativeDispatchError::InvalidState)),
                        _ => unreachable!(),
                    }
                    assert_eq!(engine.machine.state, state);
                    assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                    assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
                    if refusal == 1 {
                        arena
                            .protect(data, PAGE, NativePermissions::READ_WRITE)
                            .unwrap();
                    }
                    let mut actual = [0; 16];
                    arena
                        .pause()
                        .unwrap()
                        .read(tail, &mut actual[..size as usize])
                        .unwrap();
                    assert_eq!(actual[..size as usize], [0xa5; 16][..size as usize]);
                });
            }
        }
    }
}

#[test]
fn f16c_all_immediates_select_exact_rounding_without_a_suppress_exceptions_bit() {
    for wide in [false, true] {
        let mut stream = Vec::new();
        for immediate in 0..=255 {
            let mut instruction = code(true, wide, false, false, immediate);
            instruction.pop();
            stream.extend(instruction);
        }
        stream.push(0xc3);
        for rc in 0..4 {
            with_source(Some(&stream), |source, arena, stack, mut state| {
                let singles = [0x3f80_1000_u32, 0xbf80_1000, 0x3f80_3000, 0xbf80_3000];
                let expected: [[u16; 4]; 4] = [
                    [0x3c00, 0xbc00, 0x3c02, 0xbc02],
                    [0x3c00, 0xbc01, 0x3c01, 0xbc02],
                    [0x3c01, 0xbc00, 0x3c02, 0xbc01],
                    [0x3c00, 0xbc00, 0x3c01, 0xbc01],
                ];
                let mut input = [0; 32];
                for lane in 0..8 {
                    input[lane * 4..lane * 4 + 4].copy_from_slice(&singles[lane % 4].to_le_bytes());
                }
                set_vector(&mut state, 15, &input);
                state.mxcsr = 0x1f84 | rc << 13;
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    state,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                let mut current = state;
                for immediate in 0..=255_u16 {
                    let planned = block(
                        source,
                        &arena.pause().unwrap(),
                        current.guest_pc,
                        engine.placement(),
                    );
                    let rounding = if immediate & 4 != 0 {
                        rc
                    } else {
                        u32::from(immediate & 3)
                    };
                    let mut output = [0; 32];
                    for lane in 0..if wide { 8 } else { 4 } {
                        output[lane * 2..lane * 2 + 2]
                            .copy_from_slice(&expected[rounding as usize][lane % 4].to_le_bytes());
                    }
                    set_vector(&mut current, 0, &output);
                    current.mxcsr |= 0x20;
                    current.guest_pc = planned.exit().next_guest_pc();
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .unwrap();
                    assert_eq!(
                        engine.machine.state, current,
                        "wide={wide} imm={immediate} RC={rc}"
                    );
                }
                assert_eq!(engine.usage().vector_operations, 256);
                assert_eq!(engine.usage().vector_bytes, 0);
            });
        }
    }
}

#[test]
fn f16c_foreign_owner_and_explicit_segment_prefix_refusals_precede_publication() {
    for narrow in [false, true] {
        for foreign in [false, true] {
            let mut code = code(narrow, true, true, false, 0);
            if !foreign {
                code.insert(0, 0x64);
            }
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                state.gpr[RAX] = data;
                arena.pause().unwrap().write(data, &[0x69; 32]).unwrap();
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
                if foreign {
                    let mut other =
                        NativeAddressSpace::reserve_any(Default::default(), 64 * 1024).unwrap();
                    assert_eq!(
                        engine.preflight_block(&other.pause().unwrap(), &planned),
                        Err(NativeDispatchError::WrongOwner)
                    );
                } else {
                    assert_eq!(
                        engine.resolve(&mut arena.pause().unwrap(), &planned),
                        Err(NativeDispatchError::UnsupportedVectorInstruction)
                    );
                }
                assert_eq!(engine.machine.state, state);
                assert_eq!(engine.usage().vector_operations, 0);
                assert_eq!(engine.usage().vector_bytes, 0);
                let mut actual = [0; 32];
                arena.pause().unwrap().read(data, &mut actual).unwrap();
                assert_eq!(actual, [0x69; 32]);
            });
        }
    }
}
