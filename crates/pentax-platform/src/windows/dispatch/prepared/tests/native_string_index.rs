// SPDX-License-Identifier: GPL-2.0-only
//! Actual VEX hardware versus the independently hardware-checked comparator.
//! No supplied executable, external payload or host pointer is guest input.
use super::*;
use pentax_codegen::{CpuFeatures, CpuidFeature};

fn policy(native: bool) -> TranslationPolicy {
    let base = TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX));
    let selected = if native {
        base.with_native_ymm_registers()
    } else {
        base
    };
    selected.with_independent_memory()
}

fn encoding(left: u8, right: u8, control: u8) -> [u8; 6] {
    [
        0xc4,
        0xe3 ^ (u8::from(left >= 8) << 7) ^ (u8::from(right >= 8) << 5),
        0x79,
        0x63,
        0xc0 | ((left & 7) << 3) | (right & 7),
        control,
    ]
}

fn input(
    mut state: NativeCpuState,
    pc: u64,
    registers: [u8; 2],
    pair: ([u8; 16], [u8; 16]),
) -> NativeCpuState {
    for (index, register) in state.gpr.iter_mut().enumerate() {
        if index != RSP {
            *register = 0xfeed_1234_5678_0000 | index as u64;
        }
    }
    state.gpr[RAX] = 42;
    state.guest_pc = pc;
    state.rflags = 0x602 | 0x8d5;
    state.mxcsr = 0x5fa5;
    for (index, byte) in state.xmm.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(13);
    }
    for (index, byte) in state.ymm_upper.iter_mut().enumerate() {
        *byte = !(index as u8).wrapping_mul(17);
    }
    for (register, bytes) in [(registers[0], pair.0), (registers[1], pair.1)] {
        state.xmm[register as usize * 16..register as usize * 16 + 16].copy_from_slice(&bytes);
    }
    state
}

fn planned<'view, 'plan, 'image>(
    source: &'view MaterializedSource<'plan, 'image>,
    engine: &NativeBlockStepper<'view, 'plan, 'image>,
    pc: u64,
    bytes: &[u8],
    native: bool,
) -> pentax_codegen::MaterializedBlockPlan<'view, 'plan, 'image> {
    plan_materialized_block(
        source,
        0,
        pc,
        bytes,
        engine.placement(),
        &policy(native),
        &BlockLimits::default(),
    )
    .unwrap()
}

#[test]
fn register_string_index_all_controls_match_masked_flags_index_and_complete_state() {
    if VectorMode::detect().features().is_none() {
        println!(
            "native_string_index_available=false native_string_index_cases=0 native_string_index_mode=sse2"
        );
        return;
    }
    let aliases = [[1, 2], [9, 15], [9, 9]];
    let pairs = [
        ([1; 16], [1; 16]),
        ([0; 16], [1; 16]),
        (
            [1, 2, 3, 4, 5, 0, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            [9, 1, 2, 3, 4, 5, 6, 7, 8, 0, 10, 11, 12, 13, 14, 15],
        ),
        (
            [
                0x80, 0xff, 1, 0, 0xff, 0x7f, 0, 0x80, 4, 9, 15, 12, 0, 0, 2, 3,
            ],
            [
                0x80, 0xff, 1, 2, 0xff, 0x7f, 0, 0x80, 3, 9, 15, 20, 0, 0, 3, 2,
            ],
        ),
    ];
    let mut code = Vec::new();
    for [left, right] in aliases {
        for control in 0..=255 {
            code.extend(encoding(left, right, control));
            code.push(0xc3);
        }
    }
    let mut cases = 0_u64;
    with_source(Some(&code), |source, arena, stack, state| {
        let first = input(state, state.guest_pc, aliases[0], pairs[0]);
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            first,
            stack,
            &Default::default(),
        )
        .unwrap();
        // Actual CPU/OS admission above is mandatory; AVX2 is never declared.
        engine.machine.vector_mode = VectorMode::Avx { avx2: false };
        for (alias, registers) in aliases.into_iter().enumerate() {
            for control in 0..=255_u8 {
                let at = (alias * 256 + usize::from(control)) * 7;
                let pc = state.guest_pc + at as u64;
                let block = planned(source, &engine, pc, &code[at..at + 7], true);
                assert_eq!(block.native_admission().emitted_instructions(), 1);
                assert!(block.native_admission().is_memory_free());
                for mut pair in pairs {
                    if registers[0] == registers[1] {
                        pair.1 = pair.0;
                    }
                    let current = input(state, pc, registers, pair);
                    if cases != 0 {
                        engine
                            .begin_call(&mut arena.pause().unwrap(), current)
                            .unwrap();
                    } else {
                        assert_eq!(*engine.state(), current);
                    }
                    let result =
                        super::super::packed_vectors::compare(&pair.0, &pair.1, None, control);
                    let mut expected = current;
                    expected.gpr[RCX] = result.index;
                    expected.rflags = (expected.rflags & !0x8d5) | result.flags;
                    expected.gpr[RSP] += 8;
                    expected.guest_pc = STOP;
                    assert_eq!(
                        // SAFETY: Original immutable code, actual AVX-only mode,
                        // exclusive owned stack and full checked entry admission.
                        unsafe { engine.step_checked(&mut arena.pause().unwrap(), &block) }
                            .unwrap(),
                        NativeBlockStop::Returned { value: 42 }
                    );
                    assert_eq!(
                        *engine.state(),
                        expected,
                        "control={control}, alias={alias}"
                    );
                    cases += 1;
                }
            }
        }
        assert_eq!(engine.usage().blocks, cases);
        assert_eq!(engine.usage().native_entries, cases);
        assert_eq!(engine.usage().native_instructions, cases);
        assert_eq!(engine.usage().vector_operations, 0);
        assert_eq!(engine.usage().vector_bytes, 0);
        assert_eq!(engine.usage().memory_access_checks, 0);
    });
    assert_eq!(cases, 256 * 3 * 4);
    println!(
        "native_string_index_available=true native_string_index_cases={cases} native_string_index_mode=avx"
    );
}

#[test]
fn string_index_native_and_fallback_branches_use_new_flags_and_rcx_addresses() {
    let available = VectorMode::detect().features().is_some();
    // Independent original three-way branch: positive/negative comparisons
    // select literal10/20/30 using the actual new CF/ZF, never a copied return.
    let mut code = vec![0x41, 0xb8, 17, 0, 0, 0]; // MOV R8D,17; preserves flags.
    code.extend(encoding(9, 15, 0x18));
    code.extend([
        0x77, 8, 0x72, 12, 0xb8, 30, 0, 0, 0, 0xc3, 0xb8, 10, 0, 0, 0, 0xc3, 0xb8, 20, 0, 0, 0,
        0xc3,
    ]);
    for native in [false, true] {
        if native && !available {
            continue;
        }
        with_source(Some(&code), |source, arena, stack, state| {
            let initial = input(state, state.guest_pc, [9, 15], ([1; 16], [1; 16]));
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                initial,
                stack,
                &Default::default(),
            )
            .unwrap();
            engine.machine.vector_mode = if native {
                VectorMode::Avx { avx2: false }
            } else {
                VectorMode::Sse2
            };
            for (case, (pair, value, blocks)) in [
                (([1; 16], [1; 16]), 10, 2),
                (([1; 16], [2; 16]), 20, 3),
                (([0; 16], [0; 16]), 30, 3),
            ]
            .into_iter()
            .enumerate()
            {
                let current = input(state, state.guest_pc, [9, 15], pair);
                if case == 0 {
                    assert_eq!(*engine.state(), current);
                } else {
                    engine
                        .begin_call(&mut arena.pause().unwrap(), current)
                        .unwrap();
                }
                let before = engine.usage();
                let returned = loop {
                    let at = (engine.state().guest_pc - state.guest_pc) as usize;
                    let block = planned(
                        source,
                        &engine,
                        engine.state().guest_pc,
                        &code[at..],
                        native,
                    );
                    // SAFETY: Original source and current same-owner operands.
                    match unsafe { engine.step_checked(&mut arena.pause().unwrap(), &block) }
                        .unwrap()
                    {
                        NativeBlockStop::Resume { .. } => {}
                        NativeBlockStop::Returned { value } => break value,
                        other => panic!("unexpected original continuation: {other:?}"),
                    }
                    assert!(engine.usage().blocks - before.blocks < 5);
                };
                assert_eq!(returned, value);
                assert_eq!(engine.state().gpr[8], 17);
                assert_eq!(
                    engine.usage().blocks - before.blocks,
                    blocks + u64::from(!native)
                );
                assert_eq!(engine.usage().native_entries - before.native_entries, 2);
                assert_eq!(
                    engine.usage().native_instructions - before.native_instructions,
                    if native { 3 } else { 2 }
                );
                assert_eq!(
                    engine.usage().vector_operations - before.vector_operations,
                    u64::from(!native)
                );
                assert_eq!(engine.usage().vector_bytes - before.vector_bytes, 0);
            }
        });
    }
    if !available {
        return;
    }
    // The compare implicitly replaces RCX. A following indexed read MUST
    // start another block using that new value.
    let mut code = encoding(9, 15, 0x18).to_vec();
    code.extend([0x8b, 0x04, 0x8b, 0xc3]);
    for denied in [false, true] {
        with_source(Some(&code), |source, arena, stack, state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            let mut right = [1; 16];
            right[3] = 2;
            let mut current = input(state, state.guest_pc, [9, 15], ([1; 16], right));
            current.gpr[RCX] = 0;
            current.gpr[3] = if denied { data + PAGE - 12 } else { data + 64 };
            if !denied {
                arena
                    .pause()
                    .unwrap()
                    .write(data + 76, &42_u32.to_le_bytes())
                    .unwrap();
            }
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                current,
                stack,
                &Default::default(),
            )
            .unwrap();
            engine.machine.vector_mode = VectorMode::Avx { avx2: false };
            let first = planned(source, &engine, current.guest_pc, &code, true);
            assert_eq!(first.instructions().len(), 1);
            assert_eq!(
                first.exit().kind(),
                ExitKind::Fallthrough {
                    target: current.guest_pc + 6
                }
            );
            // SAFETY: Original register-only body and actual admitted bridge.
            unsafe { engine.step_checked(&mut arena.pause().unwrap(), &first) }.unwrap();
            assert_eq!(engine.state().gpr[RCX], 3);
            let after = *engine.state();
            let before = engine.usage();
            let second = planned(source, &engine, current.guest_pc + 6, &code[6..], true);
            // SAFETY: Exact source/owner; current index and full four-byte
            // read are checked afresh before this second native entry.
            let result = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &second) };
            if denied {
                assert!(result.is_err());
                assert_eq!(*engine.state(), after);
                assert_eq!(engine.usage().native_entries, before.native_entries);
                assert_eq!(engine.usage().cache_publications, before.cache_publications);
            } else {
                assert_eq!(result.unwrap(), NativeBlockStop::Returned { value: 42 });
                assert_eq!(engine.usage().native_entries, 2);
                assert_eq!(engine.usage().memory_access_checks, 1);
                assert_eq!(engine.usage().memory_bytes_checked, 4);
            }
        });
    }
}
