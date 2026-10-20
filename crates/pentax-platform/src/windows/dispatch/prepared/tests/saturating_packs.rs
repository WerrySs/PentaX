// SPDX-License-Identifier: GPL-2.0-only
//! Original independent pack boundaries and source-order/ownership proofs.
use super::*;

fn encode<const N: usize>(values: [i32; N], width: usize) -> [u8; 32] {
    assert!(N * width <= 32);
    let mut bytes = [0; 32];
    for (slot, value) in bytes.chunks_exact_mut(width).zip(values) {
        slot.copy_from_slice(&value.to_le_bytes()[..width]);
    }
    bytes
}
fn fixture(family: usize) -> ([u8; 32], [u8; 32], [u8; 32]) {
    match family {
        0 => (
            encode(
                [
                    -32768, -129, -128, -127, 0, 126, 127, 128, 12, 13, 14, 15, 16, 17, 18, 19,
                ],
                2,
            ),
            encode(
                [
                    32767, -300, -1, 1, 100, 255, -256, 42, 20, 21, 22, 23, 24, 25, 26, 27,
                ],
                2,
            ),
            encode(
                [
                    -128, -128, -128, -127, 0, 126, 127, 127, 127, -128, -1, 1, 100, 127, -128, 42,
                    12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
                ],
                1,
            ),
        ),
        1 => (
            encode([i32::MIN, -32769, -32768, 32767, 100, 200, 300, 400], 4),
            encode([i32::MAX, 32768, -1, 0, -100, -200, -300, -400], 4),
            encode(
                [
                    -32768, -32768, -32768, 32767, 32767, 32767, -1, 0, 100, 200, 300, 400, -100,
                    -200, -300, -400,
                ],
                2,
            ),
        ),
        2 => (
            encode(
                [
                    -32768, -1, 0, 1, 254, 255, 256, 32767, 1, 2, 3, 4, 5, 6, 7, 8,
                ],
                2,
            ),
            encode(
                [
                    -256, -2, 2, 127, 128, 254, 255, 256, 9, 10, 11, 12, 13, 14, 15, 16,
                ],
                2,
            ),
            encode(
                [
                    0, 0, 0, 1, 254, 255, 255, 255, 0, 0, 2, 127, 128, 254, 255, 255, 1, 2, 3, 4,
                    5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
                ],
                1,
            ),
        ),
        _ => (
            encode([i32::MIN, -1, 0, 65535, 10, 20, 30, 40], 4),
            encode([i32::MAX, 65536, 65534, 1, 50, 60, 70, 80], 4),
            encode(
                [
                    0, 0, 0, 65535, 65535, 65535, 65534, 1, 10, 20, 30, 40, 50, 60, 70, 80,
                ],
                2,
            ),
        ),
    }
}
fn instruction(family: usize, form: usize, destination: u8, memory: bool, wide: bool) -> Vec<u8> {
    let opcode = [0x63, 0x6b, 0x67, 0x2b][family];
    let mut bytes = if form == 0 {
        let mut bytes = vec![0x66, if memory { 0x44 } else { 0x45 }, 0x0f];
        if family == 3 {
            bytes.push(0x38);
        }
        bytes.push(opcode);
        bytes
    } else {
        vec![
            0xc4,
            (if memory { 0x61 } else { 0x41 }) + u8::from(family == 3),
            0x11 | if form == 2 { 4 } else { 0 } | if wide { 0x80 } else { 0 },
            opcode,
        ]
    };
    bytes.push(((destination & 7) << 3) | if memory { 0 } else { 0xc6 });
    bytes
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
    // Exercise the checked Rust mediator for the three legacy SSE2 forms too.
    // Production policy and their existing native authority remain unchanged.
    let planned = plan_materialized_block(
        source,
        0,
        pc,
        &bytes,
        placement,
        &TranslationPolicy::new(pentax_codegen::CpuFeatures::empty()),
        &BlockLimits::default(),
    )
    .unwrap();
    assert!(matches!(planned.exit().kind(), ExitKind::Mediated { .. }));
    assert_eq!(planned.instructions()[0].emitted_len(), 0);
    planned
}

#[test]
fn all_saturating_packs_clip_signed_sources_keep_lane_order_and_snapshot_aliases() {
    let shapes = [(9, false), (13, false), (14, false), (9, true)];
    for family in 0..4 {
        let (left, right, packed) = fixture(family);
        for form in 0..3 {
            let size = if form == 2 { 32 } else { 16 };
            let mut code = Vec::new();
            for (index, (destination, memory)) in shapes.into_iter().enumerate() {
                code.extend(instruction(
                    family,
                    form,
                    destination,
                    memory,
                    index % 2 != 0,
                ));
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
                    let planned = mediated(
                        source,
                        &arena.pause().unwrap(),
                        before.guest_pc,
                        engine.placement(),
                    );
                    let mut output = packed;
                    if form == 0 && destination == 14 && !memory {
                        // Identical destructive sources duplicate the fixed
                        // expected SRC2 half, rather than recomputing saturation.
                        for lane in (0..size).step_by(16) {
                            output[lane..lane + 8].copy_from_slice(&packed[lane + 8..lane + 16]);
                        }
                    }
                    if size == 16 {
                        output[16..].fill(0);
                    }
                    let mut after = before;
                    let slot = usize::from(destination) * 16;
                    after.xmm[slot..slot + 16].copy_from_slice(&output[..16]);
                    if form != 0 {
                        after.ymm_upper[slot..slot + 16].copy_from_slice(&output[16..]);
                    }
                    after.guest_pc = planned.exit().next_guest_pc();
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .unwrap();
                    assert_eq!(
                        engine.machine.state, after,
                        "family={family} form={form} destination={destination}"
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
}

#[test]
fn saturating_pack_full_reads_prefix_target_and_budget_refusals_publish_no_cpu_outputs() {
    for form in 0..3 {
        let size = if form == 2 { 32 } else { 16 };
        // All families share one checked admission path; distribute refusal
        // categories across them rather than repeating the same Cartesian test.
        for case in 0..10 {
            let family = case % 4;
            let mut code = instruction(family, form, 9, true, false);
            if case == 7 {
                code.insert(0, 0x64);
            }
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let end = data + PAGE;
                let backing = [0x80; 32]; // Saturated first elements still require every source byte.
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
                let planned = mediated(
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
                let outcome = engine.resolve(&mut arena.pause().unwrap(), &planned);
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
