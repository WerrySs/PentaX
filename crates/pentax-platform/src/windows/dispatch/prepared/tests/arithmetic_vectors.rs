// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn modeled_arithmetic_vectors_wrap_each_lane_without_changing_flags_or_source_aliases() {
    for (opcode, width, subtract) in [
        (0xfc, 1, false),
        (0xfd, 2, false),
        (0xfe, 4, false),
        (0xd4, 8, false),
        (0xf8, 1, true),
        (0xf9, 2, true),
        (0xfa, 4, true),
        (0xfb, 8, true),
    ] {
        for wide in [false, true] {
            for form in 0..4 {
                // Destination xmm/ymm15, or aliases either source xmm/ymm1/2.
                let code = [
                    0xc5,
                    (if form <= 1 { 0x71 } else { 0xf1 }) | if wide { 4 } else { 0 },
                    opcode,
                    match form {
                        0 => 0xfa,
                        1 => 0x38,
                        2 => 0xca,
                        _ => 0xd2,
                    },
                    0xc3,
                ];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let size = if wide { 32 } else { 16 };
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE
                        - size as u64;
                    // Original nonuniform data includes carries across bytes,
                    // zero underflow and signed boundaries within each width.
                    let mut left = [0xff; 32];
                    let mut right = [0; 32];
                    for (index, lane) in right.chunks_exact_mut(width).enumerate() {
                        lane[0] = if index % 2 == 0 { 1 } else { 0xff };
                    }
                    left[width..width * 2].fill(0);
                    left[width * 3 - 1] = 0x7f;
                    let mut expected_bytes = [0; 32];
                    // Independent bytewise carry/borrow reference, reset at
                    // each element boundary rather than using integer wrapping.
                    for lane in (0..size).step_by(width) {
                        let mut carry = 0_i16;
                        for byte in lane..lane + width {
                            let value = if subtract {
                                i16::from(left[byte]) - i16::from(right[byte]) - carry
                            } else {
                                i16::from(left[byte]) + i16::from(right[byte]) + carry
                            };
                            expected_bytes[byte] = value as u8;
                            carry = i16::from(if subtract { value < 0 } else { value > 255 });
                        }
                    }
                    arena.pause().unwrap().write(data, &right[..size]).unwrap();
                    state.gpr[RAX] = data;
                    state.xmm.fill(0xa5);
                    state.ymm_upper.fill(0x5a);
                    state.xmm[16..32].copy_from_slice(&left[..16]);
                    state.ymm_upper[16..32].copy_from_slice(&left[16..]);
                    state.xmm[32..48].copy_from_slice(&right[..16]);
                    state.ymm_upper[32..48].copy_from_slice(&right[16..]);
                    state.rflags |= 0x8d5;
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
                    let mut expected = state;
                    let slot = match form {
                        2 => 16,
                        3 => 32,
                        _ => 240,
                    };
                    expected.xmm[slot..slot + 16].copy_from_slice(&expected_bytes[..16]);
                    expected.ymm_upper[slot..slot + 16].copy_from_slice(&expected_bytes[16..]);
                    expected.guest_pc = planned.exit().next_guest_pc();
                    assert_eq!(engine.machine.state, expected);
                    assert_eq!(engine.machine.usage.vector_operations, 1);
                    assert_eq!(
                        engine.machine.usage.vector_bytes,
                        if form == 1 { size as u64 } else { 0 }
                    );
                });
            }
        }
    }
}
