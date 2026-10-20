// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn modeled_vector_equality_compares_whole_elements_and_preserves_alias_sources() {
    for (width, opcode) in [(1, 0x74), (2, 0x75), (4, 0x76), (8, 0x29)] {
        for size in [16, 32] {
            for form in 0..4 {
                let code = [
                    0xc4,
                    (if form == 0 || form == 3 { 0x60 } else { 0xe0 })
                        | if width == 8 { 2 } else { 1 },
                    0x71 | if size == 32 { 4 } else { 0 },
                    opcode,
                    match form {
                        0 => 0xfa,
                        1 => 0xca,
                        2 => 0xd2,
                        _ => 0x38,
                    },
                    0xc3,
                ];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + 17;
                    let left = std::array::from_fn::<_, 32, _>(|i| (i as u8).wrapping_mul(17));
                    let mut right = left;
                    for i in (1..32 / width).step_by(2) {
                        right[(i + 1) * width - 1] ^= 0x80;
                    }
                    arena.pause().unwrap().write(data, &right).unwrap();
                    state.gpr[RAX] = data;
                    state.xmm.fill(0xa5);
                    state.ymm_upper.fill(0x5a);
                    state.xmm[16..32].copy_from_slice(&left[..16]);
                    state.ymm_upper[16..32].copy_from_slice(&left[16..]);
                    state.xmm[32..48].copy_from_slice(&right[..16]);
                    state.ymm_upper[32..48].copy_from_slice(&right[16..]);
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
                    let destination = match form {
                        1 => 16,
                        2 => 32,
                        _ => 240,
                    };
                    let mut result = [0; 32];
                    for (i, byte) in result[..size].iter_mut().enumerate() {
                        *byte = if (i / width) % 2 == 0 { 0xff } else { 0 };
                    }
                    let mut expected = state;
                    expected.xmm[destination..destination + 16].copy_from_slice(&result[..16]);
                    expected.ymm_upper[destination..destination + 16]
                        .copy_from_slice(&result[16..]);
                    expected.guest_pc = planned.exit().next_guest_pc();
                    assert_eq!(engine.machine.state, expected);
                    assert_eq!(engine.usage().vector_operations, 1);
                    assert_eq!(
                        engine.usage().vector_bytes,
                        if form == 3 { size as u64 } else { 0 }
                    );
                    let mut actual = [0; 32];
                    arena.pause().unwrap().read(data, &mut actual).unwrap();
                    assert_eq!(actual, right);
                });
            }
        }
    }
}

#[test]
fn modeled_byte_masks_keep_byte_order_zero_upper_gpr_and_preserve_vectors() {
    for size in [16, 32] {
        for wide in [false, true] {
            let code = [
                0xc4,
                0x41,
                0x79 | if size == 32 { 4 } else { 0 } | if wide { 0x80 } else { 0 },
                0xd7,
                0xcf,
                0xc3,
            ];
            with_source(Some(&code), |source, arena, stack, mut state| {
                let mut input = [0x7f; 32];
                for i in [0, 7, 8, 15, 16, 31] {
                    input[i] = 0x80;
                }
                state.gpr[9] = u64::MAX;
                state.xmm[240..].copy_from_slice(&input[..16]);
                state.ymm_upper[240..].copy_from_slice(&input[16..]);
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
                let mut expected = state;
                expected.gpr[9] = if size == 32 { 0x8001_8181 } else { 0x8181 };
                expected.guest_pc = planned.exit().next_guest_pc();
                assert_eq!(engine.machine.state, expected);
                assert_eq!(engine.usage().vector_operations, 1);
                assert_eq!(engine.usage().vector_bytes, 0);
            });
        }
    }
}

#[test]
fn modeled_comparison_and_mask_refusals_preserve_guest_state() {
    for case in 0..8 {
        let compare = [0xc5, 0xf5, 0x74, 0x00, 0xc3];
        let mask = [0xc5, 0xfd, 0xd7, 0xe1, 0xc3]; // ESP cannot escape the owned stack.
        let test = [0xc4, 0xe2, 0x7d, 0x17, 0x00, 0xc3];
        with_source(
            Some(if case == 5 {
                &mask[..]
            } else if case >= 6 {
                &test[..]
            } else {
                &compare[..]
            }),
            |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                state.gpr[RAX] = if case == 0 || case == 6 {
                    data + PAGE - 16
                } else {
                    data
                };
                state.xmm.fill(0xa5);
                state.ymm_upper.fill(0x5a);
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
                    1 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                    2 => engine.machine.usage.vector_bytes = u64::MAX,
                    3 | 7 => arena
                        .protect(data, PAGE, NativePermissions::new(false, false, false))
                        .unwrap(),
                    4 => engine.machine.usage.vector_operations = u64::MAX,
                    _ => {}
                }
                let operations = engine.usage().vector_operations;
                engine.machine.phase = Phase::Terminal;
                assert!(
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .is_err()
                );
                assert_eq!(engine.machine.state, state);
                assert_eq!(engine.usage().vector_operations, operations);
                assert!(engine.machine.phase == Phase::Terminal);
            },
        );
    }
}

#[test]
fn modeled_vector_tests_reduce_all_bits_to_flags_without_changing_vectors() {
    for size in [16, 32] {
        for memory in [false, true] {
            // All CF/ZF combinations, a low-bit-only distinction, and upper
            // lane differences which must affect only the 256-bit form.
            for (left_byte, right_byte, position, flags128, flags256) in [
                (0x00, 0x00, 0, 0x41, 0x41),
                (0x00, 0x01, 0, 0x40, 0x40),
                (0x01, 0x01, 0, 0x01, 0x01),
                (0x01, 0x03, 0, 0x00, 0x00),
                (0x01, 0x01, 31, 0x41, 0x01),
                (0x00, 0x01, 31, 0x41, 0x40),
            ] {
                let code = [
                    0xc4,
                    0x62,
                    0x79 | if size == 32 { 4 } else { 0 },
                    0x17,
                    if memory { 0x38 } else { 0xfa },
                    0xc3,
                ];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + 17;
                    let mut left = [0; 32];
                    let mut right = [0; 32];
                    left[position] = left_byte;
                    right[position] = right_byte;
                    arena.pause().unwrap().write(data, &right).unwrap();
                    state.gpr[RAX] = data;
                    state.rflags = 0x202 | 0x8d5 | 0x400;
                    state.xmm[240..].copy_from_slice(&left[..16]);
                    state.ymm_upper[240..].copy_from_slice(&left[16..]);
                    state.xmm[32..48].copy_from_slice(&right[..16]);
                    state.ymm_upper[32..48].copy_from_slice(&right[16..]);
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
                    let mut expected = state;
                    expected.rflags = 0x602 | if size == 32 { flags256 } else { flags128 };
                    expected.guest_pc = planned.exit().next_guest_pc();
                    assert_eq!(engine.machine.state, expected);
                    assert_eq!(engine.usage().vector_operations, 1);
                    assert_eq!(
                        engine.usage().vector_bytes,
                        if memory { size as u64 } else { 0 }
                    );
                });
            }
        }
    }
}
