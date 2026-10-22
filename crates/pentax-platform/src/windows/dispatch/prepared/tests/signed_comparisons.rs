// SPDX-License-Identifier: GPL-2.0-only
//! Original signed extrema, complete operand snapshots and refusal transactions.
use super::*;

fn instruction(width: usize, size: usize, form: usize, ignored_w: bool) -> Vec<u8> {
    let memory = form == 4;
    let destination = match form {
        1 | 3 => 13,
        2 => 14,
        _ => 9,
    };
    let map = if width == 8 { 2 } else { 1 };
    let opcode = match width {
        1 => 0x64,
        2 => 0x65,
        4 => 0x66,
        8 => 0x37,
        _ => unreachable!(),
    };
    vec![
        0xc4,
        (if memory { 0x60 } else { 0x40 }) | map,
        0x11 | if size == 32 { 4 } else { 0 } | if ignored_w { 0x80 } else { 0 },
        opcode,
        ((destination & 7) << 3)
            | if memory {
                0
            } else if form == 3 {
                0xc5
            } else {
                0xc6
            },
    ]
}
fn operands(width: usize, case: usize) -> ([u8; 32], [u8; 32], [u8; 32]) {
    let (min, max) = match width {
        1 => (i64::from(i8::MIN), i64::from(i8::MAX)),
        2 => (i64::from(i16::MIN), i64::from(i16::MAX)),
        4 => (i64::from(i32::MIN), i64::from(i32::MAX)),
        8 => (i64::MIN, i64::MAX),
        _ => unreachable!(),
    };
    // Expected truth is literal, independently of the implementation's sign-bit bias.
    let cases = [
        (min, max, false),
        (max, min, true),
        (-1, 0, false),
        (0, -1, true),
        (min, min, false),
        (max, max, false),
        (1, 0, true),
        (-2, -1, false),
    ];
    let mut left = [0; 32];
    let mut right = [0; 32];
    let mut expected = [0; 32];
    for lane in 0..32 / width {
        let (a, b, greater) = cases[(lane + case * 2) % cases.len()];
        let start = lane * width;
        left[start..start + width].copy_from_slice(&a.to_le_bytes()[..width]);
        right[start..start + width].copy_from_slice(&b.to_le_bytes()[..width]);
        expected[start..start + width].fill(if greater { 0xff } else { 0 });
    }
    (left, right, expected)
}

#[test]
fn signed_compare_extrema_equality_high_aliases_and_full_vector_state_follow_exact_lane_widths() {
    for width in [1, 2, 4, 8] {
        for size in [16, 32] {
            let mut code = Vec::new();
            for form in 0..5 {
                for case in 0..4 {
                    code.extend(instruction(width, size, form, case % 2 != 0));
                }
            }
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let end = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE;
                state.gpr[RAX] = end - size as u64 - 1; // VEX unaligned, complete final byte still owned.
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
                let mut operations = 0;
                let mut transferred = 0;
                for form in 0..5 {
                    for case in 0..4 {
                        let (left, right, masks) = operands(width, case);
                        let mut neighbors = [0x55; 64];
                        let offset = 64 - size - 1;
                        neighbors[offset..offset + size].copy_from_slice(&right[..size]);
                        arena.pause().unwrap().write(end - 64, &neighbors).unwrap();
                        let mut before = engine.machine.state;
                        before.xmm.fill(0xa5);
                        before.ymm_upper.fill(0x5a);
                        before.xmm[208..224].copy_from_slice(&left[..16]);
                        before.ymm_upper[208..224].copy_from_slice(&left[16..]);
                        before.xmm[224..240].copy_from_slice(&right[..16]);
                        before.ymm_upper[224..240].copy_from_slice(&right[16..]);
                        engine.machine.state = before;
                        let planned = block(
                            source,
                            &arena.pause().unwrap(),
                            before.guest_pc,
                            engine.placement(),
                        );
                        let destination = match form {
                            1 | 3 => 208,
                            2 => 224,
                            _ => 144,
                        };
                        let mut output = [0; 32];
                        if form != 3 {
                            output[..size].copy_from_slice(&masks[..size]);
                        }
                        let mut after = before;
                        after.xmm[destination..destination + 16].copy_from_slice(&output[..16]);
                        after.ymm_upper[destination..destination + 16]
                            .copy_from_slice(&output[16..]);
                        after.guest_pc = planned.exit().next_guest_pc();
                        engine
                            .resolve(&mut arena.pause().unwrap(), &planned)
                            .unwrap();
                        operations += 1;
                        transferred += if form == 4 { size as u64 } else { 0 };
                        assert_eq!(
                            engine.machine.state, after,
                            "width={width} size={size} form={form} case={case}"
                        );
                        assert_eq!(engine.usage().vector_operations, operations);
                        assert_eq!(engine.usage().vector_bytes, transferred);
                        assert_eq!(engine.usage().native_entries, 0);
                        let mut actual = [0; 64];
                        arena.pause().unwrap().read(end - 64, &mut actual).unwrap();
                        assert_eq!(actual, neighbors);
                    }
                }
            });
        }
    }
}

#[test]
fn signed_compare_complete_span_permissions_noncanonical_sources_and_budgets_refuse_atomically() {
    for width in [1, 2, 4, 8] {
        for size in [16, 32] {
            for case in 0..9 {
                let mut code = instruction(width, size, 4, false);
                if case == 8 {
                    code.insert(0, 0x64);
                }
                code.push(0xc3);
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    let end = data + PAGE;
                    let (left, right, _) = operands(width, 0);
                    arena
                        .pause()
                        .unwrap()
                        .write(end - size as u64, &right[..size])
                        .unwrap();
                    state.gpr[RAX] = match case {
                        1 => end - size as u64 + 1,
                        2 => end,
                        3 => 0x0000_8000_0000_0000,
                        _ => end - size as u64,
                    };
                    state.xmm.fill(0xa5);
                    state.ymm_upper.fill(0x5a);
                    state.xmm[208..224].copy_from_slice(&left[..16]);
                    state.ymm_upper[208..224].copy_from_slice(&left[16..]);
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
                        4 => arena
                            .protect(data, PAGE, NativePermissions::new(false, false, false))
                            .unwrap(),
                        5 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                        6 => engine.machine.usage.vector_bytes = u64::MAX - size as u64 + 1,
                        7 => engine.machine.usage.vector_operations = u64::MAX,
                        _ => {}
                    }
                    let usage = engine.usage();
                    engine.machine.phase = Phase::Terminal;
                    let outcome = engine.resolve(&mut arena.pause().unwrap(), &planned);
                    if case == 0 {
                        outcome.unwrap();
                        assert_eq!(engine.usage().vector_bytes, size as u64);
                    } else {
                        match case {
                            1..=4 => {
                                assert!(matches!(outcome, Err(NativeDispatchError::Memory(_))))
                            }
                            5..=7 => {
                                assert!(matches!(outcome, Err(NativeDispatchError::Budget(_))))
                            }
                            _ => assert!(matches!(
                                outcome,
                                Err(NativeDispatchError::UnsupportedVectorInstruction)
                            )),
                        }
                        assert_eq!(engine.machine.state, state);
                        assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                        assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
                        assert_eq!(engine.usage().native_entries, 0);
                        assert!(engine.machine.phase == Phase::Terminal);
                    }
                    if case == 4 {
                        arena.protect(data, PAGE, NativePermissions::READ).unwrap();
                    }
                    let mut actual = [0; 32];
                    arena
                        .pause()
                        .unwrap()
                        .read(end - size as u64, &mut actual[..size])
                        .unwrap();
                    assert_eq!(&actual[..size], &right[..size]);
                });
            }
        }
    }
}

#[test]
fn signed_compare_evex_masks_mmx_and_explicit_rep_prefixes_do_not_enter_the_checked_family() {
    for opcode in [0x64, 0x65, 0x66] {
        for code in [
            vec![0x62, 0xf1, 0x15, 0x08, opcode, 0xce, 0xc3],
            vec![0x0f, opcode, 0xca, 0xc3],
            vec![0xf3, 0xc4, 0x41, 0x11, opcode, 0xce, 0xc3],
        ] {
            // A REP before VEX may be rejected by the decoder itself. Do not
            // construct a materialized block from an invalid source encoding.
            let mut decoder = pentax_codegen::Decoder::with_ip(
                64,
                &code,
                0,
                pentax_codegen::DecoderOptions::NONE,
            );
            if decoder.decode().is_invalid() {
                continue;
            }
            with_source(Some(&code), |source, arena, stack, mut state| {
                state.xmm.fill(0x80);
                state.ymm_upper.fill(0x7f);
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
                engine.machine.phase = Phase::Terminal;
                let usage = engine.usage();
                let outcome = engine.resolve(&mut arena.pause().unwrap(), &planned);
                if matches!(code[0], 0x62 | 0x0f) {
                    assert_eq!(
                        outcome,
                        Ok(NativeBlockStop::Unsupported {
                            guest_pc: state.guest_pc
                        }),
                        "code={code:x?}"
                    );
                } else {
                    assert_eq!(
                        outcome,
                        Err(NativeDispatchError::UnsupportedVectorInstruction),
                        "code={code:x?}"
                    );
                }
                assert_eq!(engine.machine.state, state);
                assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
                assert_eq!(engine.usage().native_entries, 0);
                assert!(engine.machine.phase == Phase::Terminal);
            });
        }
    }
}
