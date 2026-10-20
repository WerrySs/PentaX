// SPDX-License-Identifier: GPL-2.0-only
//! Original software-profile queries with injected, counted host boundaries.

use super::*;
use std::cell::Cell;

#[test]
fn software_cpu_profile_zero_extends_outputs_and_preserves_complete_unrelated_state() {
    let queries = [
        (0, 0),
        (0, u32::MAX),
        (1, 0),
        (1, u32::MAX),
        (2, 0),
        (3, 1),
        (4, 2),
        (5, 3),
        (6, u32::MAX),
        (7, 0),
        (7, 1),
        (7, u32::MAX),
        (0x8000_0000, 0),
        (0x8000_0000, u32::MAX),
        (0x8000_0001, 0),
        (0x8000_0001, u32::MAX),
        (8, 0),
        (8, 1),
        (0x4000_0000, 0),
        (0x7fff_ffff, u32::MAX),
        (0x8000_0002, 0),
        (u32::MAX, u32::MAX),
    ];
    with_source(
        Some(&[0x0f, 0xa2, 0xc3]),
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
            for available in [false, true] {
                for (leaf, subleaf) in queries {
                    let mut before = state;
                    before.gpr[RAX] = 0xfedc_ba98_0000_0000 | u64::from(leaf);
                    before.gpr[RCX] = 0x1234_5678_0000_0000 | u64::from(subleaf);
                    engine.machine.state = before;
                    engine.machine.phase = Phase::Terminal;
                    let probes = Cell::new(0);
                    let barriers = Cell::new(0);
                    let needs_probe = u32::from(matches!(leaf, 1 | 0x8000_0001));
                    let stop = engine
                        .emulate_cpu_identification_with(
                            &mut arena.pause().unwrap(),
                            &planned,
                            || {
                                probes.set(probes.get() + 1);
                                available
                            },
                            || {
                                assert_eq!(probes.get(), needs_probe);
                                barriers.set(barriers.get() + 1);
                            },
                        )
                        .unwrap();
                    let expected: [u32; 4] = match leaf {
                        0 => [
                            7,
                            u32::from_le_bytes(*b"Pent"),
                            u32::from_le_bytes(*b"uCPU"),
                            u32::from_le_bytes(*b"aXEm"),
                        ],
                        1 => [
                            0,
                            0,
                            0x0080_0000,
                            if available { 0x0600_8110 } else { 0x0600_8100 },
                        ],
                        0x8000_0000 => [0x8000_0001, 0, 0, 0],
                        0x8000_0001 => [
                            0,
                            0,
                            0x20,
                            if available { 0x2800_0000 } else { 0x2000_0000 },
                        ],
                        _ => [0; 4],
                    };
                    let mut after = before;
                    after.guest_pc = planned.exit().next_guest_pc();
                    for (register, word) in [RAX, 3, RCX, RDX].into_iter().zip(expected) {
                        after.gpr[register] = u64::from(word);
                    }
                    operations += 1;
                    assert_eq!(
                        stop,
                        NativeBlockStop::Resume {
                            guest_pc: after.guest_pc
                        }
                    );
                    assert_eq!(probes.get(), needs_probe);
                    assert_eq!(barriers.get(), 1);
                    assert_eq!(
                        engine.machine.state, after,
                        "leaf={leaf:x}, subleaf={subleaf:x}, counter={available}"
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

#[test]
fn cpu_admission_refusals_do_not_probe_serialize_or_publish_partial_state() {
    for leaf in [1, 0x8000_0001] {
        for case in 0..12 {
            let mut code = vec![0x0f, 0xa2, 0xc3];
            match case {
                5 => code.insert(0, 0x64),
                6 => code.insert(0, 0xf3),
                7 => code.insert(0, 0xf2),
                8 => code.insert(0, 0x66),
                9 => code.insert(0, 0x67),
                10 => code.insert(0, 0x40),
                11 => code = vec![0x0f, 0x0b, 0xc3],
                _ => {}
            }
            with_source(Some(&code), |source, arena, stack, mut state| {
                state.gpr[RAX] = leaf;
                state.gpr[3] = u64::MAX;
                state.gpr[RCX] = u64::MAX;
                state.gpr[RDX] = u64::MAX;
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
                    4 => engine.machine.usage.work_units = u64::MAX,
                    _ => {}
                }
                engine.machine.usage.integer_bytes = 37;
                engine.machine.phase = Phase::Terminal;
                let usage = engine.usage();
                let probes = Cell::new(0);
                let barriers = Cell::new(0);
                let result = engine.emulate_cpu_identification_with(
                    &mut arena.pause().unwrap(),
                    &planned,
                    || {
                        probes.set(probes.get() + 1);
                        true
                    },
                    || barriers.set(barriers.get() + 1),
                );
                match case {
                    0 | 1 | 4 => {
                        assert_eq!(result, Err(NativeDispatchError::Budget("integer work")))
                    }
                    2 => assert_eq!(
                        result,
                        Err(NativeDispatchError::Budget("integer operations"))
                    ),
                    3 => assert!(result.is_err()),
                    5..=11 => {
                        assert_eq!(result, Err(NativeDispatchError::UnsupportedCpuInstruction))
                    }
                    _ => unreachable!(),
                }
                assert_eq!((probes.get(), barriers.get()), (0, 0));
                assert_eq!(engine.machine.state, state, "leaf={leaf:x}, case={case}");
                assert!(engine.machine.phase == Phase::Terminal);
                assert_eq!(engine.usage().integer_operations, usage.integer_operations);
                assert_eq!(engine.usage().integer_bytes, usage.integer_bytes);
                assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
            });
        }
    }
}
