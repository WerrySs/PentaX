// SPDX-License-Identifier: GPL-2.0-only
//! Original SSE4a consumers and independent per-bit range oracle.
use super::*;
use pentax_codegen::{Code, OpKind, Register};

fn code(insert: bool, immediate: bool, alias: bool, length: u8, index: u8) -> Vec<u8> {
    let mut bytes = vec![
        if insert { 0xf2 } else { 0x66 },
        if !insert && immediate { 0x41 } else { 0x45 },
        0x0f,
        if immediate { 0x78 } else { 0x79 },
        if !insert && immediate {
            0xc6
        } else if alias {
            0xf6
        } else {
            0xf7
        },
    ];
    if immediate {
        bytes.extend([length, index]);
    }
    bytes.push(0xc3);
    bytes
}

fn seed(
    state: &mut NativeCpuState,
    insert: bool,
    immediate: bool,
    alias: bool,
    length: u8,
    index: u8,
) {
    state.xmm.fill(0xa5);
    state.ymm_upper.fill(0x79);
    state.rflags = 0xed7;
    state.mxcsr = 0x5f85;
    state.xmm[224..232].copy_from_slice(&0x0123_4567_89ab_cdef_u64.to_le_bytes());
    state.xmm[240..248].copy_from_slice(&0xfedc_ba98_7654_3210_u64.to_le_bytes());
    if !immediate {
        let at = (if alias { 224 } else { 240 }) + if insert { 8 } else { 0 };
        state.xmm[at] = length;
        state.xmm[at + 1] = index;
    }
}

// Deliberately independent of the production mask/shift implementation.
fn oracle(state: &NativeCpuState, insert: bool, alias: bool, length: u8, index: u8) -> Option<u64> {
    let length = if length & 63 == 0 {
        64
    } else {
        u32::from(length & 63)
    };
    let index = u32::from(index & 63);
    if index + length > 64 {
        return None;
    }
    let destination = u64::from_le_bytes(state.xmm[224..232].try_into().unwrap());
    let at = if alias { 224 } else { 240 };
    let source = u64::from_le_bytes(state.xmm[at..at + 8].try_into().unwrap());
    let mut result = if insert { destination } else { 0 };
    for bit in 0..length {
        if insert {
            let target = 1_u64 << (index + bit);
            result &= !target;
            if source & (1_u64 << bit) != 0 {
                result |= target;
            }
        } else if destination & (1_u64 << (index + bit)) != 0 {
            result |= 1_u64 << bit;
        }
    }
    Some(result)
}

#[test]
fn sse4a_all_forms_preserve_full_context_aliases_and_charge_only_vector_operations() {
    for insert in [false, true] {
        for immediate in [false, true] {
            for alias in [false, true] {
                for (length, index) in [(0, 0), (8, 8), (16, 16), (1, 63), (63, 1), (0xc8, 0xc8)] {
                    with_source(
                        Some(&code(insert, immediate, alias, length, index)),
                        |source, arena, stack, mut state| {
                            seed(&mut state, insert, immediate, alias, length, index);
                            let mut expected = state;
                            let result = oracle(&state, insert, alias, length, index).unwrap();
                            expected.xmm[224..232].copy_from_slice(&result.to_le_bytes());
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
                            // Register-only operations must not spuriously exhaust a memory-byte counter.
                            engine.machine.usage.vector_bytes = u64::MAX;
                            engine
                                .resolve(&mut arena.pause().unwrap(), &planned)
                                .unwrap();
                            assert_eq!(engine.machine.state, expected);
                            assert_eq!(engine.usage().vector_operations, 1);
                            assert_eq!(engine.usage().vector_bytes, u64::MAX);
                            assert_eq!(engine.usage().native_instructions, 0);
                        },
                    );
                }
            }
        }
    }
}

#[test]
fn sse4a_all_six_bit_control_pairs_match_per_bit_oracle_or_refuse_atomically() {
    for insert in [false, true] {
        for immediate in [false, true] {
            for alias in [false, true] {
                with_source(
                    Some(&code(insert, immediate, alias, 0, 0)),
                    |source, arena, stack, state| {
                        let engine = NativeBlockStepper::new(
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
                        let template = *planned.exit().instruction().unwrap();
                        for length in 0..64_u8 {
                            for index in 0..64_u8 {
                                let mut instruction = template;
                                let mut current = state;
                                seed(
                                    &mut current,
                                    insert,
                                    immediate,
                                    alias,
                                    length | 0xc0,
                                    index | 0x80,
                                );
                                if immediate {
                                    instruction.set_immediate8(length | 0xc0);
                                    instruction.set_immediate8_2nd(index | 0x80);
                                }
                                let mut expected = current;
                                let expected_result =
                                    oracle(&current, insert, alias, length, index);
                                let result = sse4a_bitfields::execute(&instruction, &mut current);
                                if let Some(value) = expected_result {
                                    expected.xmm[224..232].copy_from_slice(&value.to_le_bytes());
                                    assert_eq!(result, Ok(()));
                                } else {
                                    assert_eq!(
                                        result,
                                        Err(NativeDispatchError::UnsupportedVectorInstruction)
                                    );
                                }
                                assert_eq!(
                                    current, expected,
                                    "insert={insert} immediate={immediate} alias={alias} length={length} index={index}"
                                );
                            }
                        }
                    },
                );
            }
        }
    }
}

#[test]
fn sse4a_undefined_ranges_and_budget_refusals_keep_state_and_retirement_counters() {
    for insert in [false, true] {
        for immediate in [false, true] {
            for refusal in 0..4 {
                let (length, index) = if refusal < 2 {
                    if refusal == 0 { (0, 1) } else { (63, 2) }
                } else {
                    (8, 8)
                };
                with_source(
                    Some(&code(insert, immediate, false, length, index)),
                    |source, arena, stack, mut state| {
                        seed(&mut state, insert, immediate, false, length, index);
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
                        if refusal == 2 {
                            engine.machine.limits.max_work_units = engine.machine.usage.work_units;
                        } else if refusal == 3 {
                            engine.machine.usage.vector_operations = u64::MAX;
                        }
                        let usage = engine.usage();
                        let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                        assert_eq!(
                            result,
                            Err(match refusal {
                                0 | 1 => NativeDispatchError::UnsupportedVectorInstruction,
                                2 => NativeDispatchError::Budget("vector work"),
                                _ => NativeDispatchError::Budget("vector operations"),
                            })
                        );
                        assert_eq!(engine.machine.state, state);
                        assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                        assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
                        assert_eq!(
                            engine.usage().native_instructions,
                            usage.native_instructions
                        );
                    },
                );
            }
        }
    }
}

#[test]
fn sse4a_malformed_operand_shapes_refuse_before_any_state_write() {
    with_source(
        Some(&code(true, true, false, 8, 8)),
        |source, arena, stack, state| {
            let engine = NativeBlockStepper::new(
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
            let template = *planned.exit().instruction().unwrap();
            for malformed in 0..5 {
                let mut instruction = template;
                match malformed {
                    0 => instruction.set_op0_register(Register::XMM16),
                    1 => instruction.set_op1_register(Register::YMM15),
                    2 => instruction.set_op1_kind(OpKind::Memory),
                    3 => instruction.set_op3_kind(OpKind::Immediate8),
                    _ => instruction.set_code(Code::Nopd),
                }
                let mut current = state;
                assert_eq!(
                    sse4a_bitfields::execute(&instruction, &mut current),
                    Err(NativeDispatchError::UnsupportedVectorInstruction)
                );
                assert_eq!(current, state);
            }
        },
    );
}
