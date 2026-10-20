// SPDX-License-Identifier: GPL-2.0-only
//! Original IME event admission and exit-only callback provenance checks.

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
        .find(|marker| marker.registration.service == service)
        .unwrap();
    let pc = owner.plan.entry(marker) + 5;
    NativeServiceRequest {
        owner_id: owner.owner_id(),
        syscall_pc: pc,
        next_pc: pc + 2,
        service_id: marker.service_id.into(),
        args,
        vector_args: None,
        stack_pointer: stack,
    }
}
fn open(
    owner: &mut PreparedNativeKernelStartup<'_, '_>,
    proof: &PreparedNativeSource<'_, '_>,
    parameter: u64,
    target: u64,
) {
    let mut bytes = [0; 32];
    bytes[8..16].copy_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
    bytes[16..24].copy_from_slice(&target.to_le_bytes());
    owner.pause().unwrap().write(parameter, &bytes).unwrap();
    let call = request(
        owner,
        Service::ImeKeyboardOpen,
        [1, parameter, 0, 0, 0, 0],
        None,
    );
    assert!(matches!(
        owner.service_action(proof, call),
        Ok(NativeServiceAction::Returned(0))
    ));
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
fn ime_callback_admission_refuses_unbacked_uninitialized_writable_targets_and_missing_stack_capture()
 {
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
                let parameter = layout.entry.stack.address + 0x100;
                let module = &proof.materialized_source().source_plan().modules()[0];
                let segment = &module.runtime_plan().segments[0];
                let target = match kind {
                    0 => u64::MAX,
                    1 => module.load_bias() + segment.vaddr + segment.file_bytes.len() as u64,
                    2 => parameter,
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
                open(owner, &proof, parameter, target);
                let call = request(
                    owner,
                    Service::ImeUpdate,
                    [target, 0, 0, 0, 0, 0],
                    if kind == 4 {
                        None
                    } else {
                        Some(layout.entry.return_slot)
                    },
                );
                assert!(owner.service_action(&proof, call).is_err());
                assert!(owner.ime_callbacks.pending.is_empty());
                let mut unchanged = [0; 32];
                owner
                    .pause()
                    .unwrap()
                    .read(parameter, &mut unchanged)
                    .unwrap();
                assert_eq!(
                    u64::from_le_bytes(unchanged[16..24].try_into().unwrap()),
                    target
                );
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    }
}

#[test]
fn ime_callback_events_remain_owned_until_exact_current_return_and_reject_wrong_rsp_tls_replay_and_history()
 {
    let primary = crate::process::hle::unavailable::tests::original_input(0);
    with_native_image_session(
        &[&primary],
        &NativeImageLimits::default(),
        |owner, layout| {
            let proof = owner
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            let parameter = layout.entry.stack.address + 0x100;
            open(owner, &proof, parameter, layout.entry.entry);
            let raw = request(
                owner,
                Service::ImeUpdate,
                [layout.entry.entry, 0, 0, 0, 0, 0],
                Some(layout.entry.return_slot),
            );
            let (mut engine, token) = stopped(owner, &proof, raw, layout.entry, false);
            let mut altered = raw;
            altered.stack_pointer = Some(layout.entry.return_slot - 64);
            let NativeServiceAction::ImeCallback(wrong_stack) =
                owner.service_action(&proof, altered).unwrap()
            else {
                panic!("an event callback became a numeric result");
            };
            let call = engine
                .begin_syscall_call(
                    &mut owner.pause().unwrap(),
                    token,
                    wrong_stack.entry(),
                    wrong_stack.args(),
                )
                .unwrap();
            assert!(
                owner
                    .observe_ime_callback_call(&proof, wrong_stack, &call)
                    .is_err()
            );
            assert!(matches!(
                owner.ime_callbacks.pending[0].phase,
                Phase::Admitted
            ));
            let close = request(owner, Service::ImeKeyboardClose, [1, 0, 0, 0, 0, 0], None);
            assert!(matches!(
                owner.service_action(&proof, close),
                Ok(NativeServiceAction::Returned(0))
            ));
            open(owner, &proof, parameter, layout.entry.entry);
            let NativeServiceAction::ImeCallback(start) =
                owner.service_action(&proof, raw).unwrap()
            else {
                panic!("a nested update lost its independent event");
            };
            assert_ne!(start, wrong_stack);
            assert_ne!(start.args()[1], wrong_stack.args()[1]);
            assert_eq!(start.args()[0], 0x1122_3344_5566_7788);
            assert_eq!(start.args()[2], 0);
            let (mut foreign, token) = stopped(owner, &proof, raw, layout.entry, true);
            let foreign_call = foreign
                .begin_syscall_call(
                    &mut owner.pause().unwrap(),
                    token,
                    start.entry(),
                    start.args(),
                )
                .unwrap();
            assert!(
                owner
                    .observe_ime_callback_call(&proof, start, &foreign_call)
                    .is_err()
            );
            let (mut exact, token) = stopped(owner, &proof, raw, layout.entry, false);
            let call = exact
                .begin_syscall_call(
                    &mut owner.pause().unwrap(),
                    token,
                    start.entry(),
                    start.args(),
                )
                .unwrap();
            owner
                .observe_ime_callback_call(&proof, start, &call)
                .unwrap();
            assert!(
                owner
                    .observe_ime_callback_call(&proof, start, &call)
                    .is_err()
            );
            let index = owner.ime_callbacks.index(start).unwrap();
            return_only(owner, &proof, &mut exact, start.entry());
            return_only(owner, &proof, &mut foreign, start.entry());
            let returned = exact.observed_return().unwrap();
            let other = foreign.observed_return().unwrap();
            let before = (
                owner.kernel_usage().attempts,
                owner.kernel_usage().handles_live,
            );
            assert!(
                owner
                    .complete_ime_callback_call(&proof, start, &other, foreign_call, &foreign)
                    .is_err()
            );
            assert!(matches!(
                owner.ime_callbacks.pending[index].phase,
                Phase::Running(_)
            ));
            exact.validate_syscall_call_return(call, &returned).unwrap();
            exact
                .complete_syscall_call(&mut owner.pause().unwrap(), call, 0)
                .unwrap();
            assert!(
                owner
                    .complete_ime_callback_call(&proof, start, &returned, call, &exact)
                    .is_err()
            );
            assert!(matches!(
                owner.ime_callbacks.pending[index].phase,
                Phase::Running(_)
            ));
            assert_eq!(
                (
                    owner.kernel_usage().attempts,
                    owner.kernel_usage().handles_live
                ),
                before
            );
            let mut event = [0; 72];
            owner
                .pause()
                .unwrap()
                .read(start.args()[1], &mut event)
                .unwrap();
            assert_eq!(u32::from_le_bytes(event[..4].try_into().unwrap()), 256);
            assert_eq!(i32::from_le_bytes(event[8..12].try_into().unwrap()), 1);
            assert!(event[12..].iter().all(|byte| *byte == 0));
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}
