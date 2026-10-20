// SPDX-License-Identifier: GPL-2.0-only
//! Original128-bit lane transfers with immutable source and bounded setup reuse.

use super::*;

fn instruction(integer: bool, destination: Option<u8>, immediate: u8) -> Vec<u8> {
    let register = destination.unwrap_or(0);
    vec![
        0xc4,
        if destination.is_none() || register < 8 {
            0x63
        } else {
            0x43
        },
        0x7d,
        if integer { 0x39 } else { 0x19 },
        if destination.is_some() {
            0xf8 | (register & 7)
        } else {
            0x38
        },
        immediate,
    ]
}
fn source_bytes() -> [u8; 32] {
    std::array::from_fn(|i| (i as u8).wrapping_mul(17).wrapping_add(3))
}

#[test]
fn lane_extract_all_immediates_capture_high_aliases_and_preserve_exact_memory_neighbors() {
    for integer in [false, true] {
        for destination in [Some(15), Some(9), None] {
            // Each immediate is part of one original immutable program. The
            // test walks its256 stops within a single admitted source/arena.
            let mut code = Vec::new();
            for immediate in 0..=255 {
                code.extend(instruction(integer, destination, immediate));
            }
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let end = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE;
                arena.pause().unwrap().write(end - 32, &[0xa5; 32]).unwrap();
                state.gpr[RAX] = end - 17; // Deliberately unaligned; one trailing canary.
                state.xmm.fill(0x33);
                state.ymm_upper.fill(0x77);
                state.mxcsr = 0x1f84;
                state.rflags = 0x602 | 0x8d5;
                let original = source_bytes();
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    state,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                for immediate in 0..=255_u8 {
                    let mut before = engine.machine.state;
                    if let Some(register) = destination {
                        let slot = register as usize * 16;
                        before.xmm[slot..slot + 16].fill(0xcc);
                        before.ymm_upper[slot..slot + 16].fill(0xdd);
                    }
                    before.xmm[240..].copy_from_slice(&original[..16]);
                    before.ymm_upper[240..].copy_from_slice(&original[16..]);
                    engine.machine.state = before;
                    let planned = block(
                        source,
                        &arena.pause().unwrap(),
                        before.guest_pc,
                        engine.placement(),
                    );
                    let mut expected = before;
                    expected.guest_pc = planned.exit().next_guest_pc();
                    let start = usize::from(immediate & 1) * 16;
                    let selected = &original[start..start + 16];
                    let mut memory = [0xa5; 32];
                    if let Some(register) = destination {
                        let slot = register as usize * 16;
                        expected.xmm[slot..slot + 16].copy_from_slice(selected);
                        expected.ymm_upper[slot..slot + 16].fill(0);
                    } else {
                        memory[15..31].copy_from_slice(selected);
                    }
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .unwrap();
                    assert_eq!(
                        engine.machine.state, expected,
                        "integer={integer} dst={destination:?} imm={immediate}"
                    );
                    assert_eq!(engine.usage().vector_operations, u64::from(immediate) + 1);
                    assert_eq!(
                        engine.usage().vector_bytes,
                        if destination.is_none() {
                            (u64::from(immediate) + 1) * 16
                        } else {
                            0
                        }
                    );
                    let mut actual = [0; 32];
                    arena.pause().unwrap().read(end - 32, &mut actual).unwrap();
                    assert_eq!(actual, memory);
                }
            });
        }
    }
}

#[test]
fn lane_extract_exact_tail_store_and_refusals_preserve_memory_state_and_counters() {
    for integer in [false, true] {
        for case in 0..6 {
            let mut code = instruction(integer, None, 0xff);
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let end = data + PAGE;
                arena.pause().unwrap().write(end - 32, &[0xa5; 32]).unwrap();
                state.gpr[RAX] = if case == 0 {
                    end - 16
                } else if case == 1 {
                    end - 15
                } else {
                    end - 17
                };
                state.xmm.fill(0x33);
                state.ymm_upper.fill(0x77);
                let original = source_bytes();
                state.xmm[240..].copy_from_slice(&original[..16]);
                state.ymm_upper[240..].copy_from_slice(&original[16..]);
                state.mxcsr = 0x1f84;
                state.rflags = 0x602 | 0x8d5;
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
                    2 => arena.protect(data, PAGE, NativePermissions::READ).unwrap(),
                    3 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                    4 => engine.machine.usage.vector_bytes = u64::MAX - 15,
                    5 => engine.machine.usage.vector_operations = u64::MAX,
                    _ => {}
                }
                let usage = engine.usage();
                engine.machine.phase = Phase::Terminal;
                let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                let mut expected = state;
                let mut memory = [0xa5; 32];
                if case == 0 {
                    result.unwrap();
                    expected.guest_pc = planned.exit().next_guest_pc();
                    memory[16..].copy_from_slice(&original[16..]);
                    assert_eq!(engine.usage().vector_operations, 1);
                    assert_eq!(engine.usage().vector_bytes, 16);
                } else {
                    match case {
                        1 | 2 => assert!(matches!(result, Err(NativeDispatchError::Memory(_)))),
                        3..=5 => assert!(matches!(result, Err(NativeDispatchError::Budget(_)))),
                        _ => unreachable!(),
                    }
                    assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                    assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
                    assert!(engine.machine.phase == Phase::Terminal);
                }
                assert_eq!(engine.machine.state, expected);
                let mut actual = [0; 32];
                arena.pause().unwrap().read(end - 32, &mut actual).unwrap();
                assert_eq!(actual, memory, "integer={integer} case={case}");
            });
        }
    }
}
