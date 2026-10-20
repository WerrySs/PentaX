// SPDX-License-Identifier: GPL-2.0-only
//! Original element-sign extraction and atomic runtime-refusal examples.

use super::*;

fn encode(double: bool, vector_wide: bool, gpr_wide: bool, destination: u8) -> [u8; 6] {
    // Source is XMM/YMM15, including the inverted high-register B bit.
    [
        0xc4,
        if destination >= 8 { 0x41 } else { 0xc1 },
        0x78 | u8::from(double) | if vector_wide { 4 } else { 0 } | if gpr_wide { 0x80 } else { 0 },
        0x50,
        0xc7 | ((destination & 7) << 3),
        0xc3,
    ]
}
fn input(double: bool, pattern: u64, size: usize) -> [u8; 32] {
    // Interior0x80 bytes are decoys. Only each element's actual sign bit
    // belongs in the result, including signed zeros and quiet/signaling NaNs.
    let (width, sign, templates): (usize, u64, &[u64]) = if double {
        (
            8,
            1 << 63,
            &[
                0,
                0x7ff8_8080_8080_8080,
                0x7ff0_0000_0000_0001,
                0x3ff0_80ff_80ff_80ff,
            ],
        )
    } else {
        (
            4,
            1 << 31,
            &[
                0,
                0x7fc0_8080,
                0x7f80_0001,
                0x0080_8080,
                0x3f80_ff80,
                0x7f80_0000,
                0x7fff_00ff,
                0x007f_ffff,
            ],
        )
    };
    let mut bytes = [0xa5; 32];
    for lane in 0..size / width {
        let value = templates[lane] | if pattern & (1 << lane) != 0 { sign } else { 0 };
        bytes[lane * width..(lane + 1) * width].copy_from_slice(&value.to_le_bytes()[..width]);
    }
    bytes
}

#[test]
fn all_float_mask_forms_extract_every_sign_pattern_zero_extend_and_preserve_complete_state() {
    for double in [false, true] {
        for vector_wide in [false, true] {
            for gpr_wide in [false, true] {
                let code = encode(double, vector_wide, gpr_wide, 9);
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let size = if vector_wide { 32 } else { 16 };
                    let lanes = size / if double { 8 } else { 4 };
                    state.xmm.fill(0x33);
                    state.ymm_upper.fill(0x55);
                    state.gpr[9] = u64::MAX;
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
                    // Register-only semantics must not inspect the unrelated
                    // data page. Reuse one owned setup for all finite patterns.
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    arena
                        .protect(data, PAGE, NativePermissions::new(false, false, false))
                        .unwrap();
                    let mut operations = 0;
                    for pattern in 0..(1_u64 << lanes) {
                        let bytes = input(double, pattern, size);
                        state.xmm[240..].copy_from_slice(&bytes[..16]);
                        state.ymm_upper[240..].copy_from_slice(&bytes[16..]);
                        engine.machine.state = state;
                        let mut expected = state;
                        expected.gpr[9] = pattern;
                        expected.guest_pc = planned.exit().next_guest_pc();
                        engine
                            .resolve(&mut arena.pause().unwrap(), &planned)
                            .unwrap();
                        operations += 1;
                        assert_eq!(
                            engine.machine.state, expected,
                            "double={double} ymm={vector_wide} r64={gpr_wide} pattern={pattern}"
                        );
                        assert_eq!(engine.usage().vector_operations, operations);
                        assert_eq!(engine.usage().vector_bytes, 0);
                    }
                });
            }
        }
    }
}

#[test]
fn float_mask_rsp_work_and_operation_refusals_leave_registers_vectors_flags_and_stack_unchanged() {
    for double in [false, true] {
        for vector_wide in [false, true] {
            for gpr_wide in [false, true] {
                for case in 0..3 {
                    let code = encode(double, vector_wide, gpr_wide, if case == 0 { 4 } else { 9 });
                    with_source(Some(&code), |source, arena, stack, mut state| {
                        state.xmm.fill(0x80);
                        state.ymm_upper.fill(0xff);
                        state.gpr[9] = u64::MAX;
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
                            1 => {
                                engine.machine.limits.max_work_units =
                                    engine.machine.usage.work_units
                            }
                            2 => engine.machine.usage.vector_operations = u64::MAX,
                            _ => {}
                        }
                        let usage = engine.usage();
                        let mut stack_before = [0; 8];
                        arena
                            .pause()
                            .unwrap()
                            .read(stack.return_slot, &mut stack_before)
                            .unwrap();
                        engine.machine.phase = Phase::Terminal;
                        assert!(
                            engine
                                .resolve(&mut arena.pause().unwrap(), &planned)
                                .is_err()
                        );
                        assert_eq!(
                            engine.machine.state, state,
                            "double={double} ymm={vector_wide} r64={gpr_wide} case={case}"
                        );
                        assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                        assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
                        assert!(engine.machine.phase == Phase::Terminal);
                        let mut stack_after = [0; 8];
                        arena
                            .pause()
                            .unwrap()
                            .read(stack.return_slot, &mut stack_after)
                            .unwrap();
                        assert_eq!(stack_after, stack_before);
                    });
                }
            }
        }
    }
}
