// SPDX-License-Identifier: GPL-2.0-only
//! Actual selected-element backing checks; no guest/native instruction executes.
use super::*;

#[test]
fn bit_string_selected_full_element_requires_live_read_and_rmw_rights_across_pages() {
    for size in [2, 4, 8] {
        for opcode in [0xa3, 0xab, 0xb3, 0xbb] {
            let mut code = Vec::new();
            if size == 2 {
                code.push(0x66);
            } else if size == 8 {
                code.push(0x48);
            }
            code.extend([0x0f, opcode, 0x0f, 0xc3]);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                state.gpr[RDI] = data + PAGE + size - 1;
                state.gpr[RCX] = u64::MAX; // Select the previous complete element.
                arena
                    .pause()
                    .unwrap()
                    .write(data + PAGE - 1, &[0xa5])
                    .unwrap();
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
                // The unadjusted base need not be mapped when the selected
                // complete previous element is mapped and permitted.
                engine.machine.state.gpr[RDI] = data + PAGE;
                engine
                    .preflight_ordinary_memory(&arena.pause().unwrap(), &planned)
                    .unwrap();
                engine.machine.state.gpr[RDI] = state.gpr[RDI];
                assert!(matches!(
                    engine.preflight_ordinary_memory(&arena.pause().unwrap(), &planned),
                    Err(NativeDispatchError::Memory(NativeMemoryError::Unmapped(_)))
                ));
                arena
                    .map_at(data + PAGE, PAGE, NativePermissions::READ_WRITE)
                    .unwrap();
                arena
                    .pause()
                    .unwrap()
                    .write(data + PAGE, &[0x5a; 7])
                    .unwrap();
                arena
                    .protect(data + PAGE, PAGE, NativePermissions::READ)
                    .unwrap();
                let read_only = engine.preflight_ordinary_memory(&arena.pause().unwrap(), &planned);
                if opcode == 0xa3 {
                    assert_eq!(read_only, Ok(()));
                } else {
                    assert!(matches!(
                        read_only,
                        Err(NativeDispatchError::Memory(
                            NativeMemoryError::PermissionDenied(_)
                        ))
                    ));
                }
                arena
                    .protect(
                        data + PAGE,
                        PAGE,
                        NativePermissions::new(false, true, false),
                    )
                    .unwrap();
                assert!(matches!(
                    engine.preflight_ordinary_memory(&arena.pause().unwrap(), &planned),
                    Err(NativeDispatchError::Memory(
                        NativeMemoryError::PermissionDenied(_)
                    ))
                ));
                arena
                    .protect(data + PAGE, PAGE, NativePermissions::READ_WRITE)
                    .unwrap();
                engine
                    .preflight_ordinary_memory(&arena.pause().unwrap(), &planned)
                    .unwrap();
                let mut bytes = [0; 8];
                arena
                    .pause()
                    .unwrap()
                    .read(data + PAGE - 1, &mut bytes)
                    .unwrap();
                assert_eq!(bytes, [0xa5, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a]);
                assert_eq!(*engine.state(), state);
                assert_eq!(engine.usage().native_instructions, 0);
                assert!(!engine.machine.cache.executable);
            });
        }
    }
}

#[test]
fn bit_index_prior_partial_write_and_work_exhaustion_refuse_before_native_publication() {
    with_source(
        Some(&[0xb1, 0xff, 0x48, 0x0f, 0xab, 0x0f, 0xc3]),
        |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            state.gpr[RDI] = data;
            state.gpr[RCX] = 0;
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
                2
            );
            assert_eq!(
                engine.preflight_ordinary_memory(&arena.pause().unwrap(), &planned),
                Err(NativeDispatchError::MemoryIsolationRequired)
            );
            engine.machine.limits.max_work_units = engine.machine.usage.work_units;
            assert!(matches!(
                engine.preflight_ordinary_memory(&arena.pause().unwrap(), &planned),
                Err(NativeDispatchError::Budget("memory analysis"))
            ));
            let mut bytes = [0; 8];
            arena.pause().unwrap().read(data, &mut bytes).unwrap();
            assert_eq!(u64::from_le_bytes(bytes), 100);
            assert_eq!(*engine.state(), state);
            assert_eq!(engine.usage().native_instructions, 0);
            assert!(!engine.machine.cache.executable);
        },
    );
}
