// SPDX-License-Identifier: GPL-2.0-only
//! Original packed-conversion consumers through checked register/memory state.

use super::*;

#[derive(Clone, Copy, Debug)]
struct Spec {
    kind: u8,
    wide: bool,
}
impl Spec {
    fn input_element(self) -> usize {
        if matches!(self.kind, 2 | 3) { 8 } else { 4 }
    }
    fn output_element(self) -> usize {
        if self.kind == 5 { 8 } else { 4 }
    }
    fn lanes(self) -> usize {
        if matches!(self.kind, 2 | 3 | 5) {
            if self.wide { 4 } else { 2 }
        } else if self.wide {
            8
        } else {
            4
        }
    }
    fn input_bytes(self) -> usize {
        self.input_element() * self.lanes()
    }
    fn encode(self, destination: usize, memory: bool) -> [u8; 6] {
        let (opcode, pp) = match self.kind {
            0 => (0x5b, 1),
            1 => (0x5b, 2),
            2 => (0xe6, 3),
            3 => (0xe6, 1),
            4 => (0x5b, 0),
            _ => (0xe6, 2),
        };
        [
            0xc4,
            (if destination < 8 { 0x80 } else { 0 }) | 0x41 | if memory { 0x20 } else { 0 },
            0x78 | pp | if self.wide { 4 } else { 0 },
            opcode,
            ((destination as u8 & 7) << 3) | if memory { 0 } else { 0xc7 },
            0xc3,
        ]
    }
}
fn put(bytes: &mut [u8; 32], width: usize, lane: usize, value: u64) {
    bytes[lane * width..(lane + 1) * width].copy_from_slice(&value.to_le_bytes()[..width]);
}
fn vectors(state: &mut NativeCpuState, register: usize, bytes: &[u8; 32]) {
    state.xmm[register * 16..register * 16 + 16].copy_from_slice(&bytes[..16]);
    state.ymm_upper[register * 16..register * 16 + 16].copy_from_slice(&bytes[16..]);
}
fn example(spec: Spec) -> ([u8; 32], [u8; 32], u32) {
    let (inputs, outputs, status): ([u64; 4], [u64; 4], u32) = match spec.kind {
        0 => (
            [0x3fc0_0000, 0xc020_0000, 0x4040_0000, 0x8000_0000],
            [2, 0xffff_fffe, 3, 0],
            32,
        ),
        1 => (
            [0x3fc0_0000, 0xc020_0000, 0x4040_0000, 0x8000_0000],
            [1, 0xffff_fffe, 3, 0],
            32,
        ),
        2 => (
            [
                0x3ff8_0000_0000_0000,
                0xc004_0000_0000_0000,
                0x4008_0000_0000_0000,
                1 << 63,
            ],
            [2, 0xffff_fffe, 3, 0],
            32,
        ),
        3 => (
            [
                0x3ff8_0000_0000_0000,
                0xc004_0000_0000_0000,
                0x4008_0000_0000_0000,
                1 << 63,
            ],
            [1, 0xffff_fffe, 3, 0],
            32,
        ),
        4 => (
            [0x0100_0001, 0xfeff_ffff, 3, 0],
            [0x4b80_0000, 0xcb80_0000, 0x4040_0000, 0],
            32,
        ),
        _ => (
            [0x0100_0001, 0xfeff_ffff, 3, 0],
            [
                0x4170_0000_1000_0000,
                0xc170_0000_1000_0000,
                0x4008_0000_0000_0000,
                0,
            ],
            0,
        ),
    };
    let mut input = [0xa5; 32];
    let mut output = [0; 32];
    for lane in 0..spec.lanes() {
        put(&mut input, spec.input_element(), lane, inputs[lane % 4]);
        put(&mut output, spec.output_element(), lane, outputs[lane % 4]);
    }
    (input, output, status)
}

#[test]
fn packed_conversion_all_forms_capture_aliases_read_exact_tails_and_zero_unused_destinations() {
    for kind in 0..6 {
        for wide in [false, true] {
            let spec = Spec { kind, wide };
            for (destination, memory) in [(0, false), (15, false), (9, true)] {
                let code = spec.encode(destination, memory);
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let (input, output, status) = example(spec);
                    state.xmm.fill(0x33);
                    state.ymm_upper.fill(0x77);
                    vectors(&mut state, 15, &input);
                    state.mxcsr = 0x1f84;
                    state.rflags = 0x602 | 0x8d5;
                    let tail = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                        - spec.input_bytes() as u64;
                    arena
                        .pause()
                        .unwrap()
                        .write(tail, &input[..spec.input_bytes()])
                        .unwrap();
                    state.gpr[RAX] = tail;
                    let mut expected = state;
                    vectors(&mut expected, destination, &output);
                    expected.mxcsr |= status;
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
                    assert_eq!(
                        engine.machine.state, expected,
                        "spec={spec:?} dst={destination} memory={memory}"
                    );
                    assert_eq!(engine.usage().vector_operations, 1);
                    assert_eq!(
                        engine.usage().vector_bytes,
                        if memory { spec.input_bytes() as u64 } else { 0 }
                    );
                    let mut unchanged = [0; 32];
                    arena
                        .pause()
                        .unwrap()
                        .read(tail, &mut unchanged[..spec.input_bytes()])
                        .unwrap();
                    assert_eq!(unchanged[..spec.input_bytes()], input[..spec.input_bytes()]);
                });
            }
        }
    }
}

#[test]
fn packed_conversion_mixed_invalid_inexact_and_daz_are_aggregated_only_from_active_lanes() {
    for kind in 0..4 {
        let spec = Spec {
            kind,
            wide: kind & 1 != 0,
        };
        for daz in [false, true] {
            let control = 0x5f84 | if daz { 0x40 } else { 0 }; // Round upward.
            if !super::super::mxcsr_control::valid(control) {
                continue;
            }
            let code = spec.encode(0, false);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let mut input = [0xa5; 32];
                let mut output = [0; 32];
                let inputs = if spec.input_element() == 8 {
                    [
                        0x7ff8_0000_0000_0123,
                        0x3ff4_0000_0000_0000,
                        1,
                        0xbff4_0000_0000_0000,
                    ]
                } else {
                    [0x7fc0_0123, 0x3fa0_0000, 1, 0xbfa0_0000]
                };
                for lane in 0..spec.lanes() {
                    put(&mut input, spec.input_element(), lane, inputs[lane % 4]);
                    let value = match lane % 4 {
                        0 => 0x8000_0000,
                        1 => {
                            if kind & 1 != 0 {
                                1
                            } else {
                                2
                            }
                        }
                        2 => u64::from(!daz && kind & 1 == 0),
                        _ => u64::from(u32::MAX),
                    };
                    put(&mut output, 4, lane, value);
                }
                vectors(&mut state, 15, &input);
                state.mxcsr = control;
                state.rflags = 0x602 | 0x8d5;
                let mut expected = state;
                vectors(&mut expected, 0, &output);
                expected.mxcsr |= 33; // IE in a NaN lane, PE in another; never DE.
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
                assert_eq!(engine.machine.state, expected, "spec={spec:?} daz={daz}");
            });
        }
    }
}

#[test]
fn packed_conversion_memory_work_and_counter_refusals_publish_no_partial_results() {
    for kind in 0..6 {
        for wide in [false, true] {
            let spec = Spec { kind, wide };
            for case in 0..5 {
                let code = spec.encode(9, true);
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    let tail = data + PAGE - spec.input_bytes() as u64;
                    let (input, _, _) = example(spec);
                    arena
                        .pause()
                        .unwrap()
                        .write(tail, &input[..spec.input_bytes()])
                        .unwrap();
                    state.gpr[RAX] = tail + u64::from(case == 0);
                    state.xmm.fill(0xa5);
                    state.ymm_upper.fill(0x5a);
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
                    match case {
                        1 => arena
                            .protect(data, PAGE, NativePermissions::new(false, false, false))
                            .unwrap(),
                        2 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                        3 => engine.machine.usage.vector_bytes = u64::MAX,
                        4 => engine.machine.usage.vector_operations = u64::MAX,
                        _ => {}
                    }
                    let usage = engine.usage();
                    engine.machine.phase = Phase::Terminal;
                    let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                    match case {
                        0 | 1 => assert!(matches!(result, Err(NativeDispatchError::Memory(_)))),
                        2 => assert_eq!(result, Err(NativeDispatchError::Budget("vector work"))),
                        3 => assert_eq!(result, Err(NativeDispatchError::Budget("vector bytes"))),
                        4 => assert_eq!(
                            result,
                            Err(NativeDispatchError::Budget("vector operations"))
                        ),
                        _ => unreachable!(),
                    }
                    assert_eq!(engine.machine.state, state, "spec={spec:?} case={case}");
                    assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                    assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
                    assert!(engine.machine.phase == Phase::Terminal);
                });
            }
        }
    }
}
