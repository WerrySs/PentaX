// SPDX-License-Identifier: GPL-2.0-only
//! Admission checks only; actual guest alternation is exercised by the worker.
use super::*;
use crate::process::hle::{NativeImageLimits, with_native_image_session};
use pentax_kernel::Service;

#[test]
fn yield_action_requires_exact_owner_source_marker_and_continuation() {
    let primary = crate::process::hle::unavailable::tests::original_input(0);
    with_native_image_session(&[&primary], &NativeImageLimits::default(), |owner, _| {
        let proof = owner.prepare_native_source(&Default::default()).unwrap();
        let marker = *owner
            .plan
            .factory
            .markers()
            .iter()
            .find(|marker| marker.registration.service == Service::PthreadYield)
            .unwrap();
        let pc = owner.plan.entry(marker) + 5;
        let request = NativeServiceRequest {
            owner_id: owner.owner_id(),
            syscall_pc: pc,
            next_pc: pc + 2,
            service_id: marker.service_id.into(),
            args: [u64::MAX; 6],
            vector_args: None,
            stack_pointer: None,
        };
        assert!(matches!(
            owner.service_action(&proof, request),
            Ok(NativeServiceAction::YieldThread)
        ));
        let mut wrong_id = request;
        wrong_id.service_id += 1;
        assert!(matches!(
            owner.service_action(&proof, wrong_id),
            Err(KernelStartupError::ServiceIdMismatch { .. })
        ));
        let mut wrong_pc = request;
        wrong_pc.next_pc += 1;
        assert!(matches!(
            owner.service_action(&proof, wrong_pc),
            Err(KernelStartupError::ContinuationMismatch { .. })
        ));
        assert!(matches!(
            owner.service_request(request),
            Err(KernelStartupError::Kernel(
                pentax_kernel::Failure::Unsupported("cooperative guest-thread yield")
            ))
        ));
        with_native_image_session(&[&primary], &NativeImageLimits::default(), |foreign, _| {
            let foreign_proof = foreign.prepare_native_source(&Default::default()).unwrap();
            assert!(matches!(
                foreign.service_action(&proof, request),
                Err(KernelStartupError::InvalidProvider(
                    "stale native service source"
                ))
            ));
            assert!(matches!(
                foreign.service_action(&foreign_proof, request),
                Err(KernelStartupError::NativeOwnerMismatch)
            ));
            Ok::<_, ()>(())
        })
        .unwrap();
        Ok::<_, ()>(())
    })
    .unwrap();
}
