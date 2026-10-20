// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[derive(Clone, Copy, Debug)]
enum Operation {
    Left,
    Right,
    Arithmetic,
    ByteLeft,
    ByteRight,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Counts {
    Immediate,
    Shared,
    Elements,
}
#[derive(Clone, Copy, Debug)]
struct Spec {
    opcode: u8,
    map: u8,
    w: u8,
    group: u8,
    element: usize,
    operation: Operation,
    counts: Counts,
}
fn immediate(opcode: u8, group: u8, element: usize, operation: Operation) -> Spec {
    Spec {
        opcode,
        map: 1,
        w: 0,
        group,
        element,
        operation,
        counts: Counts::Immediate,
    }
}
fn shared(opcode: u8, element: usize, operation: Operation) -> Spec {
    Spec {
        opcode,
        map: 1,
        w: 0,
        group: 0,
        element,
        operation,
        counts: Counts::Shared,
    }
}
fn elements(opcode: u8, w: u8, element: usize, operation: Operation) -> Spec {
    Spec {
        opcode,
        map: 2,
        w,
        group: 0,
        element,
        operation,
        counts: Counts::Elements,
    }
}

// Independent bit routing oracle, including sign-fill and isolated byte lanes.
fn reference(spec: Spec, input: &[u8; 32], counts: &[u8; 32], count: u64, size: usize) -> [u8; 32] {
    let mut result = [0; 32];
    let byte_lane = matches!(spec.operation, Operation::ByteLeft | Operation::ByteRight);
    let width = if byte_lane { 16 } else { spec.element };
    for bit in 0..size * 8 {
        let base = bit / (width * 8) * width;
        let position = bit % (width * 8);
        let amount = match spec.counts {
            Counts::Immediate => u128::from(count),
            Counts::Shared => counts[..8]
                .iter()
                .rev()
                .fold(0_u128, |v, b| v * 256 + u128::from(*b)),
            Counts::Elements => counts[base..base + width]
                .iter()
                .rev()
                .fold(0_u128, |v, b| v * 256 + u128::from(*b)),
        } * if byte_lane { 8 } else { 1 };
        let source = if matches!(spec.operation, Operation::Left | Operation::ByteLeft) {
            position as i128 - amount as i128
        } else {
            position as i128 + amount as i128
        };
        let selected = if (0..(width * 8) as i128).contains(&source) {
            (input[base + source as usize / 8] >> (source as usize % 8)) & 1
        } else if matches!(spec.operation, Operation::Arithmetic) {
            input[base + width - 1] >> 7
        } else {
            0
        };
        result[bit / 8] |= selected << (bit % 8);
    }
    result
}

fn code(spec: Spec, wide: bool, destination: u8, memory: bool, count: u8) -> Vec<u8> {
    let lane = if wide { 4 } else { 0 };
    if spec.counts == Counts::Immediate {
        vec![
            0xc4,
            0xe1,
            ((!destination & 15) << 3) | lane | 1,
            spec.opcode,
            0xc1 | spec.group << 3,
            count,
            0xc3,
        ]
    } else {
        vec![
            0xc4,
            (if destination == 15 { 0x60 } else { 0xe0 }) | spec.map,
            spec.w << 7 | 0x71 | lane,
            spec.opcode,
            (destination & 7) << 3 | if memory { 0 } else { 0xc2 },
            0xc3,
        ]
    }
}

#[test]
fn vector_shifts_cover_all_count_forms_boundaries_and_aliases() {
    use Operation::*;
    let forms = [
        immediate(0x71, 6, 2, Left),
        immediate(0x72, 6, 4, Left),
        immediate(0x73, 6, 8, Left),
        immediate(0x71, 2, 2, Right),
        immediate(0x72, 2, 4, Right),
        immediate(0x73, 2, 8, Right),
        immediate(0x71, 4, 2, Arithmetic),
        immediate(0x72, 4, 4, Arithmetic),
        immediate(0x73, 7, 1, ByteLeft),
        immediate(0x73, 3, 1, ByteRight),
        shared(0xf1, 2, Left),
        shared(0xf2, 4, Left),
        shared(0xf3, 8, Left),
        shared(0xd1, 2, Right),
        shared(0xd2, 4, Right),
        shared(0xd3, 8, Right),
        shared(0xe1, 2, Arithmetic),
        shared(0xe2, 4, Arithmetic),
        elements(0x47, 0, 4, Left),
        elements(0x47, 1, 8, Left),
        elements(0x45, 0, 4, Right),
        elements(0x45, 1, 8, Right),
        elements(0x46, 0, 4, Arithmetic),
    ];
    for spec in forms {
        let limit = if spec.element == 1 {
            16
        } else {
            spec.element as u64 * 8
        };
        let mut amounts = vec![0, 1, limit - 1, limit, limit + 1, 255];
        if spec.counts == Counts::Shared {
            amounts.extend([1_u64 << 32, u64::MAX]);
        }
        if spec.counts == Counts::Elements {
            amounts.truncate(1);
        }
        for wide in [false, true] {
            for destination in [1, 2, 15] {
                for memory in [false, true] {
                    if memory && spec.counts == Counts::Immediate {
                        continue;
                    }
                    for amount in &amounts {
                        let bytes = code(spec, wide, destination, memory, *amount as u8);
                        with_source(Some(&bytes), |source, arena, stack, mut state| {
                            let size = if wide { 32 } else { 16 };
                            let count_width = if spec.counts == Counts::Shared {
                                16
                            } else {
                                size
                            };
                            let mut input = [0; 32];
                            for (i, byte) in input.iter_mut().enumerate() {
                                *byte = (i as u8).wrapping_mul(37).wrapping_add(0x91);
                            }
                            let mut counts = [0xff; 32];
                            if spec.counts == Counts::Elements {
                                let choices = [0, 1, limit - 1, limit, limit + 1, u64::MAX, 31, 0];
                                for (i, lane) in counts.chunks_exact_mut(spec.element).enumerate() {
                                    lane.copy_from_slice(&choices[i].to_le_bytes()[..spec.element]);
                                }
                            } else {
                                counts[..8].copy_from_slice(&amount.to_le_bytes());
                            }
                            let expected_bytes = reference(spec, &input, &counts, *amount, size);
                            let address =
                                source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                                    - count_width as u64;
                            arena
                                .pause()
                                .unwrap()
                                .write(address, &counts[..count_width])
                                .unwrap();
                            state.gpr[RAX] = address;
                            state.xmm[16..32].copy_from_slice(&input[..16]);
                            state.ymm_upper[16..32].copy_from_slice(&input[16..]);
                            state.xmm[32..48].copy_from_slice(&counts[..16]);
                            state.ymm_upper[32..48].copy_from_slice(&counts[16..]);
                            state.mxcsr |= 0x25;
                            state.rflags |= 0x8d5;
                            let mut expected = state;
                            let at = usize::from(destination) * 16;
                            expected.xmm[at..at + 16].copy_from_slice(&expected_bytes[..16]);
                            expected.ymm_upper[at..at + 16].copy_from_slice(&expected_bytes[16..]);
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
                            expected.guest_pc = planned.exit().next_guest_pc();
                            assert_eq!(
                                engine.machine.state, expected,
                                "{spec:?}, count={amount}, wide={wide}, memory={memory}"
                            );
                            assert_eq!(engine.machine.usage.vector_operations, 1);
                            assert_eq!(
                                engine.machine.usage.vector_bytes,
                                if memory { count_width as u64 } else { 0 }
                            );
                        });
                    }
                }
            }
        }
    }
}

#[test]
fn vector_shifts_validate_the_entire_count_span_before_publication() {
    for spec in [
        shared(0xd2, 4, Operation::Right),
        elements(0x45, 0, 4, Operation::Right),
    ] {
        for wide in [false, true] {
            for case in 0..4 {
                let bytes = code(spec, wide, 1, true, 0);
                with_source(Some(&bytes), |source, arena, stack, mut state| {
                    let width = if !wide || spec.counts == Counts::Shared {
                        16
                    } else {
                        32
                    };
                    let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    let end = page + PAGE;
                    arena
                        .pause()
                        .unwrap()
                        .write(end - width, &[0; 32][..width as usize])
                        .unwrap();
                    state.gpr[RAX] = end - width + if case == 1 { 8 } else { 0 };
                    state.xmm[16..32].fill(0xa5);
                    state.ymm_upper[16..32].fill(0x5a);
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
                            .protect(page, PAGE, NativePermissions::new(false, false, false))
                            .unwrap();
                    }
                    let initial = if case == 3 { u64::MAX - width + 1 } else { 0 };
                    engine.machine.usage.vector_bytes = initial;
                    engine.machine.phase = Phase::Terminal;
                    let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                    let mut expected = state;
                    if case == 0 {
                        result.unwrap();
                        if !wide {
                            expected.ymm_upper[16..32].fill(0);
                        }
                        expected.guest_pc = planned.exit().next_guest_pc();
                    } else {
                        assert!(result.is_err());
                        assert_eq!(engine.machine.usage.vector_operations, 0);
                        assert_eq!(engine.machine.usage.vector_bytes, initial);
                    }
                    assert_eq!(engine.machine.state, expected);
                });
            }
        }
    }
}
