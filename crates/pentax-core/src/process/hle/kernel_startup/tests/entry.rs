// SPDX-License-Identifier: GPL-2.0-only

//! Owned entry staging only. These tests never enter native guest code.

use super::*;
use pentax_codegen::BlockLimits;
use pentax_platform::native_memory::{NativeMemoryEffect, NativeMemoryTransaction, NativeRange};

fn request(base: u64) -> NativeEntryRequest<'static> {
    NativeEntryRequest {
        policy: NativeEntryPolicy::KytyObservedV1,
        stack: NativeRange {
            address: base + 0xc_4000,
            size: PAGE_SIZE,
        },
        program_name: b"PentaX",
        // Original code address. No callback is called or given exit semantics.
        exit_handler: base + ENTRY as u64,
        return_marker: 0x7fff_ffff_f000,
    }
}

#[test]
fn thread_attribute_snapshot_uses_owned_entry_stack_and_refuses_changed_stack_or_guards() {
    for case in 0..4 {
        let mut limits = KernelStartupLimits::default();
        limits.startup.static_tls.thread_control =
            crate::process::tls::ThreadControlBlockPolicy::KytyObservedSelfPointerV1;
        with_plan(limits, |plan, owner, base, pointers| {
            let mut native_limits = load_limits(plan);
            // Include the two runtime objects and the intentionally mapped guard.
            native_limits.session.resident_limit = initial_mapped_bytes(plan, 1) + 4 * PAGE_SIZE;
            native_limits.session.mapped_limit = native_limits.session.resident_limit;
            let mut session = prepare_in(
                plan,
                &StaticTlsStorage {
                    thread_pointers: &pointers[..1],
                },
                &native_limits,
                identity(),
                vec![],
                Some(owner),
            )
            .unwrap();
            let slot = base + OUTPUT;
            session.pause().unwrap().write(slot, &[0; 32]).unwrap();
            session
                .kernel_stop(stop(plan, Service::ThreadAttrInit, [slot, 0, 0, 0, 0, 0]))
                .unwrap();
            let thread = session
                .kernel_stop(stop(plan, Service::PthreadSelf, [0; 6]))
                .unwrap();
            let query = [thread, slot, 0, 0, 0, 0];
            assert!(
                session
                    .kernel_stop(stop(plan, Service::ThreadAttrGet, query))
                    .is_err()
            );
            let source = session
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            let entry = session
                .prepare_process_entry(&source, request(base), &Default::default())
                .unwrap();
            let layout = *entry.layout();
            // A failed snapshot must leave the destination's prior values intact.
            session
                .kernel_stop(stop(
                    plan,
                    Service::ThreadAttrSetStack,
                    [slot, 0x12345, 0x8000, 0, 0, 0],
                ))
                .unwrap();
            let effect = match case {
                1 => Some(NativeMemoryEffect::Protect {
                    address: layout.stack.address,
                    size: layout.stack.size,
                    permissions: NativePermissions::READ,
                }),
                2 => Some(NativeMemoryEffect::Protect {
                    address: layout.stack.address,
                    size: layout.stack.size,
                    permissions: NativePermissions::new(true, true, true),
                }),
                3 => Some(NativeMemoryEffect::MapZeroed {
                    address: layout.guards[0].address,
                    size: PAGE_SIZE,
                    permissions: NativePermissions::READ_WRITE,
                }),
                _ => None,
            };
            if let Some(effect) = effect {
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
                assert!(
                    session
                        .kernel_stop(stop(plan, Service::ThreadAttrGet, query))
                        .is_err()
                );
            } else {
                assert_eq!(
                    session
                        .kernel_stop(stop(plan, Service::ThreadAttrGet, query))
                        .unwrap(),
                    0
                );
            }
            session
                .kernel_stop(stop(
                    plan,
                    Service::ThreadAttrGetStack,
                    [slot, slot + 8, slot + 16, 0, 0, 0],
                ))
                .unwrap();
            assert_eq!(
                word(&mut session, slot + 8),
                if case == 0 {
                    layout.stack.address
                } else {
                    0x12345
                }
            );
            assert_eq!(
                word(&mut session, slot + 16),
                if case == 0 { layout.stack.size } else { 0x8000 }
            );
            if case == 0 {
                session
                    .kernel_stop(stop(
                        plan,
                        Service::ThreadAttrGetGuardSize,
                        [slot, slot + 24, 0, 0, 0, 0],
                    ))
                    .unwrap();
                assert_eq!(word(&mut session, slot + 24), PAGE_SIZE);
            }
        });
    }
}

#[test]
fn entry_stages_real_call_arguments_root_frame_and_guards_on_the_source_owner() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let source = session
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            let before = session.memory().mapped_bytes();
            let entry = session
                .prepare_process_entry(&source, request(base), &Default::default())
                .unwrap();
            assert_eq!(entry.owner_id(), session.owner_id());
            assert!(std::ptr::eq(entry.source_plan(), plan.link_plan()));
            let layout = *entry.layout();
            assert_eq!(layout.entry, plan.link_plan().primary_entry());
            assert_eq!(layout.return_slot % 16, 8);
            assert_eq!(layout.frame_pointer, layout.return_slot + 8);
            assert_eq!(layout.frame_pointer % 16, 0);
            assert!(layout.return_slot - 128 >= layout.stack.address);
            assert_eq!(session.memory().mapped_bytes(), before + PAGE_SIZE);
            assert_eq!(word(&mut session, layout.return_slot), layout.return_marker);
            assert_eq!(word(&mut session, layout.frame_pointer), 0);
            assert_eq!(word(&mut session, layout.frame_pointer + 8), 0);
            assert_eq!(word(&mut session, layout.arguments), 1);
            assert_eq!(
                word(&mut session, layout.arguments + 8),
                layout.argument_name
            );
            assert_eq!(word(&mut session, layout.arguments + 16), 0);
            assert_eq!(word(&mut session, layout.arguments + 24), 0);
            assert_eq!(
                word(&mut session, layout.argument_name),
                u64::from_le_bytes(*b"PentaX\0\0")
            );
            let name_object = plan.kernel_provider_bias() + PROGRAM_NAME_OBJECT_RVA;
            let name_buffer = word(&mut session, name_object);
            assert_eq!(
                name_buffer,
                plan.kernel_provider_bias() + PROGRAM_NAME_BUFFER_RVA
            );
            assert_eq!(
                word(&mut session, name_buffer),
                u64::from_le_bytes(*b"PentaX\0\0")
            );
            let paused = session.pause().unwrap();
            for guard in layout.guards {
                paused
                    .preflight_reserved_unmapped(guard.address, guard.size)
                    .unwrap();
            }
            assert!(matches!(
                paused.preflight_fetch_source(layout.stack.address, 1),
                Err(NativeMemoryError::PermissionDenied(_))
            ));
            assert_eq!(session.native_service_requests(), 0);
        },
    );
}

#[test]
fn entry_refuses_names_targets_geometry_and_work_or_transaction_limits_before_mapping() {
    for case in 0..9 {
        with_plan(
            KernelStartupLimits::default(),
            |plan, owner, base, pointers| {
                let mut session = prepared(plan, owner, &pointers);
                let source = session
                    .prepare_native_source(&BlockLimits::default())
                    .unwrap();
                let before = session.memory().mapped_bytes();
                let mut input = request(base);
                let mut limits = NativeEntryLimits::default();
                match case {
                    0 => input.program_name = b"bad\0name",
                    1 => input.program_name = b"",
                    2 => limits.max_name_bytes = 2,
                    3 => input.exit_handler = base + OUTPUT,
                    4 => input.exit_handler = base + 0x1000, // RX page padding, not initialized code.
                    5 => input.stack.address += 1,
                    6 => limits.max_work_steps = 0,
                    7 => limits.transaction.max_outputs = 1,
                    8 => limits.transaction.max_output_bytes = 279,
                    _ => unreachable!(),
                }
                let error = session
                    .prepare_process_entry(&source, input, &limits)
                    .unwrap_err();
                match case {
                    0..=2 => assert!(matches!(error, NativeEntryError::InvalidName)),
                    3..=4 => assert!(matches!(error, NativeEntryError::InvalidTarget)),
                    5 => assert!(matches!(error, NativeEntryError::InvalidLayout)),
                    6 => assert!(matches!(error, NativeEntryError::Budget)),
                    7..=8 => assert!(matches!(
                        error,
                        NativeEntryError::Memory(NativeMemoryError::BudgetExceeded)
                    )),
                    _ => unreachable!(),
                }
                assert_eq!(session.memory().mapped_bytes(), before);
                let expected = request(base).stack;
                session
                    .pause()
                    .unwrap()
                    .preflight_reserved_unmapped(expected.address, expected.size)
                    .unwrap();
                assert_eq!(session.native_service_requests(), 0);
            },
        );
    }
}

#[test]
fn entry_rechecks_live_source_permissions_and_does_not_overwrite_an_existing_stack() {
    for permissions in [
        NativePermissions::READ,
        NativePermissions::new(false, false, false),
        NativePermissions::new(true, true, true),
    ] {
        with_plan(
            KernelStartupLimits::default(),
            |plan, owner, base, pointers| {
                let mut session = prepared(plan, owner, &pointers);
                let source = session
                    .prepare_native_source(&BlockLimits::default())
                    .unwrap();
                let before = session.memory().mapped_bytes();
                session
                    .pause()
                    .unwrap()
                    .apply_transaction(
                        NativeMemoryTransaction {
                            effect: NativeMemoryEffect::Protect {
                                address: base,
                                size: PAGE_SIZE,
                                permissions,
                            },
                            writes: &[],
                        },
                        &Default::default(),
                    )
                    .unwrap();
                assert!(
                    session
                        .prepare_process_entry(&source, request(base), &Default::default())
                        .is_err()
                );
                assert_eq!(session.memory().mapped_bytes(), before);
                session
                    .pause()
                    .unwrap()
                    .apply_transaction(
                        NativeMemoryTransaction {
                            effect: NativeMemoryEffect::Protect {
                                address: base,
                                size: PAGE_SIZE,
                                permissions: NativePermissions::READ_EXECUTE,
                            },
                            writes: &[],
                        },
                        &Default::default(),
                    )
                    .unwrap();
                let entry = session
                    .prepare_process_entry(&source, request(base), &Default::default())
                    .unwrap();
                let frame = entry.layout().frame_pointer;
                session
                    .pause()
                    .unwrap()
                    .write(frame, &73_u64.to_le_bytes())
                    .unwrap();
                assert!(
                    session
                        .prepare_process_entry(&source, request(base), &Default::default())
                        .is_err()
                );
                assert_eq!(word(&mut session, frame), 73);
                assert_eq!(session.memory().mapped_bytes(), before + PAGE_SIZE);
            },
        );
    }
}

#[test]
fn entry_rejects_another_prepared_owner_before_touching_its_stack() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, _, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let source = session
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            with_plan(
                KernelStartupLimits::default(),
                |other_plan, other_owner, base, other_pointers| {
                    let mut other = prepared(other_plan, other_owner, &other_pointers);
                    let before = other.memory().mapped_bytes();
                    assert!(matches!(
                        other.prepare_process_entry(&source, request(base), &Default::default()),
                        Err(NativeEntryError::WrongPreparedSource)
                    ));
                    assert_eq!(other.memory().mapped_bytes(), before);
                    let stack = request(base).stack;
                    other
                        .pause()
                        .unwrap()
                        .preflight_reserved_unmapped(stack.address, stack.size)
                        .unwrap();
                },
            );
        },
    );
}

#[test]
fn execute_only_entry_and_callback_keep_data_reads_and_writes_denied() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let source = session
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            session
                .pause()
                .unwrap()
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::Protect {
                            address: base,
                            size: PAGE_SIZE,
                            permissions: NativePermissions::new(false, false, true),
                        },
                        writes: &[],
                    },
                    &Default::default(),
                )
                .unwrap();
            let entry = session
                .prepare_process_entry(&source, request(base), &Default::default())
                .unwrap();
            assert_eq!(entry.layout().entry, base + ENTRY as u64);
            assert_eq!(entry.layout().exit_handler, base + ENTRY as u64);
            let paused = session.pause().unwrap();
            assert!(matches!(
                paused.preflight_read(entry.layout().entry, 1),
                Err(NativeMemoryError::PermissionDenied(_))
            ));
            assert!(matches!(
                paused.preflight_write(entry.layout().entry, 1),
                Err(NativeMemoryError::PermissionDenied(_))
            ));
            let mut opcode = [0];
            paused
                .fetch_source(entry.layout().entry, &mut opcode)
                .unwrap();
            assert_eq!(opcode, [0xc3]);
        },
    );
}
