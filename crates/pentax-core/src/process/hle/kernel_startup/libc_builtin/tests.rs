// SPDX-License-Identifier: GPL-2.0-only
//! Original supplied ELF exports and checked builtin continuations. No native
//! body executes; actual paused backing, service writes and RET proofs are used.
use super::*;
use crate::process::hle::{
    HleProfile, NativeImageLimits, build_hle_providers, with_native_image_session,
};
use pentax_codegen::{
    BlockLimits, MaterializedBlockPlan, TranslationPolicy, plan_materialized_block,
};
use pentax_platform::native_dispatch::{
    NativeBlockStepper, NativeBlockStop, NativeCpuState, NativeStackFrame,
};
use pentax_platform::native_memory::{
    NativeMemoryEffect, NativeMemoryTransaction, NativePermissions,
};

fn supplied() -> Vec<u8> {
    let provider =
        build_hle_providers(HleProfile::KytyObservedLibcInternalV1, &Default::default()).unwrap();
    let mut image = provider.image().to_vec();
    // Retain original finite export metadata but supply distinct ordinary RET
    // bodies. The manifest must select metadata, never factory marker bytes.
    for at in [0x100, 0x110, 0x120] {
        image[at..at + 8].copy_from_slice(&[0xc3, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90]);
    }
    image
}

fn block<'view, 'source, 'image>(
    proof: &'view PreparedNativeSource<'source, 'image>,
    engine: &NativeBlockStepper<'view, 'source, 'image>,
    module: usize,
) -> MaterializedBlockPlan<'view, 'source, 'image> {
    plan_materialized_block(
        proof.materialized_source(),
        module,
        engine.state().guest_pc,
        &[0xc3],
        engine.placement(),
        &TranslationPolicy::x64_sse2(),
        &BlockLimits::default(),
    )
    .unwrap()
}

fn frame(layout: &crate::process::hle::NativeEntryLayout) -> NativeStackFrame {
    NativeStackFrame {
        stack: layout.stack,
        guards: Some(layout.guards),
        return_slot: layout.return_slot,
        return_marker: layout.return_marker,
    }
}

fn physical_alias(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    original: u64,
    alias: u64,
    executable: bool,
) {
    let page = crate::memory::PAGE_SIZE;
    let mut snapshot = vec![0; page as usize];
    if executable {
        session
            .pause()
            .unwrap()
            .fetch_source(original, &mut snapshot)
            .unwrap();
    } else {
        session
            .pause()
            .unwrap()
            .read(original, &mut snapshot)
            .unwrap();
    }
    for effect in [
        NativeMemoryEffect::AllocatePhysical {
            physical: 0,
            size: page,
        },
        NativeMemoryEffect::Unmap {
            address: original,
            size: page,
        },
        NativeMemoryEffect::Unmap {
            address: alias,
            size: page,
        },
        NativeMemoryEffect::MapPhysical {
            address: original,
            size: page,
            physical: 0,
            permissions: NativePermissions::READ_WRITE,
        },
        NativeMemoryEffect::MapPhysical {
            address: alias,
            size: page,
            physical: 0,
            permissions: NativePermissions::READ_WRITE,
        },
    ] {
        session
            .pause()
            .unwrap()
            .apply_transaction(
                NativeMemoryTransaction {
                    effect,
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
    }
    session.pause().unwrap().write(original, &snapshot).unwrap();
    if executable {
        session
            .pause()
            .unwrap()
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::Protect {
                        address: original,
                        size: page,
                        permissions: NativePermissions::READ_EXECUTE,
                    },
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
    }
}

#[test]
fn exact_supplied_exports_coalesce_finite_aliases_and_refuse_wrong_sources_or_conflicting_entries()
{
    let primary = crate::process::hle::unavailable::tests::original_input(0);
    for case in 0..4 {
        let mut library = supplied();
        match case {
            1 => {
                // Both offered module versions differ; no prefix fallback.
                for row in [5, 6] {
                    let at = 0x300 + row * 16 + 8;
                    let mut word = u64::from_le_bytes(library[at..at + 8].try_into().unwrap());
                    word = (word & !(255_u64 << 32)) | (2_u64 << 32);
                    library[at..at + 8].copy_from_slice(&word.to_le_bytes());
                }
            }
            2 => {
                // Same memcpy role, different entry in a second exact scope.
                let row = crate::process::hle::provider::LIBC_ALIAS_SYMBOL;
                let at = crate::process::hle::provider::SYMBOLS_AT + row * 24 + 8;
                library[at..at + 8].copy_from_slice(&0x130_u64.to_le_bytes());
            }
            3 => {
                // Cross-function entry collision in all memmove declarations.
                for row in [
                    2,
                    crate::process::hle::provider::LIBC_ALIAS_SYMBOL + 3,
                    crate::process::hle::provider::LIBC_ALIAS_SYMBOL + 4,
                    crate::process::hle::provider::LIBC_ALIAS_SYMBOL + 5,
                ] {
                    let at = crate::process::hle::provider::SYMBOLS_AT + row * 24 + 8;
                    library[at..at + 8].copy_from_slice(&0x100_u64.to_le_bytes());
                }
            }
            _ => {}
        }
        with_native_image_session(
            &[&primary, &library],
            &NativeImageLimits::default(),
            |session, _| {
                let proof = session.prepare_native_source(&Default::default()).unwrap();
                assert!(session.prepare_libc_builtins(&proof, 0).is_err());
                assert!(
                    session
                        .prepare_libc_builtins(&proof, session.plan.kernel_provider_index())
                        .is_err()
                );
                let selected = session.prepare_libc_builtins(&proof, 1);
                if case >= 2 {
                    assert!(selected.is_err());
                } else {
                    let selected = selected.unwrap();
                    let bias = proof.materialized_source().source_plan().modules()[1].load_bias();
                    for offset in [0x100, 0x110, 0x120] {
                        assert_eq!(selected.contains_pc(bias + offset), case == 0);
                        assert!(!selected.contains_pc(bias + offset + 1));
                    }
                    assert!(!format!("{selected:?}").contains("0x"));
                }
                assert_eq!(session.libc_builtin_usage(), LibcBuiltinUsage::default());
                assert_eq!(session.hle.service_attempts(), 0);
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    }
}

#[test]
fn checked_supplied_memory_helpers_use_shared_hle_and_preserve_normal_and_outer_ret_state() {
    let primary = crate::process::hle::unavailable::tests::original_input(0);
    let library = supplied();
    with_native_image_session(
        &[&primary, &library],
        &NativeImageLimits::default(),
        |session, layout| {
            let proof = session.prepare_native_source(&Default::default()).unwrap();
            let manifest = session.prepare_libc_builtins(&proof, 1).unwrap();
            let bias = proof.materialized_source().source_plan().modules()[1].load_bias();
            let source = layout.entry.stack.address + 0x200;
            let output = source + 0x100;
            for index in 0..3 {
                let mut state = NativeCpuState::new(
                    bias + 0x100 + index as u64 * 16,
                    layout.entry.return_slot - 8,
                );
                state.fs_base = Some(session.tls().instances()[0].thread_pointer);
                state.gpr[1] = 0x1234_5678_9abc_def0;
                state.gpr[11] = 0xfedc_ba98_7654_3210;
                state.rflags = 0xad7;
                state.mxcsr = 0x1fbf;
                for (at, byte) in state.xmm.iter_mut().enumerate() {
                    *byte = (at as u8).wrapping_mul(13);
                }
                for (at, byte) in state.ymm_upper.iter_mut().enumerate() {
                    *byte = (at as u8).wrapping_mul(17).wrapping_add(3);
                }
                state.gpr[7] = if index == 1 { source + 2 } else { output };
                state.gpr[6] = if index == 2 { 0x0012_34ab } else { source };
                state.gpr[2] = if index == 1 { 6 } else { 8 };
                session.pause().unwrap().write(source, b"12345678").unwrap();
                session.pause().unwrap().write(output, &[0xa5; 8]).unwrap();
                session
                    .pause()
                    .unwrap()
                    .write(state.gpr[4], &layout.entry.entry.to_le_bytes())
                    .unwrap();
                let mut engine = NativeBlockStepper::new(
                    proof.materialized_source(),
                    &mut session.pause().unwrap(),
                    state,
                    frame(&layout.entry),
                    &Default::default(),
                )
                .unwrap();
                let planned = block(&proof, &engine, 1);
                assert_eq!(
                    session
                        .try_libc_builtin(&manifest, &proof, &mut engine, &planned)
                        .unwrap(),
                    Some(NativeBlockStop::Resume {
                        guest_pc: layout.entry.entry
                    })
                );
                let mut expected = state;
                expected.gpr[0] = state.gpr[7];
                expected.gpr[4] += 8;
                expected.guest_pc = layout.entry.entry;
                assert_eq!(*engine.state(), expected);
                assert!(engine.observed_return().is_none());
                let mut bytes = [0; 8];
                session
                    .pause()
                    .unwrap()
                    .read(if index == 1 { source } else { output }, &mut bytes)
                    .unwrap();
                assert_eq!(
                    bytes,
                    match index {
                        0 => *b"12345678",
                        1 => *b"12123456",
                        _ => [0xab; 8],
                    }
                );
                let returned = block(&proof, &engine, 0);
                assert_eq!(
                    engine
                        .step_return_exit(&mut session.pause().unwrap(), &returned)
                        .unwrap(),
                    NativeBlockStop::Returned {
                        value: state.gpr[7]
                    }
                );
                engine
                    .validate_call_return(&engine.observed_return().unwrap())
                    .unwrap();
                assert_eq!(
                    (
                        engine.usage().native_entries,
                        engine.usage().native_instructions
                    ),
                    (0, 0)
                );
            }
            assert_eq!(
                session.libc_builtin_usage(),
                LibcBuiltinUsage {
                    attempted: [1; 3],
                    completed: [1; 3]
                }
            );
            assert_eq!(
                (
                    session.hle.service_attempts(),
                    session.hle.completed_services()
                ),
                (3, 3)
            );
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}

#[test]
fn builtin_fallbacks_precede_helper_attempts_and_service_refusals_are_terminal_without_output() {
    let primary = crate::process::hle::unavailable::tests::original_input(0);
    let library = supplied();
    for case in 0..8 {
        let mut limits = NativeImageLimits::default();
        if case == 4 {
            limits.startup.startup.hle.services.max_span_bytes = 7;
        }
        if case == 5 {
            limits.startup.startup.hle.services.max_attempts = 0;
        }
        with_native_image_session(&[&primary, &library], &limits, |session, layout| {
            let proof = session.prepare_native_source(&Default::default()).unwrap();
            let manifest = session.prepare_libc_builtins(&proof, 1).unwrap();
            let bias = proof.materialized_source().source_plan().modules()[1].load_bias();
            let output = bias + crate::memory::PAGE_SIZE;
            let source = layout.entry.stack.address + 0x100;
            session.pause().unwrap().write(source, b"12345678").unwrap();
            session.pause().unwrap().write(output, &[0xa5; 8]).unwrap();
            let mut state = NativeCpuState::new(
                bias + if case == 0 { 0x100 } else { 0x120 },
                layout.entry.return_slot,
            );
            state.fs_base = Some(session.tls().instances()[0].thread_pointer);
            state.gpr[7] = match case {
                0 => source + 1,
                1 => layout.entry.return_slot,
                2 => bias + 0x120,
                _ => output,
            };
            state.gpr[6] = if case == 0 { source } else { 42 };
            state.gpr[2] = 8;
            if case >= 6 {
                let original = if case == 6 {
                    layout.entry.return_slot & !(crate::memory::PAGE_SIZE - 1)
                } else {
                    bias
                };
                let alias = layout.entry.stack.address + crate::memory::PAGE_SIZE;
                physical_alias(session, original, alias, case == 7);
                state.gpr[7] = alias
                    + if case == 6 {
                        layout.entry.return_slot - original
                    } else {
                        0x120
                    };
                assert_ne!(
                    state.gpr[7],
                    if case == 6 {
                        layout.entry.return_slot
                    } else {
                        bias + 0x120
                    }
                );
            }
            if case == 3 {
                session
                    .pause()
                    .unwrap()
                    .apply_transaction(
                        NativeMemoryTransaction {
                            effect: NativeMemoryEffect::Protect {
                                address: output,
                                size: crate::memory::PAGE_SIZE,
                                permissions: NativePermissions::READ,
                            },
                            writes: &[],
                        },
                        &Default::default(),
                    )
                    .unwrap();
            }
            let mut engine = NativeBlockStepper::new(
                proof.materialized_source(),
                &mut session.pause().unwrap(),
                state,
                frame(&layout.entry),
                &Default::default(),
            )
            .unwrap();
            let planned = block(&proof, &engine, 1);
            let result = session.try_libc_builtin(&manifest, &proof, &mut engine, &planned);
            if case <= 2 || case >= 6 {
                assert_eq!(result.unwrap(), None);
                assert!(engine.libc_builtin_entry_ready());
                assert_eq!(session.libc_builtin_usage(), LibcBuiltinUsage::default());
                assert_eq!(session.hle.service_attempts(), 0);
            } else {
                assert!(matches!(result, Err(LibcBuiltinError::Service(_))));
                assert!(!engine.libc_builtin_entry_ready());
                assert_eq!(session.libc_builtin_usage().attempted, [0, 0, 1]);
                assert_eq!(session.libc_builtin_usage().completed, [0; 3]);
                assert!(
                    engine
                        .begin_libc_builtin(&session.pause().unwrap(), &planned)
                        .is_err()
                );
            }
            assert_eq!(*engine.state(), state);
            let mut bytes = [0; 8];
            session.pause().unwrap().read(output, &mut bytes).unwrap();
            assert_eq!(bytes, [0xa5; 8]);
            assert_eq!(session.hle.completed_services(), 0);
            Ok::<_, ()>(())
        })
        .unwrap();
    }
}
