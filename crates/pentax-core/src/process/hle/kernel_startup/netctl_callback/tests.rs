// SPDX-License-Identifier: GPL-2.0-only
//! Original backing/admission checks. These checks execute no guest code.
use super::*;
use crate::process::hle::{NativeImageLimits, NativeServiceRequest, with_native_image_session};
use pentax_codegen::BlockLimits;
use pentax_kernel::Service;
use pentax_platform::native_memory::{
    NativeMemoryEffect, NativeMemoryTransaction, NativePermissions,
};

fn request(
    owner: &PreparedNativeKernelStartup<'_, '_>,
    service: Service,
    args: [u64; 6],
) -> NativeServiceRequest {
    let marker = *owner
        .plan
        .factory
        .markers()
        .iter()
        .find(|marker| marker.registration.service == service)
        .unwrap();
    let pc = owner.plan.entry(marker) + 5;
    NativeServiceRequest {
        stack_pointer: None,
        vector_args: None,
        owner_id: owner.owner_id(),
        syscall_pc: pc,
        next_pc: pc + 2,
        service_id: marker.service_id.into(),
        args,
    }
}

#[test]
fn netctl_callback_admission_refuses_unbacked_zero_tail_and_writable_targets() {
    for target_kind in 0..4 {
        let mut primary = crate::process::hle::unavailable::tests::original_input(0);
        primary[104..112].copy_from_slice(&(2 * crate::memory::PAGE_SIZE).to_le_bytes());
        with_native_image_session(
            &[&primary],
            &NativeImageLimits::default(),
            |owner, layout| {
                let proof = owner
                    .prepare_native_source(&BlockLimits::default())
                    .unwrap();
                let segment = &proof.materialized_source().source_plan().modules()[0]
                    .runtime_plan()
                    .segments[0];
                let module = &proof.materialized_source().source_plan().modules()[0];
                let target = match target_kind {
                    0 => u64::MAX,
                    1 => module.load_bias() + segment.vaddr + segment.file_bytes.len() as u64,
                    2 => layout.entry.stack.address + 0x100,
                    _ => {
                        owner
                            .pause()
                            .unwrap()
                            .apply_transaction(
                                NativeMemoryTransaction {
                                    effect: NativeMemoryEffect::Protect {
                                        address: module.load_bias() + segment.vaddr,
                                        size: crate::memory::PAGE_SIZE,
                                        permissions: NativePermissions::READ_WRITE,
                                    },
                                    writes: &[],
                                },
                                &Default::default(),
                            )
                            .unwrap();
                        layout.entry.entry
                    }
                };
                let id = layout.entry.stack.address + 0x200;
                for (service, args) in [
                    (Service::NetCtlInit, [0; 6]),
                    (
                        Service::NetCtlRegisterCallback,
                        [target, id + 8, id, 0, 0, 0],
                    ),
                ] {
                    assert!(matches!(
                        owner.service_action(&proof, request(owner, service, args)),
                        Ok(NativeServiceAction::Returned(0))
                    ));
                }
                assert!(
                    owner
                        .service_action(
                            &proof,
                            request(owner, Service::NetCtlCheckCallback, [0; 6]),
                        )
                        .is_err()
                );
                assert!(owner.netctl_callbacks.pending.is_none());
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    }
}

#[test]
fn netctl_callback_admission_retains_one_opaque_pending_ticket_without_completion() {
    let primary = crate::process::hle::unavailable::tests::original_input(0);
    with_native_image_session(
        &[&primary],
        &NativeImageLimits::default(),
        |owner, layout| {
            let proof = owner
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            let id = layout.entry.stack.address + 0x200;
            for (service, args) in [
                (Service::NetCtlInit, [0; 6]),
                (
                    Service::NetCtlRegisterCallback,
                    [layout.entry.entry, id + 8, id, 0, 0, 0],
                ),
            ] {
                assert!(matches!(
                    owner.service_action(&proof, request(owner, service, args)),
                    Ok(NativeServiceAction::Returned(0))
                ));
            }
            let NativeServiceAction::NetCtl(start) = owner
                .service_action(&proof, request(owner, Service::NetCtlCheckCallback, [0; 6]))
                .unwrap()
            else {
                panic!("callback was replaced with a numeric result");
            };
            assert_eq!(start.entry(), layout.entry.entry);
            assert_eq!(start.args(), [1, id + 8, 0]);
            assert!(matches!(
                owner.netctl_callbacks.pending(start).unwrap().phase,
                Phase::Admitted
            ));
            assert!(
                owner
                    .service_action(&proof, request(owner, Service::NetCtlCheckCallback, [0; 6]),)
                    .is_err()
            );
            assert_eq!(owner.netctl_callbacks.pending(start).unwrap().start, start);
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}
