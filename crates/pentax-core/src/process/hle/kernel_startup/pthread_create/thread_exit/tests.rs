// SPDX-License-Identifier: GPL-2.0-only
//! Original checked exits prove cleanup cannot replace a child's entry value.

use super::*;
use crate::process::hle::{NativeImageLimits, NativeServiceRequest, with_native_image_session};
use pentax_codegen::{BlockLimits, TranslationPolicy, plan_materialized_block};
use pentax_kernel::Service;
use pentax_platform::native_dispatch::{
    NativeBlockStepper, NativeBlockStop, NativeCpuState, NativeStackFrame, PreSyscallToken,
};

fn service_request(
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
        owner_id: owner.owner_id(),
        syscall_pc: pc,
        next_pc: pc + 2,
        service_id: marker.service_id.into(),
        args,
        vector_args: None,
        stack_pointer: None,
    }
}

fn syscall_only<'view, 'plan, 'image>(
    owner: &mut PreparedNativeKernelStartup<'plan, 'image>,
    source: &'view PreparedNativeSource<'plan, 'image>,
    engine: &mut NativeBlockStepper<'view, 'plan, 'image>,
) -> PreSyscallToken {
    let block = plan_materialized_block(
        source.materialized_source(),
        owner.plan.kernel_provider_index(),
        engine.state().guest_pc,
        &[0x0f, 0x05],
        engine.placement(),
        &TranslationPolicy::x64_sse2(),
        &BlockLimits::default(),
    )
    .unwrap();
    let NativeBlockStop::PreSyscall(token) = engine
        .step_syscall_exit(&mut owner.pause().unwrap(), &block)
        .unwrap()
    else {
        panic!("original provider syscall did not stop");
    };
    token
}

fn return_only<'view, 'plan, 'image>(
    owner: &mut PreparedNativeKernelStartup<'plan, 'image>,
    source: &'view PreparedNativeSource<'plan, 'image>,
    engine: &mut NativeBlockStepper<'view, 'plan, 'image>,
    module: usize,
    expected: u64,
) {
    let block = plan_materialized_block(
        source.materialized_source(),
        module,
        engine.state().guest_pc,
        &[0xc3],
        engine.placement(),
        &TranslationPolicy::x64_sse2(),
        &BlockLimits::default(),
    )
    .unwrap();
    assert_eq!(block.bytes(), [0x0f, 0x0b]);
    assert_eq!(
        engine.step_return_exit(&mut owner.pause().unwrap(), &block),
        Ok(NativeBlockStop::Returned { value: expected })
    );
    assert_eq!(
        (
            engine.usage().native_entries,
            engine.usage().native_instructions
        ),
        (0, 0)
    );
}

#[test]
fn actual_created_thread_cleanup_preserves_entry_value_after_distinct_callback_result_and_refuses_replay()
 {
    let primary = crate::process::hle::unavailable::tests::original_input(0);
    with_native_image_session(
        &[&primary],
        &NativeImageLimits::default(),
        |owner, layout| {
            let source = owner.prepare_native_source(&Default::default()).unwrap();
            let getpid = service_request(owner, Service::Getpid, [0; 6]);
            let original_value = getpid.service_id;
            assert_ne!(original_value, 1);
            // This original callback begins at the exact generated Getpid SYSCALL
            // instruction. Its inherited RAX is deliberately the original result,
            // so one real admitted syscall changes callback RAX to process ID1.
            let register = service_request(
                owner,
                Service::SetThreadDtors,
                [getpid.syscall_pc, 0, 0, 0, 0, 0],
            );
            assert!(matches!(
                owner.service_action(&source, register),
                Ok(NativeServiceAction::Returned(0))
            ));
            let output = layout.entry.stack.address + 0x100;
            owner
                .pause()
                .unwrap()
                .write(output, &u64::MAX.to_le_bytes())
                .unwrap();
            let create = service_request(
                owner,
                Service::PthreadCreate,
                [output, 0, layout.entry.entry, 0, 0, 0],
            );
            let main = NativeStackFrame {
                stack: layout.entry.stack,
                guards: Some(layout.entry.guards),
                return_slot: layout.entry.return_slot,
                return_marker: layout.entry.return_marker,
            };
            owner
                .pause()
                .unwrap()
                .write(main.return_slot, &main.return_marker.to_le_bytes())
                .unwrap();
            let mut state = NativeCpuState::new(create.syscall_pc, main.return_slot);
            state.fs_base = Some(owner.tls().instances()[owner.active_tls].thread_pointer);
            state.gpr[0] = create.service_id;
            for (register, value) in [7, 6, 2, 1, 8, 9].into_iter().zip(create.args) {
                state.gpr[register] = value;
            }
            let mut engine = NativeBlockStepper::new(
                source.materialized_source(),
                &mut owner.pause().unwrap(),
                state,
                main,
                &Default::default(),
            )
            .unwrap();
            let create_token = syscall_only(owner, &source, &mut engine);
            let NativeServiceAction::CreateThread(pending) = owner
                .service_action(
                    &source,
                    NativeServiceRequest {
                        owner_id: create_token.owner_id(),
                        syscall_pc: create_token.syscall_pc(),
                        next_pc: create_token.next_pc(),
                        service_id: create_token.service_id(),
                        args: create_token.args(),
                        vector_args: Some(create_token.vector_args()),
                        stack_pointer: Some(create_token.stack_pointer()),
                    },
                )
                .unwrap()
            else {
                panic!("actual create stop did not admit a child");
            };
            let child = owner.prepare_created_thread(&source, pending).unwrap();
            let frame = NativeStackFrame {
                stack: child.layout.stack,
                guards: child.layout.guards,
                return_slot: child.layout.return_slot,
                return_marker: child.layout.return_marker,
            };
            let mut child_state = NativeCpuState::new(child.layout.entry, frame.return_slot);
            child_state.gpr[0] = original_value;
            child_state.gpr[7] = child.layout.argument;
            child_state.fs_base = Some(child.thread.thread_pointer());
            let mut parked = engine
                .prepare_thread(&mut owner.pause().unwrap(), child_state, frame)
                .unwrap();
            assert_eq!(
                owner
                    .publish_created_thread(&source, &child, &parked, create_token)
                    .unwrap(),
                0
            );
            let mut published = [0; 8];
            owner.pause().unwrap().read(output, &mut published).unwrap();
            assert_eq!(u64::from_le_bytes(published), child.layout.pthread_pointer);
            assert!(owner.thread_runtime.records[child.index].published);
            assert!(owner.thread_runtime.records[child.index].returned.is_none());
            engine.complete_syscall(create_token, 0).unwrap();
            engine
                .exchange_thread(&mut owner.pause().unwrap(), &mut parked)
                .unwrap();
            owner.activate_thread_context(child.thread()).unwrap();

            return_only(owner, &source, &mut engine, 0, original_value);
            let original_return = engine.observed_return().unwrap();
            assert!(
                !owner
                    .advance_created_thread_exit(&source, &child, &mut engine)
                    .unwrap()
            );
            assert!(owner.thread_runtime.records[child.index].returned.is_none());
            assert_eq!(
                owner.thread_runtime.records[child.index]
                    .exit
                    .unwrap()
                    .value,
                original_value
            );
            assert_eq!(engine.state().guest_pc, getpid.syscall_pc);
            assert_eq!(engine.state().fs_base, Some(child.thread.thread_pointer()));
            let callback_token = syscall_only(owner, &source, &mut engine);
            assert_eq!(
                (callback_token.syscall_pc(), callback_token.service_id()),
                (getpid.syscall_pc, original_value)
            );
            let NativeServiceAction::Returned(callback_value) = owner
                .service_action(
                    &source,
                    NativeServiceRequest {
                        owner_id: callback_token.owner_id(),
                        syscall_pc: callback_token.syscall_pc(),
                        next_pc: callback_token.next_pc(),
                        service_id: callback_token.service_id(),
                        args: callback_token.args(),
                        vector_args: Some(callback_token.vector_args()),
                        stack_pointer: Some(callback_token.stack_pointer()),
                    },
                )
                .unwrap()
            else {
                panic!("actual callback syscall did not return");
            };
            assert_eq!(callback_value, 1);
            engine
                .complete_syscall(callback_token, callback_value)
                .unwrap();
            let provider_module = owner.plan.kernel_provider_index();
            return_only(owner, &source, &mut engine, provider_module, callback_value);
            let callback_return = engine.observed_return().unwrap();
            assert_ne!(
                callback_return.call_sequence(),
                original_return.call_sequence()
            );
            assert!(engine.validate_call_return(&original_return).is_err());
            assert!(
                owner
                    .advance_created_thread_exit(&source, &child, &mut engine)
                    .unwrap()
            );
            let record = &owner.thread_runtime.records[child.index];
            assert_eq!(record.returned, Some(original_value));
            assert!(record.exit.is_none());
            assert_ne!(record.returned, Some(callback_return.value()));
            assert!(
                owner
                    .advance_created_thread_exit(&source, &child, &mut engine)
                    .is_err()
            );
            assert_eq!(
                owner.thread_runtime.records[child.index].returned,
                Some(original_value)
            );
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}
