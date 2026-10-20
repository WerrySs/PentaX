// SPDX-License-Identifier: GPL-2.0-only
//! Actual admitted execution of original raw bits; no host floating arithmetic
//! oracle or guest-memory authority is introduced by this register profile.
use super::*;
use pentax_codegen::CpuidFeature;

const FORMS: [(u8, u8); 16] = [
    (0x10, 0),
    (0x10, 1),
    (0x11, 0),
    (0x11, 1),
    (0x28, 0),
    (0x28, 1),
    (0x29, 0),
    (0x29, 1),
    (0x54, 0),
    (0x54, 1),
    (0x55, 0),
    (0x55, 1),
    (0x56, 0),
    (0x56, 1),
    (0x57, 0),
    (0x57, 1),
];
const CONTROLS: [u32; 4] = [0x1f80, 0x3fa5, 0xdfbf, 0xffbf];
fn encoding(opcode: u8, pp: u8, wide: bool, destination: u8) -> [u8; 5] {
    let copy = opcode < 0x50;
    let (reg, rm) = if matches!(opcode, 0x11 | 0x29) {
        (13, destination)
    } else {
        (destination, if copy { 13 } else { 14 })
    };
    [
        0xc4,
        0xe1 ^ (u8::from(reg >= 8) << 7) ^ (u8::from(rm >= 8) << 5),
        (if copy { 0x78 } else { 0x10 }) | (u8::from(wide) << 2) | pp,
        opcode,
        0xc0 | ((reg & 7) << 3) | (rm & 7),
    ]
}
fn policy() -> TranslationPolicy {
    TranslationPolicy::new(pentax_codegen::CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
        .with_native_ymm_registers()
}
fn host_control() -> (u32, u16) {
    let mut mxcsr = 0u32;
    let mut x87 = 0u16;
    // SAFETY: exact live scalar outputs; mandatory x86-64 control stores only.
    unsafe {
        std::arch::asm!("stmxcsr [{mx}]","fnstcw [{cw}]",mx=in(reg)&mut mxcsr,cw=in(reg)&mut x87,options(nostack,preserves_flags));
    }
    (mxcsr, x87)
}
fn input(mut state: NativeCpuState, case: usize, control: u32) -> NativeCpuState {
    const BITS: [u64; 8] = [
        0,
        u64::MAX,
        0x8000_0000_8000_0000,
        0x7fa0_0031_ffc0_0072,
        0x7ff0_0000_0000_0031,
        0xfff8_0000_0000_0072,
        0x0123_4567_89ab_cdef,
        0xfedc_ba98_7654_3210,
    ];
    for (index, word) in state.gpr.iter_mut().enumerate() {
        if index != RSP {
            *word = 0x1020_3040_5060_7080 ^ index as u64;
        }
    }
    state.gpr[RAX] = 42;
    state.rflags = 0x202 | 0x8d5 | 0x400;
    for register in 0..16 {
        for word in 0..4 {
            // Preserve real signaling NaNs in both actual sources, including
            // their low128 words. Tag only unrelated registers for context proof.
            let value = if matches!(register, 13 | 14) {
                BITS[(case * 2 + word + register - 13) % BITS.len()]
            } else {
                BITS[(register + word + case) % BITS.len()]
                    ^ (register as u64 * 0x0101_0101_0101_0101)
            };
            let bank = if word < 2 {
                &mut state.xmm
            } else {
                &mut state.ymm_upper
            };
            let offset = register * 16 + (word % 2) * 8;
            bank[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
    }
    state.mxcsr = control;
    state
}
fn expected(
    mut state: NativeCpuState,
    opcode: u8,
    wide: bool,
    destination: usize,
) -> NativeCpuState {
    let mut result = [0u8; 32];
    for offset in 0..if wide { 32 } else { 16 } {
        let bank = if offset < 16 {
            &state.xmm
        } else {
            &state.ymm_upper
        };
        let a = bank[13 * 16 + offset % 16];
        let b = bank[14 * 16 + offset % 16];
        result[offset] = match opcode {
            0x54 => a & b,
            0x55 => !a & b,
            0x56 => a | b,
            0x57 => a ^ b,
            _ => a,
        };
    }
    state.xmm[destination * 16..destination * 16 + 16].copy_from_slice(&result[..16]);
    state.ymm_upper[destination * 16..destination * 16 + 16].copy_from_slice(&result[16..]);
    state
}

#[test]
fn native_vector_register_bits_match_literal_order_upper_and_complete_context() {
    let mode = VectorMode::detect();
    if mode.features().is_none() {
        println!(
            "native_vector_register_bits_available=false native_vector_register_bits_cases=0 native_vector_register_bits_mode=sse2"
        );
        return;
    }
    let host = host_control();
    let mut code = Vec::new();
    for (opcode, pp) in FORMS {
        for wide in [false, true] {
            for destination in [9, 13, 14] {
                code.extend(encoding(opcode, pp, wide, destination));
                code.push(0xc3);
            }
        }
    }
    let mut cases = 0;
    with_source(Some(&code), |source, arena, stack, state| {
        let raw = input(state, 1, 0x1f80);
        assert_eq!(
            u32::from_le_bytes(raw.xmm[220..224].try_into().unwrap()),
            0x7fa0_0031
        );
        assert_eq!(
            u64::from_le_bytes(raw.xmm[232..240].try_into().unwrap()),
            0x7ff0_0000_0000_0031
        );
        let initial = input(state, 0, 0x1f80);
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            initial,
            stack,
            &Default::default(),
        )
        .unwrap();
        let mut index = 0;
        for (opcode, _) in FORMS {
            for wide in [false, true] {
                for destination in [9, 13, 14] {
                    let pc = state.guest_pc + (index * 6) as u64;
                    let plan = plan_materialized_block(
                        source,
                        0,
                        pc,
                        &code[index * 6..index * 6 + 6],
                        engine.placement(),
                        &policy(),
                        &BlockLimits::default(),
                    )
                    .unwrap();
                    assert!(plan.instructions()[0].emitted_len() > 0);
                    for control in CONTROLS {
                        if !mxcsr_control::valid(control) {
                            continue;
                        }
                        for case in 0..4 {
                            let mut current = input(state, case, control);
                            current.guest_pc = pc;
                            if cases != 0 {
                                engine
                                    .begin_call(&mut arena.pause().unwrap(), current)
                                    .unwrap();
                            } else {
                                assert_eq!(*engine.state(), current);
                            }
                            let mut wanted = expected(current, opcode, wide, destination as usize);
                            wanted.gpr[RSP] += 8;
                            wanted.guest_pc = stack.return_marker;
                            // SAFETY: original exact register-only body, actual admitted
                            // AVX mode and the current authenticated owner/stack.
                            let stop =
                                unsafe { engine.step_checked(&mut arena.pause().unwrap(), &plan) }
                                    .unwrap();
                            assert!(matches!(stop, NativeBlockStop::Returned { value: 42 }));
                            assert_eq!(*engine.state(), wanted);
                            assert_eq!(host_control(), host);
                            assert_eq!(engine.usage().vector_operations, 0);
                            assert_eq!(engine.usage().vector_bytes, 0);
                            cases += 1;
                        }
                    }
                    index += 1;
                }
            }
        }
        assert_eq!(engine.usage().native_entries, cases);
        assert_eq!(engine.usage().native_instructions, cases);
    });
    let controls = CONTROLS
        .iter()
        .filter(|control| mxcsr_control::valid(**control))
        .count() as u64;
    assert!(controls > 0);
    assert_eq!(cases, 32 * 3 * 4 * controls);
    let label = if matches!(mode, VectorMode::Avx { avx2: true }) {
        "avx2"
    } else {
        "avx"
    };
    println!(
        "native_vector_register_bits_available=true native_vector_register_bits_cases={cases} native_vector_register_bits_mode={label}"
    );
}

#[test]
fn native_vector_register_bits_keep_preentry_guards_and_memory_mediation() {
    for case in 0..8 {
        let mut code = encoding(0x55, 0, true, 9).to_vec();
        if case >= 6 {
            code = vec![0xc5, 0xfc, 0x10, 0x08];
        } // VMOVUPS YMM1,[RAX] remains checked.
        code.push(0xc3);
        with_source(Some(&code), |source, arena, stack, state| {
            let mut state = input(state, 1, 0x1fa5);
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
            let plan = plan_materialized_block(
                source,
                0,
                state.guest_pc,
                &code,
                engine.placement(),
                &policy().with_native_packed_float_memory(),
                &BlockLimits::default(),
            )
            .unwrap();
            if case >= 6 {
                assert_eq!(plan.instructions()[0].emitted_len(), 0);
                if case == 7 {
                    arena
                        .protect(page, PAGE, NativePermissions::new(false, false, false))
                        .unwrap();
                }
            } else {
                assert!(plan.instructions()[0].emitted_len() > 0);
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
            // SAFETY: original fixture deliberately fails before native entry;
            // the only memory form retains the existing live full-span checks.
            assert!(unsafe { engine.step_checked(&mut arena.pause().unwrap(), &plan) }.is_err());
            assert_eq!(*engine.state(), before);
            assert_eq!(engine.usage().native_entries, 0);
            assert_eq!(engine.usage().cache_publications, 0);
            assert_eq!(engine.usage().vector_operations, 0);
        });
    }
}
