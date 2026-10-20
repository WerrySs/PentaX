// SPDX-License-Identifier: GPL-2.0-only
use super::*;

fn host_control() -> u32 {
    let mut value = 0_u32;
    // SAFETY: mandatory SSE writes exactly this live owned u32.
    unsafe {
        std::arch::asm!("stmxcsr [{p}]", p = in(reg) &mut value, options(nostack, preserves_flags));
    }
    value
}

fn code(store: bool, vex: bool) -> Vec<u8> {
    let mut bytes = if vex { vec![0xc5, 0xf8] } else { vec![0x0f] };
    bytes.extend([0xae, if store { 0x18 } else { 0x10 }, 0xc3]);
    bytes
}

#[test]
fn mxcsr_transfers_preserve_other_state_and_use_exact_four_byte_spans() {
    let host = host_control();
    let mut controls = vec![0x1f80_u32, 0x3fa5, 0x5f9a, 0xffbf];
    if mxcsr_control::mask() & 0x40 != 0 {
        controls.push(0x9fc0);
    }
    for store in [false, true] {
        for vex in [false, true] {
            for &control in &controls {
                with_source(
                    Some(&code(store, vex)),
                    |source, arena, stack, mut state| {
                        let address =
                            source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE - 4;
                        arena
                            .pause()
                            .unwrap()
                            .write(address - 4, &[0xa5; 8])
                            .unwrap();
                        if !store {
                            arena
                                .pause()
                                .unwrap()
                                .write(address, &control.to_le_bytes())
                                .unwrap();
                        }
                        state.gpr[RAX] = address;
                        state.mxcsr = if store { control } else { 0x1f80 };
                        state.xmm.fill(0xa5);
                        state.ymm_upper.fill(0x5a);
                        state.rflags |= 0x8d5;
                        let mut expected = state;
                        expected.mxcsr = control;
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
                        let mut actual = [0; 8];
                        arena
                            .pause()
                            .unwrap()
                            .read(address - 4, &mut actual)
                            .unwrap();
                        assert_eq!(&actual[..4], &[0xa5; 4]);
                        assert_eq!(&actual[4..], &control.to_le_bytes());
                        assert_eq!(engine.machine.usage.vector_operations, 1);
                        assert_eq!(engine.machine.usage.vector_bytes, 4);
                        assert_eq!(host_control(), host);
                    },
                );
            }
        }
    }
}

#[test]
fn mxcsr_failed_transfers_never_publish_memory_or_cpu_state() {
    for store in [false, true] {
        for vex in [false, true] {
            for case in 0..6 {
                if store && matches!(case, 3 | 4) {
                    continue;
                }
                with_source(
                    Some(&code(store, vex)),
                    |source, arena, stack, mut state| {
                        let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                        let address = page + PAGE - 4;
                        let input: u32 = match case {
                            3 => 0x8000_1f80,
                            4 => 0x1f00,
                            _ => 0x1f80,
                        };
                        arena
                            .pause()
                            .unwrap()
                            .write(address, &input.to_le_bytes())
                            .unwrap();
                        state.gpr[RAX] = match case {
                            0 => address + 2,
                            5 => u64::MAX - 2,
                            _ => address,
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
                        if case == 1 {
                            arena
                                .protect(page, PAGE, NativePermissions::new(store, false, false))
                                .unwrap();
                        }
                        let used = if case == 2 { u64::MAX - 3 } else { 0 };
                        engine.machine.usage.vector_bytes = used;
                        engine.machine.phase = Phase::Terminal;
                        assert!(
                            engine
                                .resolve(&mut arena.pause().unwrap(), &planned)
                                .is_err()
                        );
                        assert_eq!(engine.machine.state, state);
                        assert_eq!(engine.machine.usage.vector_operations, 0);
                        assert_eq!(engine.machine.usage.vector_bytes, used);
                        assert!(matches!(engine.machine.phase, Phase::Terminal));
                        if case == 1 {
                            arena
                                .protect(page, PAGE, NativePermissions::READ_WRITE)
                                .unwrap();
                        }
                        let mut actual = [0; 4];
                        arena.pause().unwrap().read(address, &mut actual).unwrap();
                        assert_eq!(actual, input.to_le_bytes());
                    },
                );
            }
        }
    }
}

#[test]
fn mxcsr_detected_daz_controls_copied_float_inputs_and_restores_host_state() {
    let host = host_control();
    let supported = mxcsr_control::mask() & 0x40 != 0;
    assert_eq!(mxcsr_control::valid(0x1fc0), supported);
    if supported {
        for (double, one) in [(false, 0x3f80_0000), (true, 0x3ff0_0000_0000_0000)] {
            let (plain, _) =
                float_ops::calculate(float_ops::Operation::Mul, double, 1, one, 0x1f80).unwrap();
            let (daz, _) =
                float_ops::calculate(float_ops::Operation::Mul, double, 1, one, 0x1fc0).unwrap();
            assert_eq!(plain, 1);
            assert_eq!(daz, 0);
            assert_eq!(
                float_ops::compare(false, double, 1, 0, 0x1fc0).unwrap(),
                (0x40, 0x1fc0)
            );
            assert_eq!(host_control(), host);
        }
    }
    assert_eq!(host_control(), host);
}
