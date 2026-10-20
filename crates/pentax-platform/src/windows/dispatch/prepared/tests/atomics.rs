// SPDX-License-Identifier: GPL-2.0-only

//! Actual backing admission, without entering guest/cache/native code.
use super::*;

#[test]
fn manual_atomic_gate_requires_both_permissions_even_for_a_failed_comparison() {
    with_source(
        Some(&[0xf0, 0x48, 0x0f, 0xb1, 0x0f, 0xc3]),
        |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            state.gpr[RDI] = data;
            state.gpr[RAX] = 999;
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
            assert_eq!(
                planned
                    .instructions()
                    .iter()
                    .filter(|r| r.emitted_len() != 0)
                    .count(),
                1
            );
            for rights in [
                NativePermissions::READ,
                NativePermissions::new(false, true, false),
            ] {
                arena.protect(data, PAGE, rights).unwrap();
                assert!(matches!(
                    engine.preflight_atomic_memory(&arena.pause().unwrap(), &planned),
                    Err(NativeDispatchError::Memory(
                        NativeMemoryError::PermissionDenied(_)
                    ))
                ));
            }
            arena
                .protect(data, PAGE, NativePermissions::READ_WRITE)
                .unwrap();
            engine
                .preflight_atomic_memory(&arena.pause().unwrap(), &planned)
                .unwrap();
            let mut bytes = [0; 8];
            arena.pause().unwrap().read(data, &mut bytes).unwrap();
            assert_eq!(u64::from_le_bytes(bytes), 100);
            assert_eq!(*engine.state(), state);
            assert_eq!(engine.usage().native_instructions, 0);
            assert!(!engine.machine.cache.executable);
        },
    );
}

#[test]
fn both_atomic_gates_refuse_unaligned_or_foreign_backing_and_budget_before_publication() {
    for code in [
        &[0xf0, 0x48, 0x01, 0x0f, 0xc3][..],
        &[0x48, 0x87, 0x0f, 0xc3][..],
    ] {
        for refusal in 0..3 {
            with_source(Some(code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let foreign = [0xa5_u64; 2];
                state.gpr[RDI] = match refusal {
                    0 => data + 1,
                    1 => foreign.as_ptr() as u64,
                    _ => data,
                };
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
                if refusal == 2 {
                    engine.machine.limits.max_work_units = engine.machine.usage.work_units;
                }
                for manual in [true, false] {
                    let result = if manual {
                        engine.preflight_atomic_memory(&arena.pause().unwrap(), &planned)
                    } else {
                        engine.preflight_ordinary_memory(&arena.pause().unwrap(), &planned)
                    };
                    assert!(match refusal {
                        0 => matches!(
                            result,
                            Err(NativeDispatchError::MemoryFootprint {
                                reason: MemoryAccessError::UnalignedAtomic,
                                form,
                                instruction_index: 0,
                            }) if form == planned.instructions()[0].instruction().code()
                        ),
                        1 => matches!(result, Err(NativeDispatchError::Memory(_))),
                        _ => matches!(result, Err(NativeDispatchError::Budget(_))),
                    });
                }
                assert_eq!(*engine.state(), state);
                assert_eq!(foreign, [0xa5; 2]);
                let mut bytes = [0; 8];
                arena.pause().unwrap().read(data, &mut bytes).unwrap();
                assert_eq!(u64::from_le_bytes(bytes), 100);
                assert!(!engine.machine.cache.executable);
                assert_eq!(engine.usage().native_instructions, 0);
            });
        }
    }
}
