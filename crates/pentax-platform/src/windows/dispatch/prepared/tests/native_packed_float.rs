// SPDX-License-Identifier: GPL-2.0-only
//! Original copied-bit cases compare the admitted packed body with the existing
//! scalar SSE mediator. Intel Vol1 Table4-7 preserves logical source order;
//! section11.5.3.1 ORs masked exceptions across lanes. No guest controls expand.
use super::*;
use pentax_codegen::CpuidFeature;

fn instruction(opcode: u8, double: bool, wide: bool, destination: u8, memory: bool) -> [u8; 5] {
    [
        0xc4,
        if memory { 0x61 } else { 0x41 },
        0x10 | u8::from(double) | (u8::from(wide) << 2),
        opcode,
        ((destination & 7) << 3) | if memory { 0 } else { 0xc6 },
    ]
}

fn requested_policy() -> TranslationPolicy {
    TranslationPolicy::new(pentax_codegen::CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
        .with_native_ymm_registers()
}

fn host_control() -> (u32, u16) {
    let mut mxcsr = 0_u32;
    let mut x87 = 0_u16;
    // SAFETY: mandatory x86-64 SSE/x87 stores into exact live local objects.
    // No control is changed and all outputs are private copied values.
    unsafe {
        std::arch::asm!("stmxcsr [{mx}]", "fnstcw [{cw}]",
            mx = in(reg) &mut mxcsr, cw = in(reg) &mut x87,
            options(nostack, preserves_flags));
    }
    (mxcsr, x87)
}

const SINGLE: [(u64, u64); 12] = [
    (0x40c0_0000, 0x4000_0000), // exact6 and2
    (0x3f80_0000, 0x3380_0000), // halfULP at1
    (0x7fa0_0031, 0xffc0_0072), // distinct ordered SNaN/QNaN payloads
    (0x7f7f_ffff, 0x4000_0000),
    (0x0080_0000, 0x3eaa_aaab),
    (1, 0x3f80_0000),
    (0xffc0_0072, 0x7fa0_0031),
    (0x7fc0_0011, 0xffc0_0022),
    (0x8000_0000, 0),
    (0x7f80_0000, 0xff80_0000),
    (0, 0x7f80_0000),
    (0x7fc0_0055, 1), // same-lane NaN/denormal; other lanes still raise flags
];
const DOUBLE: [(u64, u64); 12] = [
    (0x4018_0000_0000_0000, 0x4000_0000_0000_0000),
    (0x3ff0_0000_0000_0000, 0x3ca0_0000_0000_0000),
    (0x7ff0_0000_0000_0031, 0xfff8_0000_0000_0072),
    (0x7fef_ffff_ffff_ffff, 0x4000_0000_0000_0000),
    (0x0010_0000_0000_0000, 0x3fd5_5555_5555_5555),
    (1, 0x3ff0_0000_0000_0000),
    (0xfff8_0000_0000_0072, 0x7ff0_0000_0000_0031),
    (0x7ff8_0000_0000_0011, 0xfff8_0000_0000_0022),
    (0x8000_0000_0000_0000, 0),
    (0x7ff0_0000_0000_0000, 0xfff0_0000_0000_0000),
    (0, 0x7ff0_0000_0000_0000),
    (0x7ff8_0000_0000_0055, 1),
];

fn input(
    mut state: NativeCpuState,
    double: bool,
    wide: bool,
    case: usize,
    control: u32,
) -> NativeCpuState {
    for (index, word) in state.gpr.iter_mut().enumerate() {
        if index != RSP {
            *word = 0x1020_3040_5060_7080 ^ index as u64;
        }
    }
    state.gpr[RAX] = 42;
    state.rflags = 0x202 | 0x8d5 | 0x400;
    for (index, byte) in state.xmm.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(17);
    }
    for (index, byte) in state.ymm_upper.iter_mut().enumerate() {
        *byte = !(index as u8).wrapping_mul(13);
    }
    let pairs = if double { &DOUBLE } else { &SINGLE };
    let element = if double { 8 } else { 4 };
    for lane in 0..(if wide { 32 } else { 16 }) / element {
        let (left, right) = pairs[(case + lane) % pairs.len()];
        for (register, bits) in [(13, left), (14, right)] {
            let offset = lane * element;
            let bank = if offset < 16 {
                &mut state.xmm
            } else {
                &mut state.ymm_upper
            };
            let start = register * 16 + offset % 16;
            bank[start..start + element].copy_from_slice(&bits.to_le_bytes()[..element]);
        }
    }
    state.mxcsr = control;
    state
}

fn expected(
    state: NativeCpuState,
    opcode: u8,
    double: bool,
    wide: bool,
    destination: usize,
) -> NativeCpuState {
    let operation = match opcode {
        0x58 => float_ops::Operation::Add,
        0x5c => float_ops::Operation::Sub,
        _ => float_ops::Operation::Mul,
    };
    let element = if double { 8 } else { 4 };
    let mut result = [0_u8; 32];
    let mut control = state.mxcsr;
    for offset in (0..if wide { 32 } else { 16 }).step_by(element) {
        let bank = if offset < 16 {
            &state.xmm
        } else {
            &state.ymm_upper
        };
        let mut left = [0_u8; 8];
        let mut right = [0_u8; 8];
        left[..element]
            .copy_from_slice(&bank[13 * 16 + offset % 16..13 * 16 + offset % 16 + element]);
        right[..element]
            .copy_from_slice(&bank[14 * 16 + offset % 16..14 * 16 + offset % 16 + element]);
        let a = u64::from_le_bytes(left);
        let b = u64::from_le_bytes(right);
        let (bits, next) = float_ops::calculate(operation, double, a, b, control).unwrap();
        // Independent primary Table4-7 oracle, not merely hardware-to-hardware
        // agreement: select the first logical NaN and quiet its payload.
        let (exponent, fraction, quiet) = if double {
            (
                0x7ff0_0000_0000_0000,
                0x000f_ffff_ffff_ffff,
                0x0008_0000_0000_0000,
            )
        } else {
            (0x7f80_0000, 0x007f_ffff, 0x0040_0000)
        };
        let nan = |value: u64| value & exponent == exponent && value & fraction != 0;
        if nan(a) || nan(b) {
            assert_eq!(bits, if nan(a) { a | quiet } else { b | quiet });
        }
        if a == (if double { DOUBLE[0].0 } else { SINGLE[0].0 })
            && b == (if double { DOUBLE[0].1 } else { SINGLE[0].1 })
        {
            let literal = match (opcode, double) {
                (0x58, false) => 0x4100_0000,
                (0x5c, false) => 0x4080_0000,
                (0x59, false) => 0x4140_0000,
                (0x58, true) => 0x4020_0000_0000_0000,
                (0x5c, true) => 0x4010_0000_0000_0000,
                _ => 0x4028_0000_0000_0000,
            };
            assert_eq!(bits, literal);
        }
        result[offset..offset + element].copy_from_slice(&bits.to_le_bytes()[..element]);
        control = next;
    }
    let mut expected = state;
    expected.xmm[destination * 16..destination * 16 + 16].copy_from_slice(&result[..16]);
    expected.ymm_upper[destination * 16..destination * 16 + 16].copy_from_slice(&result[16..]);
    expected.mxcsr = control;
    expected
}

#[test]
fn native_packed_float_matches_ordered_literal_and_mediated_bits_under_every_control() {
    let mode = VectorMode::detect();
    if mode.features().is_none() {
        println!(
            "native_packed_float_available=false native_packed_float_cases=0 native_packed_float_mode=sse2"
        );
        return;
    }
    let host = host_control();
    let mut cases = 0;
    for opcode in [0x58, 0x5c, 0x59] {
        for double in [false, true] {
            for wide in [false, true] {
                let mut code = Vec::new();
                for destination in [9, 13, 14] {
                    code.extend(instruction(opcode, double, wide, destination, false));
                    code.push(0xc3);
                }
                with_source(Some(&code), |source, arena, stack, state| {
                    let initial = input(state, double, wide, 0, 0x1f84);
                    let mut engine = NativeBlockStepper::new(
                        source,
                        &mut arena.pause().unwrap(),
                        initial,
                        stack,
                        &Default::default(),
                    )
                    .unwrap();
                    for (alias, destination) in [9, 13, 14].into_iter().enumerate() {
                        let pc = state.guest_pc + (alias * 6) as u64;
                        let planned = plan_materialized_block(
                            source,
                            0,
                            pc,
                            &code[alias * 6..alias * 6 + 6],
                            engine.placement(),
                            &requested_policy(),
                            &BlockLimits::default(),
                        )
                        .unwrap();
                        assert!(planned.instructions()[0].emitted_len() > 0);
                        for rounding in 0..4 {
                            for daz in [false, true] {
                                for ftz in [false, true] {
                                    let control = 0x1f84
                                        | (rounding << 13)
                                        | (u32::from(daz) << 6)
                                        | (u32::from(ftz) << 15);
                                    if !mxcsr_control::valid(control) {
                                        continue;
                                    }
                                    for case in 0..SINGLE.len() {
                                        let mut current = input(state, double, wide, case, control);
                                        current.guest_pc = pc;
                                        if engine.usage().blocks != 0 {
                                            engine
                                                .begin_call(&mut arena.pause().unwrap(), current)
                                                .unwrap();
                                        } else {
                                            assert_eq!(*engine.state(), current);
                                        }
                                        let mut wanted = expected(
                                            current,
                                            opcode,
                                            double,
                                            wide,
                                            destination as usize,
                                        );
                                        wanted.gpr[RSP] += 8;
                                        wanted.guest_pc = stack.return_marker;
                                        // SAFETY: original immutable register-only code, this
                                        // exact live owner/stack and actual admitted AVX mode.
                                        let stop = unsafe {
                                            engine
                                                .step_checked(&mut arena.pause().unwrap(), &planned)
                                        }
                                        .unwrap();
                                        assert!(matches!(
                                            stop,
                                            NativeBlockStop::Returned { value: 42 }
                                        ));
                                        assert_eq!(*engine.state(), wanted);
                                        assert_eq!(host_control(), host);
                                        assert_eq!(engine.usage().vector_operations, 0);
                                        assert_eq!(engine.usage().vector_bytes, 0);
                                        cases += 1;
                                    }
                                }
                            }
                        }
                    }
                    assert_eq!(
                        engine.usage().native_entries,
                        engine.usage().native_instructions
                    );
                });
            }
        }
    }
    let label = if matches!(mode, VectorMode::Avx { avx2: true }) {
        "avx2"
    } else {
        "avx"
    };
    println!(
        "native_packed_float_available=true native_packed_float_cases={cases} native_packed_float_mode={label}"
    );
}

#[test]
fn native_packed_float_retains_mode_control_source_and_memory_refusals_before_effects() {
    for case in 0..9 {
        let mut code = instruction(0x59, false, true, 9, case >= 6).to_vec();
        if case == 8 {
            code = instruction(0x58, false, true, 9, false).to_vec();
            code.extend([0x8b, 0x00]); //later ordinary MOV EAX,[RAX]
        }
        code.push(0xc3);
        with_source(Some(&code), |source, arena, stack, mut state| {
            if case == 8 {
                state = input(state, false, true, 2, 0x1f84);
            }
            let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            state.gpr[RAX] = page + PAGE - 32 + u64::from(case == 6);
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let policy = if case == 8 {
                requested_policy().with_independent_memory()
            } else if case >= 6 {
                TranslationPolicy::x64_sse2()
            } else {
                requested_policy()
            };
            let planned = plan_materialized_block(
                source,
                0,
                state.guest_pc,
                &code,
                engine.placement(),
                &policy,
                &BlockLimits::default(),
            )
            .unwrap();
            if case == 8 {
                assert!(planned.instructions()[0].emitted_len() > 0);
                assert!(planned.instructions()[1].emitted_len() > 0);
                // The whole joined body is admitted before its first FP op.
                // This deliberately does not promise old per-exit fault state.
                arena
                    .protect(page, PAGE, NativePermissions::new(false, false, false))
                    .unwrap();
            } else if case >= 6 {
                assert_eq!(planned.instructions()[0].emitted_len(), 0);
                if case == 7 {
                    arena
                        .protect(page, PAGE, NativePermissions::new(false, false, false))
                        .unwrap();
                }
            } else {
                assert!(planned.instructions()[0].emitted_len() > 0);
                match case {
                    0 => engine.machine.vector_mode = VectorMode::Sse2,
                    1 => engine.machine.state.mxcsr &= !0x80,
                    2 => engine.machine.state.mxcsr |= 1 << 16,
                    3 => engine.machine.limits.max_work_units = 0,
                    4 => engine.machine.usage.native_instructions = u64::MAX,
                    _ => {
                        arena
                            .protect(
                                state.guest_pc & !(PAGE - 1),
                                PAGE,
                                NativePermissions::READ_WRITE,
                            )
                            .unwrap();
                    }
                }
            }
            let before = *engine.state();
            // SAFETY: only original code is supplied; every fixture deliberately
            // fails a real admission check before any possible hardware entry.
            let result = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &planned) };
            assert!(result.is_err());
            assert_eq!(*engine.state(), before);
            assert_eq!(engine.usage().native_entries, 0);
            assert_eq!(engine.usage().cache_publications, 0);
            assert_eq!(engine.usage().vector_operations, 0);
        });
    }
}
