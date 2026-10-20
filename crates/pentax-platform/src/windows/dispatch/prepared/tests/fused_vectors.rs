// SPDX-License-Identifier: GPL-2.0-only
use super::*;

fn encode(
    kind: u8,
    order: u8,
    scalar: bool,
    wide: bool,
    double: bool,
    destination: usize,
    memory: bool,
) -> [u8; 6] {
    let opcode =
        [0x98, 0x9a, 0x9c, 0x9e, 0x96, 0x97][kind as usize] + order * 16 + u8::from(scalar);
    let prefix = 2 | 0x40 | if destination < 8 { 0x80 } else { 0 } | if memory { 0x20 } else { 0 };
    [
        0xc4,
        prefix,
        0x31 | if double { 0x80 } else { 0 } | if wide { 4 } else { 0 },
        opcode,
        ((destination as u8 & 7) << 3) | if memory { 0 } else { 0xc2 },
        0xc3,
    ]
}
fn floating(value: i64, double: bool) -> [u8; 8] {
    if double {
        (value as f64).to_bits().to_le_bytes()
    } else {
        u64::from((value as f32).to_bits()).to_le_bytes()
    }
}

#[test]
fn all_fma3_forms_keep_operand_order_aliases_scalar_dest_lanes_and_full_memory_spans() {
    let mut cases = 0;
    for kind in 0..6 {
        for order in 0..3 {
            for (scalar, wide, double) in [
                (true, false, false),
                (true, false, true),
                (false, false, false),
                (false, true, false),
                (false, false, true),
                (false, true, true),
            ] {
                if kind >= 4 && scalar {
                    continue;
                }
                for memory in [false, true] {
                    for destination in [0, 9, 10, 15] {
                        let code = encode(kind, order, scalar, wide, double, destination, memory);
                        let element = if double { 8 } else { 4 };
                        let vector = if wide { 32 } else { 16 };
                        let width = if scalar { element } else { vector };
                        with_source(Some(&code), |source, arena, stack, mut state| {
                            let mut a = [0; 32];
                            let mut b = [0; 32];
                            let mut c = [0; 32];
                            for (lane, at) in (0..32).step_by(element).enumerate() {
                                for (output, value) in [(&mut a, 2), (&mut b, 3), (&mut c, 4)] {
                                    output[at..at + element].copy_from_slice(
                                        &floating(value + lane as i64, double)[..element],
                                    );
                                }
                            }
                            state.ymm_upper.fill(0xa5);
                            for (register, input) in [(9, &b), (10, &c), (destination, &a)] {
                                state.xmm[register * 16..register * 16 + 16]
                                    .copy_from_slice(&input[..16]);
                                state.ymm_upper[register * 16..register * 16 + 16]
                                    .copy_from_slice(&input[16..]);
                            }
                            let tail =
                                source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                                    - width as u64;
                            arena.pause().unwrap().write(tail, &c[..width]).unwrap();
                            state.gpr[RAX] = tail;
                            state.mxcsr = 0x1f84;
                            let mut expected = state;
                            let mut output = if scalar { a } else { [0; 32] };
                            output[16..].fill(0);
                            for (lane, at) in (0..width).step_by(element).enumerate() {
                                let a = 2 + lane as i64;
                                let b = if destination == 9 { a } else { 3 + lane as i64 };
                                let c = if !memory && destination == 10 {
                                    a
                                } else {
                                    4 + lane as i64
                                };
                                let (x, y, z) = match order {
                                    0 => (a, c, b),
                                    1 => (b, a, c),
                                    _ => (b, c, a),
                                };
                                let product = x * y;
                                let result = match kind {
                                    0 => product + z,
                                    1 => product - z,
                                    2 => -product + z,
                                    3 => -product - z,
                                    4 => {
                                        if lane % 2 == 0 {
                                            product - z
                                        } else {
                                            product + z
                                        }
                                    }
                                    _ => {
                                        if lane % 2 == 0 {
                                            product + z
                                        } else {
                                            product - z
                                        }
                                    }
                                };
                                output[at..at + element]
                                    .copy_from_slice(&floating(result, double)[..element]);
                            }
                            expected.xmm[destination * 16..destination * 16 + 16]
                                .copy_from_slice(&output[..16]);
                            expected.ymm_upper[destination * 16..destination * 16 + 16]
                                .copy_from_slice(&output[16..]);
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
                                "kind={kind} order={order} scalar={scalar} wide={wide} double={double} dest={destination} memory={memory}"
                            );
                            assert_eq!(engine.machine.usage.vector_operations, 1);
                            assert_eq!(
                                engine.machine.usage.vector_bytes,
                                if memory { width as u64 } else { 0 }
                            );
                        });
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 768);
}

#[test]
fn fma_memory_and_budget_refusals_publish_no_cpu_register_or_status_changes() {
    for (scalar, wide, double) in [
        (true, false, false),
        (true, false, true),
        (false, false, true),
        (false, true, true),
    ] {
        let width = if scalar {
            if double { 8 } else { 4 }
        } else if wide {
            32
        } else {
            16
        };
        for case in 0..5 {
            let code = encode(0, 1, scalar, wide, double, 0, true);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let tail = data + PAGE - width;
                arena
                    .pause()
                    .unwrap()
                    .write(tail, &[0; 32][..width as usize])
                    .unwrap();
                state.gpr[RAX] = tail + u64::from(case == 0);
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
                        .protect(data, PAGE, NativePermissions::new(false, false, false))
                        .unwrap(),
                    2 => {
                        engine.machine.limits.max_work_units =
                            engine.machine.usage.work_units + engine.machine.target_visits + 640
                    }
                    3 => engine.machine.usage.vector_bytes = u64::MAX,
                    4 => engine.machine.usage.vector_operations = u64::MAX,
                    _ => {}
                }
                let usage = engine.machine.usage;
                engine.machine.phase = Phase::Terminal;
                assert!(
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .is_err()
                );
                assert_eq!(engine.machine.state, state);
                assert_eq!(engine.machine.usage.vector_bytes, usage.vector_bytes);
                assert_eq!(
                    engine.machine.usage.vector_operations,
                    usage.vector_operations
                );
                assert!(engine.machine.phase == Phase::Terminal);
            });
        }
    }
}
