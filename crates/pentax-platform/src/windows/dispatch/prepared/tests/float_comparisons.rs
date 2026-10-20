// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn scalar_float_comparisons_preserve_vectors_and_read_only_the_scalar_memory_span() {
    for (double, one, two, quiet) in [
        (false, 0x3f80_0000_u64, 0x4000_0000_u64, 0x7fc0_0000_u64),
        (
            true,
            0x3ff0_0000_0000_0000,
            0x4000_0000_0000_0000,
            0x7ff8_0000_0000_0000,
        ),
    ] {
        let width = if double { 8 } else { 4 };
        for ordered in [false, true] {
            for memory in [false, true] {
                for (left, right, flags, invalid) in [
                    (one, two, 1, false),
                    (two, one, 0, false),
                    (one, one, 0x40, false),
                    (one, quiet, 0x45, ordered),
                ] {
                    let code = [
                        0xc5,
                        0xf8 | u8::from(double),
                        if ordered { 0x2f } else { 0x2e },
                        if memory { 0x08 } else { 0xca },
                        0xc3,
                    ];
                    with_source(Some(&code), |source, arena, stack, mut state| {
                        state.xmm.fill(0xa5);
                        state.ymm_upper.fill(0x5a);
                        state.xmm[16..24].copy_from_slice(&left.to_le_bytes());
                        state.xmm[32..40].copy_from_slice(&right.to_le_bytes());
                        state.rflags |= 0x8d5;
                        state.mxcsr |= 4;
                        let tail = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                            - width as u64;
                        arena
                            .pause()
                            .unwrap()
                            .write(tail, &right.to_le_bytes()[..width])
                            .unwrap();
                        state.gpr[RAX] = tail;
                        let mut expected = state;
                        expected.rflags = (state.rflags & !0x8d5) | flags;
                        expected.mxcsr |= u32::from(invalid);
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
fn scalar_float_comparisons_refuse_partial_reads_and_budgets_without_publishing_flags() {
    for (prefix, width) in [(0xf8, 4_usize), (0xf9, 8)] {
        for case in 0..5 {
            let code = [0xc5, prefix, 0x2f, 0x00, 0xc3];
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let tail = data + PAGE - width as u64;
                arena
                    .pause()
                    .unwrap()
                    .write(tail, &[0xff; 8][..width])
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
