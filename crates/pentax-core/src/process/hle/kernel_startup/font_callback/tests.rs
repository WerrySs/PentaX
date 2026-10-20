// SPDX-License-Identifier: GPL-2.0-only
//! Original admission/frame proofs; exit-only plans execute no native body.

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
    source: u64,
    output: u64,
    stack: u64,
) -> NativeServiceRequest {
    let marker = *owner
        .plan
        .factory
        .markers()
        .iter()
        .find(|marker| marker.registration.service == Service::FontCreateString)
        .unwrap();
    let pc = owner.plan.entry(marker) + 5;
    NativeServiceRequest {
        owner_id: owner.owner_id(),
        syscall_pc: pc,
        next_pc: pc + 2,
        service_id: marker.service_id.into(),
        args: [0, source, 0, output, 0, 0],
        vector_args: None,
        stack_pointer: Some(stack),
    }
}
fn initialize(owner: &mut PreparedNativeKernelStartup<'_, '_>, source: u64, target: u64) {
    let mut bytes = [0; 96];
    bytes[32..40].copy_from_slice(&target.to_le_bytes());
    owner.pause().unwrap().write(source, &bytes).unwrap();
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
        let pointer = owner.tls().instances()[owner.active_tls].thread_pointer;
        state.fs_base = Some(pointer + if wrong_tls { 0x1000 } else { 0 });
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

#[test]
fn font_parser_admission_refuses_unbacked_uninitialized_writable_targets_and_bad_scratch() {
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
                let source = layout.entry.stack.address + 0x100;
                let output = source + 0x100;
                let module = &proof.materialized_source().source_plan().modules()[0];
                let segment = &module.runtime_plan().segments[0];
                let target = match kind {
                    0 => u64::MAX,
                    1 => module.load_bias() + segment.vaddr + segment.file_bytes.len() as u64,
                    2 => source,
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
                initialize(owner, source, target);
                owner.pause().unwrap().write(output, &[0xa5; 8]).unwrap();
                let stack = if kind == 4 {
                    32
                } else {
                    layout.entry.return_slot
                };
                assert!(
                    owner
                        .service_action(&proof, request(owner, source, output, stack))
                        .is_err()
                );
                assert!(owner.font_parsers.pending.is_empty());
                let mut bytes = [0; 8];
                owner.pause().unwrap().read(output, &mut bytes).unwrap();
                assert_eq!(bytes, [0xa5; 8]);
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    }
}

#[test]
fn font_parser_frame_attests_actual_parent_rsp_and_consumes_observation_once_with_independent_tickets()
 {
    let primary = crate::process::hle::unavailable::tests::original_input(0);
    with_native_image_session(
        &[&primary],
        &NativeImageLimits::default(),
        |owner, layout| {
            let proof = owner
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            let source = layout.entry.stack.address + 0x100;
            let output = source + 0x100;
            initialize(owner, source, layout.entry.entry);
            let raw = request(owner, source, output, layout.entry.return_slot);
            let (mut engine, token) = stopped(owner, &proof, raw, layout.entry, false);
            let mut altered = raw;
            altered.stack_pointer = Some(layout.entry.return_slot - 64);
            let NativeServiceAction::FontParser(wrong_stack) =
                owner.service_action(&proof, altered).unwrap()
            else {
                panic!("parser callback was replaced with numeric success");
            };
            let call = engine
                .begin_syscall_call(
                    &mut owner.pause().unwrap(),
                    token,
                    wrong_stack.entry(),
                    wrong_stack.args(),
                )
                .unwrap();
            assert_eq!(call.parent_stack_pointer(), layout.entry.return_slot);
            assert!(
                owner
                    .observe_font_parser_call(&proof, wrong_stack, &call)
                    .is_err()
            );
            assert!(matches!(
                owner.font_parsers.pending[0].phase,
                Phase::Admitted
            ));

            let NativeServiceAction::FontParser(start) = owner.service_action(&proof, raw).unwrap()
            else {
                panic!("second independent parser was rejected");
            };
            assert_ne!(start, wrong_stack);
            assert_eq!(owner.font_parsers.pending.len(), 2);
            let (mut wrong_tls, token) = stopped(owner, &proof, raw, layout.entry, true);
            let call = wrong_tls
                .begin_syscall_call(
                    &mut owner.pause().unwrap(),
                    token,
                    start.entry(),
                    start.args(),
                )
                .unwrap();
            let foreign_call = call;
            assert!(
                owner
                    .observe_font_parser_call(&proof, start, &call)
                    .is_err()
            );
            assert!(matches!(
                owner.font_parsers.pending[owner.font_parsers.index(start).unwrap()].phase,
                Phase::Admitted
            ));
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
                .observe_font_parser_call(&proof, start, &call)
                .unwrap();
            let index = owner.font_parsers.index(start).unwrap();
            assert!(matches!(
                owner.font_parsers.pending[index].phase,
                Phase::Running(_)
            ));
            assert!(
                owner
                    .observe_font_parser_call(&proof, start, &call)
                    .is_err()
            );
            assert!(exact.observed_return().is_none());
            assert_eq!(exact.usage().native_entries, 0);
            let block = plan_materialized_block(
                proof.materialized_source(),
                0,
                start.entry(),
                &[0xc3],
                exact.placement(),
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
            // The safe exit-only boundary admits no native body. Its only
            // payload read is the owner-checked original stack return slot.
            assert!(matches!(
                exact
                    .step_return_exit(&mut owner.pause().unwrap(), &block)
                    .unwrap(),
                NativeBlockStop::Returned { .. }
            ));
            let returned = exact.observed_return().unwrap();
            let foreign_block = plan_materialized_block(
                proof.materialized_source(),
                0,
                start.entry(),
                &[0xc3],
                wrong_tls.placement(),
                &TranslationPolicy::x64_sse2(),
                &BlockLimits::default(),
            )
            .unwrap();
            assert!(
                foreign_block
                    .instructions()
                    .iter()
                    .all(|record| record.emitted_len() == 0)
            );
            assert_eq!(foreign_block.bytes(), [0x0f, 0x0b]);
            // The foreign frame likewise resolves only its original RET.
            assert!(matches!(
                wrong_tls
                    .step_return_exit(&mut owner.pause().unwrap(), &foreign_block)
                    .unwrap(),
                NativeBlockStop::Returned { .. }
            ));
            let foreign_return = wrong_tls.observed_return().unwrap();
            let before = (
                owner.kernel_usage().attempts,
                owner.kernel_usage().handles_live,
            );
            assert!(
                owner
                    .complete_font_parser_call(
                        &proof,
                        start,
                        &foreign_return,
                        foreign_call,
                        &wrong_tls
                    )
                    .is_err()
            );
            assert!(
                owner
                    .complete_font_parser_call(&proof, start, &returned, call, &wrong_tls)
                    .is_err()
            );
            assert!(matches!(
                owner.font_parsers.pending[index].phase,
                Phase::Running(_)
            ));
            exact.validate_syscall_call_return(call, &returned).unwrap();
            exact
                .complete_syscall_call(&mut owner.pause().unwrap(), call, 0)
                .unwrap();
            // Historical actual-return evidence is insufficient after the real
            // executor consumes that continuation, even with an unconsumed ticket.
            assert!(
                owner
                    .complete_font_parser_call(&proof, start, &returned, call, &exact)
                    .is_err()
            );
            assert!(matches!(
                owner.font_parsers.pending[index].phase,
                Phase::Running(_)
            ));
            assert_eq!(
                (
                    owner.kernel_usage().attempts,
                    owner.kernel_usage().handles_live
                ),
                before
            );
            let mut value = [0; 8];
            owner.pause().unwrap().read(output, &mut value).unwrap();
            assert_eq!(value, [0; 8]);
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}
