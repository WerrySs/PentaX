// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use crate::process::hle::{NativeImageLimits, with_native_image_session};
use pentax_kernel::Service;

fn action(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    source: &PreparedNativeSource<'_, '_>,
    service: Service,
    args: [u64; 6],
) -> NativeServiceAction {
    let marker = *session
        .plan
        .factory
        .markers()
        .iter()
        .find(|marker| marker.registration.service == service)
        .unwrap();
    let pc = session.plan.entry(marker) + 5;
    session
        .service_action(
            source,
            NativeServiceRequest {
                owner_id: session.owner_id(),
                syscall_pc: pc,
                next_pc: pc + 2,
                service_id: marker.service_id.into(),
                args,
                vector_args: None,
                stack_pointer: None,
            },
        )
        .unwrap()
}

#[test]
fn created_thread_retains_custom_policy_priority_and_first_start_affinity() {
    let primary = crate::process::hle::unavailable::tests::original_input(0);
    with_native_image_session(
        &[&primary],
        &NativeImageLimits::default(),
        |session, layout| {
            let source = session.prepare_native_source(&Default::default()).unwrap();
            let output = layout.entry.stack.address + 0x100;
            let attribute = output + 8;
            let parameter = output + 16;
            session
                .pause()
                .unwrap()
                .write(parameter, &128_i32.to_le_bytes())
                .unwrap();
            for (service, args) in [
                (Service::ThreadAttrInit, [attribute, 0, 0, 0, 0, 0]),
                (
                    Service::ThreadAttrSetSchedPolicy,
                    [attribute, (-7_i32) as u32 as u64, 0, 0, 0, 0],
                ),
                (
                    Service::ThreadAttrSetSchedParam,
                    [attribute, parameter, 0, 0, 0, 0],
                ),
                (Service::ThreadAttrSetAffinity, [attribute, 1, 0, 0, 0, 0]),
            ] {
                assert!(matches!(
                    action(session, &source, service, args),
                    NativeServiceAction::Returned(0)
                ));
            }
            let NativeServiceAction::CreateThread(pending) = action(
                session,
                &source,
                Service::PthreadCreate,
                [output, attribute, layout.entry.entry, 0, 0, 0],
            ) else {
                panic!("expected creation");
            };
            let child = session.prepare_created_thread(&source, pending).unwrap();
            let handle = child.layout().pthread_pointer;
            assert!(matches!(
                action(
                    session,
                    &source,
                    Service::PosixThreadGetSchedParam,
                    [handle, output + 32, output + 36, 0, 0, 0]
                ),
                NativeServiceAction::Returned(0)
            ));
            let mut bytes = [0; 8];
            session
                .pause()
                .unwrap()
                .read(output + 32, &mut bytes)
                .unwrap();
            assert_eq!(&bytes[..4], &(-7_i32).to_le_bytes());
            assert_eq!(&bytes[4..], &128_i32.to_le_bytes());
            // First activation applies the source's once-only single-core demotion.
            // Exact metadata remains high while its effective class becomes normal.
            session.activate_thread_context(child.thread()).unwrap();
            assert_eq!(
                session.active_thread_priority_class(),
                pentax_kernel::ThreadPriorityClass::Normal
            );
            assert!(matches!(
                action(
                    session,
                    &source,
                    Service::PthreadSetaffinity,
                    [handle, 0x7f, 0, 0, 0, 0]
                ),
                NativeServiceAction::Returned(0)
            ));
            assert!(matches!(
                action(
                    session,
                    &source,
                    Service::PthreadSetprio,
                    [handle, 128, 0, 0, 0, 0]
                ),
                NativeServiceAction::Returned(0)
            ));
            assert_eq!(
                session.active_thread_priority_class(),
                pentax_kernel::ThreadPriorityClass::Normal
            );
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}

#[test]
fn caller_stack_bytes_and_mapping_survive_preparation_and_tls_failure() {
    let primary = crate::process::hle::unavailable::tests::original_input(0);
    for fail_tls in [false, true] {
        let mut limits = NativeImageLimits {
            kernel_window_bytes: 16 * PAGE_SIZE,
            ..Default::default()
        };
        if fail_tls {
            limits.startup.startup.static_tls.max_instances = 1;
        }
        with_native_image_session(&[&primary], &limits, |session, layout| {
            let source = session.prepare_native_source(&Default::default()).unwrap();
            let base = session.plan.limits.kernel.virtual_base + 8 * PAGE_SIZE;
            let payload = vec![0xa5; (2 * PAGE_SIZE) as usize];
            let output = layout.entry.stack.address + 0x100;
            let attr = output + 8;
            session
                .pause()
                .unwrap()
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::MapZeroed {
                            address: base,
                            size: 2 * PAGE_SIZE,
                            permissions: NativePermissions::READ_WRITE,
                        },
                        writes: &[pentax_platform::native_memory::NativeMemoryWrite {
                            address: base,
                            bytes: &payload,
                        }],
                    },
                    &Default::default(),
                )
                .unwrap();
            session
                .pause()
                .unwrap()
                .write(output, &99_u64.to_le_bytes())
                .unwrap();
            assert!(matches!(
                action(
                    session,
                    &source,
                    Service::ThreadAttrInit,
                    [attr, 0, 0, 0, 0, 0]
                ),
                NativeServiceAction::Returned(0)
            ));
            assert!(matches!(
                action(
                    session,
                    &source,
                    Service::ThreadAttrSetStack,
                    [attr, base + 13, PAGE_SIZE + 23, 0, 0, 0]
                ),
                NativeServiceAction::Returned(0)
            ));
            let NativeServiceAction::CreateThread(pending) = action(
                session,
                &source,
                Service::PthreadCreate,
                [output, attr, layout.entry.entry, 0, 0, 0],
            ) else {
                panic!("expected creation");
            };
            let start = session.thread_runtime.next;
            let prepared = session.prepare_created_thread(&source, pending);
            if fail_tls {
                assert!(prepared.is_err());
                assert_eq!(session.thread_runtime.next, start);
                assert!(session.thread_runtime.records.is_empty());
                assert_eq!(session.tls().instances().len(), 1);
            } else {
                let prepared = prepared.unwrap();
                assert_eq!(
                    prepared.layout().stack,
                    NativeRange {
                        address: base + 13,
                        size: PAGE_SIZE + 23
                    }
                );
                assert_eq!(prepared.layout().guards, None);
                assert_eq!(
                    prepared.layout().return_slot,
                    ((base + PAGE_SIZE + 36) & !15) - 8
                );
                assert_eq!(session.tls().instances().len(), 2);
            }
            let paused = session.pause().unwrap();
            let mut actual = vec![0; payload.len()];
            paused.read(base, &mut actual).unwrap();
            assert_eq!(actual, payload);
            paused.preflight_write(base, 2 * PAGE_SIZE).unwrap();
            let mut word = [0; 8];
            paused.read(output, &mut word).unwrap();
            assert_eq!(u64::from_le_bytes(word), 99);
            Ok::<_, ()>(())
        })
        .unwrap();
    }
}

#[test]
fn failed_thread_storage_or_registration_restores_unmapped_arena_and_keeps_output() {
    let primary = crate::process::hle::unavailable::tests::original_input(0);
    for fail_registration in [false, true] {
        let mut limits = NativeImageLimits::default();
        if fail_registration {
            // Admission uses17 work units; new context registration needs2 more.
            limits.startup.kernel.max_work = 18;
        } else {
            limits.startup.startup.static_tls.max_instances = 1;
        }
        with_native_image_session(&[&primary], &limits, |session, layout| {
            let source = session.prepare_native_source(&Default::default()).unwrap();
            let output = layout.entry.stack.address + 0x100;
            session
                .pause()
                .unwrap()
                .write(output, &99_u64.to_le_bytes())
                .unwrap();
            let marker = *session
                .plan
                .factory
                .markers()
                .iter()
                .find(|marker| marker.registration.service == pentax_kernel::Service::PthreadCreate)
                .unwrap();
            let pc = session.plan.entry(marker) + 5;
            let action = session
                .service_action(
                    &source,
                    NativeServiceRequest {
                        owner_id: session.owner_id(),
                        syscall_pc: pc,
                        next_pc: pc + 2,
                        service_id: marker.service_id.into(),
                        args: [output, 0, layout.entry.entry, 0, 0, 0],
                        vector_args: None,
                        stack_pointer: None,
                    },
                )
                .unwrap();
            let NativeServiceAction::CreateThread(pending) = action else {
                panic!("unexpected creation result");
            };
            let window = session.thread_runtime.window.unwrap();
            let start = session.thread_runtime.next;
            assert!(session.prepare_created_thread(&source, pending).is_err());
            assert!(session.thread_runtime.records.is_empty());
            assert_eq!(session.thread_runtime.next, start);
            assert_eq!(session.tls().instances().len(), 1);
            let paused = session.pause().unwrap();
            paused
                .preflight_reserved_unmapped(window.address, window.size)
                .unwrap();
            let mut actual = [0; 8];
            paused.read(output, &mut actual).unwrap();
            assert_eq!(u64::from_le_bytes(actual), 99);
            Ok::<_, ()>(())
        })
        .unwrap();
    }
}
