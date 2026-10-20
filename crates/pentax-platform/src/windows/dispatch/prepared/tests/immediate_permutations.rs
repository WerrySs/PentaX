// SPDX-License-Identifier: GPL-2.0-only
//! Original exhaustive immediate controls with bounded immutable-source reuse.

use super::*;

fn instruction(kind: usize, destination: u8, memory: bool, immediate: u8) -> Vec<u8> {
    vec![
        0xc4,
        if memory { 0x63 } else { 0x43 },
        if kind < 2 { 0x0d } else { 0xfd },
        [0x06, 0x46, 0x01, 0x00][kind],
        ((destination & 7) << 3) | if memory { 0 } else { 0xc5 },
        immediate,
    ]
}
fn inputs() -> ([u8; 32], [u8; 32]) {
    (
        std::array::from_fn(|i| (i as u8).wrapping_mul(17).wrapping_add(3)),
        std::array::from_fn(|i| (i as u8).wrapping_mul(29).wrapping_add(101)),
    )
}
fn expected(kind: usize, a: &[u8; 32], b: &[u8; 32], immediate: u8) -> [u8; 32] {
    let mut output = [0; 32];
    if kind < 2 {
        let lanes = [&a[..16], &a[16..], &b[..16], &b[16..]];
        for (half, bytes) in output.chunks_exact_mut(16).enumerate() {
            let control = (immediate >> (half * 4)) & 15;
            if control & 8 == 0 {
                bytes.copy_from_slice(lanes[usize::from(control & 3)]);
            }
        }
    } else {
        for (index, bytes) in output.chunks_exact_mut(8).enumerate() {
            let offset = usize::from((immediate >> (index * 2)) & 3) * 8;
            bytes.copy_from_slice(&b[offset..offset + 8]);
        }
    }
    output
}
fn put(state: &mut NativeCpuState, register: usize, value: &[u8; 32]) {
    let slot = register * 16;
    state.xmm[slot..slot + 16].copy_from_slice(&value[..16]);
    state.ymm_upper[slot..slot + 16].copy_from_slice(&value[16..]);
}

#[test]
fn immediate_permutations_all_controls_snapshot_either_alias_and_preserve_full_state() {
    let (a, b) = inputs();
    for kind in 0..4 {
        let shapes: &[(u8, bool)] = if kind < 2 {
            &[(14, false), (13, false), (9, false), (14, true)]
        } else {
            &[(13, false), (9, false), (9, true)]
        };
        for &(destination, memory) in shapes {
            let mut code = Vec::new();
            for immediate in 0..=255 {
                code.extend(instruction(kind, destination, memory, immediate));
            }
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let end = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE;
                let mut neighbors = [0xa5; 64];
                neighbors[31..63].copy_from_slice(&b);
                arena.pause().unwrap().write(end - 64, &neighbors).unwrap();
                state.gpr[RAX] = end - 33; // Unaligned complete m256 with a trailing canary.
                state.xmm.fill(0xcc);
                state.ymm_upper.fill(0xdd);
                state.rflags = 0x602 | 0x8d5;
                state.mxcsr = 0x1f84;
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
                    put(&mut before, usize::from(destination), &[0xee; 32]);
                    put(&mut before, 14, &a);
                    put(&mut before, 13, &b);
                    engine.machine.state = before;
                    let planned = block(
                        source,
                        &arena.pause().unwrap(),
                        before.guest_pc,
                        engine.placement(),
                    );
                    let mut after = before;
                    after.guest_pc = planned.exit().next_guest_pc();
                    put(
                        &mut after,
                        usize::from(destination),
                        &expected(kind, &a, &b, immediate),
                    );
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .unwrap();
                    assert_eq!(
                        engine.machine.state, after,
                        "kind={kind} dst={destination} mem={memory} imm={immediate}"
                    );
                    assert_eq!(engine.usage().vector_operations, u64::from(immediate) + 1);
                    assert_eq!(
                        engine.usage().vector_bytes,
                        if memory {
                            (u64::from(immediate) + 1) * 32
                        } else {
                            0
                        }
                    );
                    let mut actual = [0; 64];
                    arena.pause().unwrap().read(end - 64, &mut actual).unwrap();
                    assert_eq!(actual, neighbors);
                }
            });
        }
    }
}

#[test]
fn immediate_permutations_require_full_memory_even_for_zero_or_repeated_selection_atomically() {
    let (a, b) = inputs();
    for kind in 0..4 {
        // Both zero overrides, or four repetitions of only the first qword,
        // must still admit and read the entire architectural memory operand.
        let immediate = if kind < 2 { 0x88 } else { 0 };
        for case in 0..6 {
            let mut code = instruction(kind, 14, true, immediate);
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let end = data + PAGE;
                arena.pause().unwrap().write(end - 32, &b).unwrap();
                state.gpr[RAX] = if case == 1 { end - 31 } else { end - 32 };
                state.xmm.fill(0xcc);
                state.ymm_upper.fill(0xdd);
                put(&mut state, 14, &a);
                put(&mut state, 13, &b);
                state.rflags = 0x602 | 0x8d5;
                state.mxcsr = 0x1f84;
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
                    2 => arena
                        .protect(data, PAGE, NativePermissions::new(false, false, false))
                        .unwrap(),
                    3 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                    4 => engine.machine.usage.vector_bytes = u64::MAX - 31,
                    5 => engine.machine.usage.vector_operations = u64::MAX,
                    _ => {}
                }
                let usage = engine.usage();
                engine.machine.phase = Phase::Terminal;
                let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                let mut after = state;
                if case == 0 {
                    result.unwrap();
                    after.guest_pc = planned.exit().next_guest_pc();
                    put(&mut after, 14, &expected(kind, &a, &b, immediate));
                    assert_eq!(engine.usage().vector_operations, 1);
                    assert_eq!(engine.usage().vector_bytes, 32);
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
                assert_eq!(engine.machine.state, after, "kind={kind} case={case}");
                if case == 2 {
                    arena.protect(data, PAGE, NativePermissions::READ).unwrap();
                }
                let mut actual = [0; 32];
                arena.pause().unwrap().read(end - 32, &mut actual).unwrap();
                assert_eq!(actual, b);
            });
        }
    }
}
