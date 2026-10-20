// SPDX-License-Identifier: GPL-2.0-only
use super::*;

fn hint(group: u8, form: usize) -> Vec<u8> {
    let mut bytes = match form {
        1 => vec![0x64],
        2 => vec![0x65],
        3 => vec![0x67],
        4 => vec![0x64, 0x67],
        5 => vec![0x65, 0x67],
        7 => vec![0x67],
        _ => vec![],
    };
    if matches!(form, 6 | 7) {
        bytes.extend([0x0f, 0x18, 0x05 | (group << 3)]);
        bytes.extend(i32::MAX.to_le_bytes());
    } else if form == 8 {
        // R9 + R9*4 +127 can overflow; no effective address is evaluated.
        bytes.extend([0x43, 0x0f, 0x18, 0x44 | (group << 3), 0x89, 0x7f]);
    } else {
        bytes.extend([0x41, 0x0f, 0x18, 0x01 | (group << 3)]);
    }
    bytes.push(0xc3);
    bytes
}

#[test]
fn cache_hints_preserve_complete_state_without_accessing_any_target() {
    for group in 0..4 {
        for form in 0..9 {
            let code = hint(group, form);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let marker = [0x5a; 16];
                arena.pause().unwrap().write(data, &marker).unwrap();
                arena
                    .protect(data, PAGE, NativePermissions::new(false, false, false))
                    .unwrap();
                for target in [0, data, 0x0000_8000_0000_0000, u64::MAX, data + PAGE] {
                    for (index, register) in state.gpr.iter_mut().enumerate() {
                        if index != RSP {
                            *register = 0xa5a5_0000_0000_0000 | index as u64;
                        }
                    }
                    state.gpr[9] = target;
                    state.rflags = 0xed7;
                    state.mxcsr = 0xffbf;
                    state.xmm.fill(0xa5);
                    state.ymm_upper.fill(0x96);
                    state.fs_base = Some(0x1234);
                    state.gs_base = Some(0x5678);
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
                    assert!(planned.instructions().iter().all(|r| r.emitted_len() == 0));
                    let next_pc = planned.exit().next_guest_pc();
                    let mut expected = state;
                    expected.guest_pc = next_pc;
                    assert_eq!(
                        engine
                            .resolve(&mut arena.pause().unwrap(), &planned)
                            .unwrap(),
                        NativeBlockStop::Resume { guest_pc: next_pc }
                    );
                    assert_eq!(engine.machine.state, expected);
                    assert_eq!(engine.machine.usage.integer_bytes, 0);
                    assert_eq!(engine.machine.usage.vector_bytes, 0);
                    assert_eq!(engine.machine.usage.native_instructions, 0);
                    assert!(!engine.machine.cache.executable);
                }
                arena
                    .protect(data, PAGE, NativePermissions::READ_WRITE)
                    .unwrap();
                let mut after = [0; 16];
                arena.pause().unwrap().read(data, &mut after).unwrap();
                assert_eq!(after, marker);
            });
        }
    }
}

#[test]
fn cache_hints_keep_prefix_feature_continuation_and_work_refusals() {
    for case in 0..5 {
        let mut code = hint(0, 0);
        if case < 2 {
            code.insert(0, if case == 0 { 0xf2 } else { 0xf3 });
        }
        with_source(Some(&code), |source, arena, stack, state| {
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let policy = if case == 2 {
                TranslationPolicy::new(pentax_codegen::CpuFeatures::empty())
            } else {
                TranslationPolicy::x64_sse2()
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
            if case == 3 {
                engine.machine.limits.max_work_units = engine.machine.usage.work_units;
            } else if case == 4 {
                let bias = source.source_plan().modules()[0].load_bias();
                arena
                    .protect(bias, PAGE, NativePermissions::READ_WRITE)
                    .unwrap();
            }
            let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
            if case < 3 {
                assert_eq!(
                    result,
                    Ok(NativeBlockStop::Unsupported {
                        guest_pc: state.guest_pc
                    })
                );
            } else {
                assert!(
                    result.is_err(),
                    "continuation/work admission must remain enforced"
                );
            }
            assert_eq!(engine.machine.state, state);
            assert_eq!(engine.machine.usage.integer_bytes, 0);
            assert_eq!(engine.machine.usage.vector_bytes, 0);
        });
    }
    for code in [
        vec![0x0f, 0x0d, 0x09, 0xc3],
        vec![0x0f, 0x0d, 0x11, 0xc3],
        vec![0x0f, 0xae, 0x39, 0xc3],
    ] {
        with_source(Some(&code), |source, arena, stack, state| {
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
                engine
                    .resolve(&mut arena.pause().unwrap(), &planned)
                    .unwrap(),
                NativeBlockStop::Unsupported {
                    guest_pc: state.guest_pc
                }
            );
            assert_eq!(engine.machine.state, state);
        });
    }
}
