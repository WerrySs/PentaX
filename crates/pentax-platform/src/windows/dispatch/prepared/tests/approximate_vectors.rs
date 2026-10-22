// SPDX-License-Identifier: GPL-2.0-only
//! Original copied-lane estimate, VEX merge and checked-memory regressions.
use super::*;
use std::arch::x86_64::{
    _mm_castsi128_ps, _mm_cvtsi32_si128, _mm_cvtss_f32, _mm_rcp_ss, _mm_rsqrt_ss,
};

const INPUTS: [u32; 20] = [
    0,
    0x8000_0000,
    1,
    0x8000_0001,
    0x007f_ffff,
    0x807f_ffff,
    0x0080_0000,
    0x8080_0000,
    0x3f00_0000,
    0x3f80_0000,
    0x4000_0000,
    0x4040_0000,
    0x4080_0000,
    0xc080_0000,
    0x7f7f_ffff,
    0xff7f_ffff,
    0x7f80_0000,
    0xff80_0000,
    0x7fc0_1234,
    0xff80_5678,
];

fn host_controls() -> (u32, u16) {
    let mut mxcsr = 0;
    let mut x87 = 0;
    // SAFETY: Mandatory x86-64 control stores into exact live local objects.
    unsafe {
        std::arch::asm!("stmxcsr [{mx}]", "fnstcw [{cw}]",
        mx = in(reg) &mut mxcsr, cw = in(reg) &mut x87,
        options(nostack, preserves_flags));
    }
    (mxcsr, x87)
}

fn oracle(opcode: u8, input: u32) -> u32 {
    // SAFETY: SSE is mandatory on x86-64. Only copied numeric bits enter the
    // independent intrinsic oracle. RCPSS/RSQRTSS signal no SIMD exceptions
    // and ignore rounding/DAZ/FTZ, so this does not install guest controls.
    unsafe {
        let input = _mm_castsi128_ps(_mm_cvtsi32_si128(input as i32));
        _mm_cvtss_f32(if opcode == 0x53 {
            _mm_rcp_ss(input)
        } else {
            _mm_rsqrt_ss(input)
        })
        .to_bits()
    }
}

fn operation(opcode: u8) -> float_ops::Operation {
    if opcode == 0x53 {
        float_ops::Operation::Reciprocal
    } else {
        float_ops::Operation::ReciprocalSqrt
    }
}

fn encoding(
    opcode: u8,
    wide: bool,
    scalar: bool,
    memory: bool,
    destination: u8,
    merge: u8,
    source: u8,
) -> Vec<u8> {
    vec![
        0xc4,
        1 | 0x40
            | if destination < 8 { 0x80 } else { 0 }
            | if memory || source < 8 { 0x20 } else { 0 },
        if scalar {
            ((!merge & 15) << 3) | 2
        } else {
            0x78 | (u8::from(wide) * 4)
        },
        opcode,
        ((destination & 7) << 3) | if memory { 0 } else { 0xc0 | (source & 7) },
        0xc3,
    ]
}

#[test]
fn hardware_estimates_preserve_guest_status_special_values_and_host_controls() {
    let host = host_controls();
    for opcode in [0x52, 0x53] {
        for input in INPUTS {
            let expected = oracle(opcode, input);
            assert_eq!(host_controls(), host);
            for rounding in 0..4 {
                for daz_ftz in [0, 0x40, 0x8000, 0x8040] {
                    let control = 0x1fa5 | (rounding << 13) | daz_ftz;
                    if !mxcsr_control::valid(control) {
                        continue;
                    }
                    let (bits, next) = float_ops::calculate(
                        operation(opcode),
                        false,
                        u64::MAX,
                        u64::from(input),
                        control,
                    )
                    .unwrap();
                    assert_eq!((bits, next), (u64::from(expected), control));
                    assert_eq!(host_controls(), host);
                }
            }
            let sign = input & 0x8000_0000;
            if input & 0x7f80_0000 == 0 {
                assert_eq!(expected, sign | 0x7f80_0000);
            } else if input == 0x7f80_0000 {
                assert_eq!(expected, 0);
            } else if input == 0xff80_0000 {
                assert_eq!(expected, if opcode == 0x53 { sign } else { 0xffc0_0000 });
            } else if input == 0x7fc0_1234 {
                assert_eq!(expected, input);
            } else if opcode == 0x52 && input == 0xc080_0000 {
                assert_eq!(expected, 0xffc0_0000);
            } else if opcode == 0x53 && input & 0x7fff_ffff == 0x7f7f_ffff {
                assert_eq!(expected, sign);
            }
        }
        for invalid in [0, 0x1f80 | (1 << 31)] {
            assert!(matches!(
                float_ops::calculate(operation(opcode), false, 0, 0, invalid),
                Err(NativeDispatchError::InvalidState)
            ));
            assert_eq!(host_controls(), host);
        }
        assert!(matches!(
            float_ops::calculate(operation(opcode), true, 0, 0, 0x1f80),
            Err(NativeDispatchError::UnsupportedVectorInstruction)
        ));
        assert_eq!(host_controls(), host);
    }
}

#[test]
fn copied_estimate_lanes_satisfy_documented_error_bounds_without_ieee_substitution() {
    // For a positive exact power-of-two result, the SDM relative error limit
    // 1.5*2^-12 gives these exact monotonic bit bounds across its binade edge.
    // No test reference DIV/SQRT changes the caller's host floating controls.
    for (opcode, input, center) in [
        (0x53, 0x3f80_0000, 0x3f80_0000u32),
        (0x53, 0x4000_0000, 0x3f00_0000),
        (0x53, 0x3f00_0000, 0x4000_0000),
        (0x52, 0x3f80_0000, 0x3f80_0000),
        (0x52, 0x4080_0000, 0x3f00_0000),
        (0x52, 0x4180_0000, 0x3e80_0000),
    ] {
        let (actual, control) =
            float_ops::calculate(operation(opcode), false, 0, input, 0x1fa5).unwrap();
        assert!((u64::from(center - 0x1800)..=u64::from(center + 0xc00)).contains(&actual));
        assert_eq!(control, 0x1fa5);
    }
}

#[test]
fn vex_estimates_snapshot_aliases_merge_scalar_lanes_and_read_exact_unaligned_widths() {
    let host = host_controls();
    for opcode in [0x52, 0x53] {
        for (wide, scalar) in [(false, false), (true, false), (false, true)] {
            for memory in [false, true] {
                for high in [false, true] {
                    let (merge, source, other) = if high { (13, 14, 15) } else { (1, 2, 0) };
                    for destination in [merge, source, other] {
                        for end_of_page in [false, true] {
                            let code =
                                encoding(opcode, wide, scalar, memory, destination, merge, source);
                            with_source(Some(&code), |materialized, arena, stack, mut state| {
                                let width = if scalar {
                                    4
                                } else if wide {
                                    32
                                } else {
                                    16
                                };
                                let data =
                                    materialized.source_plan().modules()[0].load_bias() + DATA_OFF;
                                let at = if end_of_page {
                                    data + PAGE - width as u64
                                } else {
                                    data + 3
                                };
                                state.xmm = std::array::from_fn(|i| (i as u8).wrapping_mul(17));
                                state.ymm_upper.fill(0xa5);
                                state.rflags = 0x202 | 0x8d5;
                                state.mxcsr = 0x1fa5;
                                let mut input = [0; 32];
                                for (lane, slot) in input.chunks_exact_mut(4).enumerate() {
                                    slot.copy_from_slice(
                                        &INPUTS[(lane + 8) % INPUTS.len()].to_le_bytes(),
                                    );
                                }
                                let source_at = source as usize * 16;
                                state.xmm[source_at..source_at + 16].copy_from_slice(&input[..16]);
                                state.ymm_upper[source_at..source_at + 16]
                                    .copy_from_slice(&input[16..]);
                                state.gpr[RAX] = at;
                                arena.pause().unwrap().write(at, &input[..width]).unwrap();
                                let mut output = [0; 32];
                                if scalar {
                                    let start = merge as usize * 16;
                                    output[..16].copy_from_slice(&state.xmm[start..start + 16]);
                                }
                                for offset in (0..width).step_by(4) {
                                    let bits = u32::from_le_bytes(
                                        input[offset..offset + 4].try_into().unwrap(),
                                    );
                                    output[offset..offset + 4]
                                        .copy_from_slice(&oracle(opcode, bits).to_le_bytes());
                                }
                                let mut expected = state;
                                let start = destination as usize * 16;
                                expected.xmm[start..start + 16].copy_from_slice(&output[..16]);
                                expected.ymm_upper[start..start + 16]
                                    .copy_from_slice(&output[16..]);
                                let mut engine = NativeBlockStepper::new(
                                    materialized,
                                    &mut arena.pause().unwrap(),
                                    state,
                                    stack,
                                    &Default::default(),
                                )
                                .unwrap();
                                let planned = block(
                                    materialized,
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
                                assert_eq!(engine.machine.usage.native_instructions, 0);
                                let mut unchanged = [0; 32];
                                arena
                                    .pause()
                                    .unwrap()
                                    .read(at, &mut unchanged[..width])
                                    .unwrap();
                                assert_eq!(&unchanged[..width], &input[..width]);
                                assert_eq!(host_controls(), host);
                            });
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn checked_estimates_refuse_partial_reads_permissions_and_quotas_before_cpu_effects() {
    let host = host_controls();
    for opcode in [0x52, 0x53] {
        for (wide, scalar) in [(false, false), (true, false), (false, true)] {
            for refusal in 0..5 {
                let code = encoding(opcode, wide, scalar, true, 0, 1, 2);
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let width = if scalar {
                        4
                    } else if wide {
                        32
                    } else {
                        16
                    };
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    let at = data + PAGE - width as u64;
                    let input = [0xa5; 32];
                    arena.pause().unwrap().write(at, &input[..width]).unwrap();
                    state.gpr[RAX] = at + u64::from(refusal == 0);
                    state.mxcsr = 0x1fa5;
                    state.xmm.fill(0x12);
                    state.ymm_upper.fill(0x34);
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
                    match refusal {
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
                    arena
                        .protect(data, PAGE, NativePermissions::READ_WRITE)
                        .unwrap();
                    let mut actual = [0; 32];
                    arena
                        .pause()
                        .unwrap()
                        .read(at, &mut actual[..width])
                        .unwrap();
                    assert_eq!(&actual[..width], &input[..width]);
                    assert_eq!(host_controls(), host);
                });
            }
        }
    }
}
