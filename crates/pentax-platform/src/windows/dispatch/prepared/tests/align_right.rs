// SPDX-License-Identifier: GPL-2.0-only
//! Original paused-backing proofs reuse bounded immutable programs for all imm8.
use super::*;

fn encoding(form: usize, destination: u8, memory: bool, immediate: u8) -> Vec<u8> {
    let mut code = if form == 0 {
        vec![0x66, if memory { 0x44 } else { 0x45 }, 0x0f, 0x3a, 0x0f]
    } else {
        vec![
            0xc4,
            if memory { 0x63 } else { 0x43 },
            if form == 2 { 0x15 } else { 0x11 },
            0x0f,
        ]
    };
    code.extend([
        ((destination & 7) << 3) | if memory { 0 } else { 0xc6 },
        immediate,
    ]);
    code
}
fn put(state: &mut NativeCpuState, register: u8, bytes: &[u8; 32]) {
    let slot = usize::from(register) * 16;
    state.xmm[slot..slot + 16].copy_from_slice(&bytes[..16]);
    state.ymm_upper[slot..slot + 16].copy_from_slice(&bytes[16..]);
}
fn oracle(high: &[u8; 32], low: &[u8; 32], size: usize, immediate: u8) -> [u8; 32] {
    let mut output = [0; 32];
    if immediate < 32 {
        for lane in (0..size).step_by(16) {
            let mut concatenated = [0; 48];
            concatenated[..16].copy_from_slice(&low[lane..lane + 16]);
            concatenated[16..32].copy_from_slice(&high[lane..lane + 16]);
            let offset = usize::from(immediate);
            output[lane..lane + 16].copy_from_slice(&concatenated[offset..offset + 16]);
        }
    }
    output
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
fn all_align_immediates_snapshot_both_aliases_and_independent_lanes_with_complete_state() {
    let high = std::array::from_fn(|i| i as u8 + 0x81);
    let low = std::array::from_fn(|i| i as u8 + 1);
    for form in 0..3 {
        for (destination, memory) in [(13, false), (14, false), (9, false), (9, true)] {
            let mut code = Vec::new();
            for immediate in 0..=255 {
                code.extend(encoding(form, destination, memory, immediate));
            }
            code.push(0xc3);
            // Each setup has only256 original instructions; planning sees at
            // most the existing source cap, never the entire oversized tail.
            with_source(Some(&code), |source, arena, stack, mut state| {
                let size = if form == 2 { 32 } else { 16 };
                let end = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE;
                state.gpr[RAX] = end - size as u64 - u64::from(form != 0);
                state.mxcsr = 0x1fa5;
                state.rflags = 0x602 | 0x8d5;
                state.xmm.fill(0xaa);
                state.ymm_upper.fill(0xbb);
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    state,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                let mut backing = [0x5a; 64];
                let offset = (state.gpr[RAX] - (end - 64)) as usize;
                backing[offset..offset + size].copy_from_slice(&low[..size]);
                arena.pause().unwrap().write(end - 64, &backing).unwrap();
                for immediate in 0..=255_u8 {
                    let mut before = engine.machine.state;
                    put(&mut before, destination, &high);
                    if form != 0 {
                        put(&mut before, 13, &high);
                    }
                    put(&mut before, 14, &low);
                    engine.machine.state = before;
                    let high_input = if form == 0 && destination == 14 {
                        &low
                    } else {
                        &high
                    };
                    let mut expected = oracle(high_input, &low, size, immediate);
                    let slot = usize::from(destination) * 16;
                    if form == 0 {
                        expected[16..].copy_from_slice(&before.ymm_upper[slot..slot + 16]);
                    }
                    let plan = bounded_block(
                        source,
                        &arena.pause().unwrap(),
                        before.guest_pc,
                        engine.placement(),
                    );
                    let mut after = before;
                    put(&mut after, destination, &expected);
                    after.guest_pc = plan.exit().next_guest_pc();
                    engine.resolve(&mut arena.pause().unwrap(), &plan).unwrap();
                    assert_eq!(
                        engine.machine.state, after,
                        "form={form} dst={destination} mem={memory} imm={immediate}"
                    );
                    assert_eq!(engine.usage().vector_operations, u64::from(immediate) + 1);
                    assert_eq!(
                        engine.usage().vector_bytes,
                        if memory {
                            (u64::from(immediate) + 1) * size as u64
                        } else {
                            0
                        }
                    );
                }
                let mut unchanged = [0; 64];
                arena
                    .pause()
                    .unwrap()
                    .read(end - 64, &mut unchanged)
                    .unwrap();
                assert_eq!(unchanged, backing);
            });
        }
    }
}

#[test]
fn zero_result_still_requires_full_operand_alignment_permissions_and_atomic_admission() {
    for form in 0..3 {
        for case in 0..9 {
            let mut code = encoding(form, 9, true, 255);
            if case == 8 {
                code.insert(0, 0x64);
            }
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let end = data + PAGE;
                let size = if form == 2 { 32 } else { 16 };
                let backing = [0x7b; 64];
                arena.pause().unwrap().write(end - 64, &backing).unwrap();
                state.gpr[RAX] = match case {
                    1 => end,
                    2 => end - size as u64 + 1,
                    3 => end - size as u64 - 1,
                    _ => end - size as u64,
                };
                state.xmm.fill(0xa5);
                state.ymm_upper.fill(0x9b);
                state.mxcsr = 0x1fa5;
                state.rflags = 0x602 | 0x8d5;
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    state,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                let plan = block(
                    source,
                    &arena.pause().unwrap(),
                    state.guest_pc,
                    engine.placement(),
                );
                match case {
                    4 => arena
                        .protect(data, PAGE, NativePermissions::new(false, false, false))
                        .unwrap(),
                    5 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                    6 => engine.machine.usage.vector_bytes = u64::MAX - size as u64 + 1,
                    7 => engine.machine.usage.vector_operations = u64::MAX,
                    _ => {}
                }
                let usage = engine.usage();
                engine.machine.phase = Phase::Terminal;
                let result = engine.resolve(&mut arena.pause().unwrap(), &plan);
                let mut after = state;
                if case == 0 || case == 3 && form != 0 {
                    result.unwrap();
                    after.guest_pc = plan.exit().next_guest_pc();
                    after.xmm[144..160].fill(0);
                    if form != 0 {
                        after.ymm_upper[144..160].fill(0);
                    }
                    assert_eq!(engine.usage().vector_operations, 1);
                    assert_eq!(engine.usage().vector_bytes, size as u64);
                } else {
                    assert!(result.is_err(), "form={form} case={case}");
                    assert_eq!(engine.usage().vector_operations, usage.vector_operations);
                    assert_eq!(engine.usage().vector_bytes, usage.vector_bytes);
                    assert!(engine.machine.phase == Phase::Terminal);
                }
                assert_eq!(engine.machine.state, after, "form={form} case={case}");
                if case == 4 {
                    arena.protect(data, PAGE, NativePermissions::READ).unwrap();
                }
                let mut unchanged = [0; 64];
                arena
                    .pause()
                    .unwrap()
                    .read(end - 64, &mut unchanged)
                    .unwrap();
                assert_eq!(unchanged, backing);
            });
        }
    }
}
