// SPDX-License-Identifier: GPL-2.0-only
//! Actual prepared backing/admission checks; no native execution is claimed.
use super::*;
use crate::process::hle::{NativeImageLimits, NativeServiceRequest, with_native_image_session};
use pentax_codegen::BlockLimits;
fn session(test: impl FnOnce(&mut PreparedNativeKernelStartup<'_, '_>, u64, u64)) {
    let mut primary = crate::process::hle::unavailable::tests::original_input(0);
    // Give the original RX segment a real zero-fill tail. Initialized zero
    // bytes inside its file extent are legitimate source, not an invalid PC.
    primary[104..112].copy_from_slice(&(2 * crate::memory::PAGE_SIZE).to_le_bytes());
    with_native_image_session(
        &[&primary],
        &NativeImageLimits::default(),
        |owner, layout| {
            test(
                owner,
                layout.entry.stack.address + 0x100,
                layout.entry.entry,
            );
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}
fn request(
    owner: &PreparedNativeKernelStartup<'_, '_>,
    control: u64,
    entry: u64,
) -> NativeServiceRequest {
    let marker = *owner
        .plan
        .factory
        .markers()
        .iter()
        .find(|marker| marker.registration.service == pentax_kernel::Service::PthreadOnce)
        .unwrap();
    let pc = owner.plan.entry(marker) + 5;
    NativeServiceRequest {
        stack_pointer: None,
        vector_args: None,
        owner_id: owner.owner_id(),
        syscall_pc: pc,
        next_pc: pc + 2,
        service_id: marker.service_id.into(),
        args: [control, entry, 0, 0, 0, 0],
    }
}
fn word(owner: &mut PreparedNativeKernelStartup<'_, '_>, control: u64) -> u32 {
    let mut bytes = [0; 4];
    owner.pause().unwrap().read(control, &mut bytes).unwrap();
    u32::from_le_bytes(bytes)
}
#[test]
fn once_admission_checks_live_initialized_target_without_publishing_in_progress_or_done() {
    session(|owner, control, entry| {
        owner
            .pause()
            .unwrap()
            .write(control, &0_u32.to_le_bytes())
            .unwrap();
        let proof = owner
            .prepare_native_source(&BlockLimits::default())
            .unwrap();
        let module = &proof.materialized_source().source_plan().modules()[0];
        let segment = &module.runtime_plan().segments[0];
        let uninitialized = module.load_bias() + segment.vaddr + segment.file_bytes.len() as u64;
        assert!(segment.mem_size > segment.file_bytes.len() as u64);
        for target in [u64::MAX, uninitialized, control] {
            assert!(
                owner
                    .service_action(&proof, request(owner, control, target))
                    .is_err()
            );
            assert_eq!(word(owner, control), 0);
            assert!(owner.once.pending.is_empty());
        }
        let NativeServiceAction::Once(start) = owner
            .service_action(&proof, request(owner, control, entry))
            .unwrap()
        else {
            panic!("callback synthesized a return")
        };
        assert_eq!(start.entry(), entry);
        assert_eq!(word(owner, control), 0);
        assert_eq!(owner.once.pending.len(), 1);
        assert!(matches!(owner.once.pending[0].phase, Phase::Admitted));
        assert!(
            owner
                .service_action(&proof, request(owner, control, entry))
                .is_err()
        );
        assert_eq!(word(owner, control), 0);
        // The simple scalar API cannot execute this request or mark it done.
        assert!(
            owner
                .service_request(request(owner, control, entry))
                .is_err()
        );
        assert_eq!(word(owner, control), 0);
    });
}
#[test]
fn once_pending_admissions_are_bounded_and_done_calls_need_no_callback_target_access() {
    session(|owner, control, entry| {
        let proof = owner
            .prepare_native_source(&BlockLimits::default())
            .unwrap();
        for index in 0..MAX_PENDING {
            let cell = control + index as u64 * 8;
            owner
                .pause()
                .unwrap()
                .write(cell, &0_u64.to_le_bytes())
                .unwrap();
            assert!(matches!(
                owner.service_action(&proof, request(owner, cell, entry)),
                Ok(NativeServiceAction::Once(_))
            ));
            assert_eq!(word(owner, cell), 0);
        }
        let cell = control + MAX_PENDING as u64 * 8;
        owner
            .pause()
            .unwrap()
            .write(cell, &0_u32.to_le_bytes())
            .unwrap();
        assert!(matches!(
            owner.service_action(&proof, request(owner, cell, entry)),
            Err(KernelStartupError::Budget(_))
        ));
        assert_eq!(word(owner, cell), 0);
        assert_eq!(owner.once.pending.len(), MAX_PENDING);
        owner
            .pause()
            .unwrap()
            .write(cell, &1_u32.to_le_bytes())
            .unwrap();
        assert!(matches!(
            owner.service_action(&proof, request(owner, cell, u64::MAX)),
            Ok(NativeServiceAction::Returned(0))
        ));
        assert!(matches!(
            owner.service_action(&proof, request(owner, cell, 0)),
            Ok(NativeServiceAction::Returned(22))
        ));
        assert_eq!(word(owner, cell), 1);
    });
}
