// SPDX-License-Identifier: GPL-2.0-only
//! Actual paused backing, full state comparisons and late refusal boundaries.
use super::*;

fn encoding(form: usize, destination: u8, memory: bool) -> Vec<u8> {
    let (opcode, scalar, wide) = match form {
        0 | 4 => (8, false, false),
        1 | 6 => (9, false, false),
        2 | 8 => (10, true, false),
        3 | 9 => (11, true, false),
        5 => (8, false, true),
        7 => (9, false, true),
        _ => unreachable!(),
    };
    let mut code = if form < 4 {
        vec![0x66, 0x0f, 0x3a, opcode]
    } else {
        vec![
            0xc4,
            0xe3,
            if scalar {
                0x61
            } else if wide {
                0x7d
            } else {
                0x79
            },
            opcode,
        ]
    };
    code.extend([(destination << 3) | if memory { 0 } else { 0xc2 }, 0x84]);
    code
}
fn operands(double: bool) -> ([u8; 32], [u8; 32]) {
    let mut input = [0; 32];
    let mut output = [0; 32];
    if double {
        for (index, (a, b)) in [
            (0x3ff8_0000_0000_0000_u64, 0x3ff0_0000_0000_0000_u64),
            (0xc004_0000_0000_0000, 0xc008_0000_0000_0000),
            (0x3fe0_0000_0000_0000, 0),
            (0xbfe0_0000_0000_0000, 0xbff0_0000_0000_0000),
        ]
        .into_iter()
        .enumerate()
        {
            input[index * 8..index * 8 + 8].copy_from_slice(&a.to_le_bytes());
            output[index * 8..index * 8 + 8].copy_from_slice(&b.to_le_bytes());
        }
    } else {
        for (index, (a, b)) in [
            (0x3fc0_0000_u32, 0x3f80_0000_u32),
            (0xc020_0000, 0xc040_0000),
            (0x3f00_0000, 0),
            (0xbf00_0000, 0xbf80_0000),
            (0x4020_0000, 0x4000_0000),
            (0x4060_0000, 0x4040_0000),
            (0x3fa0_0000, 0x3f80_0000),
            (0xbfa0_0000, 0xc000_0000),
        ]
        .into_iter()
        .enumerate()
        {
            input[index * 4..index * 4 + 4].copy_from_slice(&a.to_le_bytes());
            output[index * 4..index * 4 + 4].copy_from_slice(&b.to_le_bytes());
        }
    }
    (input, output)
}
fn layout(form: usize) -> rounding::Form {
    rounding::Form {
        vector_bytes: if matches!(form, 5 | 7) { 32 } else { 16 },
        element_bytes: if matches!(form, 1 | 3 | 6 | 7 | 9) {
            8
        } else {
            4
        },
        scalar: matches!(form, 2 | 3 | 8 | 9),
        legacy: form < 4,
    }
}

#[test]
fn all_round_forms_snapshot_aliases_merge_scalar_lanes_and_preserve_complete_state() {
    for form in 0..10 {
        let mut code = Vec::new();
        for (destination, memory) in [(1, false), (2, false), (3, false), (1, true)] {
            code.extend(encoding(form, destination, memory));
        }
        code.push(0xc3);
        with_source(Some(&code), |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            let end = data + PAGE;
            let layout = layout(form);
            let width = if layout.scalar {
                layout.element_bytes
            } else {
                layout.vector_bytes
            };
            // Scalar memory ends exactly at an unmapped page: no extra XMM read.
            let displacement = usize::from(!layout.legacy && !layout.scalar);
            state.gpr[RAX] = end - width as u64 - displacement as u64;
            state.mxcsr = 0x3f84;
            state.rflags = 0x602 | 0x8d5;
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let (input, expected) = operands(layout.element_bytes == 8);
            let mut transfers = 0;
            for (index, (destination, memory)) in [(1, false), (2, false), (3, false), (1, true)]
                .into_iter()
                .enumerate()
            {
                arena
                    .pause()
                    .unwrap()
                    .write(state.gpr[RAX], &input[..width])
                    .unwrap();
                let mut before = engine.machine.state;
                before.xmm.fill(0xa5);
                before.ymm_upper.fill(0x9b);
                before.mxcsr = 0x3f84;
                // Inactive scalar merge lanes contain sNaNs. They are copied,
                // never rounded or allowed to raise an invalid exception.
                if layout.element_bytes == 8 {
                    for lane in before.xmm.chunks_exact_mut(8) {
                        lane.copy_from_slice(&0x7ff0_0000_0000_0001_u64.to_le_bytes());
                    }
                } else {
                    for lane in before.xmm.chunks_exact_mut(4) {
                        lane.copy_from_slice(&0x7f80_0001_u32.to_le_bytes());
                    }
                }
                before.xmm[32..48].copy_from_slice(&input[..16]);
                before.ymm_upper[32..48].copy_from_slice(&input[16..]);
                engine.machine.state = before;
                let planned = block(
                    source,
                    &arena.pause().unwrap(),
                    before.guest_pc,
                    engine.placement(),
                );
                let mut after = before;
                let slot = usize::from(destination) * 16;
                if layout.scalar && !layout.legacy {
                    after.xmm[slot..slot + 16].copy_from_slice(&before.xmm[48..64]);
                }
                after.xmm[slot..slot + width.min(16)].copy_from_slice(&expected[..width.min(16)]);
                if width == 32 {
                    after.ymm_upper[slot..slot + 16].copy_from_slice(&expected[16..]);
                }
                // The short forms clear exactly the upper half.
                if !layout.legacy && width != 32 {
                    after.ymm_upper[slot..slot + 16].fill(0);
                }
                after.mxcsr |= 0x20;
                after.guest_pc = planned.exit().next_guest_pc();
                engine
                    .resolve(&mut arena.pause().unwrap(), &planned)
                    .unwrap();
                assert_eq!(
                    engine.machine.state, after,
                    "form={form} dst={destination} mem={memory}"
                );
                transfers += if memory { width as u64 } else { 0 };
                assert_eq!(engine.usage().vector_operations, index as u64 + 1);
                assert_eq!(engine.usage().vector_bytes, transfers);
                let mut unchanged = [0; 32];
                arena
                    .pause()
                    .unwrap()
                    .read(state.gpr[RAX], &mut unchanged[..width])
                    .unwrap();
                assert_eq!(&unchanged[..width], &input[..width]);
            }
        });
    }
}

#[test]
fn rounding_full_memory_control_prefix_and_budget_refusals_publish_no_state() {
    for form in [0, 2, 3, 4, 5, 7, 8, 9] {
        for case in 0..8 {
            let mut code = encoding(form, 1, true);
            if case == 7 {
                code.insert(0, 0x64);
            }
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let layout = layout(form);
                let width = if layout.scalar {
                    layout.element_bytes
                } else {
                    layout.vector_bytes
                };
                state.gpr[RAX] = if case == 0 {
                    data + PAGE
                } else if case == 1 {
                    data + PAGE - width as u64 + 1
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
                match case {
                    2 => arena
                        .protect(data, PAGE, NativePermissions::new(false, false, false))
                        .unwrap(),
                    3 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                    4 => engine.machine.usage.vector_bytes = u64::MAX,
                    5 => engine.machine.usage.vector_operations = u64::MAX,
                    6 => engine.machine.state.mxcsr &= !0x80,
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
                assert_eq!(engine.machine.state, before);
                assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
                assert!(engine.machine.phase == Phase::Terminal);
            });
        }
    }
}
