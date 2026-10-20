// SPDX-License-Identifier: GPL-2.0-only
//! Original attested-body diagnostics; no CPU runs or private input data.
#![cfg(all(target_os = "windows", target_arch = "x86_64"))]

use super::*;
use crate::process::hle::{
    NativeImageLimits, unavailable::tests::original_input, with_native_image_session,
};
use pentax_platform::native_memory::{
    NativeAddressSpace, NativeMemoryEffect, NativeMemoryTransaction, NativePermissions,
};

fn request(
    session: &PreparedNativeKernelStartup<'_, '_>,
    slot: usize,
    object: u64,
) -> NativeServiceRequest {
    let binding = session.plan.json.unwrap();
    let marker = binding.factory.markers()[slot];
    NativeServiceRequest {
        owner_id: session.owner_id(),
        syscall_pc: binding.bias + marker.rva + 5,
        next_pc: binding.bias + marker.rva + 7,
        service_id: u64::from(marker.service_id),
        args: [object, u64::MAX, u64::MAX, 0, 0, 0],
        vector_args: None,
        stack_pointer: None,
    }
}

#[test]
fn actual_json_body_failures_have_public_labels_but_success_and_private_sites_do_not() {
    let input = original_input(0);
    with_native_image_session(&[&input], &NativeImageLimits::default(), |session, _| {
        let binding = session.plan.json.unwrap();
        for marker in binding.factory.markers() {
            assert_eq!(
                marker.function.public_name().is_some(),
                marker.function != JsonFunction::VirtualCall
            );
        }
        // Both bodies fail after attestation, including a body-local `?` path.
        let invalid_value = request(session, 2, binding.factory.address_point());
        assert!(matches!(
            session.service_request(invalid_value),
            Err(KernelStartupError::Kernel(pentax_kernel::Failure::Guest(_)))
        ));
        assert_eq!(
            session.last_admitted_kernel_service(),
            Some("LibJson2::JsonValueCtor")
        );
        let invalid_allocator = request(session, 0, 0);
        assert!(matches!(
            session.service_request(invalid_allocator),
            Err(KernelStartupError::InvalidProvider(
                "null JSON constructor object"
            ))
        ));
        assert_eq!(
            session.last_admitted_kernel_service(),
            Some("LibJson2::JsonMemAllocatorCtor")
        );

        let opaque = request(session, 34, u64::MAX);
        assert_eq!(session.service_request(opaque), Ok(u64::MAX));
        assert_eq!(session.last_admitted_kernel_service(), None);
        let private = request(session, 1, 0);
        assert_eq!(
            session.service_request(private),
            Err(KernelStartupError::UnavailableJsonAllocatorVirtual)
        );
        assert_eq!(session.last_admitted_kernel_service(), None);
        Ok::<_, ()>(())
    })
    .unwrap();
}

#[test]
fn rejected_requests_and_live_attestation_failures_clear_prior_json_body_attribution() {
    let input = original_input(0);
    with_native_image_session(&[&input], &NativeImageLimits::default(), |session, _| {
        let binding = session.plan.json.unwrap();
        let body_failure = request(session, 2, binding.factory.address_point());
        let foreign = NativeAddressSpace::reserve_any(Default::default(), 65536).unwrap();
        for variant in 0..4 {
            assert!(session.service_request(body_failure).is_err());
            assert_eq!(
                session.last_admitted_kernel_service(),
                Some("LibJson2::JsonValueCtor")
            );
            let mut forged = body_failure;
            match variant {
                0 => forged.owner_id = foreign.owner_id(),
                1 => forged.next_pc += 1,
                2 => forged.service_id |= 1 << 32,
                _ => {
                    forged.syscall_pc += 1;
                    forged.next_pc += 1;
                }
            }
            assert!(session.service_request(forged).is_err());
            assert_eq!(session.last_admitted_kernel_service(), None);
        }

        assert!(session.service_request(body_failure).is_err());
        let table = binding.bias + PAGE_SIZE;
        {
            let mut paused = session.pause().unwrap();
            paused
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::Protect {
                            address: table,
                            size: PAGE_SIZE,
                            permissions: NativePermissions::READ_WRITE,
                        },
                        writes: &[],
                    },
                    &Default::default(),
                )
                .unwrap();
        }
        assert!(matches!(
            session.service_request(body_failure),
            Err(KernelStartupError::InvalidProvider(_))
        ));
        assert_eq!(session.last_admitted_kernel_service(), None);
        {
            let mut paused = session.pause().unwrap();
            paused
                .write(binding.factory.address_point(), &[0; 8])
                .unwrap();
            paused
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::Protect {
                            address: table,
                            size: PAGE_SIZE,
                            permissions: NativePermissions::READ,
                        },
                        writes: &[],
                    },
                    &Default::default(),
                )
                .unwrap();
        }
        assert!(matches!(
            session.service_request(body_failure),
            Err(KernelStartupError::InvalidProvider(_))
        ));
        assert_eq!(session.last_admitted_kernel_service(), None);
        Ok::<_, ()>(())
    })
    .unwrap();
}
