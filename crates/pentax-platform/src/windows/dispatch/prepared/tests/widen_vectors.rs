// SPDX-License-Identifier: GPL-2.0-only

use super::*;

const FORMS: [(u8, usize, usize); 6] = [
    (0, 1, 2),
    (1, 1, 4),
    (2, 1, 8),
    (3, 2, 4),
    (4, 2, 8),
    (5, 4, 8),
];

fn widened(source: &[u8], input: usize, output: usize, signed: bool) -> Vec<u8> {
    source
        .chunks_exact(input)
        .flat_map(|value| {
            let number: i64 = match (input, signed) {
                (1, true) => i64::from(value[0] as i8),
                (2, true) => i64::from(i16::from_le_bytes(value.try_into().unwrap())),
                (4, true) => i64::from(i32::from_le_bytes(value.try_into().unwrap())),
                (1, false) => i64::from(value[0]),
                (2, false) => i64::from(u16::from_le_bytes(value.try_into().unwrap())),
                (4, false) => i64::from(u32::from_le_bytes(value.try_into().unwrap())),
                _ => unreachable!(),
            };
            number.to_le_bytes()[..output].to_vec()
        })
        .collect()
}

#[test]
fn integer_widening_preserves_signedness_exact_tuple_width_and_aliased_sources() {
    for (opcode, input, output) in FORMS {
        for signed in [false, true] {
            for size in [16, 32] {
                for operand in 0..3 {
                    // xmm15/ymm15 <- xmm1, self or narrow [RAX].
                    let code = [
                        0xc4,
                        if operand == 1 { 0x42 } else { 0x62 },
                        if size == 32 { 0x7d } else { 0x79 },
                        (if signed { 0x20 } else { 0x30 }) + opcode,
                        match operand {
                            0 => 0xf9,
                            1 => 0xff,
                            _ => 0x38,
                        },
                        0xc3,
                    ];
                    with_source(Some(&code), |source, arena, stack, mut state| {
                        let count = size / output * input;
                        let target =
                            source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                                - count as u64;
                        let bytes = [
                            0x00, 0x7f, 0x80, 0xff, 0xfe, 0x01, 0x90, 0x70, 0x81, 0, 0xff, 0x7f,
                            0x55, 0xaa, 0x80, 0xff,
                        ];
                        arena
                            .pause()
                            .unwrap()
                            .write(target, &bytes[..count])
                            .unwrap();
                        state.gpr[RAX] = target;
                        state.xmm.fill(0xa5);
                        state.ymm_upper.fill(0x69);
                        let from = if operand == 1 { 240 } else { 16 };
                        state.xmm[from..from + 16].copy_from_slice(&bytes);
                        let mut expected = state;
                        expected.xmm[240..256].copy_from_slice(
                            &widened(&bytes[..count], input, output, signed)[..16],
                        );
                        let values = widened(&bytes[..count], input, output, signed);
                        expected.ymm_upper[240..256].copy_from_slice(if size == 32 {
                            &values[16..]
                        } else {
                            &[0; 16]
                        });
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
                        assert_eq!(engine.machine.state, expected);
                        assert_eq!(engine.machine.usage.vector_operations, 1);
                        assert_eq!(
                            engine.machine.usage.vector_bytes,
                            if operand == 2 { count as u64 } else { 0 }
                        );
                        let mut actual = [0; 16];
                        arena
                            .pause()
                            .unwrap()
                            .read(target, &mut actual[..count])
                            .unwrap();
                        assert_eq!(&actual[..count], &bytes[..count]);
                    });
                }
            }
        }
    }
}

#[test]
fn integer_widening_refuses_incomplete_inputs_and_exhausted_budgets_before_cpu_changes() {
    for (opcode, input, output) in FORMS {
        for size in [16, 32] {
            for refusal in 0..4 {
                let code = [
                    0xc4,
                    0x62,
                    if size == 32 { 0x7d } else { 0x79 },
                    0x30 + opcode,
                    0x38,
                    0xc3,
                ];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let count = size / output * input;
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    state.gpr[RAX] = if refusal == 0 {
                        data + PAGE - count as u64 + 1
                    } else {
                        data
                    };
                    state.xmm.fill(0xa5);
                    state.ymm_upper.fill(0x69);
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
                        1 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                        2 => engine.machine.usage.vector_bytes = u64::MAX,
                        3 => engine.machine.usage.vector_operations = u64::MAX,
                        _ => {}
                    }
                    let operations = engine.machine.usage.vector_operations;
                    let bytes = engine.machine.usage.vector_bytes;
                    let error = engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .unwrap_err();
                    assert!(if refusal == 0 {
                        matches!(error, NativeDispatchError::Memory(_))
                    } else {
                        matches!(error, NativeDispatchError::Budget(_))
                    });
                    assert_eq!(engine.machine.state, state);
                    assert_eq!(engine.machine.usage.vector_operations, operations);
                    assert_eq!(engine.machine.usage.vector_bytes, bytes);
                });
            }
        }
    }
}
