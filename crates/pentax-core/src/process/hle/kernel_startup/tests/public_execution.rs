// SPDX-License-Identifier: GPL-2.0-only

//! Original data-only checks of the public prepared-owner handoff. No CPU runs.

use super::*;
use pentax_codegen::{
    BlockLimits, CachePlacement, ExitKind, TranslationPolicy, plan_materialized_block,
};
use pentax_platform::native_memory::{NativeMemoryEffect, NativeMemoryTransaction};

fn request(
    session: &PreparedNativeKernelStartup<'_, '_>,
    stop: KernelServiceStop,
) -> NativeServiceRequest {
    NativeServiceRequest {
        stack_pointer: None,
        vector_args: None,
        owner_id: session.owner_id(),
        syscall_pc: stop.syscall_pc,
        next_pc: stop.syscall_pc + 2,
        service_id: stop.service_id,
        args: stop.args.0,
    }
}

#[test]
fn public_materialization_uses_actual_owner_and_typed_values_without_borrowing_session() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let proof = session
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            assert_eq!(proof.owner_id(), session.owner_id());
            assert!(std::ptr::eq(
                proof.materialized_source().source_plan(),
                plan.link_plan()
            ));
            assert_eq!(proof.materialized_source().deferred_relocations().len(), 3);
            for patch in proof.materialized_source().declared_writes() {
                assert_eq!(word(&mut session, patch.address), patch.value);
            }
            // Keeping the proof does not retain a shared borrow of the session.
            let mut source = [0];
            session
                .pause()
                .unwrap()
                .fetch_source(base + ENTRY as u64, &mut source)
                .unwrap();
            let block = plan_materialized_block(
                proof.materialized_source(),
                0,
                base + ENTRY as u64,
                &source,
                CachePlacement {
                    address: base + 2 * RESERVATION,
                    capacity: 4096,
                },
                &TranslationPolicy::x64_sse2(),
                &BlockLimits::default(),
            )
            .unwrap();
            assert_eq!(block.exit().kind(), ExitKind::Return { adjust: 0 });
            assert!(matches!(
                session.prepare_native_source(&BlockLimits {
                    max_allocation_bytes: 0,
                    ..Default::default()
                }),
                Err(KernelStartupError::Budget("source declaration bytes"))
            ));
            assert!(matches!(
                session.prepare_native_source(&BlockLimits {
                    max_work_units: 0,
                    ..Default::default()
                }),
                Err(KernelStartupError::Budget("source declarations"))
            ));
            assert_eq!(session.native_service_requests(), 0);
        },
    );
}

#[test]
fn public_pre_requests_preserve_original_rcx_and_dispatch_on_the_same_native_pages() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let proof = session
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            let call = stop(
                plan,
                Service::CreateSema,
                [base + OUTPUT, base + NAME, 1, 17, 23, 0],
            );
            let create_request = request(&session, call);
            assert_eq!(session.service_request(create_request).unwrap(), 0);
            assert_eq!(
                session.last_admitted_kernel_service(),
                Some("KernelCreateSema")
            );
            let handle = word(&mut session, base + OUTPUT);
            assert_ne!(handle, 0);
            let poll = request(
                &session,
                stop(plan, Service::PollSema, [handle, 17, 0, 0, 0, 0]),
            );
            assert_eq!(session.service_request(poll).unwrap(), 0);
            assert_eq!(
                session.last_admitted_kernel_service(),
                Some("KernelPollSema")
            );
            let site = plan.hle.hle.sites()[KnownFunction::Memcpy.index()];
            let copy = NativeServiceRequest {
                stack_pointer: None,
                vector_args: None,
                owner_id: proof.owner_id(),
                syscall_pc: site.trap,
                next_pc: site.resume,
                service_id: u64::from(KnownFunction::Memcpy.service_id()),
                args: [pointers[0] - TLS_OFFSET + 24, base + OUTPUT, 8, 17, 23, 29],
            };
            assert_eq!(
                session.service_request(copy).unwrap(),
                pointers[0] - TLS_OFFSET + 24
            );
            assert_eq!(word(&mut session, pointers[0] - TLS_OFFSET + 24), handle);
            assert_eq!(word(&mut session, pointers[1] - TLS_OFFSET + 24), 37);
            assert_eq!(session.native_service_requests(), 3);
            assert_eq!(session.kernel_stop_attempts(), 2);
            assert_eq!(session.libc_service_attempts(), 1);
            assert_eq!(session.last_admitted_kernel_service(), None);
        },
    );
}

#[test]
fn public_pre_requests_reject_foreign_owner_continuation_and_changed_live_provider_before_outputs()
{
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let foreign = NativeAddressSpace::new(NativeMemoryLimits::default()).unwrap();
            let valid = request(
                &session,
                stop(
                    plan,
                    Service::CreateSema,
                    [base + OUTPUT, base + NAME, 1, 17, 23, 0],
                ),
            );
            assert_eq!(
                session.service_request(NativeServiceRequest {
                    stack_pointer: None,
                    owner_id: foreign.owner_id(),
                    ..valid
                }),
                Err(KernelStartupError::NativeOwnerMismatch)
            );
            assert!(matches!(
                session.service_request(NativeServiceRequest {
                    next_pc: valid.next_pc + 1,
                    ..valid
                }),
                Err(KernelStartupError::ContinuationMismatch { .. })
            ));
            assert!(matches!(
                session.service_request(NativeServiceRequest {
                    service_id: valid.service_id | (1 << 32),
                    ..valid
                }),
                Err(KernelStartupError::ServiceIdMismatch { .. })
            ));
            assert_eq!(session.kernel_usage().handles_live, 0);
            assert_eq!(session.last_admitted_kernel_service(), None);
            assert_eq!(word(&mut session, base + OUTPUT), 0);

            let site = plan.hle.hle.sites()[KnownFunction::Memcpy.index()];
            let copy = NativeServiceRequest {
                stack_pointer: None,
                vector_args: None,
                owner_id: session.owner_id(),
                syscall_pc: site.trap,
                next_pc: site.resume,
                service_id: u64::from(KnownFunction::Memcpy.service_id()),
                args: [base + OUTPUT, base + TLS_RVA + 24, 8, 17, 23, 29],
            };
            let page = plan.hle.provider_bias();
            let protect = |session: &mut PreparedNativeKernelStartup<'_, '_>, permissions| {
                session
                    .pause()
                    .unwrap()
                    .apply_transaction(
                        NativeMemoryTransaction {
                            effect: NativeMemoryEffect::Protect {
                                address: page,
                                size: PAGE_SIZE,
                                permissions,
                            },
                            writes: &[],
                        },
                        &Default::default(),
                    )
                    .unwrap();
            };
            protect(&mut session, NativePermissions::new(true, true, true));
            assert!(matches!(
                session.service_request(copy),
                Err(KernelStartupError::Hle(HleError::InvalidProvider(
                    "native libc provider lost immutable permissions"
                )))
            ));
            protect(&mut session, NativePermissions::READ_WRITE);
            session.pause().unwrap().write(site.entry, &[0x90]).unwrap();
            protect(&mut session, NativePermissions::READ_EXECUTE);
            assert!(matches!(
                session.service_request(copy),
                Err(KernelStartupError::Hle(HleError::InvalidProvider(
                    "native libc marker bytes"
                )))
            ));
            assert!(matches!(
                session.service_request(NativeServiceRequest {
                    service_id: copy.service_id | (1 << 32),
                    ..copy
                }),
                Err(KernelStartupError::Hle(HleError::ServiceIdMismatch { .. }))
            ));
            assert_eq!(word(&mut session, base + OUTPUT), 0);
            assert_eq!(session.libc_service_attempts(), 3);
            assert_eq!(session.native_service_requests(), 6);
        },
    );
}
