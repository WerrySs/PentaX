// SPDX-License-Identifier: GPL-2.0-only
//! Original lane-local selectors, aliases, exact reads and atomic refusals.

use super::*;

fn instruction(
    variable: bool,
    double: bool,
    wide: bool,
    destination: u8,
    memory: bool,
    immediate: u8,
) -> Vec<u8> {
    let mut bytes = vec![
        0xc4,
        (if memory { 0x60 } else { 0x40 }) | if variable { 2 } else { 3 },
        (if variable { 0x09 } else { 0x79 }) | if wide { 4 } else { 0 },
        if variable { 0x0c } else { 0x04 } + u8::from(double),
        ((destination & 7) << 3) | if memory { 0 } else { 0xc5 },
    ];
    if !variable {
        bytes.push(immediate);
    }
    bytes
}
fn data() -> [u8; 32] {
    std::array::from_fn(|i| (i as u8).wrapping_mul(17).wrapping_add(3))
}
fn controls(double: bool, seed: u8) -> [u8; 32] {
    let mut bytes = [0; 32];
    if double {
        for (i, slot) in bytes.chunks_exact_mut(8).enumerate() {
            let value = 0xf135_79bd_2468_ace0_u64
                | (u64::from((seed >> i) & 1) << 1)
                | u64::from((seed >> (i + 4)) & 1);
            slot.copy_from_slice(&value.to_le_bytes());
        }
    } else {
        for (i, slot) in bytes.chunks_exact_mut(4).enumerate() {
            let index = ((seed >> ((i % 4) * 2)) & 3) ^ if i >= 4 { 3 } else { 0 };
            slot.copy_from_slice(&(0xf135_79bc_u32 | u32::from(index)).to_le_bytes());
        }
    }
    bytes
}
fn expected(
    variable: bool,
    double: bool,
    wide: bool,
    input: &[u8; 32],
    control: &[u8; 32],
    immediate: u8,
) -> [u8; 32] {
    let size = if wide { 32 } else { 16 };
    let element = if double { 8 } else { 4 };
    let mut output = [0; 32];
    for (index, slot) in output[..size].chunks_exact_mut(element).enumerate() {
        let selected = if variable {
            if double {
                ((control[index * 8] >> 1) & 1) as usize
            } else {
                (control[index * 4] & 3) as usize
            }
        } else if double {
            usize::from((immediate >> index) & 1)
        } else {
            usize::from((immediate >> ((index % 4) * 2)) & 3)
        };
        let offset = (index * element / 16) * 16 + selected * element;
        slot.copy_from_slice(&input[offset..offset + element]);
    }
    output
}
fn put(state: &mut NativeCpuState, register: usize, value: &[u8; 32]) {
    let offset = register * 16;
    state.xmm[offset..offset + 16].copy_from_slice(&value[..16]);
    state.ymm_upper[offset..offset + 16].copy_from_slice(&value[16..]);
}

fn bounded_block<'view, 'plan, 'image>(
    source: &'view MaterializedSource<'plan, 'image>,
    memory: &PausedNativeMemory<'_>,
    pc: u64,
    placement: CachePlacement,
) -> MaterializedBlockPlan<'view, 'plan, 'image> {
    let module = &source.source_plan().modules()[0];
    let segment = &module.runtime_plan().segments[0];
    let end = module.load_bias() + segment.vaddr + segment.file_bytes.len() as u64;
    let limits = BlockLimits::default();
    // This immutable multi-shape program is larger than one planning window.
    // Admit only the existing bounded window, rather than its entire tail.
    let mut bytes = vec![0; (end - pc).min(limits.max_source_bytes) as usize];
    memory.fetch_source(pc, &mut bytes).unwrap();
    plan_materialized_block(
        source,
        0,
        pc,
        &bytes,
        placement,
        &TranslationPolicy::x64_sse2(),
        &limits,
    )
    .unwrap()
}

#[test]
fn element_permutations_all_immediates_and_variable_decoys_are_lane_local_and_alias_safe() {
    let input = data();
    for variable in [false, true] {
        for double in [false, true] {
            for wide in [false, true] {
                let shapes: &[(u8, bool)] = if variable {
                    &[(14, false), (13, false), (9, false), (14, true)]
                } else {
                    &[(13, false), (9, false), (9, true)]
                };
                let mut code = Vec::new();
                for &(destination, memory) in shapes {
                    for seed in 0..=255 {
                        code.extend(instruction(
                            variable,
                            double,
                            wide,
                            destination,
                            memory,
                            seed,
                        ));
                    }
                }
                code.push(0xc3);
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let end = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE;
                    let size = if wide { 32 } else { 16 };
                    state.gpr[RAX] = end - 33;
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
                    let mut operations = 0_u64;
                    let mut transferred = 0_u64;
                    for &(destination, memory) in shapes {
                        for seed in 0..=255_u8 {
                            let control = controls(double, seed);
                            let mut neighbors = [0xa5; 64];
                            let payload = if variable { &control } else { &input };
                            neighbors[31..31 + size].copy_from_slice(&payload[..size]);
                            arena.pause().unwrap().write(end - 64, &neighbors).unwrap();
                            let mut before = engine.machine.state;
                            put(&mut before, usize::from(destination), &[0xee; 32]);
                            if variable {
                                put(&mut before, 14, &input);
                                put(&mut before, 13, &control);
                            } else {
                                put(&mut before, 13, &input);
                            }
                            engine.machine.state = before;
                            let planned = bounded_block(
                                source,
                                &arena.pause().unwrap(),
                                before.guest_pc,
                                engine.placement(),
                            );
                            let mut after = before;
                            after.guest_pc = planned.exit().next_guest_pc();
                            put(
                                &mut after,
                                usize::from(destination),
                                &expected(variable, double, wide, &input, &control, seed),
                            );
                            engine
                                .resolve(&mut arena.pause().unwrap(), &planned)
                                .unwrap();
                            operations += 1;
                            transferred += if memory { size as u64 } else { 0 };
                            assert_eq!(
                                engine.machine.state, after,
                                "var={variable} pd={double} wide={wide} dst={destination} mem={memory} control={seed}"
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
    }
}

#[test]
fn element_permutations_exact_width_reads_and_failures_preserve_state_memory_and_counters() {
    let input = data();
    for variable in [false, true] {
        for double in [false, true] {
            for wide in [false, true] {
                let size = if wide { 32 } else { 16 };
                for case in 0..6 {
                    let mut code = instruction(variable, double, wide, 14, true, 0);
                    code.push(0xc3);
                    with_source(Some(&code), |source, arena, stack, mut state| {
                        let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                        let end = data + PAGE;
                        let mut backing = [0xa5; 32];
                        // All variable controls select the first element of their own
                        // lane; all bytes still belong to the architectural input.
                        let payload = if variable { [0; 32] } else { input };
                        backing[32 - size..].copy_from_slice(&payload[..size]);
                        arena.pause().unwrap().write(end - 32, &backing).unwrap();
                        state.gpr[RAX] = end - size as u64 + u64::from(case == 1);
                        state.xmm.fill(0xcc);
                        state.ymm_upper.fill(0xdd);
                        put(&mut state, 14, &input);
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
                            2 => arena
                                .protect(data, PAGE, NativePermissions::new(false, false, false))
                                .unwrap(),
                            3 => {
                                engine.machine.limits.max_work_units =
                                    engine.machine.usage.work_units
                            }
                            4 => engine.machine.usage.vector_bytes = u64::MAX - size as u64 + 1,
                            5 => engine.machine.usage.vector_operations = u64::MAX,
                            _ => {}
                        }
                        let usage = engine.usage();
                        engine.machine.phase = Phase::Terminal;
                        let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                        let mut after = state;
                        if case == 0 {
                            result.unwrap();
                            after.guest_pc = planned.exit().next_guest_pc();
                            put(
                                &mut after,
                                14,
                                &expected(variable, double, wide, &input, &[0; 32], 0),
                            );
                            assert_eq!(engine.usage().vector_operations, 1);
                            assert_eq!(engine.usage().vector_bytes, size as u64);
                        } else {
                            match case {
                                1 | 2 => {
                                    assert!(matches!(result, Err(NativeDispatchError::Memory(_))))
                                }
                                3..=5 => {
                                    assert!(matches!(result, Err(NativeDispatchError::Budget(_))))
                                }
                                _ => unreachable!(),
                            }
                            assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                            assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
                            assert!(engine.machine.phase == Phase::Terminal);
                        }
                        assert_eq!(engine.machine.state, after);
                        if case == 2 {
                            arena.protect(data, PAGE, NativePermissions::READ).unwrap();
                        }
                        let mut actual = [0; 32];
                        arena.pause().unwrap().read(end - 32, &mut actual).unwrap();
                        assert_eq!(actual, backing);
                    });
                }
            }
        }
    }
}

#[test]
fn element_permutation_pd_variable_hardware_confirms_bit_one_and_ignored_decoys() {
    if !std::is_x86_feature_detected!("avx") {
        println!("element_permutation_oracle_available=false element_permutation_oracle_cases=0");
        return;
    }
    let input = data();
    let patterns = [
        [0_u64; 4],
        [1; 4],
        [2; 4],
        [3; 4],
        [
            0xfedc_ba98_7654_32e1,
            0xfedc_ba98_7654_32e2,
            0xfedc_ba98_7654_32e3,
            0xfedc_ba98_7654_32e0,
        ],
        [u64::MAX, u64::MAX - 1, u64::MAX - 2, u64::MAX - 3],
    ];
    let mut cases = 0;
    for wide in [false, true] {
        for words in patterns {
            let mut control = [0; 32];
            for (slot, value) in control.chunks_exact_mut(8).zip(words) {
                slot.copy_from_slice(&value.to_le_bytes());
            }
            let mut actual = [0_u8; 32];
            let mut before = 0_u32;
            let mut after = 0_u32;
            // SAFETY: AVX+OS support was detected. All operands are original
            // bounded arrays; every clobbered vector register is declared. This
            // block only reads MXCSR and never changes its host control profile.
            unsafe {
                if wide {
                    core::arch::asm!("vmovdqu ymm1, [{input}]", "vmovdqu ymm2, [{control}]",
                    "stmxcsr [{before}]", "vpermilpd ymm0, ymm1, ymm2",
                    "vmovdqu [{actual}], ymm0", "stmxcsr [{after}]",
                    input = in(reg) input.as_ptr(), control = in(reg) control.as_ptr(),
                    actual = in(reg) actual.as_mut_ptr(), before = in(reg) &mut before,
                    after = in(reg) &mut after, out("ymm0") _, out("ymm1") _, out("ymm2") _,
                    options(nostack, preserves_flags));
                } else {
                    core::arch::asm!("vmovdqu ymm1, [{input}]", "vmovdqu ymm2, [{control}]",
                    "stmxcsr [{before}]", "vpermilpd xmm0, xmm1, xmm2",
                    "vmovdqu [{actual}], ymm0", "stmxcsr [{after}]",
                    input = in(reg) input.as_ptr(), control = in(reg) control.as_ptr(),
                    actual = in(reg) actual.as_mut_ptr(), before = in(reg) &mut before,
                    after = in(reg) &mut after, out("ymm0") _, out("ymm1") _, out("ymm2") _,
                    options(nostack, preserves_flags));
                }
            }
            assert_eq!(
                actual,
                expected(true, true, wide, &input, &control, 0),
                "wide={wide} controls={words:?}"
            );
            assert_eq!(before, after);
            cases += 1;
        }
    }
    println!("element_permutation_oracle_available=true element_permutation_oracle_cases={cases}");
}
