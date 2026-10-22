// SPDX-License-Identifier: GPL-2.0-only
//! Original admission-only fixtures on actual owned pages. No guest code runs.
use super::*;
use crate::process::hle::{NativeImageLimits, with_native_image_session};
use pentax_kernel::Service;
use pentax_platform::native_dispatch::{NativeBlockStepper, NativeCpuState};
fn request(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    proof: &PreparedNativeSource<'_, '_>,
    service: Service,
    args: [u64; 6],
    rsp: Option<u64>,
) -> Result<NativeServiceAction, KernelStartupError> {
    let marker = *session
        .plan
        .factory
        .markers()
        .iter()
        .find(|m| m.registration.service == service)
        .unwrap();
    let pc = session.plan.entry(marker) + 5;
    session.service_action(
        proof,
        NativeServiceRequest {
            owner_id: session.owner_id(),
            syscall_pc: pc,
            next_pc: pc + 2,
            service_id: marker.service_id.into(),
            args,
            vector_args: None,
            stack_pointer: rsp,
        },
    )
}
fn initialize(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    proof: &PreparedNativeSource<'_, '_>,
    root: NativeEntryLayout,
    entry: u64,
    context: u64,
) -> u64 {
    let object = root.stack.address + 0x100;
    let name = object + 0x200;
    let rsp = root.return_slot - 0x100;
    {
        let mut paused = session.pause().unwrap();
        paused.write(name, b"original fiber\0").unwrap();
        paused.write(rsp + 8, &[0; 16]).unwrap();
    }
    assert!(matches!(
        request(
            session,
            proof,
            Service::FiberInitialize,
            [object, name, entry, 0x1234, context, 512],
            Some(rsp)
        ),
        Ok(NativeServiceAction::Returned(0))
    ));
    object
}
#[test]
fn source_bound_layout_and_real_prepared_origin_do_not_fabricate_a_syscall_transfer() {
    let primary = crate::process::hle::unavailable::tests::original_input(0);
    let limits = NativeImageLimits {
        kernel_window_bytes: 16 * PAGE_SIZE,
        ..Default::default()
    };
    with_native_image_session(&[&primary], &limits, |session, layout| {
        let proof = session.prepare_native_source(&Default::default()).unwrap();
        let address = session.plan.limits.kernel.virtual_base + 8 * PAGE_SIZE;
        session
            .pause()
            .unwrap()
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::MapZeroed {
                        address,
                        size: PAGE_SIZE,
                        permissions: NativePermissions::READ_WRITE,
                    },
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
        let object = initialize(session, &proof, layout.entry, layout.entry.entry, address);
        let NativeServiceAction::Fiber(pending) = request(
            session,
            &proof,
            Service::FiberRun,
            [object, 0x5678, 0, 0, 0, 0],
            Some(layout.entry.return_slot),
        )
        .unwrap() else {
            panic!("expected fiber action")
        };
        let fresh = pending.target_layout().unwrap();
        assert_eq!(fresh.args, [0x1234, 0x5678]);
        {
            let paused = session.hle.pause().unwrap();
            assert!(
                session
                    .fiber_runtime
                    .preflight_thread_stack(
                        &paused,
                        fresh.frame.stack,
                        &session.kernel,
                        &mut session.provider_work,
                        session.plan.limits.max_provider_work
                    )
                    .is_err()
            );
        }

        assert_eq!(fresh.frame.stack, NativeRange { address, size: 512 });
        assert!(fresh.frame.guards.is_none());
        let mut marker = [0; 8];
        session
            .pause()
            .unwrap()
            .read(fresh.frame.return_slot, &mut marker)
            .unwrap();
        assert_eq!(u64::from_le_bytes(marker), layout.entry.return_marker);
        let root_frame = NativeStackFrame {
            stack: layout.entry.stack,
            guards: Some(layout.entry.guards),
            return_slot: layout.entry.return_slot,
            return_marker: layout.entry.return_marker,
        };
        let root_state = NativeCpuState::new(layout.entry.entry, root_frame.return_slot);
        let mut engine = NativeBlockStepper::new(
            proof.materialized_source(),
            &mut session.pause().unwrap(),
            root_state,
            root_frame,
            &Default::default(),
        )
        .unwrap();
        let mut state = root_state;
        state.guest_pc = fresh.entry;
        state.gpr[4] = fresh.frame.return_slot;
        state.gpr[7] = fresh.args[0];
        state.gpr[6] = fresh.args[1];
        let mut parked = engine
            .prepare_fiber(&mut session.pause().unwrap(), state, fresh.frame)
            .unwrap();
        assert!(
            session
                .commit_fiber_action(&proof, pending, &parked)
                .is_err()
        );
        session
            .observe_fiber_target(&proof, &pending, &parked)
            .unwrap();
        assert!(
            session
                .observe_fiber_target(&proof, &pending, &parked)
                .is_err()
        );
        engine
            .exchange_fiber(&mut session.pause().unwrap(), &mut parked)
            .unwrap();
        // The original root has never reached Run's real PRE-SYSCALL token.
        // A valid context exchange alone cannot authorize ABI publication.
        assert!(
            session
                .commit_fiber_action(&proof, pending, &parked)
                .is_err()
        );
        let mut state = [0; 4];
        session
            .pause()
            .unwrap()
            .read(object + 4, &mut state)
            .unwrap();
        assert_eq!(u32::from_le_bytes(state), 2);
        assert!(session.kernel.active_fiber().is_none());
        Ok::<_, ()>(())
    })
    .unwrap();
}
#[test]
fn writable_non_source_entry_refuses_before_stack_marker_or_running_publication() {
    let primary = crate::process::hle::unavailable::tests::original_input(0);
    let limits = NativeImageLimits {
        kernel_window_bytes: 16 * PAGE_SIZE,
        ..Default::default()
    };
    with_native_image_session(&[&primary], &limits, |session, layout| {
        let proof = session.prepare_native_source(&Default::default()).unwrap();
        let address = session.plan.limits.kernel.virtual_base + 8 * PAGE_SIZE;
        session
            .pause()
            .unwrap()
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::MapZeroed {
                        address,
                        size: PAGE_SIZE,
                        permissions: NativePermissions::READ_WRITE,
                    },
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
        let object = initialize(session, &proof, layout.entry, address, address);
        let slot = address + 512 - 8;
        session
            .pause()
            .unwrap()
            .write(slot, &0x5555_aaaa_u64.to_le_bytes())
            .unwrap();
        assert!(matches!(
            request(
                session,
                &proof,
                Service::FiberRun,
                [object, 7, 0, 0, 0, 0],
                Some(layout.entry.return_slot)
            ),
            Err(KernelStartupError::InvalidProvider(_))
        ));
        let mut marker = [0; 8];
        session.pause().unwrap().read(slot, &mut marker).unwrap();
        assert_eq!(u64::from_le_bytes(marker), 0x5555_aaaa);
        let mut state = [0; 4];
        session
            .pause()
            .unwrap()
            .read(object + 4, &mut state)
            .unwrap();
        assert_eq!(u32::from_le_bytes(state), 2);
        assert!(session.kernel.active_fiber().is_none());
        Ok::<_, ()>(())
    })
    .unwrap();
}

#[test]
fn provided_context_in_pthread_stack_refuses_before_return_slot_canary_write() {
    let primary = crate::process::hle::unavailable::tests::original_input(0);
    with_native_image_session(
        &[&primary],
        &NativeImageLimits::default(),
        |session, layout| {
            let proof = session.prepare_native_source(&Default::default()).unwrap();
            let context = layout.entry.stack.address + 0x800;
            let object = initialize(session, &proof, layout.entry, layout.entry.entry, context);
            let slot = context + 512 - 8;
            session
                .pause()
                .unwrap()
                .write(slot, &0x75aa_31cc_u64.to_le_bytes())
                .unwrap();
            assert!(
                request(
                    session,
                    &proof,
                    Service::FiberRun,
                    [object, 7, 0, 0, 0, 0],
                    Some(layout.entry.return_slot)
                )
                .is_err()
            );
            let mut marker = [0; 8];
            session.pause().unwrap().read(slot, &mut marker).unwrap();
            assert_eq!(u64::from_le_bytes(marker), 0x75aa_31cc);
            let mut state = [0; 4];
            session
                .pause()
                .unwrap()
                .read(object + 4, &mut state)
                .unwrap();
            assert_eq!(u32::from_le_bytes(state), 2);
            assert!(session.kernel.active_fiber().is_none());
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}
