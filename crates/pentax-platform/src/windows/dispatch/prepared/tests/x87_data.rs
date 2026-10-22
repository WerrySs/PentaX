// SPDX-License-Identifier: GPL-2.0-only
//! Original literal80-bit transitions, physical aliases and atomic refusals.
use super::*;

const INDEFINITE: [u8; 10] = [0, 0, 0, 0, 0, 0, 0, 0xc0, 0xff, 0xff];
fn bits(significand: u64, exponent: u16) -> [u8; 10] {
    let mut out = [0; 10];
    out[..8].copy_from_slice(&significand.to_le_bytes());
    out[8..].copy_from_slice(&exponent.to_le_bytes());
    out
}
fn tag(state: &mut NativeCpuState, slot: usize, value: u16) {
    state.x87.tag_word = (state.x87.tag_word & !(3 << (slot * 2))) | (value << (slot * 2));
}
fn top(state: &mut NativeCpuState, value: usize) {
    state.x87.status_word = (state.x87.status_word & !0x3800) | ((value as u16) << 11);
}
fn seed(state: &mut NativeCpuState, stack_top: usize) {
    state.rflags = 0x602 | 0x8d5;
    state.mxcsr = 0x1fa5;
    state.xmm.fill(0x56);
    state.ymm_upper.fill(0xa7);
    state.x87.control_word = 0x0b7f;
    state.x87.status_word = 0x4700 | ((stack_top as u16) << 11);
    state.x87.tag_word = 0xffff;
    state.x87.instruction_opcode = 0x456;
    state.x87.instruction_pointer = 0x1234;
    state.x87.data_pointer = 0x5678;
    for (index, register) in state.x87.registers.iter_mut().enumerate() {
        *register = bits(0x8000_0000_0000_0000 | index as u64, 0x3fff + index as u16);
    }
}
fn program(bytes: &[u8]) -> Vec<u8> {
    let mut code = bytes.to_vec();
    code.push(0xc3);
    code
}
fn metadata(
    after: &mut NativeCpuState,
    before: &NativeCpuState,
    opcode: u16,
    data: Option<u64>,
    next: u64,
) {
    after.x87.instruction_pointer = before.guest_pc;
    after.x87.instruction_opcode = opcode;
    if let Some(data) = data {
        after.x87.data_pointer = data;
    }
    after.guest_pc = next;
}

#[test]
fn x87_loads_widen_literal_ieee_values_without_rounding_and_keep_all_unrelated_cpu_state() {
    let single = [
        (0x0000_0000_u64, 0, 0, 1, 0),
        (0x8000_0000, 0, 0x8000, 1, 0),
        (0x3f80_0000, 0x8000_0000_0000_0000, 0x3fff, 0, 0),
        (0xbf80_0000, 0x8000_0000_0000_0000, 0xbfff, 0, 0),
        (0x7f7f_ffff, 0xffff_ff00_0000_0000, 0x407e, 0, 0),
        (0x0080_0000, 0x8000_0000_0000_0000, 0x3f81, 0, 0),
        (0x0000_0001, 0x8000_0000_0000_0000, 0x3f6a, 0, 2),
        (0x007f_ffff, 0xffff_fe00_0000_0000, 0x3f80, 0, 2),
        (0x7f80_0000, 0x8000_0000_0000_0000, 0x7fff, 2, 0),
        (0x7fc0_0001, 0xc000_0100_0000_0000, 0x7fff, 2, 0),
        (0x7f80_0001, 0xc000_0100_0000_0000, 0x7fff, 2, 1),
    ];
    let double = [
        (0x0000_0000_0000_0000_u64, 0, 0, 1, 0),
        (0x8000_0000_0000_0000, 0, 0x8000, 1, 0),
        (0x3ff0_0000_0000_0000, 0x8000_0000_0000_0000, 0x3fff, 0, 0),
        (0xc004_0000_0000_0000, 0xa000_0000_0000_0000, 0xc000, 0, 0),
        (0x7fef_ffff_ffff_ffff, 0xffff_ffff_ffff_f800, 0x43fe, 0, 0),
        (0x0010_0000_0000_0000, 0x8000_0000_0000_0000, 0x3c01, 0, 0),
        (1, 0x8000_0000_0000_0000, 0x3bcd, 0, 2),
        (0x000f_ffff_ffff_ffff, 0xffff_ffff_ffff_f000, 0x3c00, 0, 2),
        (0xfff0_0000_0000_0000, 0x8000_0000_0000_0000, 0xffff, 2, 0),
        (0x7ff8_0000_0000_0001, 0xc000_0000_0000_0800, 0x7fff, 2, 0),
        (0x7ff0_0000_0000_0001, 0xc000_0000_0000_0800, 0x7fff, 2, 1),
    ];
    for (width, opcode, cases) in [(4, 0xd9, &single), (8, 0xdd, &double)] {
        for &(raw, significand, exponent, value_tag, flags) in cases {
            for stack_top in [0, 3, 7] {
                let code = program(&[opcode, 0x00]);
                with_source(Some(&code), |source, arena, stack, mut state| {
                    seed(&mut state, stack_top);
                    state.x87.status_word |= 4; // Preserve an unrelated sticky ZE.
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                        - width as u64;
                    arena
                        .pause()
                        .unwrap()
                        .write(data, &raw.to_le_bytes()[..width])
                        .unwrap();
                    state.gpr[RAX] = data;
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
                    let mut after = state;
                    let destination = (stack_top + 7) & 7;
                    after.x87.registers[destination] = bits(significand, exponent);
                    tag(&mut after, destination, value_tag);
                    top(&mut after, destination);
                    after.x87.status_word = (after.x87.status_word & !0x200) | flags;
                    metadata(
                        &mut after,
                        &state,
                        u16::from(opcode & 7) << 8,
                        Some(data),
                        planned.exit().next_guest_pc(),
                    );
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .unwrap();
                    assert_eq!(
                        *engine.state(),
                        after,
                        "width={width} raw={raw:x} TOP={stack_top}"
                    );
                    assert_eq!(engine.usage().vector_operations, 1);
                    assert_eq!(engine.usage().vector_bytes, width as u64);
                    assert_eq!(engine.usage().native_entries, 0);
                });
            }
        }
    }
    for (value, value_tag) in [
        (bits(0, 0x8000), 1),
        (bits(1, 0), 2),
        (bits(0x8000_0000_0000_0001, 0x3fff), 0),
        (bits(u64::MAX, 0x7ffe), 0),
        (bits(0x8000_0000_0000_0000, 0x7fff), 2),
        (bits(0xc000_0000_0000_0001, 0xffff), 2),
    ] {
        let code = program(&[0x66, 0xdb, 0x28]);
        with_source(Some(&code), |source, arena, stack, mut state| {
            seed(&mut state, 0);
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE - 10;
            arena.pause().unwrap().write(data, &value).unwrap();
            state.gpr[RAX] = data;
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
            let mut after = state;
            after.x87.registers[7] = value;
            tag(&mut after, 7, value_tag);
            top(&mut after, 7);
            after.x87.status_word &= !0x200;
            metadata(
                &mut after,
                &state,
                0x328,
                Some(data),
                planned.exit().next_guest_pc(),
            );
            engine
                .resolve(&mut arena.pause().unwrap(), &planned)
                .unwrap();
            assert_eq!(*engine.state(), after);
            assert_eq!(engine.usage().vector_bytes, 10); // True80denormal causes no DE under the AMD FLD table.
        });
    }
}

#[test]
fn x87_register_copies_exchange_free_and_pop_use_old_top_and_physical_tags_for_every_alias() {
    for stack_top in [0, 3, 7] {
        for index in 0..8 {
            for operation in 0..5 {
                let (first, second) = match operation {
                    0 => (0xd9, 0xc0 + index),
                    1 => (0xdd, 0xd0 + index),
                    2 => (0xdd, 0xd8 + index),
                    3 => (0xd9, 0xc8 + index),
                    _ => (0xdd, 0xc0 + index),
                };
                let code = program(&[first, second]);
                with_source(Some(&code), |source, arena, stack, mut state| {
                    seed(&mut state, stack_top);
                    state.x87.tag_word = 0;
                    let target = (stack_top + usize::from(index)) & 7;
                    let pushed = (stack_top + 7) & 7;
                    if operation == 0 {
                        tag(&mut state, pushed, 3);
                    }
                    let mut after = state;
                    match operation {
                        0 => {
                            if index == 7 {
                                after.x87.registers[pushed] = INDEFINITE;
                                tag(&mut after, pushed, 2);
                                after.x87.status_word |= 0x41;
                            } else {
                                after.x87.registers[pushed] = state.x87.registers[target];
                                tag(&mut after, pushed, 0);
                            }
                            top(&mut after, pushed);
                        }
                        1 | 2 => {
                            after.x87.registers[target] = state.x87.registers[stack_top];
                            tag(&mut after, target, 0);
                            if operation == 2 {
                                tag(&mut after, stack_top, 3);
                                top(&mut after, (stack_top + 1) & 7);
                            }
                        }
                        3 => {
                            after.x87.registers.swap(stack_top, target);
                        }
                        _ => {
                            tag(&mut after, target, 3);
                        }
                    }
                    if operation != 4 {
                        after.x87.status_word &= !0x200;
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
                    metadata(
                        &mut after,
                        &state,
                        (u16::from(first & 7) << 8) | u16::from(second),
                        None,
                        planned.exit().next_guest_pc(),
                    );
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .unwrap();
                    assert_eq!(
                        *engine.state(),
                        after,
                        "TOP={stack_top} ST{index} op={operation}"
                    );
                    assert_eq!(engine.usage().vector_bytes, 0);
                    assert_eq!(engine.usage().vector_operations, 1);
                    assert_eq!(engine.usage().native_entries, 0);
                });
            }
        }
    }
}

#[test]
fn x87_exact_constants_sign_and_top_rotations_preserve_precision_and_unrelated_physical_storage() {
    for stack_top in [0, 7] {
        for opcode in [0xe8, 0xee, 0xe0, 0xe1, 0xf6, 0xf7] {
            for raw in [
                bits(0, 0x8000),
                bits(0x8000_0000_0000_0001, 0xbfff),
                bits(0xc000_0000_0000_0001, 0xffff),
            ] {
                let code = program(&[0xd9, opcode]);
                with_source(Some(&code), |source, arena, stack, mut state| {
                    seed(&mut state, stack_top);
                    state.x87.registers[stack_top] = raw;
                    tag(
                        &mut state,
                        stack_top,
                        if raw[..8] == [0; 8] { 1 } else { 2 },
                    );
                    let mut after = state;
                    after.x87.status_word &= !0x200;
                    match opcode {
                        0xe8 | 0xee => {
                            let destination = (stack_top + 7) & 7;
                            after.x87.registers[destination] = if opcode == 0xe8 {
                                bits(0x8000_0000_0000_0000, 0x3fff)
                            } else {
                                [0; 10]
                            };
                            tag(&mut after, destination, if opcode == 0xe8 { 0 } else { 1 });
                            top(&mut after, destination);
                        }
                        0xe0 => after.x87.registers[stack_top][9] ^= 0x80,
                        0xe1 => after.x87.registers[stack_top][9] &= 0x7f,
                        0xf6 => top(&mut after, (stack_top + 7) & 7),
                        _ => top(&mut after, (stack_top + 1) & 7),
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
                    metadata(
                        &mut after,
                        &state,
                        0x100 | u16::from(opcode),
                        None,
                        planned.exit().next_guest_pc(),
                    );
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .unwrap();
                    assert_eq!(*engine.state(), after);
                    assert_eq!(engine.usage().vector_bytes, 0);
                    assert_eq!(engine.usage().vector_operations, 1);
                });
            }
        }
    }
}

#[test]
fn x87_masked_stack_faults_publish_indefinite_and_exact_top_tags_while_unmasked_faults_refuse() {
    for case in 0..11 {
        for unmasked in [false, true] {
            let instruction: &[u8] = match case {
                0 => &[0xd9, 0xe8],
                1 => &[0xd9, 0xc7],
                2 => &[0xdd, 0xd1],
                3 => &[0xdd, 0xd8],
                4 => &[0xdb, 0x38],
                5 | 6 => &[0xd9, 0xc9],
                7 => &[0xd9, 0xc8],
                8 => &[0xd9, 0xe0],
                9 => &[0xd9, 0xe1],
                _ => &[0xd9, 0x00],
            };
            let code = program(instruction);
            with_source(Some(&code), |source, arena, stack, mut state| {
                seed(&mut state, 0);
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE - 10;
                arena.pause().unwrap().write(data - 6, &[0xa5; 16]).unwrap();
                state.gpr[RAX] = data;
                if case == 0 || case == 10 {
                    state.x87.tag_word = 0;
                }
                if case == 5 {
                    tag(&mut state, 1, 0);
                }
                if case == 10 {
                    state.x87.control_word &= !2;
                    arena
                        .pause()
                        .unwrap()
                        .write(data, &1_u32.to_le_bytes())
                        .unwrap();
                }
                if unmasked {
                    state.x87.control_word &= !1;
                }
                let mut unchanged = [0; 16];
                arena
                    .pause()
                    .unwrap()
                    .read(data - 6, &mut unchanged)
                    .unwrap();
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
                let outcome = engine.resolve(&mut arena.pause().unwrap(), &planned);
                if unmasked {
                    assert_eq!(
                        outcome,
                        Err(NativeDispatchError::UnsupportedX87Exception),
                        "case={case}"
                    );
                    assert_eq!(*engine.state(), state);
                    assert_eq!(engine.usage().vector_operations, 0);
                    assert_eq!(engine.usage().vector_bytes, 0);
                    assert!(engine.machine.phase == Phase::Terminal);
                } else {
                    outcome.unwrap();
                    let mut after = state;
                    after.x87.status_word = (after.x87.status_word & !0x200) | 0x41;
                    match case {
                        0 | 10 => {
                            after.x87.status_word |= 0x200;
                            top(&mut after, 7);
                            after.x87.registers[7] = INDEFINITE;
                            tag(&mut after, 7, 2);
                        }
                        1 => {
                            top(&mut after, 7);
                            after.x87.registers[7] = INDEFINITE;
                            tag(&mut after, 7, 2);
                        }
                        2 => {
                            after.x87.registers[1] = INDEFINITE;
                            tag(&mut after, 1, 2);
                        }
                        3 => {
                            after.x87.registers[0] = INDEFINITE;
                            top(&mut after, 1);
                        }
                        4 => {
                            top(&mut after, 1);
                        }
                        5 => {
                            after.x87.registers[0] = state.x87.registers[1];
                            tag(&mut after, 0, 0);
                            after.x87.registers[1] = INDEFINITE;
                            tag(&mut after, 1, 2);
                        }
                        6 => {
                            after.x87.registers[0] = INDEFINITE;
                            tag(&mut after, 0, 2);
                            after.x87.registers[1] = INDEFINITE;
                            tag(&mut after, 1, 2);
                        }
                        _ => {
                            after.x87.registers[0] = INDEFINITE;
                            tag(&mut after, 0, 2);
                        }
                    }
                    metadata(
                        &mut after,
                        &state,
                        (u16::from(instruction[0] & 7) << 8) | u16::from(instruction[1]),
                        if case == 4 || case == 10 {
                            Some(data)
                        } else {
                            None
                        },
                        planned.exit().next_guest_pc(),
                    );
                    assert_eq!(*engine.state(), after, "case={case}");
                    assert_eq!(engine.usage().vector_operations, 1);
                    assert_eq!(
                        engine.usage().vector_bytes,
                        if case == 4 {
                            10
                        } else if case == 10 {
                            4
                        } else {
                            0
                        }
                    );
                }
                let mut actual = [0; 16];
                arena.pause().unwrap().read(data - 6, &mut actual).unwrap();
                if case == 4 && !unmasked {
                    assert_eq!(&actual[..6], &unchanged[..6]);
                    assert_eq!(&actual[6..], &INDEFINITE);
                } else {
                    assert_eq!(actual, unchanged);
                }
                assert_eq!(engine.usage().native_entries, 0);
            });
        }
    }
}

#[test]
fn x87_pending_new_unmasked_and_disputed_extended_sources_refuse_without_partial_publication() {
    // Every admitted operation remains waiting, including register-only Free/TOP operations.
    for instruction in [
        &[0xd9, 0x00][..],
        &[0xdd, 0x00],
        &[0xdb, 0x28],
        &[0xd9, 0xc0],
        &[0xdd, 0xd0],
        &[0xdd, 0xd8],
        &[0xdb, 0x38],
        &[0xd9, 0xc9],
        &[0xdd, 0xc0],
        &[0xd9, 0xf7],
        &[0xd9, 0xf6],
        &[0xd9, 0xe8],
        &[0xd9, 0xee],
        &[0xd9, 0xe0],
        &[0xd9, 0xe1],
    ] {
        let code = program(instruction);
        with_source(Some(&code), |source, arena, stack, mut state| {
            seed(&mut state, 0);
            state.x87.control_word &= !1;
            state.x87.status_word |= 0x8081;
            state.gpr[RAX] = 0x0000_8000_0000_0000; // Pending exception precedes an invalid data address.
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
            assert_eq!(
                engine.resolve(&mut arena.pause().unwrap(), &planned),
                Err(NativeDispatchError::UnsupportedX87Exception)
            );
            assert_eq!(*engine.state(), state);
            assert_eq!(engine.usage().vector_operations, 0);
            assert_eq!(engine.usage().vector_bytes, 0);
            assert!(engine.machine.phase == Phase::Terminal);
        });
    }
    for (width, opcode, raw, mask) in [
        (4, 0xd9, 1_u64, 2),
        (8, 0xdd, 1, 2),
        (4, 0xd9, 0x7f80_0001, 1),
        (8, 0xdd, 0x7ff0_0000_0000_0001, 1),
    ] {
        let code = program(&[opcode, 0]);
        with_source(Some(&code), |source, arena, stack, mut state| {
            seed(&mut state, 0);
            state.x87.control_word &= !mask;
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            arena
                .pause()
                .unwrap()
                .write(data, &raw.to_le_bytes()[..width])
                .unwrap();
            state.gpr[RAX] = data;
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
            assert_eq!(
                engine.resolve(&mut arena.pause().unwrap(), &planned),
                Err(NativeDispatchError::UnsupportedX87Exception)
            );
            assert_eq!(*engine.state(), state);
            assert_eq!(engine.usage().vector_operations, 0);
            assert_eq!(engine.usage().vector_bytes, 0);
        });
    }
    for value in [
        bits(0x8000_0000_0000_0001, 0x7fff),
        bits(0x8000_0000_0000_0000, 0),
        bits(1, 1),
        bits(0, 0x7fff),
    ] {
        for instruction in [
            &[0xdb, 0x28][..],
            &[0xd9, 0xc0],
            &[0xdd, 0xd1],
            &[0xdd, 0xd9],
            &[0xdb, 0x38],
            &[0xd9, 0xc9],
            &[0xd9, 0xe0],
            &[0xd9, 0xe1],
        ] {
            let code = program(instruction);
            with_source(Some(&code), |source, arena, stack, mut state| {
                seed(&mut state, 0);
                state.x87.registers[0] = value;
                tag(&mut state, 0, 2);
                tag(&mut state, 1, 0);
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                arena.pause().unwrap().write(data, &value).unwrap();
                state.gpr[RAX] = data;
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
                assert_eq!(
                    engine.resolve(&mut arena.pause().unwrap(), &planned),
                    Err(NativeDispatchError::UnsupportedVectorInstruction),
                    "{instruction:02x?} {value:02x?}"
                );
                assert_eq!(*engine.state(), state);
                assert_eq!(engine.usage().vector_operations, 0);
                assert_eq!(engine.usage().vector_bytes, 0);
                assert!(engine.machine.phase == Phase::Terminal);
                let mut actual = [0; 10];
                arena.pause().unwrap().read(data, &mut actual).unwrap();
                assert_eq!(actual, value);
            });
        }
    }
}

#[test]
fn x87_complete_memory_spans_permissions_prefixes_owner_and_budgets_refuse_transactionally() {
    for (instruction, width, store) in [
        (&[0xd9, 0x00][..], 4, false),
        (&[0xdd, 0x00][..], 8, false),
        (&[0xdb, 0x28][..], 10, false),
        (&[0xdb, 0x38][..], 10, true),
    ] {
        for case in 0..9 {
            let mut code = instruction.to_vec();
            if case == 8 {
                code.insert(0, 0x64);
            }
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, mut state| {
                seed(&mut state, 0);
                state.x87.registers[0] = bits(0x8000_0000_0000_0001, 0x3fff);
                tag(&mut state, 0, 0);
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let end = data + PAGE;
                let input = if width == 10 {
                    bits(0x8000_0000_0000_0001, 0x3fff)
                } else {
                    [0; 10]
                };
                arena
                    .pause()
                    .unwrap()
                    .write(end - width as u64, &input[..width])
                    .unwrap();
                state.gpr[RAX] = match case {
                    1 => end - width as u64 + 1,
                    2 => 0x0000_8000_0000_0000,
                    _ => end - width as u64,
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
                    3 => arena
                        .protect(
                            data,
                            PAGE,
                            if store {
                                NativePermissions::READ
                            } else {
                                NativePermissions::new(false, true, false)
                            },
                        )
                        .unwrap(),
                    4 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                    5 => engine.machine.usage.vector_bytes = u64::MAX - width as u64 + 1,
                    6 => engine.machine.usage.vector_operations = u64::MAX,
                    7 => {
                        let code_page = source.source_plan().modules()[0].load_bias();
                        arena
                            .protect(code_page, PAGE, NativePermissions::READ_WRITE)
                            .unwrap();
                    }
                    _ => {}
                }
                let usage = engine.usage();
                engine.machine.phase = Phase::Terminal;
                let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                if case == 0 {
                    result.unwrap();
                    assert_eq!(engine.usage().vector_operations, 1);
                    assert_eq!(engine.usage().vector_bytes, width as u64);
                } else {
                    match case {
                        1..=3 | 7 => assert!(
                            matches!(result, Err(NativeDispatchError::Memory(_))),
                            "case={case} {result:?}"
                        ),
                        4..=6 => assert!(
                            matches!(result, Err(NativeDispatchError::Budget(_))),
                            "case={case} {result:?}"
                        ),
                        _ => assert_eq!(
                            result,
                            Err(NativeDispatchError::UnsupportedVectorInstruction)
                        ),
                    }
                    assert_eq!(*engine.state(), state);
                    assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                    assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
                    assert!(engine.machine.phase == Phase::Terminal);
                }
                if case == 3 {
                    arena
                        .protect(data, PAGE, NativePermissions::READ_WRITE)
                        .unwrap();
                }
                let mut actual = [0; 10];
                arena
                    .pause()
                    .unwrap()
                    .read(end - width as u64, &mut actual[..width])
                    .unwrap();
                assert_eq!(
                    &actual[..width],
                    if case == 0 && store {
                        &state.x87.registers[0][..]
                    } else {
                        &input[..width]
                    }
                );
                assert_eq!(engine.usage().native_entries, 0);
            });
        }
    }
    let code = program(&[0xdd, 0x00]);
    with_source(Some(&code), |source, arena, stack, state| {
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
        with_source(Some(&code), |_, foreign, _, _| {
            assert!(matches!(
                engine.resolve(&mut foreign.pause().unwrap(), &planned),
                Err(NativeDispatchError::Memory(_))
            ));
            assert_eq!(*engine.state(), state);
            assert_eq!(engine.usage().vector_operations, 0);
            assert_eq!(engine.usage().vector_bytes, 0);
        });
    });
}

#[test]
fn x87_software_transfer_preserves_nonempty_host_stack_environment_and_mxcsr() {
    if !std::is_x86_feature_detected!("fxsr") {
        return;
    }
    #[repr(align(16))]
    struct Area([u8; 512]);
    struct Restore(Area);
    impl Drop for Restore {
        fn drop(&mut self) {
            // SAFETY: This exact aligned image was captured from this thread
            // before the closed host probe. No guest state enters this image.
            unsafe {
                std::arch::x86_64::_fxrstor64(self.0.0.as_ptr());
            }
        }
    }
    let code = program(&[0xdd, 0x00]);
    with_source(Some(&code), |source, arena, stack, mut state| {
        seed(&mut state, 0);
        let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
        arena
            .pause()
            .unwrap()
            .write(data, &0x3ff0_0000_0000_0000_u64.to_le_bytes())
            .unwrap();
        state.gpr[RAX] = data;
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
        let mut restore = Restore(Area([0; 512]));
        let mut before = Area([0; 512]);
        let mut after = Area([0; 512]);
        let left = bits(0x8000_0000_0000_0001, 0x3fff);
        let right = bits(0, 0x8000);
        let control = 0x0b7f_u16;
        // SAFETY: FXSR checked, aligned local save images and exact local10B
        // operands. The temporary host stack masks all exceptions. RAII restores
        // the complete original host state before any assertion or unwinding.
        unsafe {
            std::arch::asm!("fxsave64 [{saved}]", "fninit", "fld tbyte ptr [{left}]", "fld tbyte ptr [{right}]",
                "fldcw [{control}]", "fxsave64 [{before}]", saved=in(reg) restore.0.0.as_mut_ptr(),
                left=in(reg) left.as_ptr(), right=in(reg) right.as_ptr(), control=in(reg) &control,
                before=in(reg) before.0.as_mut_ptr(), options(nostack, preserves_flags));
        }
        let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
        // SAFETY: The same fixed-size aligned local snapshot, without an x87
        // instruction or Rust assertion between execution and this observation.
        unsafe {
            std::arch::x86_64::_fxsave64(after.0.as_mut_ptr());
        }
        drop(restore);
        result.unwrap();
        assert!(
            before.0[..32] == after.0[..32],
            "host x87 environment or MXCSR changed"
        ); // CW/SW/tags/FOP/FIP/FDP/MXCSR.
        assert!(
            before.0[32..160] == after.0[32..160],
            "host x87 register storage changed"
        ); // All eight complete80-bit registers.
        assert_eq!(engine.usage().native_entries, 0);
        assert_eq!(engine.usage().vector_operations, 1);
    });
}
