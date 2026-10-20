// SPDX-License-Identifier: GPL-2.0-only
//! Original checked-state consumers; these tests do not enter the native bridge.

use super::*;

#[derive(Clone, Copy, Debug)]
struct Shape {
    scalar: bool,
    double: bool,
    wide: bool,
}
impl Shape {
    fn element(self) -> usize {
        if self.double { 8 } else { 4 }
    }
    fn vector(self) -> usize {
        if self.wide { 32 } else { 16 }
    }
    fn memory(self) -> usize {
        if self.scalar {
            self.element()
        } else {
            self.vector()
        }
    }
}
const SHAPES: [Shape; 6] = [
    Shape {
        scalar: false,
        double: false,
        wide: false,
    },
    Shape {
        scalar: false,
        double: false,
        wide: true,
    },
    Shape {
        scalar: false,
        double: true,
        wide: false,
    },
    Shape {
        scalar: false,
        double: true,
        wide: true,
    },
    Shape {
        scalar: true,
        double: false,
        wide: false,
    },
    Shape {
        scalar: true,
        double: true,
        wide: false,
    },
];
fn encode(shape: Shape, destination: usize, memory: bool, immediate: u8) -> [u8; 7] {
    let pp = if shape.scalar {
        if shape.double { 3 } else { 2 }
    } else {
        u8::from(shape.double)
    };
    [
        0xc4,
        if destination < 8 { 0xe1 } else { 0x61 },
        0x70 | if shape.wide { 4 } else { 0 } | pp,
        0xc2,
        ((destination as u8 & 7) << 3) | if memory { 0 } else { 0xc2 },
        immediate,
        0xc3,
    ]
}
fn put(vector: &mut [u8; 32], width: usize, lane: usize, value: u64) {
    vector[lane * width..(lane + 1) * width].copy_from_slice(&value.to_le_bytes()[..width]);
}
fn source(state: &mut NativeCpuState, register: usize, bytes: &[u8; 32]) {
    state.xmm[register * 16..register * 16 + 16].copy_from_slice(&bytes[..16]);
    state.ymm_upper[register * 16..register * 16 + 16].copy_from_slice(&bytes[16..]);
}

#[test]
fn mask_compare_all_forms_capture_alias_sources_merge_scalar_and_read_exact_page_tail() {
    for shape in SHAPES {
        for (destination, memory) in [(0, false), (1, false), (2, false), (15, true)] {
            let code = encode(shape, destination, memory, 0xf1); // LT_OQ with all reserved bits set.
            with_source(Some(&code), |declaration, arena, stack, mut state| {
                let width = shape.element();
                let (one, two, negative_one, negative_zero) = if shape.double {
                    (
                        0x3ff0_0000_0000_0000_u64,
                        0x4000_0000_0000_0000,
                        0xbff0_0000_0000_0000,
                        1 << 63,
                    )
                } else {
                    (0x3f80_0000, 0x4000_0000, 0xbf80_0000, 1 << 31)
                };
                let mut left = [0xa5; 32];
                let mut right = [0x5a; 32];
                let lanes = if shape.scalar {
                    1
                } else {
                    shape.vector() / width
                };
                for lane in 0..lanes {
                    let (a, b) = match lane % 4 {
                        0 => (one, two),
                        1 => (two, one),
                        2 => (negative_one, negative_one),
                        _ => (0, negative_zero),
                    };
                    put(&mut left, width, lane, a);
                    put(&mut right, width, lane, b);
                }
                state.xmm.fill(0x33);
                state.ymm_upper.fill(0x77);
                source(&mut state, 1, &left);
                source(&mut state, 2, &right);
                state.rflags = 0x602 | 0x8d5;
                state.mxcsr = 0x1f84;
                let tail = declaration.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                    - shape.memory() as u64;
                arena
                    .pause()
                    .unwrap()
                    .write(tail, &right[..shape.memory()])
                    .unwrap();
                state.gpr[RAX] = tail;
                let mut expected = state;
                let mut output = [0; 32];
                if shape.scalar {
                    output[..16].copy_from_slice(&left[..16]);
                }
                for lane in 0..lanes {
                    output[lane * width..(lane + 1) * width].fill(if lane % 4 == 0 {
                        0xff
                    } else {
                        0
                    });
                }
                source(&mut expected, destination, &output);
                let mut engine = NativeBlockStepper::new(
                    declaration,
                    &mut arena.pause().unwrap(),
                    state,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                let planned = block(
                    declaration,
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
                    "shape={shape:?} dst={destination} memory={memory}"
                );
                assert_eq!(engine.usage().vector_operations, 1);
                assert_eq!(
                    engine.usage().vector_bytes,
                    if memory { shape.memory() as u64 } else { 0 }
                );
                let mut actual = [0; 32];
                arena
                    .pause()
                    .unwrap()
                    .read(tail, &mut actual[..shape.memory()])
                    .unwrap();
                assert_eq!(actual[..shape.memory()], right[..shape.memory()]);
            });
        }
    }
}

#[test]
fn mask_compare_nan_denormal_status_is_per_active_lane_and_preserves_flags_and_sticky_bits() {
    for shape in [SHAPES[1], SHAPES[3], SHAPES[4], SHAPES[5]] {
        for (signaling_nan, immediate) in [(false, 17), (false, 1), (true, 17)] {
            for daz in [false, true] {
                let control = 0xdf84 | if daz { 0x40 } else { 0 };
                if !super::super::mxcsr_control::valid(control) {
                    continue;
                }
                let code = encode(shape, 0, false, immediate);
                with_source(Some(&code), |declaration, arena, stack, mut state| {
                    let width = shape.element();
                    let (one, quiet, signaling) = if shape.double {
                        (
                            0x3ff0_0000_0000_0000_u64,
                            0x7ff8_0000_0000_0123,
                            0x7ff0_0000_0000_0456,
                        )
                    } else {
                        (0x3f80_0000, 0x7fc0_0123, 0x7f80_0456)
                    };
                    let mut left = [0xa5; 32];
                    let mut right = [0x5a; 32];
                    for lane in 0..32 / width {
                        put(&mut left, width, lane, one);
                        put(&mut right, width, lane, one);
                    }
                    put(
                        &mut left,
                        width,
                        0,
                        if signaling_nan { signaling } else { quiet },
                    );
                    put(&mut right, width, 0, 1); // Same-lane denormal loses to NaN.
                    put(&mut left, width, 1, 0);
                    // Zero is less than a positive subnormal; DAZ makes it equal.
                    // This separate packed lane still contributes DE without DAZ.
                    put(&mut right, width, 1, 1);
                    source(&mut state, 1, &left);
                    source(&mut state, 2, &right);
                    state.mxcsr = control;
                    state.rflags = 0x602 | 0x8d5;
                    let mut expected = state;
                    let mut output = [0; 32];
                    if shape.scalar {
                        output[..16].copy_from_slice(&left[..16]);
                        output[..width].fill(0);
                    } else if !daz {
                        output[width..width * 2].fill(0xff);
                    }
                    source(&mut expected, 0, &output);
                    expected.mxcsr |= u32::from(signaling_nan || immediate == 1)
                        | if !shape.scalar && !daz { 2 } else { 0 };
                    let mut engine = NativeBlockStepper::new(
                        declaration,
                        &mut arena.pause().unwrap(),
                        state,
                        stack,
                        &Default::default(),
                    )
                    .unwrap();
                    let planned = block(
                        declaration,
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
                        "shape={shape:?} snan={signaling_nan} predicate={immediate} daz={daz}"
                    );
                });
            }
        }
    }
}

#[test]
fn mask_compare_read_budget_counter_and_unmasked_control_refusals_publish_no_state() {
    for shape in SHAPES {
        for case in 0..6 {
            let code = encode(shape, 0, true, 1);
            with_source(Some(&code), |declaration, arena, stack, mut state| {
                let data = declaration.source_plan().modules()[0].load_bias() + DATA_OFF;
                let tail = data + PAGE - shape.memory() as u64;
                arena
                    .pause()
                    .unwrap()
                    .write(tail, &[0; 32][..shape.memory()])
                    .unwrap();
                state.gpr[RAX] = tail + u64::from(case == 0);
                state.xmm.fill(0xa5);
                state.ymm_upper.fill(0x5a);
                state.rflags = 0x602 | 0x8d5;
                state.mxcsr = 0x1f84;
                let mut engine = NativeBlockStepper::new(
                    declaration,
                    &mut arena.pause().unwrap(),
                    state,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                let planned = block(
                    declaration,
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
                    5 => engine.machine.state.mxcsr = 0x1f04, // Unmasked invalid is outside the admitted profile.
                    _ => {}
                }
                let before = engine.machine.state;
                let usage = engine.usage();
                engine.machine.phase = Phase::Terminal;
                assert!(
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .is_err()
                );
                assert_eq!(engine.machine.state, before, "shape={shape:?} case={case}");
                assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
                assert!(engine.machine.phase == Phase::Terminal);
            });
        }
    }
}
