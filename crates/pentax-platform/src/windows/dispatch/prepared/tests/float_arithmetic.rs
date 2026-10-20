// SPDX-License-Identifier: GPL-2.0-only

use super::*;

fn bits(value: u64, double: bool) -> [u8; 8] {
    if double {
        (value as f64).to_bits().to_le_bytes()
    } else {
        u64::from((value as f32).to_bits()).to_le_bytes()
    }
}

#[test]
fn float_arithmetic_handles_each_form_lane_alias_and_exact_memory_width() {
    for (scalar, wide, double) in [
        (true, false, false),
        (true, false, true),
        (false, false, false),
        (false, false, true),
        (false, true, false),
        (false, true, true),
    ] {
        for opcode in [0x58, 0x5c, 0x59, 0x5e, 0x5d, 0x5f, 0x51] {
            for memory in [false, true] {
                for destination in 0..3 {
                    let unary = !scalar && opcode == 0x51;
                    let element = if double { 8 } else { 4 };
                    let vector = if wide { 32 } else { 16 };
                    let width = if scalar { element } else { vector };
                    let pp = if scalar {
                        if double { 3 } else { 2 }
                    } else {
                        u8::from(double)
                    };
                    let prefix =
                        0x80 | if unary { 0x78 } else { 0x70 } | if wide { 4 } else { 0 } | pp;
                    let code = [
                        0xc5,
                        prefix,
                        opcode,
                        (destination as u8) << 3 | if memory { 0 } else { 0xc2 },
                        0xc3,
                    ];
                    with_source(Some(&code), |source, arena, stack, mut state| {
                        let mut left = [0; 32];
                        let mut right = [0; 32];
                        let mut output = [0; 32];
                        for (index, offset) in (0..vector).step_by(element).enumerate() {
                            let a = 6 + 2 * index as u64;
                            let b = if opcode == 0x51 {
                                (2 + index as u64).pow(2)
                            } else {
                                2
                            };
                            let value = match opcode {
                                0x58 => a + b,
                                0x5c => a - b,
                                0x59 => a * b,
                                0x5e => a / b,
                                0x5d => b,
                                0x5f => a,
                                _ => 2 + index as u64,
                            };
                            left[offset..offset + element]
                                .copy_from_slice(&bits(a, double)[..element]);
                            right[offset..offset + element]
                                .copy_from_slice(&bits(b, double)[..element]);
                            output[offset..offset + element]
                                .copy_from_slice(&bits(value, double)[..element]);
                        }
                        if scalar {
                            output[element..16].copy_from_slice(&left[element..16]);
                        }
                        state.xmm[16..32].copy_from_slice(&left[..16]);
                        state.xmm[32..48].copy_from_slice(&right[..16]);
                        state.ymm_upper.fill(0xa5);
                        if wide {
                            state.ymm_upper[16..32].copy_from_slice(&left[16..]);
                            state.ymm_upper[32..48].copy_from_slice(&right[16..]);
                        }
                        state.mxcsr |= 4; // Existing sticky state survives exact arithmetic.
                        let tail = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                            - width as u64;
                        arena.pause().unwrap().write(tail, &right[..width]).unwrap();
                        state.gpr[RAX] = tail;
                        let mut expected = state;
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
                        assert_eq!(engine.machine.state, expected);
                        assert_eq!(engine.machine.usage.vector_operations, 1);
                        assert_eq!(
                            engine.machine.usage.vector_bytes,
                            if memory { width as u64 } else { 0 }
                        );
                    });
                }
            }
        }
    }
}

#[test]
fn float_arithmetic_refuses_partial_reads_and_budgets_without_publishing_cpu_status() {
    for (prefix, width) in [(0xf2, 4_usize), (0xf3, 8), (0xf4, 32)] {
        for case in 0..5 {
            let code = [0xc5, prefix, 0x5e, 0x00, 0xc3];
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let tail = data + PAGE - width as u64;
                arena
                    .pause()
                    .unwrap()
                    .write(tail, &[0; 32][..width])
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
                    2 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
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
                assert_eq!(
                    engine.machine.usage.vector_operations,
                    usage.vector_operations
                );
                assert_eq!(engine.machine.usage.vector_bytes, usage.vector_bytes);
                assert!(engine.machine.phase == Phase::Terminal);
            });
        }
    }
}
