// SPDX-License-Identifier: GPL-2.0-only
//! Original checked admissions and exit-only native continuation receipts.
use super::*;
use crate::process::hle::{
    NativeEntryLayout, NativeImageLimits, NativeServiceRequest, with_native_image_session,
};
use pentax_codegen::{BlockLimits, TranslationPolicy, plan_materialized_block};
use pentax_kernel::Service;
use pentax_platform::native_dispatch::{
    NativeBlockStepper, NativeBlockStop, NativeCpuState, NativeStackFrame, PreSyscallToken,
};
use pentax_platform::native_memory::{
    NativeMemoryEffect, NativeMemoryTransaction, NativePermissions,
};
fn request(
    owner: &PreparedNativeKernelStartup<'_, '_>,
    service: Service,
    args: [u64; 6],
    stack: Option<u64>,
) -> NativeServiceRequest {
    let marker = *owner
        .plan
        .factory
        .markers()
        .iter()
        .find(|m| m.registration.service == service)
        .unwrap();
    let pc = owner.plan.entry(marker) + 5;
    NativeServiceRequest {
        stack_pointer: stack,
        vector_args: None,
        owner_id: owner.owner_id(),
        syscall_pc: pc,
        next_pc: pc + 2,
        service_id: marker.service_id.into(),
        args,
    }
}
fn register(
    owner: &mut PreparedNativeKernelStartup<'_, '_>,
    proof: &PreparedNativeSource<'_, '_>,
    entry: u64,
    data: u64,
    ordinal: u64,
) {
    let raw = request(
        owner,
        Service::NpRegisterStateCallbackA,
        [entry, data, 0, 0, 0, 0],
        None,
    );
    assert!(
        matches!(owner.service_action(proof,raw),Ok(NativeServiceAction::Returned(value))if value==ordinal)
    );
}
fn stopped<'view, 'plan, 'image>(
    owner: &mut PreparedNativeKernelStartup<'plan, 'image>,
    proof: &'view PreparedNativeSource<'plan, 'image>,
    request: NativeServiceRequest,
    layout: NativeEntryLayout,
    wrong_tls: bool,
) -> (NativeBlockStepper<'view, 'plan, 'image>, PreSyscallToken) {
    let stack = NativeStackFrame {
        stack: layout.stack,
        guards: None,
        return_slot: layout.return_slot,
        return_marker: layout.return_marker,
    };
    owner
        .pause()
        .unwrap()
        .write(stack.return_slot, &stack.return_marker.to_le_bytes())
        .unwrap();
    let mut state = NativeCpuState::new(request.syscall_pc, layout.return_slot);
    if owner.tls().thread_control_policy() != crate::process::tls::ThreadControlBlockPolicy::None {
        state.fs_base = Some(
            owner.tls().instances()[owner.active_tls].thread_pointer
                + if wrong_tls { 0x1000 } else { 0 },
        );
    }
    state.gpr[0] = request.service_id;
    for (register, value) in [7, 6, 2, 1, 8, 9].into_iter().zip(request.args) {
        state.gpr[register] = value;
    }
    let mut engine = NativeBlockStepper::new(
        proof.materialized_source(),
        &mut owner.pause().unwrap(),
        state,
        stack,
        &Default::default(),
    )
    .unwrap();
    let block = plan_materialized_block(
        proof.materialized_source(),
        owner.plan.kernel_provider_index(),
        request.syscall_pc,
        &[0x0f, 0x05],
        engine.placement(),
        &TranslationPolicy::x64_sse2(),
        &BlockLimits::default(),
    )
    .unwrap();
    assert!(
        block
            .instructions()
            .iter()
            .all(|record| record.emitted_len() == 0)
    );
    let NativeBlockStop::PreSyscall(token) = engine
        .step_syscall_exit(&mut owner.pause().unwrap(), &block)
        .unwrap()
    else {
        panic!("expected an exit-only syscall stop");
    };
    assert_eq!(engine.usage().native_entries, 0);
    (engine, token)
}
fn return_only<'view, 'plan, 'image>(
    owner: &mut PreparedNativeKernelStartup<'plan, 'image>,
    proof: &'view PreparedNativeSource<'plan, 'image>,
    engine: &mut NativeBlockStepper<'view, 'plan, 'image>,
    entry: u64,
) {
    let block = plan_materialized_block(
        proof.materialized_source(),
        0,
        entry,
        &[0xc3],
        engine.placement(),
        &TranslationPolicy::x64_sse2(),
        &BlockLimits::default(),
    )
    .unwrap();
    assert!(
        block
            .instructions()
            .iter()
            .all(|record| record.emitted_len() == 0)
    );
    assert_eq!(block.bytes(), [0x0f, 0x0b]);
    assert!(matches!(
        engine
            .step_return_exit(&mut owner.pause().unwrap(), &block)
            .unwrap(),
        NativeBlockStop::Returned { .. }
    ));
    assert_eq!(engine.usage().native_entries, 0);
}

#[test]
fn np_callback_failed_target_and_missing_parent_admission_are_terminal_and_never_empty_success() {
    for kind in 0..5 {
        let mut primary = crate::process::hle::unavailable::tests::original_input(0);
        primary[104..112].copy_from_slice(&(2 * crate::memory::PAGE_SIZE).to_le_bytes());
        with_native_image_session(
            &[&primary],
            &NativeImageLimits::default(),
            |owner, layout| {
                let proof = owner
                    .prepare_native_source(&BlockLimits::default())
                    .unwrap();
                let module = &proof.materialized_source().source_plan().modules()[0];
                let segment = &module.runtime_plan().segments[0];
                let target = match kind {
                    0 => u64::MAX,
                    1 => module.load_bias() + segment.vaddr + segment.file_bytes.len() as u64,
                    2 => layout.entry.stack.address + 0x100,
                    3 => {
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
                    _ => layout.entry.entry,
                };
                register(owner, &proof, target, 7, 1);
                let raw = request(
                    owner,
                    Service::NpCheckCallback,
                    [0; 6],
                    if kind == 4 {
                        None
                    } else {
                        Some(layout.entry.return_slot)
                    },
                );
                assert!(owner.service_action(&proof, raw).is_err());
                assert!(owner.np_callbacks.pending.iter().all(Option::is_none));
                assert!(owner.service_action(&proof, raw).is_err()); // No poisoned snapshot becomes numeric success.
                assert_eq!(owner.kernel_usage().handles_live, 1);
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    }
}
#[test]
fn np_callback_empty_recursive_check_and_independent_new_snapshot_retain_outer_ticket_and_reject_foreign_owner()
 {
    let primary = crate::process::hle::unavailable::tests::original_input(0);
    with_native_image_session(
        &[&primary],
        &NativeImageLimits::default(),
        |owner, layout| {
            let proof = owner
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            register(owner, &proof, layout.entry.entry, 11, 1);
            let raw = request(
                owner,
                Service::NpCheckCallback,
                [0; 6],
                Some(layout.entry.return_slot),
            );
            let foreign =
                pentax_platform::native_memory::NativeAddressSpace::new(Default::default())
                    .unwrap();
            let mut bad = raw;
            bad.owner_id = foreign.owner_id();
            assert!(owner.service_action(&proof, bad).is_err());
            let NativeServiceAction::Np(outer) = owner.service_action(&proof, raw).unwrap() else {
                panic!("lost pending callback")
            };
            assert!(matches!(
                owner.service_action(&proof, raw),
                Ok(NativeServiceAction::Returned(0))
            ));
            register(owner, &proof, layout.entry.entry + 1, 22, 2);
            let NativeServiceAction::Np(inner) = owner.service_action(&proof, raw).unwrap() else {
                panic!("lost reentrant callback")
            };
            assert_ne!(outer, inner);
            assert_eq!(outer.args(), [1, 1, 11]);
            assert_eq!(inner.args(), [1, 1, 22]);
            assert!(matches!(
                owner.np_callbacks.pending(outer).unwrap().phase,
                Phase::Admitted
            ));
            assert!(matches!(
                owner.np_callbacks.pending(inner).unwrap().phase,
                Phase::Admitted
            ));
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}
#[test]
fn np_callback_exact_active_return_rejects_wrong_tls_executor_replay_and_consumed_parent_frame() {
    for consumed in [false, true] {
        let primary = crate::process::hle::unavailable::tests::original_input(0);
        with_native_image_session(
            &[&primary],
            &NativeImageLimits::default(),
            |owner, layout| {
                let proof = owner
                    .prepare_native_source(&BlockLimits::default())
                    .unwrap();
                register(owner, &proof, layout.entry.entry, 77, 1);
                let raw = request(
                    owner,
                    Service::NpCheckCallback,
                    [0; 6],
                    Some(layout.entry.return_slot),
                );
                let NativeServiceAction::Np(start) = owner.service_action(&proof, raw).unwrap()
                else {
                    panic!("pending callback became success")
                };
                let (mut foreign, token) = stopped(owner, &proof, raw, layout.entry, true);
                let other = foreign
                    .begin_syscall_call(
                        &mut owner.pause().unwrap(),
                        token,
                        start.entry(),
                        start.args(),
                    )
                    .unwrap();
                assert!(
                    owner
                        .observe_np_callback_call(&proof, start, &other)
                        .is_err()
                );
                let (mut exact, token) = stopped(owner, &proof, raw, layout.entry, false);
                let nested = exact
                    .begin_syscall_call(
                        &mut owner.pause().unwrap(),
                        token,
                        start.entry(),
                        start.args(),
                    )
                    .unwrap();
                owner
                    .observe_np_callback_call(&proof, start, &nested)
                    .unwrap();
                assert!(
                    owner
                        .observe_np_callback_call(&proof, start, &nested)
                        .is_err()
                );
                return_only(owner, &proof, &mut exact, start.entry());
                return_only(owner, &proof, &mut foreign, start.entry());
                let returned = exact.observed_return().unwrap();
                let wrong = foreign.observed_return().unwrap();
                assert!(
                    owner
                        .complete_np_callback_call(&proof, start, &wrong, other, &foreign)
                        .is_err()
                );
                if consumed {
                    exact
                        .complete_syscall_call(&mut owner.pause().unwrap(), nested, 0)
                        .unwrap();
                    assert!(
                        owner
                            .complete_np_callback_call(&proof, start, &returned, nested, &exact)
                            .is_err()
                    );
                    assert!(matches!(
                        owner.np_callbacks.pending(start).unwrap().phase,
                        Phase::Running(_)
                    ));
                } else {
                    assert!(matches!(
                        owner.complete_np_callback_call(&proof, start, &returned, nested, &exact),
                        Ok(NativeServiceAction::Returned(0))
                    ));
                    assert!(
                        owner
                            .complete_np_callback_call(&proof, start, &returned, nested, &exact)
                            .is_err()
                    );
                    exact
                        .complete_syscall_call(&mut owner.pause().unwrap(), nested, 0)
                        .unwrap();
                    assert!(owner.np_callbacks.pending.iter().all(Option::is_none));
                }
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    }
}

#[test]
fn np_callback_actual_return_budget_failure_and_terminal_abort_do_not_discard_undelivered_snapshot()
{
    for work_failure in [false, true] {
        let primary = crate::process::hle::unavailable::tests::original_input(0);
        with_native_image_session(
            &[&primary],
            &NativeImageLimits::default(),
            |owner, layout| {
                let proof = owner
                    .prepare_native_source(&BlockLimits::default())
                    .unwrap();
                register(owner, &proof, layout.entry.entry, 11, 1);
                register(owner, &proof, layout.entry.entry + 1, 22, 2);
                let raw = request(
                    owner,
                    Service::NpCheckCallback,
                    [0; 6],
                    Some(layout.entry.return_slot),
                );
                let NativeServiceAction::Np(start) = owner.service_action(&proof, raw).unwrap()
                else {
                    panic!("pending callback became success")
                };
                if work_failure {
                    let (mut exact, token) = stopped(owner, &proof, raw, layout.entry, false);
                    let nested = exact
                        .begin_syscall_call(
                            &mut owner.pause().unwrap(),
                            token,
                            start.entry(),
                            start.args(),
                        )
                        .unwrap();
                    owner
                        .observe_np_callback_call(&proof, start, &nested)
                        .unwrap();
                    return_only(owner, &proof, &mut exact, start.entry());
                    let returned = exact.observed_return().unwrap();
                    // Model already-consumed work, never increase/refund the production quota.
                    owner.provider_work = owner.plan.limits.max_provider_work - 88;
                    assert!(
                        owner
                            .complete_np_callback_call(&proof, start, &returned, nested, &exact)
                            .is_err()
                    );
                } else {
                    let mut forged = start;
                    forged.serial += 1;
                    assert!(owner.abort_np_callback_call(&proof, forged).is_err());
                    assert!(matches!(
                        owner.np_callbacks.pending(start).unwrap().phase,
                        Phase::Admitted
                    ));
                    owner.abort_np_callback_call(&proof, start).unwrap();
                    owner.abort_np_callbacks(); // Same unconditional cleanup used on every worker terminal failure.
                }
                assert!(owner.kernel.admit_np_callbacks().is_err());
                assert!(matches!(
                    owner.np_callbacks.pending(start).unwrap().phase,
                    Phase::Failed
                ));
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    }
}
