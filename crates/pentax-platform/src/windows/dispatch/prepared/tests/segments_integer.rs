// SPDX-License-Identifier: GPL-2.0-only

//! Direct Rust semantics and actual backing checks; no native body is entered.

use super::*;

#[test]
fn modeled_segment_integer_fs_and_gs_use_current_sib_displacement_and_address_width() {
    // ADD qword [segment:RAX+RCX*4-8], 5, with a wrapped 32-bit address variant.
    for segment in [0x64, 0x65] {
        for narrow in [false, true] {
            let mut code = vec![segment];
            if narrow {
                code.push(0x67);
            }
            code.extend([0x48, 0x83, 0x44, 0x88, 0xf8, 5, 0xc3]);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                if segment == 0x64 {
                    state.fs_base = Some(data);
                } else {
                    state.gs_base = Some(data);
                }
                state.gpr[RAX] = if narrow { 0xffff_ffff_0000_0004 } else { 4 };
                state.gpr[RCX] = 1;
                state.rflags = 0x202 | 0x400;
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
                engine.machine.phase = Phase::Terminal;
                engine
                    .emulate_segment(&mut arena.pause().unwrap(), &planned)
                    .unwrap();
                let mut actual = [0; 8];
                arena.pause().unwrap().read(data, &mut actual).unwrap();
                assert_eq!(u64::from_le_bytes(actual), 105);
                assert_eq!(engine.machine.state.gpr, state.gpr);
                assert_eq!(engine.machine.state.fs_base, state.fs_base);
                assert_eq!(engine.machine.state.gs_base, state.gs_base);
                assert_eq!(engine.machine.state.rflags, 0x202 | 0x400 | 4);
                assert_eq!(engine.machine.usage.segment_operations, 1);
                assert_eq!(engine.machine.usage.segment_bytes, 16);
                assert!(engine.machine.phase == Phase::Ready);
                assert!(!engine.machine.cache.executable);
            });
        }
    }
}

#[test]
fn modeled_segment_integer_rip_address_uses_original_guest_pc() {
    let mut code = vec![0x64, 0x48, 0x83, 0x05];
    code.extend_from_slice(&((DATA_OFF as i64 - ENTRY as i64 - 9) as i32).to_le_bytes());
    code.extend([5, 0xc3]);
    with_source(Some(&code), |source, arena, stack, mut state| {
        let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
        state.fs_base = Some(0);
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
        engine
            .emulate_segment(&mut arena.pause().unwrap(), &planned)
            .unwrap();
        let mut actual = [0; 8];
        arena.pause().unwrap().read(data, &mut actual).unwrap();
        assert_eq!(u64::from_le_bytes(actual), 105);
        assert_eq!(engine.machine.state.gpr, state.gpr);
        assert_eq!(engine.machine.usage.segment_bytes, 16);
    });
}

#[test]
fn modeled_segment_comparison_and_test_admit_read_only_backing_without_writes() {
    for (operation, tail, flags) in [
        (
            0x83,
            vec![0x3c, 0x25, 0, 0, 0, 0, 100, 0xc3],
            0x202 | 0x40 | 4,
        ),
        (
            0xf7,
            vec![0x04, 0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0xc3],
            0x202 | 0x40 | 4 | 0x10,
        ),
    ] {
        let mut code = vec![0x64, 0x48, operation];
        code.extend(tail);
        with_source(Some(&code), |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            arena.protect(data, PAGE, NativePermissions::READ).unwrap();
            state.fs_base = Some(data);
            state.rflags = 0x202 | 0x8d5;
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
            engine
                .emulate_segment(&mut arena.pause().unwrap(), &planned)
                .unwrap();
            let mut actual = [0; 8];
            arena.pause().unwrap().read(data, &mut actual).unwrap();
            assert_eq!(u64::from_le_bytes(actual), 100);
            assert_eq!(engine.machine.state.gpr, state.gpr);
            assert_eq!(engine.machine.state.rflags, flags);
            assert_eq!(engine.machine.usage.segment_bytes, 8);
            assert_eq!(engine.machine.usage.segment_operations, 1);
        });
    }
}

#[test]
fn modeled_segment_integer_refusals_leave_all_cpu_state_success_counters_and_output_unchanged() {
    #[derive(Debug, Clone, Copy)]
    enum Refusal {
        ReadOnly,
        CrossGuard,
        PartialWrite,
        Unconfigured,
        Rsp,
        Work,
        Operations,
        Bytes,
        Continuation,
    }
    for reason in [
        Refusal::ReadOnly,
        Refusal::CrossGuard,
        Refusal::PartialWrite,
        Refusal::Unconfigured,
        Refusal::Rsp,
        Refusal::Work,
        Refusal::Operations,
        Refusal::Bytes,
        Refusal::Continuation,
    ] {
        let mut code = if matches!(reason, Refusal::Rsp) {
            vec![0x64, 0x48, 0x03, 0x24, 0x25, 0, 0, 0, 0]
        } else {
            vec![0x64, 0x48, 0x83, 0x04, 0x25, 0, 0, 0, 0, 1]
        };
        if !matches!(reason, Refusal::Continuation) {
            code.push(0xc3);
        }
        with_source(Some(&code), |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            if !matches!(reason, Refusal::Unconfigured) {
                state.fs_base = Some(
                    if matches!(reason, Refusal::CrossGuard | Refusal::PartialWrite) {
                        data + PAGE - 4
                    } else {
                        data
                    },
                );
            }
            if matches!(reason, Refusal::ReadOnly) {
                arena.protect(data, PAGE, NativePermissions::READ).unwrap();
            }
            if matches!(reason, Refusal::PartialWrite) {
                arena
                    .map_at(data + PAGE, PAGE, NativePermissions::READ)
                    .unwrap();
            }
            state.rflags = 0x202 | 0x8d5 | 0x400;
            state.xmm.fill(0xa7);
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
            engine.machine.phase = Phase::Terminal;
            match reason {
                Refusal::Work => {
                    engine.machine.limits.max_work_units =
                        engine.machine.usage.work_units + engine.machine.target_visits + 128
                }
                Refusal::Operations => engine.machine.usage.segment_operations = u64::MAX,
                Refusal::Bytes => engine.machine.usage.segment_bytes = u64::MAX - 15,
                _ => {}
            }
            let before = engine.machine.usage;
            let mut output = vec![0; PAGE as usize];
            arena.pause().unwrap().read(data, &mut output).unwrap();
            let result = engine.emulate_segment(&mut arena.pause().unwrap(), &planned);
            match reason {
                Refusal::ReadOnly | Refusal::CrossGuard | Refusal::PartialWrite => assert!(
                    matches!(result, Err(NativeDispatchError::Memory(_))),
                    "{reason:?}: {result:?}"
                ),
                Refusal::Unconfigured => {
                    assert_eq!(result, Err(NativeDispatchError::UnconfiguredSegment))
                }
                Refusal::Rsp => assert_eq!(result, Err(NativeDispatchError::InvalidStack)),
                Refusal::Work => {
                    assert_eq!(result, Err(NativeDispatchError::Budget("segment bytes")))
                }
                Refusal::Operations => assert_eq!(
                    result,
                    Err(NativeDispatchError::Budget("segment operations"))
                ),
                Refusal::Bytes => {
                    assert_eq!(result, Err(NativeDispatchError::Budget("segment bytes")))
                }
                Refusal::Continuation => {
                    assert!(matches!(result, Err(NativeDispatchError::InvalidTarget(_))))
                }
            }
            let mut after = vec![0; PAGE as usize];
            arena.pause().unwrap().read(data, &mut after).unwrap();
            assert_eq!(after, output, "{reason:?}");
            assert_eq!(engine.machine.state, state, "{reason:?}");
            assert_eq!(
                engine.machine.usage.segment_operations,
                before.segment_operations
            );
            assert_eq!(engine.machine.usage.segment_bytes, before.segment_bytes);
            assert!(engine.machine.phase == Phase::Terminal);
            assert!(!engine.machine.cache.executable);
        });
    }
}

#[test]
fn modeled_segment_atomic_and_repeat_forms_remain_explicit_refusals() {
    for code in [
        vec![0xf0, 0x64, 0x48, 0x83, 0, 1, 0xc3],       // LOCK ADD
        vec![0xf2, 0xf0, 0x64, 0x48, 0x83, 0, 1, 0xc3], // XACQUIRE LOCK ADD
        vec![0xf3, 0x64, 0x48, 0x83, 0, 1, 0xc3],       // REP ADD
        vec![0x64, 0x48, 0x87, 0x08, 0xc3],             // XCHG implicitly locks memory
        vec![0x64, 0x48, 0x0f, 0xc1, 0x08, 0xc3],       // XADD
        vec![0x64, 0x48, 0x0f, 0xb1, 0x08, 0xc3],       // CMPXCHG
    ] {
        with_source(Some(&code), |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            state.fs_base = Some(data);
            state.gpr[RAX] = 0;
            state.gpr[RCX] = 42;
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
            engine.machine.phase = Phase::Terminal;
            assert_eq!(
                engine.emulate_segment(&mut arena.pause().unwrap(), &planned),
                Err(NativeDispatchError::UnsupportedSegmentInstruction)
            );
            assert_eq!(engine.machine.state, state);
            assert_eq!(engine.machine.usage.segment_operations, 0);
            assert_eq!(engine.machine.usage.segment_bytes, 0);
            let mut output = [0; 8];
            arena.pause().unwrap().read(data, &mut output).unwrap();
            assert_eq!(u64::from_le_bytes(output), 100);
        });
    }
}
