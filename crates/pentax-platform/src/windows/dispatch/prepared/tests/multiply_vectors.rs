// SPDX-License-Identifier: GPL-2.0-only
use super::*;

// Map/opcode, input bytes, signed, high result, widened result.
const FORMS: [(u8, u8, usize, bool, bool, bool); 6] = [
    (1, 0xd5, 2, false, false, false),
    (1, 0xe5, 2, true, true, false),
    (1, 0xe4, 2, false, true, false),
    (2, 0x40, 4, false, false, false),
    (1, 0xf4, 4, false, false, true),
    (2, 0x28, 4, true, false, true),
];

// Wide arithmetic oracle keeps sign extension and result selection independent
// of the dispatcher's fixed u64/i64 arithmetic.
fn reference(
    a: &[u8; 32],
    b: &[u8; 32],
    width: usize,
    signed: bool,
    high: bool,
    widen: bool,
    size: usize,
) -> [u8; 32] {
    let mut result = [0; 32];
    let output = if widen { 8 } else { width };
    let decode = |bytes: &[u8]| -> i128 {
        let bits = bytes
            .iter()
            .rev()
            .fold(0_i128, |value, byte| value * 256 + i128::from(*byte));
        if signed && bytes[bytes.len() - 1] & 0x80 != 0 {
            bits - (1_i128 << (bytes.len() * 8))
        } else {
            bits
        }
    };
    for at in (0..size).step_by(output) {
        let product = decode(&a[at..at + width]) * decode(&b[at..at + width]);
        let selected = if high {
            product >> (width * 8)
        } else {
            product
        };
        result[at..at + output].copy_from_slice(&selected.to_le_bytes()[..output]);
    }
    result
}

#[test]
fn vector_multiply_forms_keep_signed_high_low_and_even_dword_products() {
    for (map, opcode, width, signed, high, widen) in FORMS {
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
                        let mut a = [0; 32];
                        let mut b = [0; 32];
                        let values = [
                            0xffffffff_u32,
                            0x80000000,
                            0x7fffffff,
                            0x12345678,
                            0xffff8000,
                            0x0000ffff,
                            0x80008000,
                            0,
                        ];
                        for (i, value) in values.into_iter().enumerate() {
                            a[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
                            b[i * 4..i * 4 + 4].copy_from_slice(&values[(i + 3) % 8].to_le_bytes());
                        }
                        let expected_bytes = reference(&a, &b, width, signed, high, widen, size);
                        let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                        let address = page + PAGE - size as u64;
                        arena.pause().unwrap().write(address, &b[..size]).unwrap();
                        state.gpr[RAX] = address;
                        state.xmm[16..32].copy_from_slice(&a[..16]);
                        state.ymm_upper[16..32].copy_from_slice(&a[16..]);
                        state.xmm[32..48].copy_from_slice(&b[..16]);
                        state.ymm_upper[32..48].copy_from_slice(&b[16..]);
                        state.rflags |= 0x8d5;
                        state.mxcsr |= 0x25;
                        let mut expected = state;
                        let at = usize::from(destination) * 16;
                        expected.xmm[at..at + 16].copy_from_slice(&expected_bytes[..16]);
                        expected.ymm_upper[at..at + 16].copy_from_slice(&expected_bytes[16..]);
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
fn vector_multiply_requires_full_memory_even_for_ignored_odd_dwords() {
    for wide in [false, true] {
        for case in 0..4 {
            let code = [
                0xc4,
                0xe1,
                0x71 | if wide { 4 } else { 0 },
                0xf4,
                0x08,
                0xc3,
            ];
            with_source(Some(&code), |source, arena, stack, mut state| {
                let size = if wide { 32 } else { 16 };
                let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let address = page + PAGE - size;
                arena
                    .pause()
                    .unwrap()
                    .write(address, &[0xff; 32][..size as usize])
                    .unwrap();
                state.gpr[RAX] = address + if case == 1 { 4 } else { 0 };
                state.xmm[16..32].fill(0xff);
                state.ymm_upper[16..32].fill(0xff);
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
                if case == 2 {
                    arena
                        .protect(page, PAGE, NativePermissions::new(false, false, false))
                        .unwrap();
                }
                let initial = if case == 3 { u64::MAX - size + 1 } else { 0 };
                engine.machine.usage.vector_bytes = initial;
                engine.machine.phase = Phase::Terminal;
                let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                if case == 0 {
                    result.unwrap();
                    let mut expected = state;
                    for lane in expected.xmm[16..32].chunks_exact_mut(8) {
                        lane.copy_from_slice(&0xffff_fffe_0000_0001_u64.to_le_bytes());
                    }
                    if wide {
                        expected.ymm_upper[16..32].copy_from_slice(&expected.xmm[16..32]);
                    } else {
                        expected.ymm_upper[16..32].fill(0);
                    }
                    expected.guest_pc = planned.exit().next_guest_pc();
                    assert_eq!(engine.machine.state, expected);
                } else {
                    assert!(result.is_err());
                    assert_eq!(engine.machine.state, state);
                    assert_eq!(engine.machine.usage.vector_operations, 0);
                    assert_eq!(engine.machine.usage.vector_bytes, initial);
                }
            });
        }
    }
}
