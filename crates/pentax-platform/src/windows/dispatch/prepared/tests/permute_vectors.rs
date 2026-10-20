// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn modeled_full_width_permutations_select_across_halves_and_snapshot_aliases() {
    let indices = [
        0xffff_fffc_u32,
        0x8000_0007,
        0xffff_fffe,
        0x4000_0000,
        0xffff_fff9,
        0x8000_0003,
        0x7fff_fffa,
        0x1234_567d,
    ];
    let input = [
        0x7f80_0001_u32,
        0x8000_0000,
        0x0000_0001,
        0xff80_0000,
        0x7fc0_1234,
        0x3f80_0000,
        0xffc0_5678,
        0x0080_0000,
    ];
    let selection = [4, 7, 6, 0, 1, 3, 2, 5];
    for opcode in [0x16, 0x36] {
        for destination in [0, 1, 2, 15] {
            for form in 0..4 {
                let memory = form >= 2;
                let source2 = if form == 1 { 1 } else { 2 };
                let code = [
                    0xc4,
                    if destination < 8 { 0xe2 } else { 0x62 },
                    0x75,
                    opcode,
                    ((destination & 7) << 3) | if memory { 0 } else { 0xc0 | source2 },
                    0xc3,
                ];
                with_source(Some(&code), |source, arena, stack, mut state| {
                    let data = source.source_plan().modules()[0].load_bias()
                        + DATA_OFF
                        + if form == 3 { 3 } else { PAGE - 32 };
                    let bytes: Vec<_> = input.into_iter().flat_map(u32::to_le_bytes).collect();
                    let controls: Vec<_> = indices.into_iter().flat_map(u32::to_le_bytes).collect();
                    arena.pause().unwrap().write(data, &bytes).unwrap();
                    state.gpr[RAX] = data;
                    state.xmm.fill(0xa5);
                    state.ymm_upper.fill(0x5a);
                    state.xmm[16..32].copy_from_slice(&controls[..16]);
                    state.ymm_upper[16..32].copy_from_slice(&controls[16..]);
                    state.xmm[32..48].copy_from_slice(&bytes[..16]);
                    state.ymm_upper[32..48].copy_from_slice(&bytes[16..]);
                    state.rflags |= 0x8d5;
                    state.mxcsr |= 0x807f;
                    let mut expected = state;
                    let table = if form == 1 { indices } else { input };
                    let output: Vec<_> = selection
                        .into_iter()
                        .flat_map(|index| table[index].to_le_bytes())
                        .collect();
                    let slot = usize::from(destination) * 16;
                    expected.xmm[slot..slot + 16].copy_from_slice(&output[..16]);
                    expected.ymm_upper[slot..slot + 16].copy_from_slice(&output[16..]);
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
                    expected.guest_pc = planned.exit().next_guest_pc();
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .unwrap();
                    assert_eq!(engine.machine.state, expected);
                    assert_eq!(engine.machine.usage.vector_operations, 1);
                    assert_eq!(
                        engine.machine.usage.vector_bytes,
                        if memory { 32 } else { 0 }
                    );
                    let mut unchanged = [0; 32];
                    arena.pause().unwrap().read(data, &mut unchanged).unwrap();
                    assert_eq!(unchanged.as_slice(), bytes);
                });
            }
        }
    }
}

#[test]
fn modeled_full_width_permutations_require_the_entire_source_before_publication() {
    for opcode in [0x16, 0x36] {
        for case in 0..5 {
            let code = [0xc4, 0xe2, 0x75, opcode, 0x00, 0xc3];
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                state.gpr[RAX] = if case == 0 { data + PAGE - 31 } else { data };
                // Every index selects only the first dword, but the memory
                // operand still requires all 32 bytes, including unused data.
                state.xmm[16..32].fill(0);
                state.ymm_upper[16..32].fill(0);
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
                    1 => arena
                        .protect(data, PAGE, NativePermissions::new(false, false, false))
                        .unwrap(),
                    2 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                    3 => engine.machine.usage.vector_bytes = u64::MAX,
                    4 => engine.machine.usage.vector_operations = u64::MAX,
                    _ => {}
                }
                let before = engine.machine.usage;
                engine.machine.phase = Phase::Terminal;
                assert!(
                    engine
                        .resolve(&mut arena.pause().unwrap(), &planned)
                        .is_err()
                );
                assert_eq!(engine.machine.state, state);
                assert_eq!(
                    engine.machine.usage.vector_operations,
                    before.vector_operations
                );
                assert_eq!(engine.machine.usage.vector_bytes, before.vector_bytes);
                assert!(engine.machine.phase == Phase::Terminal);
            });
        }
    }
}

#[test]
fn modeled_full_width_permutations_refuse_vex128_w1_and_evex_forms() {
    for (case, code) in [
        &[0xc4, 0xe2, 0x71, 0x16, 0xc2, 0xc3][..],
        &[0xc4, 0xe2, 0xf5, 0x16, 0xc2, 0xc3][..],
        &[0xc4, 0xe2, 0x71, 0x36, 0xc2, 0xc3][..],
        &[0xc4, 0xe2, 0xf5, 0x36, 0xc2, 0xc3][..],
        &[0x62, 0xf2, 0x75, 0x28, 0x16, 0xc2, 0xc3][..],
        &[0x62, 0xf2, 0x75, 0x28, 0x36, 0xc2, 0xc3][..],
    ]
    .into_iter()
    .enumerate()
    {
        with_source(Some(code), |source, arena, stack, state| {
            let engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let instruction = plan_materialized_block(
                source,
                0,
                state.guest_pc,
                code,
                engine.placement(),
                &TranslationPolicy::x64_sse2(),
                &BlockLimits::default(),
            );
            if case >= 4 {
                assert!(
                    instruction.is_ok(),
                    "the EVEX refusal must cover a valid decoded form"
                );
            }
            // Invalid encodings may be rejected at decode. Valid EVEX or W1
            // instruction variants must never enter this VEX.256 handler.
            if let Ok(decoded) = instruction {
                assert!(!super::super::vectors::supported(
                    decoded.exit().instruction().unwrap()
                ));
            }
            assert_eq!(engine.machine.state, state);
            assert_eq!(engine.machine.usage.vector_operations, 0);
        });
    }
}
