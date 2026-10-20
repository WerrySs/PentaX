// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn floating_bit_tests_use_only_sign_bits_preserve_vectors_and_set_exact_flags() {
    for (opcode, element) in [(0x0e, 4), (0x0f, 8)] {
        for size in [16, 32] {
            for operand in 0..3 {
                for (left_mask, right_mask) in
                    [(0, 0), (0, 255), (255, 0), (255, 255), (1, 3), (0x55, 0xaa)]
                {
                    let code = [
                        0xc4,
                        if operand == 1 { 0x42 } else { 0x62 },
                        if size == 32 { 0x7d } else { 0x79 },
                        opcode,
                        match operand {
                            0 => 0xf9,
                            1 => 0xff,
                            _ => 0x38,
                        },
                        0xc3,
                    ];
                    with_source(Some(&code), |source, arena, stack, mut state| {
                        let address =
                            source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                                - size as u64;
                        // Every ignored bit is set in both sources; these
                        // patterns distinguish floating sign tests from VPTEST.
                        let mut left = [0xff; 32];
                        let mut right = [0xff; 32];
                        for lane in 0..size / element {
                            left[lane * element + element - 1] = if left_mask & (1 << lane) != 0 {
                                0xff
                            } else {
                                0x7f
                            };
                            right[lane * element + element - 1] = if right_mask & (1 << lane) != 0 {
                                0xff
                            } else {
                                0x7f
                            };
                        }
                        state.xmm[240..256].copy_from_slice(&left[..16]);
                        state.ymm_upper[240..256].copy_from_slice(&left[16..]);
                        state.xmm[16..32].copy_from_slice(&right[..16]);
                        state.ymm_upper[16..32].copy_from_slice(&right[16..]);
                        state.rflags |= 0x8d5;
                        state.gpr[RAX] = address;
                        arena
                            .pause()
                            .unwrap()
                            .write(address, &right[..size])
                            .unwrap();
                        let mut expected = state;
                        let lane_mask = (1 << (size / element)) - 1;
                        let a = left_mask & lane_mask;
                        let b = (if operand == 1 { left_mask } else { right_mask }) & lane_mask;
                        expected.rflags = (state.rflags & !0x8d5)
                            | (u64::from(a & b == 0) << 6)
                            | u64::from(!a & b == 0);
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
                            if operand == 2 { size as u64 } else { 0 }
                        );
                        let mut actual = [0; 32];
                        arena
                            .pause()
                            .unwrap()
                            .read(address, &mut actual[..size])
                            .unwrap();
                        assert_eq!(&actual[..size], &right[..size]);
                    });
                }
            }
        }
    }
}

#[test]
fn floating_bit_tests_refuse_partial_memory_and_counter_work_limits_before_flags() {
    for opcode in [0x0e, 0x0f] {
        for size in [16, 32] {
            for refusal in 0..4 {
                let code = [
                    0xc4,
                    0x62,
                    if size == 32 { 0x7d } else { 0x79 },
                    opcode,
                    0x38,
                    0xc3,
                ];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    state.gpr[RAX] = if refusal == 0 {
                        data + PAGE - size as u64 + 1
                    } else {
                        data
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
