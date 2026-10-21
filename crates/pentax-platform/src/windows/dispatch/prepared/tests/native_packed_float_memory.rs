// SPDX-License-Identifier: GPL-2.0-only
//! Original checked native reads; no guest source or host identity is exported.
use super::*;
use pentax_codegen::{CpuFeatures, CpuidFeature, Decoder, DecoderOptions, ExitKind, Register};

const BASE: usize = 3; // RBX, independently observed for every checked entry.

fn policy() -> TranslationPolicy {
    TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
        .with_native_ymm_registers()
        .with_native_packed_float_memory()
        .with_independent_memory()
}
fn instruction(opcode: u8, double: bool, wide: bool, destination: u8) -> [u8; 5] {
    [
        0xc4,
        0x61,
        0x10 | u8::from(double) | (u8::from(wide) << 2),
        opcode,
        ((destination & 7) << 3) | 3,
    ]
}
fn vectors(double: bool, wide: bool, case: usize) -> ([u8; 32], [u8; 32]) {
    let single: [(u64, u64); 8] = [
        (0x40c0_0000, 0x4000_0000),
        (0x3f80_0000, 0x3380_0000),
        (0x7fa0_0031, 0xffc0_0072),
        (0xffc0_0072, 0x7fa0_0031),
        (1, 0x3f80_0000),
        (0x7f7f_ffff, 0x4000_0000),
        (0x8000_0000, 0),
        (0x7f80_0000, 0xff80_0000),
    ];
    let doubles: [(u64, u64); 8] = [
        (0x4018_0000_0000_0000, 0x4000_0000_0000_0000),
        (0x3ff0_0000_0000_0000, 0x3ca0_0000_0000_0000),
        (0x7ff0_0000_0000_0031, 0xfff8_0000_0000_0072),
        (0xfff8_0000_0000_0072, 0x7ff0_0000_0000_0031),
        (1, 0x3ff0_0000_0000_0000),
        (0x7fef_ffff_ffff_ffff, 0x4000_0000_0000_0000),
        (0x8000_0000_0000_0000, 0),
        (0x7ff0_0000_0000_0000, 0xfff0_0000_0000_0000),
    ];
    let pairs = if double { doubles } else { single };
    let element = if double { 8 } else { 4 };
    let mut left = [0; 32];
    let mut right = [0; 32];
    for offset in (0..if wide { 32 } else { 16 }).step_by(element) {
        let (a, b) = pairs[(case * 2 + offset / element) % pairs.len()];
        left[offset..offset + element].copy_from_slice(&a.to_le_bytes()[..element]);
        right[offset..offset + element].copy_from_slice(&b.to_le_bytes()[..element]);
    }
    (left, right)
}
fn input(mut state: NativeCpuState, left: &[u8; 32], address: u64, control: u32) -> NativeCpuState {
    for (index, word) in state.gpr.iter_mut().enumerate() {
        if index != RSP {
            *word = 0x1020_3040_5060_7080 ^ index as u64;
        }
    }
    state.gpr[RAX] = 42;
    state.gpr[BASE] = address;
    state.rflags = 0x202 | 0x8d5 | 0x400;
    state.mxcsr = control;
    for (i, byte) in state.xmm.iter_mut().enumerate() {
        *byte = (i as u8).wrapping_mul(17);
    }
    for (i, byte) in state.ymm_upper.iter_mut().enumerate() {
        *byte = !(i as u8).wrapping_mul(13);
    }
    state.xmm[208..224].copy_from_slice(&left[..16]);
    state.ymm_upper[208..224].copy_from_slice(&left[16..]);
    state // XMM/YMM14 deliberately differs from the actual memory operand.
}
fn expected(
    mut state: NativeCpuState,
    right: &[u8; 32],
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
    let mut output = [0; 32];
    for offset in (0..if wide { 32 } else { 16 }).step_by(element) {
        let bank = if offset < 16 {
            &state.xmm
        } else {
            &state.ymm_upper
        };
        let mut a = [0; 8];
        let mut b = [0; 8];
        a[..element].copy_from_slice(&bank[208 + offset % 16..208 + offset % 16 + element]);
        b[..element].copy_from_slice(&right[offset..offset + element]);
        let a = u64::from_le_bytes(a);
        let b = u64::from_le_bytes(b);
        let (bits, control) = float_ops::calculate(operation, double, a, b, state.mxcsr).unwrap();
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
        if (a, b)
            == if double {
                (0x4018_0000_0000_0000, 0x4000_0000_0000_0000)
            } else {
                (0x40c0_0000, 0x4000_0000)
            }
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
        output[offset..offset + element].copy_from_slice(&bits.to_le_bytes()[..element]);
        state.mxcsr = control;
    }
    state.xmm[destination * 16..destination * 16 + 16].copy_from_slice(&output[..16]);
    state.ymm_upper[destination * 16..destination * 16 + 16].copy_from_slice(&output[16..]);
    state.gpr[RSP] += 8;
    state.guest_pc = STOP;
    state
}
fn plan<'view, 'plan, 'image>(
    source: &'view MaterializedSource<'plan, 'image>,
    pc: u64,
    code: &[u8],
    placement: CachePlacement,
) -> MaterializedBlockPlan<'view, 'plan, 'image> {
    plan_materialized_block(
        source,
        0,
        pc,
        code,
        placement,
        &policy(),
        &BlockLimits::default(),
    )
    .unwrap()
}

#[test]
fn all_native_memory_fp_forms_match_full_bits_controls_aliases_and_exact_page_edges() {
    let mode = VectorMode::detect();
    if mode.features().is_none() {
        println!(
            "native_packed_float_memory_available=false native_packed_float_memory_cases=0 native_packed_float_memory_mode=sse2"
        );
        return;
    }
    let mut cases = 0_u64;
    for opcode in [0x58, 0x5c, 0x59] {
        for double in [false, true] {
            for wide in [false, true] {
                let mut code = Vec::new();
                for destination in [9, 13] {
                    code.extend(instruction(opcode, double, wide, destination));
                    code.push(0xc3);
                }
                with_source(Some(&code), |source, arena, stack, state| {
                    let end = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE;
                    let size = if wide { 32 } else { 16 };
                    let (left, _) = vectors(double, wide, 0);
                    let initial = input(state, &left, end - size as u64, 0x1f84);
                    let mut engine = NativeBlockStepper::new(
                        source,
                        &mut arena.pause().unwrap(),
                        initial,
                        stack,
                        &Default::default(),
                    )
                    .unwrap();
                    assert!(
                        engine
                            .native_ymm_features()
                            .unwrap()
                            .contains(CpuidFeature::AVX)
                    );
                    let mut count = 0;
                    for (alias, destination) in [9, 13].into_iter().enumerate() {
                        let pc = state.guest_pc + (alias * 6) as u64;
                        let planned = plan(
                            source,
                            pc,
                            &code[alias * 6..alias * 6 + 6],
                            engine.placement(),
                        );
                        assert!(planned.instructions()[0].emitted_len() > 0);
                        for unaligned in [false, true] {
                            for case in 0..4 {
                                let address = end - size as u64 - u64::from(unaligned);
                                let (left, right) = vectors(double, wide, case);
                                arena
                                    .protect(end - PAGE, PAGE, NativePermissions::READ_WRITE)
                                    .unwrap();
                                arena
                                    .pause()
                                    .unwrap()
                                    .write(address, &right[..size])
                                    .unwrap();
                                arena
                                    .protect(end - PAGE, PAGE, NativePermissions::READ)
                                    .unwrap();
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
                                            let mut current = input(state, &left, address, control);
                                            current.guest_pc = pc;
                                            if count != 0 {
                                                engine
                                                    .begin_call(
                                                        &mut arena.pause().unwrap(),
                                                        current,
                                                    )
                                                    .unwrap();
                                            } else {
                                                assert_eq!(*engine.state(), current);
                                            }
                                            let wanted = expected(
                                                current,
                                                &right,
                                                opcode,
                                                double,
                                                wide,
                                                destination,
                                            );
                                            // SAFETY: original immutable code; this exact paused owner,
                                            // checked full input span, stack and actual AVX context.
                                            let returned = unsafe {
                                                engine.step_checked(
                                                    &mut arena.pause().unwrap(),
                                                    &planned,
                                                )
                                            }
                                            .unwrap();
                                            assert!(matches!(
                                                returned,
                                                NativeBlockStop::Returned { value: 42 }
                                            ));
                                            assert_eq!(
                                                *engine.state(),
                                                wanted,
                                                "op={opcode} double={double} wide={wide} alias={alias} unaligned={unaligned} case={case} mxcsr={control}"
                                            );
                                            let mut unchanged = [0; 32];
                                            arena
                                                .pause()
                                                .unwrap()
                                                .read(address, &mut unchanged[..size])
                                                .unwrap();
                                            assert_eq!(&unchanged[..size], &right[..size]);
                                            count += 1;
                                            cases += 1;
                                            assert_eq!(engine.usage().memory_access_checks, count);
                                            assert_eq!(
                                                engine.usage().memory_bytes_checked,
                                                count * size as u64
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                    assert_eq!(engine.usage().native_entries, count);
                    assert_eq!(engine.usage().native_instructions, count);
                    assert_eq!(engine.usage().vector_operations, 0);
                    assert_eq!(engine.usage().vector_bytes, 0);
                });
            }
        }
    }
    let label = if matches!(mode, VectorMode::Avx { avx2: true, .. }) {
        "avx2"
    } else {
        "avx"
    };
    let controls = (0..16_u32)
        .filter(|&bits| {
            mxcsr_control::valid(
                0x1f84 | ((bits & 3) << 13) | ((bits & 4) << 4) | ((bits & 8) << 12),
            )
        })
        .count() as u64;
    assert!(controls > 0);
    assert_eq!(cases, 12 * 2 * 2 * 4 * controls);
    println!(
        "native_packed_float_memory_available=true native_packed_float_memory_cases={cases} native_packed_float_memory_mode={label}"
    );
}

#[test]
fn native_memory_fp_preflight_refuses_full_span_faults_and_rechecks_live_cached_inputs() {
    if VectorMode::detect().features().is_none() {
        return;
    }
    for wide in [false, true] {
        for case in 0..11 {
            let mut code = instruction(0x59, false, wide, 9).to_vec();
            if case == 7 {
                code.extend([0x8b, 0x00]);
            } // Later MOV EAX,[RAX].
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let end = data + PAGE;
                let size = if wide { 32 } else { 16 };
                if case == 1 {
                    arena
                        .map_at(end, PAGE, NativePermissions::READ_WRITE)
                        .unwrap();
                }
                let address = if case == 0 {
                    end - size as u64 + 1
                } else if case == 1 {
                    end - size as u64 / 2
                } else {
                    end - size as u64
                };
                let (left, right) = vectors(false, wide, 0);
                if case != 0 {
                    arena
                        .pause()
                        .unwrap()
                        .write(address, &right[..size])
                        .unwrap();
                }
                let current = input(state, &left, address, 0x1f84);
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    current,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                let planned = plan(source, current.guest_pc, &code, engine.placement());
                assert!(planned.instructions()[0].emitted_len() > 0);
                if case == 7 {
                    assert!(planned.instructions()[1].emitted_len() > 0);
                }
                match case {
                    1 => arena
                        .protect(end, PAGE, NativePermissions::new(false, false, false))
                        .unwrap(),
                    2 => arena
                        .protect(data, PAGE, NativePermissions::new(false, true, false))
                        .unwrap(),
                    3 => engine.machine.vector_mode = VectorMode::Sse2,
                    4 => engine.machine.state.mxcsr &= !0x80,
                    5 => engine.machine.state.rflags |= 1 << 18, // AC is outside this profile.
                    6 => engine.machine.limits.max_work_units = 0,
                    8 => arena
                        .protect(
                            state.guest_pc & !(PAGE - 1),
                            PAGE,
                            NativePermissions::READ_WRITE,
                        )
                        .unwrap(),
                    9 => engine.machine.usage.memory_bytes_checked = u64::MAX,
                    _ => {}
                }
                let before = *engine.state();
                let outcome = if case == 10 {
                    let mut other =
                        NativeAddressSpace::reserve_any(Default::default(), 65_536).unwrap();
                    // SAFETY: wrong-owner admission is the first check; no entry.
                    unsafe { engine.step_checked(&mut other.pause().unwrap(), &planned) }
                } else {
                    // SAFETY: original owned fixture; each controlled failure is
                    // checked before publication or the first native instruction.
                    unsafe { engine.step_checked(&mut arena.pause().unwrap(), &planned) }
                };
                match case {
                    0..=2 | 7 => assert!(matches!(outcome, Err(NativeDispatchError::Memory(_)))),
                    3..=5 => assert_eq!(outcome, Err(NativeDispatchError::InvalidState)),
                    6 | 9 => assert!(matches!(outcome, Err(NativeDispatchError::Budget(_)))),
                    8 => assert_eq!(outcome, Err(NativeDispatchError::WritableSource)),
                    10 => assert_eq!(outcome, Err(NativeDispatchError::WrongOwner)),
                    _ => unreachable!(),
                }
                assert_eq!(*engine.state(), before);
                assert_eq!(engine.usage().native_entries, 0);
                assert_eq!(engine.usage().native_instructions, 0);
                assert_eq!(engine.usage().cache_publications, 0);
                assert_eq!(engine.usage().vector_operations, 0);
            });
        }
    }
    let mut code = instruction(0x59, false, true, 9).to_vec();
    code.push(0xc3);
    with_source(Some(&code), |source, arena, stack, state| {
        let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
        arena
            .map_at(data + PAGE, PAGE, NativePermissions::READ_WRITE)
            .unwrap();
        let bytes =
            |word: u32| -> [u8; 32] { std::array::from_fn(|index| word.to_le_bytes()[index % 4]) };
        let left = bytes(0x40c0_0000);
        let addresses = [data + 64, data + 128, data + PAGE - 16];
        for (address, value) in [
            (addresses[0], 0x4000_0000),
            (addresses[1], 0x4040_0000),
            (addresses[2], 0x4000_0000),
        ] {
            arena
                .pause()
                .unwrap()
                .write(address, &bytes(value))
                .unwrap();
        }
        arena
            .protect(data, 2 * PAGE, NativePermissions::READ)
            .unwrap();
        let current = input(state, &left, addresses[0], 0x1f84);
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            current,
            stack,
            &Default::default(),
        )
        .unwrap();
        let planned = plan(source, current.guest_pc, &code, engine.placement());
        for (index, address) in [addresses[0], addresses[1], addresses[0], addresses[2]]
            .into_iter()
            .enumerate()
        {
            if index == 2 {
                arena
                    .protect(data, PAGE, NativePermissions::READ_WRITE)
                    .unwrap();
                arena
                    .pause()
                    .unwrap()
                    .write(addresses[0], &bytes(0x4080_0000))
                    .unwrap();
                arena.protect(data, PAGE, NativePermissions::READ).unwrap();
            }
            let current = input(state, &left, address, 0x1f84);
            if index != 0 {
                engine
                    .begin_call(&mut arena.pause().unwrap(), current)
                    .unwrap();
            }
            let right = bytes(match index {
                1 => 0x4040_0000,
                2 => 0x4080_0000,
                _ => 0x4000_0000,
            });
            let wanted = expected(current, &right, 0x59, false, true, 9);
            // SAFETY: exact original same-plan code; live checked readable
            // input spans, including the fourth call across two owned pages.
            unsafe { engine.step_checked(&mut arena.pause().unwrap(), &planned) }.unwrap();
            assert_eq!(*engine.state(), wanted);
        }
        assert_eq!(engine.usage().native_entries, 4);
        assert_eq!(engine.usage().cache_publications, 1);
        assert_eq!(engine.usage().cache_reuses, 3);
        let current = input(state, &left, addresses[2], 0x1f84);
        engine
            .begin_call(&mut arena.pause().unwrap(), current)
            .unwrap();
        arena
            .protect(
                data + PAGE,
                PAGE,
                NativePermissions::new(false, false, false),
            )
            .unwrap();
        let usage = engine.usage();
        // SAFETY: a missing permission on the final half must refuse before
        // entering even an already sealed, otherwise reusable native body.
        let refused = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &planned) };
        assert!(matches!(refused, Err(NativeDispatchError::Memory(_))));
        assert_eq!(*engine.state(), current);
        assert_eq!(engine.usage().native_entries, usage.native_entries);
        assert_eq!(engine.usage().cache_reuses, usage.cache_reuses);
        arena
            .protect(data + PAGE, PAGE, NativePermissions::READ)
            .unwrap();
        // SAFETY: the same still-pending call now has its complete live rights.
        unsafe { engine.step_checked(&mut arena.pause().unwrap(), &planned) }.unwrap();
        assert_eq!(
            *engine.state(),
            expected(current, &bytes(0x4000_0000), 0x59, false, true, 9)
        );
        assert_eq!(engine.usage().native_entries, 5);
        assert_eq!(engine.usage().cache_reuses, 4);
    });
}

#[test]
fn native_memory_fp_retains_rip_targets_dependency_splits_and_explicit_policy_boundaries() {
    // Old register-only requests, a memory request without full-YMM opt-in,
    // scalar/divide/EVEX neighbors and addr32 do not acquire this authority.
    for (code, requested) in [
        (
            vec![0xc4, 0x61, 0x14, 0x59, 0x0b, 0xc3],
            TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
                .with_native_ymm_registers(),
        ),
        (
            vec![0xc4, 0x61, 0x14, 0x59, 0x0b, 0xc3],
            TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
                .with_native_packed_float_memory(),
        ),
        (vec![0xc4, 0x61, 0x12, 0x58, 0x0b, 0xc3], policy()), // Scalar VADDSS.
        (vec![0xc4, 0x61, 0x14, 0x5e, 0x0b, 0xc3], policy()), // Packed VDIVPS.
        (vec![0x62, 0x71, 0x14, 0x28, 0x58, 0x0b, 0xc3], policy()), // EVEX VADDPS.
        (vec![0x67, 0xc4, 0x61, 0x14, 0x59, 0x0b, 0xc3], policy()),
    ] {
        with_source(Some(&code), |source, arena, stack, state| {
            let engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = plan_materialized_block(
                source,
                0,
                state.guest_pc,
                &code,
                engine.placement(),
                &requested,
                &BlockLimits::default(),
            )
            .unwrap();
            assert_eq!(planned.instructions()[0].emitted_len(), 0);
            assert!(matches!(planned.exit().kind(), ExitKind::Mediated { .. }));
        });
    }
    if VectorMode::detect().features().is_none() {
        return;
    }
    // The source target is explicit; relocating the native cache must neither
    // substitute its RIP nor turn a numeric address into a permission proof.
    let mut rip_code = vec![0xc4, 0x61, 0x14, 0x59, 0x0d];
    let displacement = i32::try_from((DATA_OFF + 128) as i64 - (ENTRY + 9) as i64).unwrap();
    rip_code.extend(displacement.to_le_bytes());
    rip_code.push(0xc3);
    with_source(Some(&rip_code), |source, arena, stack, state| {
        let target = source.source_plan().modules()[0].load_bias() + DATA_OFF + 128;
        let left = std::array::from_fn(|i| 0x40c0_0000_u32.to_le_bytes()[i % 4]);
        let right = std::array::from_fn(|i| 0x4000_0000_u32.to_le_bytes()[i % 4]);
        arena.pause().unwrap().write(target, &right).unwrap();
        arena
            .protect(target & !(PAGE - 1), PAGE, NativePermissions::READ)
            .unwrap();
        let current = input(state, &left, 42, 0x1f84);
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            current,
            stack,
            &Default::default(),
        )
        .unwrap();
        let planned = plan(source, current.guest_pc, &rip_code, engine.placement());
        assert_eq!(
            planned.instructions()[0].rip_relative_target(),
            Some(target)
        );
        let emitted = Decoder::with_ip(
            64,
            planned.bytes(),
            planned.placement().address,
            DecoderOptions::NONE,
        )
        .decode();
        assert_eq!(emitted.memory_base(), Register::RIP);
        assert_eq!(emitted.ip_rel_memory_address(), target);
        // SAFETY: this original RIP reference was independently checked against
        // the same readable owned target used by the relocated native body.
        unsafe { engine.step_checked(&mut arena.pause().unwrap(), &planned) }.unwrap();
        assert_eq!(
            *engine.state(),
            expected(current, &right, 0x59, false, true, 9)
        );
        assert_eq!(engine.usage().memory_bytes_checked, 32);
    });
    let mut dependent = vec![0x48, 0x89, 0xcb]; // MOV RBX,RCX.
    dependent.extend(instruction(0x59, false, true, 9));
    dependent.push(0xc3);
    with_source(Some(&dependent), |source, arena, stack, state| {
        let target = source.source_plan().modules()[0].load_bias() + DATA_OFF + 128;
        let left = std::array::from_fn(|i| 0x40c0_0000_u32.to_le_bytes()[i % 4]);
        let right = std::array::from_fn(|i| 0x4000_0000_u32.to_le_bytes()[i % 4]);
        arena.pause().unwrap().write(target, &right).unwrap();
        let mut current = input(state, &left, 42, 0x1f84);
        current.gpr[RCX] = target;
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            current,
            stack,
            &Default::default(),
        )
        .unwrap();
        let first = plan(source, current.guest_pc, &dependent, engine.placement());
        assert_eq!(first.instructions().len(), 1);
        assert!(matches!(first.exit().kind(), ExitKind::Fallthrough { .. }));
        // SAFETY: the first body only moves a copied GPR; the dependent read
        // remains unconsumed until its new address is independently observed.
        unsafe { engine.step_checked(&mut arena.pause().unwrap(), &first) }.unwrap();
        current.gpr[BASE] = target;
        current.guest_pc += 3;
        assert_eq!(*engine.state(), current);
        let second = plan(
            source,
            current.guest_pc,
            &dependent[3..],
            engine.placement(),
        );
        // SAFETY: the second original body uses the freshly observed RBX and
        // the usual live full32-byte read/owner/source/context admission.
        unsafe { engine.step_checked(&mut arena.pause().unwrap(), &second) }.unwrap();
        assert_eq!(
            *engine.state(),
            expected(current, &right, 0x59, false, true, 9)
        );
        assert_eq!(engine.usage().native_entries, 2);
        assert_eq!(engine.usage().native_instructions, 2);
        assert_eq!(engine.usage().memory_access_checks, 1);
    });
}
