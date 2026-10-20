// SPDX-License-Identifier: GPL-2.0-only
//! Original injected counter samples; these tests never query the host clock.

use super::*;
use crate::time_counter::{CounterError, CounterSample};
use std::cell::Cell;

fn encoding(serial: bool) -> Vec<u8> {
    if serial {
        vec![0x0f, 0x01, 0xf9, 0xc3]
    } else {
        vec![0x0f, 0x31, 0xc3]
    }
}

#[test]
fn timestamp_samples_split_boundaries_zero_extend_and_preserve_all_unrelated_state() {
    for serial in [false, true] {
        with_source(
            Some(&encoding(serial)),
            |source, arena, stack, mut state| {
                for (index, register) in state.gpr.iter_mut().enumerate() {
                    if index != RSP {
                        *register = 0xa5a5_9876_0000_0000 | index as u64;
                    }
                }
                state.rflags = 0x602 | 0x8d5;
                state.mxcsr = 0x1f84;
                state.xmm.fill(0x97);
                state.ymm_upper.fill(0xa6);
                state.fs_base = Some(0x1234);
                state.gs_base = Some(0x5678);
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
                engine.machine.usage.integer_bytes = 37;
                engine.machine.usage.integer_operations = 9;
                let mut operations = 9;
                for ticks in [
                    0,
                    1,
                    0xffff_ffff,
                    0x1_0000_0000,
                    0x8000_0000_ffff_ffff,
                    u64::MAX,
                ] {
                    for frequency in [1, u64::MAX] {
                        engine.machine.state = state;
                        engine.machine.phase = Phase::Terminal;
                        let calls = Cell::new(0);
                        let stop = engine
                            .emulate_timestamp_with(&mut arena.pause().unwrap(), &planned, || {
                                calls.set(calls.get() + 1);
                                Ok(CounterSample { ticks, frequency })
                            })
                            .unwrap();
                        let mut after = state;
                        after.guest_pc = planned.exit().next_guest_pc();
                        after.gpr[RAX] = u64::from(ticks as u32);
                        after.gpr[RDX] = ticks >> 32;
                        if serial {
                            after.gpr[RCX] = 0;
                        }
                        operations += 1;
                        assert_eq!(calls.get(), 1);
                        assert_eq!(
                            stop,
                            NativeBlockStop::Resume {
                                guest_pc: after.guest_pc
                            }
                        );
                        assert_eq!(
                            engine.machine.state, after,
                            "serial={serial} ticks={ticks:x} frequency={frequency}"
                        );
                        assert!(engine.machine.phase == Phase::Ready);
                        assert_eq!(engine.usage().integer_operations, operations);
                        assert_eq!(engine.usage().integer_bytes, 37);
                        assert_eq!(engine.usage().vector_operations, 0);
                        assert_eq!(engine.usage().vector_bytes, 0);
                    }
                }
            },
        );
    }
}

#[test]
fn timestamp_admission_and_sample_failures_never_publish_state_or_success_counters() {
    for serial in [false, true] {
        for case in 0..11 {
            let mut code = encoding(serial);
            match case {
                7 => code.insert(0, 0x64),
                8 => code.insert(0, 0xf3),
                9 => code.insert(0, 0xf2),
                10 => code = vec![0x0f, 0x0b, 0xc3], // Wrong zero-operand instruction.
                _ => {}
            }
            with_source(Some(&code), |source, arena, stack, mut state| {
                state.gpr[RAX] = u64::MAX;
                state.gpr[RDX] = u64::MAX;
                state.gpr[RCX] = 0xa5a5_9876_1234_5678;
                state.rflags = 0x602 | 0x8d5;
                state.mxcsr = 0x1f84;
                state.xmm.fill(0x97);
                state.ymm_upper.fill(0xa6);
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
                    0 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                    1 => engine.machine.target_visits = u64::MAX,
                    2 => engine.machine.usage.integer_operations = u64::MAX,
                    3 => {
                        let bias = source.source_plan().modules()[0].load_bias();
                        arena
                            .protect(bias, PAGE, NativePermissions::READ_WRITE)
                            .unwrap();
                    }
                    _ => {}
                }
                engine.machine.usage.integer_bytes = 37;
                engine.machine.phase = Phase::Terminal;
                let usage = engine.usage();
                let calls = Cell::new(0);
                let outcome =
                    engine.emulate_timestamp_with(&mut arena.pause().unwrap(), &planned, || {
                        calls.set(calls.get() + 1);
                        match case {
                            4 => Err(CounterError::Unsupported),
                            5 => Err(CounterError::Calibration),
                            6 => Ok(CounterSample {
                                ticks: u64::MAX,
                                frequency: 0,
                            }),
                            _ => Ok(CounterSample {
                                ticks: 0x1234_5678_9abc_def0,
                                frequency: 1,
                            }),
                        }
                    });
                match case {
                    0 | 1 => assert_eq!(outcome, Err(NativeDispatchError::Budget("integer work"))),
                    2 => assert_eq!(
                        outcome,
                        Err(NativeDispatchError::Budget("integer operations"))
                    ),
                    3 => assert!(outcome.is_err()),
                    4 => assert_eq!(
                        outcome,
                        Err(NativeDispatchError::TimeCounter(CounterError::Unsupported))
                    ),
                    5 | 6 => assert_eq!(
                        outcome,
                        Err(NativeDispatchError::TimeCounter(CounterError::Calibration))
                    ),
                    7..=10 => assert_eq!(
                        outcome,
                        Err(NativeDispatchError::UnsupportedIntegerInstruction)
                    ),
                    _ => unreachable!(),
                }
                assert_eq!(calls.get(), u32::from(matches!(case, 4..=6)));
                assert_eq!(engine.machine.state, state, "serial={serial} case={case}");
                assert!(engine.machine.phase == Phase::Terminal);
                assert_eq!(engine.usage().integer_operations, usage.integer_operations);
                assert_eq!(engine.usage().integer_bytes, usage.integer_bytes);
                assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
            });
        }
    }
}
