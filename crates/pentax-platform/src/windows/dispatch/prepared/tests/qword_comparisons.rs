// SPDX-License-Identifier: GPL-2.0-only
//! Original signed-qword endpoints, alias snapshots and atomic refusals.
use super::*;

fn instruction(form: usize, destination: u8, memory: bool, wide_bit: bool) -> Vec<u8> {
    let mut bytes = if form == 0 {
        vec![0x66, if memory { 0x44 } else { 0x45 }, 0x0f, 0x38, 0x37]
    } else {
        // SRC1=13; W is ignored, L selects the complete 128/256-bit vector.
        vec![
            0xc4,
            if memory { 0x62 } else { 0x42 },
            0x11 | if form == 2 { 4 } else { 0 } | if wide_bit { 0x80 } else { 0 },
            0x37,
        ]
    };
    bytes.push(((destination & 7) << 3) | if memory { 0 } else { 0xc6 });
    bytes
}
fn bytes(words: [i64; 4]) -> [u8; 32] {
    let mut result = [0; 32];
    for (slot, word) in result.chunks_exact_mut(8).zip(words) {
        slot.copy_from_slice(&word.to_le_bytes());
    }
    result
}

#[test]
fn qword_signed_endpoints_full_width_alias_sources_and_all_upper_rules_are_preserved() {
    let cases = [
        (
            [i64::MIN, i64::MAX, -1, 0],
            [0, -1, 0, -1],
            [0, u64::MAX, 0, u64::MAX],
        ),
        (
            [0, -1, 0, -1],
            [i64::MIN, i64::MAX, -1, 0],
            [u64::MAX, 0, u64::MAX, 0],
        ),
        (
            [i64::MIN, i64::MAX, -1, 0],
            [i64::MIN, i64::MAX, -1, 0],
            [0; 4],
        ),
        (
            [0x1_0000_0000, -0x1_0000_0000, 0, i64::MIN + 1],
            [0xffff_ffff, -0x1_0000_0001, -1, i64::MIN],
            [u64::MAX; 4],
        ),
        (
            [-0x1_0000_0000, 0x1_0000_0000, i64::MIN, i64::MAX],
            [0, 0, -1, 0],
            [0, u64::MAX, 0, u64::MAX],
        ),
    ];
    for form in 0..3 {
        let size = if form == 2 { 32 } else { 16 };
        // Distinct output, SRC1 alias, SRC2 alias, and real memory operand.
        let shapes = [(9, false), (13, false), (14, false), (9, true)];
        let mut code = Vec::new();
        for (destination, memory) in shapes {
            for (case, _) in cases.iter().enumerate() {
                code.extend(instruction(form, destination, memory, case % 2 != 0));
            }
        }
        code.push(0xc3);
        assert!(code.len() < 256);
        with_source(Some(&code), |source, arena, stack, mut state| {
            let end = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE;
            // The legacy operand ends exactly on the page boundary. Both VEX
            // widths deliberately use an unaligned complete source span.
            let displacement = if form == 0 { 0 } else { 1 };
            state.gpr[RAX] = end - size as u64 - displacement;
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
            for (destination, memory) in shapes {
                for &(a, b, masks) in &cases {
                    let left = bytes(a);
                    let right = bytes(b);
                    let mut neighbors = [0xa5; 64];
                    let offset = 64 - size - displacement as usize;
                    neighbors[offset..offset + size].copy_from_slice(&right[..size]);
                    arena.pause().unwrap().write(end - 64, &neighbors).unwrap();
                    let mut before = engine.machine.state;
                    before.xmm.fill(0xcc);
                    before.ymm_upper.fill(0xdd);
                    let left_slot = if form == 0 {
                        usize::from(destination) * 16
                    } else {
                        208
                    };
                    before.xmm[left_slot..left_slot + 16].copy_from_slice(&left[..16]);
                    before.ymm_upper[left_slot..left_slot + 16].copy_from_slice(&left[16..]);
                    before.xmm[224..240].copy_from_slice(&right[..16]);
                    before.ymm_upper[224..240].copy_from_slice(&right[16..]);
                    engine.machine.state = before;
                    let planned = block(
                        source,
                        &arena.pause().unwrap(),
                        before.guest_pc,
                        engine.placement(),
                    );
                    let mut output = [0; 32];
                    // Only active lanes receive comparison masks. VEX.128
                    // clears the initialized upper half instead of comparing it.
                    for (slot, mask) in output[..size].chunks_exact_mut(8).zip(masks) {
                        // A destructive legacy DEST==SRC2 compares a vector
                        // with itself, regardless of the originally staged SRC1.
                        let value = if form == 0 && destination == 14 && !memory {
                            0
                        } else {
                            mask
                        };
                        slot.copy_from_slice(&value.to_le_bytes());
                    }
                    let mut after = before;
                    let slot = usize::from(destination) * 16;
                    after.xmm[slot..slot + 16].copy_from_slice(&output[..16]);
                    if form != 0 {
                        after.ymm_upper[slot..slot + 16].copy_from_slice(&output[16..]);
                    }
                    after.guest_pc = planned.exit().next_guest_pc();
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .unwrap();
                    operations += 1;
                    transferred += if memory { size as u64 } else { 0 };
                    assert_eq!(
                        engine.machine.state, after,
                        "form={form} dst={destination} memory={memory}"
                    );
                    assert_eq!(engine.usage().vector_operations, operations);
                    assert_eq!(engine.usage().vector_bytes, transferred);
                    let mut actual = [0; 64];
                    arena.pause().unwrap().read(end - 64, &mut actual).unwrap();
                    assert_eq!(actual, neighbors);
                }
            }
        });
    }
}

#[test]
fn qword_complete_reads_alignment_permissions_prefixes_and_budgets_refuse_atomically() {
    for form in 0..3 {
        let size = if form == 2 { 32 } else { 16 };
        for case in 0..9 {
            let mut code = instruction(form, 9, true, false);
            if case == 7 {
                code.insert(0, 0x64);
            } // Unsupported FS-based vector operand.
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let end = data + PAGE;
                let backing = bytes([i64::MIN, i64::MIN, i64::MIN, i64::MIN]);
                arena
                    .pause()
                    .unwrap()
                    .write(end - size as u64, &backing[..size])
                    .unwrap();
                // Even a predetermined false first lane requires all bytes.
                state.gpr[RAX] = match case {
                    1 => end,
                    2 => end - size as u64 + 1,
                    8 => 0x0000_8000_0000_0000, // Noncanonical complete memory source.
                    _ => end - size as u64,
                };
                state.xmm.fill(0x80);
                state.ymm_upper.fill(0xa5);
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
                    5 => engine.machine.usage.vector_bytes = u64::MAX - size as u64 + 1,
                    6 => engine.machine.usage.vector_operations = u64::MAX,
                    _ => {}
                }
                let usage = engine.usage();
                engine.machine.phase = Phase::Terminal;
                let outcome = engine.resolve(&mut arena.pause().unwrap(), &planned);
                if case == 0 {
                    outcome.unwrap();
                    assert_eq!(engine.usage().vector_operations, 1);
                    assert_eq!(engine.usage().vector_bytes, size as u64);
                } else {
                    match case {
                        2 if form == 0 => assert!(matches!(
                            outcome,
                            Err(NativeDispatchError::UnsupportedVectorInstruction)
                        )),
                        1..=3 | 8 => {
                            assert!(matches!(outcome, Err(NativeDispatchError::Memory(_))))
                        }
                        4..=6 => assert!(matches!(outcome, Err(NativeDispatchError::Budget(_)))),
                        _ => assert!(matches!(
                            outcome,
                            Err(NativeDispatchError::UnsupportedVectorInstruction)
                        )),
                    }
                    assert_eq!(engine.machine.state, state);
                    assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                    assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
                    assert!(engine.machine.phase == Phase::Terminal);
                }
                if case == 3 {
                    arena.protect(data, PAGE, NativePermissions::READ).unwrap();
                }
                let mut actual = [0; 32];
                arena
                    .pause()
                    .unwrap()
                    .read(end - size as u64, &mut actual[..size])
                    .unwrap();
                assert_eq!(actual[..size], backing[..size]);
            });
        }
    }
}
