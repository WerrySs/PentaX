// SPDX-License-Identifier: GPL-2.0-only
//! Original unsigned reductions with first-index ties and exact source reads.

use super::*;

fn instruction(vex: bool, destination: u8, memory: bool, wide_bit: bool) -> Vec<u8> {
    let mut bytes = if vex {
        vec![
            0xc4,
            if memory { 0x62 } else { 0x42 },
            if wide_bit { 0xf9 } else { 0x79 },
            0x41,
        ]
    } else {
        vec![0x66, if memory { 0x44 } else { 0x45 }, 0x0f, 0x38, 0x41]
    };
    bytes.push(((destination & 7) << 3) | if memory { 0 } else { 0xc5 });
    bytes
}
fn cases() -> Vec<([u16; 8], u16, u16)> {
    let mut cases = vec![
        ([0xffff; 8], 0xffff, 0),
        (
            [
                0x8000, 0x8001, 0xffff, 0x9000, 0xfffe, 0xbfff, 0xefff, 0x8123,
            ],
            0x8000,
            0,
        ),
        (
            [
                0xffff, 0x8000, 0x7fff, 0xffff, 0x8001, 0x7ffe, 0xfffe, 0x8123,
            ],
            0x7ffe,
            5,
        ),
        ([0x7fff; 8], 0x7fff, 0),
        ([0xffff, 1, 0xffff, 0, 0, 0xffff, 0xffff, 0], 0, 3),
    ];
    for index in 0..8 {
        let mut values = [0xffff; 8];
        values[index] = 0;
        cases.push((values, 0, index as u16));
    }
    cases
}
fn bytes(values: [u16; 8]) -> [u8; 16] {
    let mut output = [0; 16];
    for (slot, value) in output.chunks_exact_mut(2).zip(values) {
        slot.copy_from_slice(&value.to_le_bytes());
    }
    output
}
fn result(minimum: u16, index: u16) -> [u8; 16] {
    let mut output = [0; 16];
    output[..2].copy_from_slice(&minimum.to_le_bytes());
    output[2..4].copy_from_slice(&index.to_le_bytes());
    output
}

#[test]
fn minimum_position_unsigned_order_first_ties_aliases_and_legacy_vex_upper_rules() {
    let cases = cases();
    for vex in [false, true] {
        let shapes = [(13, false, false), (9, false, vex), (9, true, false)];
        let mut code = Vec::new();
        for (destination, memory, wide_bit) in shapes {
            for _ in &cases {
                code.extend(instruction(vex, destination, memory, wide_bit));
            }
        }
        code.push(0xc3);
        // At most235 bytes: every remaining planning window is below the
        // default source-byte cap without changing the common block helper.
        with_source(Some(&code), |source, arena, stack, mut state| {
            let end = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE;
            state.gpr[RAX] = end - if vex { 17 } else { 16 };
            state.xmm.fill(0xcc);
            state.ymm_upper.fill(0xdd);
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
            for (destination, memory, _) in shapes {
                for &(values, minimum, index) in &cases {
                    let input = bytes(values);
                    let mut neighbors = [0xa5; 32];
                    let offset = if vex { 15 } else { 16 };
                    neighbors[offset..offset + 16].copy_from_slice(&input);
                    arena.pause().unwrap().write(end - 32, &neighbors).unwrap();
                    let mut before = engine.machine.state;
                    let slot = usize::from(destination) * 16;
                    before.xmm[slot..slot + 16].fill(0xee);
                    before.ymm_upper[slot..slot + 16].fill(0xdd);
                    before.xmm[208..224].copy_from_slice(&input);
                    before.ymm_upper[208..224].fill(0x97);
                    engine.machine.state = before;
                    let planned = block(
                        source,
                        &arena.pause().unwrap(),
                        before.guest_pc,
                        engine.placement(),
                    );
                    let mut after = before;
                    after.guest_pc = planned.exit().next_guest_pc();
                    after.xmm[slot..slot + 16].copy_from_slice(&result(minimum, index));
                    if vex {
                        after.ymm_upper[slot..slot + 16].fill(0);
                    }
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .unwrap();
                    operations += 1;
                    transferred += if memory { 16 } else { 0 };
                    assert_eq!(
                        engine.machine.state, after,
                        "vex={vex} dst={destination} mem={memory} values={values:?}"
                    );
                    assert_eq!(engine.usage().vector_operations, operations);
                    assert_eq!(engine.usage().vector_bytes, transferred);
                    let mut actual = [0; 32];
                    arena.pause().unwrap().read(end - 32, &mut actual).unwrap();
                    assert_eq!(actual, neighbors);
                }
            }
        });
    }
}

#[test]
fn minimum_position_requires_all_words_and_legacy_alignment_before_atomic_publication() {
    for vex in [false, true] {
        for case in 0..7 {
            let mut code = instruction(vex, 9, true, false);
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let end = data + PAGE;
                // Zero in the first word cannot permit a shortened read.
                let backing = bytes([0, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff]);
                arena.pause().unwrap().write(end - 16, &backing).unwrap();
                state.gpr[RAX] = match case {
                    1 => end,
                    2 => end - 15,
                    _ => end - 16,
                };
                state.xmm.fill(0xcc);
                state.ymm_upper.fill(0xdd);
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
                    3 => arena
                        .protect(data, PAGE, NativePermissions::new(false, false, false))
                        .unwrap(),
                    4 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                    5 => engine.machine.usage.vector_bytes = u64::MAX - 15,
                    6 => engine.machine.usage.vector_operations = u64::MAX,
                    _ => {}
                }
                let usage = engine.usage();
                engine.machine.phase = Phase::Terminal;
                let outcome = engine.resolve(&mut arena.pause().unwrap(), &planned);
                let mut after = state;
                if case == 0 {
                    outcome.unwrap();
                    after.guest_pc = planned.exit().next_guest_pc();
                    after.xmm[144..160].copy_from_slice(&result(0, 0));
                    if vex {
                        after.ymm_upper[144..160].fill(0);
                    }
                    assert_eq!(engine.usage().vector_operations, 1);
                    assert_eq!(engine.usage().vector_bytes, 16);
                } else {
                    match case {
                        2 if !vex => assert!(matches!(
                            outcome,
                            Err(NativeDispatchError::UnsupportedVectorInstruction)
                        )),
                        1..=3 => assert!(matches!(outcome, Err(NativeDispatchError::Memory(_)))),
                        4..=6 => assert!(matches!(outcome, Err(NativeDispatchError::Budget(_)))),
                        _ => unreachable!(),
                    }
                    assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                    assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
                    assert!(engine.machine.phase == Phase::Terminal);
                }
                assert_eq!(engine.machine.state, after);
                if case == 3 {
                    arena.protect(data, PAGE, NativePermissions::READ).unwrap();
                }
                let mut actual = [0; 16];
                arena.pause().unwrap().read(end - 16, &mut actual).unwrap();
                assert_eq!(actual, backing);
            });
        }
    }
}
