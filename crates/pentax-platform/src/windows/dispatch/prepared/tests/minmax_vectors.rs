// SPDX-License-Identifier: GPL-2.0-only
use super::*;

// VEX map/opcode, element bytes, signed comparison, maximum selection.
const FORMS: [(u8, u8, usize, bool, bool); 12] = [
    (2, 0x38, 1, true, false),
    (1, 0xea, 2, true, false),
    (2, 0x39, 4, true, false),
    (1, 0xda, 1, false, false),
    (2, 0x3a, 2, false, false),
    (2, 0x3b, 4, false, false),
    (2, 0x3c, 1, true, true),
    (1, 0xee, 2, true, true),
    (2, 0x3d, 4, true, true),
    (1, 0xde, 1, false, true),
    (2, 0x3e, 2, false, true),
    (2, 0x3f, 4, false, true),
];
fn number(bytes: &[u8], signed: bool) -> i64 {
    match (bytes.len(), signed) {
        (1, true) => i64::from(bytes[0] as i8),
        (2, true) => i64::from(i16::from_le_bytes(bytes.try_into().unwrap())),
        (4, true) => i64::from(i32::from_le_bytes(bytes.try_into().unwrap())),
        (_, false) => bytes
            .iter()
            .rev()
            .fold(0, |value, byte| value * 256 + i64::from(*byte)),
        _ => unreachable!(),
    }
}

#[test]
fn vector_minmax_uses_signed_or_unsigned_element_order_and_preserves_aliases_flags_and_upper_rules()
{
    for (map, opcode, width, signed, maximum) in FORMS {
        for wide in [false, true] {
            for destination in [1_u8, 2, 15] {
                for memory in [false, true] {
                    let code = [
                        0xc4,
                        if destination == 15 { 0x60 } else { 0xe0 } | map,
                        0x71 | if wide { 4 } else { 0 },
                        opcode,
                        (destination & 7) << 3 | if memory { 0 } else { 0xc2 },
                        0xc3,
                    ];
                    with_source(Some(&code), |source, arena, stack, mut state| {
                        let size = if wide { 32 } else { 16 };
                        let values = [
                            0_u32,
                            u32::MAX,
                            0x8000_0000,
                            0x7fff_ffff,
                            0x8000_7f80,
                            0xff00_0001,
                            0x0123_4567,
                            0,
                        ];
                        let mut left = [0; 32];
                        let mut right = [0; 32];
                        for (index, value) in values.into_iter().enumerate() {
                            left[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
                            right[index * 4..index * 4 + 4]
                                .copy_from_slice(&values[(index + 3) % 8].to_le_bytes());
                        }
                        let mut output = [0; 32];
                        for at in (0..size).step_by(width) {
                            let a = number(&left[at..at + width], signed);
                            let b = number(&right[at..at + width], signed);
                            let expected = if maximum { a.max(b) } else { a.min(b) };
                            output[at..at + width]
                                .copy_from_slice(&expected.to_le_bytes()[..width]);
                        }
                        let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                        let address = page + PAGE - size as u64;
                        arena
                            .pause()
                            .unwrap()
                            .write(address, &right[..size])
                            .unwrap();
                        state.gpr[RAX] = address;
                        state.xmm[16..32].copy_from_slice(&left[..16]);
                        state.ymm_upper[16..32].copy_from_slice(&left[16..]);
                        state.xmm[32..48].copy_from_slice(&right[..16]);
                        state.ymm_upper[32..48].copy_from_slice(&right[16..]);
                        state.rflags |= 0x8d5;
                        state.mxcsr |= 0x25;
                        let mut expected = state;
                        let at = usize::from(destination) * 16;
                        expected.xmm[at..at + 16].copy_from_slice(&output[..16]);
                        expected.ymm_upper[at..at + 16].copy_from_slice(&output[16..]);
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
                        assert_eq!(engine.machine.usage.vector_operations, 1);
                        assert_eq!(
                            engine.machine.usage.vector_bytes,
                            if memory { size as u64 } else { 0 }
                        );
                    });
                }
            }
        }
    }
}

#[test]
fn vector_minmax_requires_the_full_memory_operand_and_refuses_without_publishing_cpu_state() {
    for wide in [false, true] {
        for case in 0..3 {
            let code = [
                0xc4,
                0xe2,
                0x71 | if wide { 4 } else { 0 },
                0x3b,
                0x08,
                0xc3,
            ];
            with_source(Some(&code), |source, arena, stack, mut state| {
                let size = if wide { 32 } else { 16 };
                let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                state.gpr[RAX] = page + PAGE - size + if case == 0 { 1 } else { 0 };
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
                        .protect(page, PAGE, NativePermissions::new(false, false, false))
                        .unwrap();
                }
                if case == 2 {
                    engine.machine.usage.vector_bytes = u64::MAX - size + 1;
                }
                engine.machine.phase = Phase::Terminal;
                let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                if case == 2 {
                    assert!(matches!(
                        result,
                        Err(NativeDispatchError::Budget("vector bytes"))
                    ));
                } else {
                    assert!(matches!(result, Err(NativeDispatchError::Memory(_))));
                }
                assert_eq!(engine.machine.state, state);
                assert_eq!(engine.machine.usage.vector_operations, 0);
                assert_eq!(
                    engine.machine.usage.vector_bytes,
                    if case == 2 { u64::MAX - size + 1 } else { 0 }
                );
            });
        }
    }
}
