// SPDX-License-Identifier: GPL-2.0-only
use super::*;

fn encoding(size: usize, high: bool, w: bool) -> [u8; 6] {
    [
        0xc4,
        if high { 0x62 } else { 0xe2 },
        (if size == 32 { 0x7d } else { 0x79 }) | if w { 0x80 } else { 0 },
        0x2a,
        if high { 0x38 } else { 0 },
        0xc3,
    ]
}

#[test]
fn streaming_loads_read_complete_aligned_span_preserve_sources_and_zero_vex128_upper() {
    for size in [16, 32] {
        for (high, w) in [(false, false), (false, true), (true, false), (true, true)] {
            let code = encoding(size, high, w);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let target = data + PAGE - size as u64;
                let input: [u8; 32] = std::array::from_fn(|i| i as u8 ^ 0x69);
                arena
                    .pause()
                    .unwrap()
                    .write(target, &input[..size])
                    .unwrap();
                arena.protect(data, PAGE, NativePermissions::READ).unwrap();
                state.gpr[RAX] = target;
                state.xmm.fill(0xa5);
                state.ymm_upper.fill(0x96);
                state.rflags = 0xed7;
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
                assert_eq!(
                    planned.exit().instruction().unwrap().mnemonic(),
                    pentax_codegen::Mnemonic::Vmovntdqa
                );
                engine
                    .resolve(&mut arena.pause().unwrap(), &planned)
                    .unwrap();
                let slot = if high { 15 * 16 } else { 0 };
                let mut expected = state;
                expected.xmm[slot..slot + 16].copy_from_slice(&input[..16]);
                if size == 32 {
                    expected.ymm_upper[slot..slot + 16].copy_from_slice(&input[16..]);
                } else {
                    expected.ymm_upper[slot..slot + 16].fill(0);
                }
                expected.guest_pc = planned.exit().next_guest_pc();
                assert_eq!(engine.machine.state, expected);
                assert_eq!(engine.machine.usage.vector_operations, 1);
                assert_eq!(engine.machine.usage.vector_bytes, size as u64);
                assert_eq!(engine.machine.usage.native_instructions, 0);
                let mut after = [0; 32];
                arena
                    .pause()
                    .unwrap()
                    .read(target, &mut after[..size])
                    .unwrap();
                assert_eq!(&after[..size], &input[..size]);
            });
        }
    }
}

#[test]
fn streaming_loads_refuse_alignment_backing_permissions_and_limits_before_cpu_publication() {
    for size in [16, 32] {
        for case in 0..6 {
            let code = encoding(size, true, false);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                state.gpr[RAX] = data
                    + match case {
                        0 => 1,
                        1 => PAGE,
                        _ => 0,
                    };
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
                match case {
                    2 => arena
                        .protect(data, PAGE, NativePermissions::new(false, false, false))
                        .unwrap(),
                    3 => engine.machine.limits.max_work_units = engine.machine.usage.work_units,
                    4 => engine.machine.usage.vector_bytes = u64::MAX,
                    5 => engine.machine.usage.vector_operations = u64::MAX,
                    _ => {}
                }
                let prior = engine.machine.usage;
                let error = engine
                    .resolve(&mut arena.pause().unwrap(), &planned)
                    .unwrap_err();
                if case == 0 {
                    assert_eq!(error, NativeDispatchError::VectorAlignment);
                }
                assert_eq!(engine.machine.state, state);
                assert_eq!(
                    engine.machine.usage.vector_operations,
                    prior.vector_operations
                );
                assert_eq!(engine.machine.usage.vector_bytes, prior.vector_bytes);
            });
        }
    }
}

#[test]
fn streaming_loads_reject_reserved_vvvv_and_register_source_encodings() {
    for size in [16, 32] {
        for register in [false, true] {
            let mut code = encoding(size, false, false);
            if register {
                code[4] = 0xc0;
            } else {
                code[2] &= !8;
            }
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
                    &TranslationPolicy::x64_sse2(),
                    &BlockLimits::default(),
                );
                assert!(matches!(
                    planned,
                    Err(pentax_codegen::CodegenError::InvalidInstruction { .. })
                ));
                assert_eq!(engine.machine.state, state);
                assert_eq!(engine.machine.usage.vector_operations, 0);
            });
        }
    }
}
