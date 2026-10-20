// SPDX-License-Identifier: GPL-2.0-only
//! Original pairwise signed-product oracles and atomic admission refusals.
use super::*;

fn instruction(form: usize, destination: u8, memory: bool, wide: bool) -> Vec<u8> {
    let mut code = if form == 0 {
        vec![0x66, if memory { 0x44 } else { 0x45 }, 0x0f, 0xf5]
    } else {
        vec![
            0xc4,
            if memory { 0x61 } else { 0x41 },
            0x11 | if form == 2 { 4 } else { 0 } | if wide { 0x80 } else { 0 },
            0xf5,
        ]
    };
    code.push(((destination & 7) << 3) | if memory { 0 } else { 0xc6 });
    code
}
fn words(values: [i16; 16]) -> [u8; 32] {
    let mut result = [0; 32];
    for (slot, value) in result.chunks_exact_mut(2).zip(values) {
        slot.copy_from_slice(&value.to_le_bytes());
    }
    result
}
fn dwords(values: [i32; 8]) -> [u8; 32] {
    let mut result = [0; 32];
    for (slot, value) in result.chunks_exact_mut(4).zip(values) {
        slot.copy_from_slice(&value.to_le_bytes());
    }
    result
}
fn mediated<'view, 'plan, 'image>(
    source: &'view MaterializedSource<'plan, 'image>,
    memory: &PausedNativeMemory<'_>,
    pc: u64,
    placement: CachePlacement,
) -> MaterializedBlockPlan<'view, 'plan, 'image> {
    let module = &source.source_plan().modules()[0];
    let segment = &module.runtime_plan().segments[0];
    let end = module.load_bias() + segment.vaddr + segment.file_bytes.len() as u64;
    let mut bytes = vec![0; (end - pc) as usize];
    memory.fetch_source(pc, &mut bytes).unwrap();
    // Only this pure fixture omits SSE2 so legacy executes the Rust path;
    // production keeps its existing native SSE2 admission unchanged.
    let plan = plan_materialized_block(
        source,
        0,
        pc,
        &bytes,
        placement,
        &TranslationPolicy::new(pentax_codegen::CpuFeatures::empty()),
        &BlockLimits::default(),
    )
    .unwrap();
    assert!(matches!(plan.exit().kind(), ExitKind::Mediated { .. }));
    assert_eq!(plan.instructions()[0].emitted_len(), 0);
    plan
}

#[test]
fn multiply_add_words_wraps_only_dword_sums_and_preserves_aliases_lanes_and_upper_rules() {
    let left = words([
        -32768, -32768, 32767, 32767, -32768, 32767, 300, -400, 1, 2, -3, 4, 0, -32768, 32767, -1,
    ]);
    let right = words([
        -32768, -32768, 32767, -32767, 1, 1, -500, -600, 5, 6, 7, -8, -32768, 0, -1, 32767,
    ]);
    let expected = dwords([i32::MIN, 0, -1, 90000, 17, -53, 0, -65534]);
    let identical_right = dwords([
        i32::MIN,
        2147352578,
        2,
        610000,
        61,
        113,
        1073741824,
        1073676290,
    ]);
    let shapes = [(9, false), (13, false), (14, false), (9, true)];
    for form in 0..3 {
        let size = if form == 2 { 32 } else { 16 };
        let mut code = Vec::new();
        for (index, (destination, memory)) in shapes.into_iter().enumerate() {
            code.extend(instruction(form, destination, memory, index % 2 != 0));
        }
        code.push(0xc3);
        with_source(Some(&code), |source, arena, stack, mut state| {
            let end = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE;
            let offset = if form == 0 { 64 - size } else { 63 - size };
            state.gpr[RAX] = end - 64 + offset as u64;
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
            for (index, (destination, memory)) in shapes.into_iter().enumerate() {
                let mut neighbors = [0xa5; 64];
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
                let plan = mediated(
                    source,
                    &arena.pause().unwrap(),
                    before.guest_pc,
                    engine.placement(),
                );
                let mut output = if form == 0 && destination == 14 && !memory {
                    identical_right
                } else {
                    expected
                };
                if size == 16 {
                    output[16..].fill(0);
                }
                let mut after = before;
                let slot = usize::from(destination) * 16;
                after.xmm[slot..slot + 16].copy_from_slice(&output[..16]);
                if form != 0 {
                    after.ymm_upper[slot..slot + 16].copy_from_slice(&output[16..]);
                }
                after.guest_pc = plan.exit().next_guest_pc();
                engine.resolve(&mut arena.pause().unwrap(), &plan).unwrap();
                assert_eq!(
                    engine.machine.state, after,
                    "form={form} destination={destination} memory={memory}"
                );
                assert_eq!(engine.usage().vector_operations, index as u64 + 1);
                assert_eq!(
                    engine.usage().vector_bytes,
                    if memory { size as u64 } else { 0 }
                );
                let mut actual = [0; 64];
                arena.pause().unwrap().read(end - 64, &mut actual).unwrap();
                assert_eq!(actual, neighbors);
            }
        });
    }
}

#[test]
fn multiply_add_words_requires_complete_source_and_refuses_before_cpu_publication() {
    for form in 0..3 {
        let size = if form == 2 { 32 } else { 16 };
        for case in 0..10 {
            let mut code = instruction(form, 9, true, false);
            if case == 7 {
                code.insert(0, 0x64);
            }
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let end = data + PAGE;
                let backing = words([-32768; 16]);
                arena
                    .pause()
                    .unwrap()
                    .write(end - size as u64, &backing[..size])
                    .unwrap();
                state.gpr[RAX] = match case {
                    1 => end,
                    2 => end - size as u64 + 1,
                    8 => 0x0000_8000_0000_0000,
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
                let plan = mediated(
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
                    9 => arena
                        .protect(
                            source.source_plan().modules()[0].load_bias(),
                            PAGE,
                            NativePermissions::READ,
                        )
                        .unwrap(),
                    _ => {}
                }
                let usage = engine.usage();
                engine.machine.phase = Phase::Terminal;
                let outcome = engine.resolve(&mut arena.pause().unwrap(), &plan);
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
                        1..=3 | 8 | 9 => {
                            assert!(matches!(outcome, Err(NativeDispatchError::Memory(_))))
                        }
                        4..=6 => assert!(matches!(outcome, Err(NativeDispatchError::Budget(_)))),
                        7 => assert!(matches!(
                            outcome,
                            Err(NativeDispatchError::UnsupportedVectorInstruction)
                        )),
                        _ => unreachable!(),
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
