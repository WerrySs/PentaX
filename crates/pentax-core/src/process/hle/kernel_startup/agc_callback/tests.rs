// SPDX-License-Identifier: GPL-2.0-only
//! Original genuine nested-callback and source-ordered non-replay proofs.
use super::*;
use crate::process::hle::{NativeEntryLayout, NativeImageLimits, with_native_image_session};
use pentax_codegen::{BlockLimits, TranslationPolicy, plan_materialized_block};
use pentax_platform::native_dispatch::{
    NativeBlockStepper, NativeBlockStop, NativeCpuState, NativeStackFrame, PreSyscallToken,
};
use pentax_platform::native_memory::{
    NativeMemoryEffect, NativeMemoryTransaction, NativePermissions,
};

fn request(
    owner: &PreparedNativeKernelStartup<'_, '_>,
    args: [u64; 6],
    stack: u64,
) -> NativeServiceRequest {
    let binding = owner.plan.graphics.unwrap();
    let marker = binding
        .factory
        .markers()
        .find(|marker| marker.function == crate::gpu::driver::Function::AgcDcbWriteData)
        .unwrap();
    let pc = binding.bias + marker.rva() + 5;
    NativeServiceRequest {
        owner_id: owner.owner_id(),
        syscall_pc: pc,
        next_pc: pc + 2,
        service_id: marker.id().into(),
        args,
        vector_args: None,
        stack_pointer: Some(stack),
    }
}
fn stopped<'view, 'plan, 'image>(
    owner: &mut PreparedNativeKernelStartup<'plan, 'image>,
    proof: &'view PreparedNativeSource<'plan, 'image>,
    request: NativeServiceRequest,
    layout: NativeEntryLayout,
) -> (NativeBlockStepper<'view, 'plan, 'image>, PreSyscallToken) {
    let stack = NativeStackFrame {
        stack: layout.stack,
        guards: None,
        return_slot: request.stack_pointer.unwrap(),
        return_marker: layout.return_marker,
    };
    owner
        .pause()
        .unwrap()
        .write(stack.return_slot, &stack.return_marker.to_le_bytes())
        .unwrap();
    let mut state = NativeCpuState::new(request.syscall_pc, stack.return_slot);
    if owner.tls().thread_control_policy() != crate::process::tls::ThreadControlBlockPolicy::None {
        state.fs_base = Some(owner.tls().instances()[owner.active_tls].thread_pointer);
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
    let binding = owner.plan.graphics.unwrap();
    let block = plan_materialized_block(
        proof.materialized_source(),
        binding.index,
        request.syscall_pc,
        &[15, 5],
        engine.placement(),
        &TranslationPolicy::x64_sse2(),
        &BlockLimits::default(),
    )
    .unwrap();
    let NativeBlockStop::PreSyscall(token) = engine
        .step_syscall_exit(&mut owner.pause().unwrap(), &block)
        .unwrap()
    else {
        panic!("expected syscall")
    };
    (engine, token)
}
fn descriptor(
    owner: &mut PreparedNativeKernelStartup<'_, '_>,
    buf: u64,
    up: u64,
    target: u64,
    user: u64,
) {
    let mut bytes = [0; 56];
    bytes[16..24].copy_from_slice(&up.to_le_bytes());
    bytes[24..32].copy_from_slice(&up.to_le_bytes()); // Requires actual growth.
    bytes[32..40].copy_from_slice(&target.to_le_bytes());
    bytes[40..48].copy_from_slice(&user.to_le_bytes());
    bytes[48..52].copy_from_slice(&2u32.to_le_bytes());
    owner.pause().unwrap().write(buf, &bytes).unwrap();
}

#[test]
fn actual_grow_callback_mutations_feed_current_descriptor_and_payload_once_or_preserve_fault_prefix()
 {
    // Original callback: load replacement UP from user data, publish UP/DOWN,
    // change one payload byte, and return true. No proprietary input is used.
    let code = [
        0x48, 0x8b, 0x02, 0x48, 0x89, 0x47, 0x10, 0x48, 0x05, 0x00, 0x01, 0x00, 0x00, 0x48, 0x89,
        0x47, 0x18, 0xc6, 0x42, 0x08, 0x5c, 0xb8, 0x01, 0x00, 0x00, 0x00, 0xc3,
    ];
    for bad_payload in [false, true] {
        let mut primary = crate::process::hle::unavailable::tests::original_input(0);
        primary[0x100..0x100 + code.len()].copy_from_slice(&code);
        with_native_image_session(
            &[&primary],
            &NativeImageLimits::default(),
            |owner, layout| {
                let proof = owner
                    .prepare_native_source(&BlockLimits::default())
                    .unwrap();
                let base = layout.entry.stack.address;
                let (buf, user, up) = (base + 0x100, base + 0x200, base + 0x300);
                descriptor(owner, buf, up + 0x100, layout.entry.entry, user);
                owner
                    .pause()
                    .unwrap()
                    .write(user, &up.to_le_bytes())
                    .unwrap();
                owner.pause().unwrap().write(user + 8, &[0x4a; 8]).unwrap();
                owner.pause().unwrap().write(up, &[0xa5; 32]).unwrap();
                // Two real extra stack arguments: increment and write-confirm.
                let parent_rsp = layout.entry.return_slot - 64;
                owner
                    .pause()
                    .unwrap()
                    .write(parent_rsp + 8, &[0; 16])
                    .unwrap();
                let raw = request(
                    owner,
                    [buf, 1, 0, 0, if bad_payload { 1 } else { user + 8 }, 2],
                    parent_rsp,
                );
                let (mut engine, token) = stopped(owner, &proof, raw, layout.entry);
                let NativeServiceAction::AgcCommandGrow(start) =
                    owner.service_action(&proof, raw).unwrap()
                else {
                    panic!("expected growth")
                };
                assert_eq!(start.args(), [buf, 8, user]);
                let nested = engine
                    .begin_syscall_call(
                        &mut owner.pause().unwrap(),
                        token,
                        start.entry(),
                        start.args(),
                    )
                    .unwrap();
                owner
                    .observe_agc_command_buffer_call(&proof, start, &nested)
                    .unwrap();
                assert!(
                    owner
                        .observe_agc_command_buffer_call(&proof, start, &nested)
                        .is_err()
                );
                let bias = proof.materialized_source().source_plan().modules()[0].load_bias();
                let mut finished = false;
                for _ in 0..8 {
                    let offset = (engine.state().pc - bias) as usize;
                    let block = plan_materialized_block(
                        proof.materialized_source(),
                        0,
                        engine.state().pc,
                        &primary[offset..(offset + 64).min(primary.len())],
                        engine.placement(),
                        &TranslationPolicy::x64_sse2(),
                        &BlockLimits::default(),
                    )
                    .unwrap();
                    // SAFETY: original immutable callback source and exclusively
                    // paused owner; the checked executor validates every operand.
                    let stop = unsafe { engine.step_checked(&mut owner.pause().unwrap(), &block) }
                        .unwrap();
                    match stop {
                        NativeBlockStop::Resume { .. } => {}
                        NativeBlockStop::Returned { .. } => {
                            finished = true;
                            break;
                        }
                        other => panic!("unexpected {other:?}"),
                    }
                }
                assert!(finished);
                assert!(engine.usage().native_entries > 0);
                let returned = engine.observed_return().unwrap();
                assert_eq!(returned.value(), 1);
                let result = owner
                    .complete_agc_command_buffer_call(&proof, start, &returned, nested, &engine);
                if bad_payload {
                    assert!(matches!(
                        result,
                        Err(KernelStartupError::Graphics(
                            crate::gpu::driver::Failure::Memory
                        ))
                    ));
                    let index = owner.agc_callbacks.index(start).unwrap();
                    assert!(matches!(
                        owner.agc_callbacks.pending[index].phase,
                        Phase::Failed
                    ));
                    assert!(owner.agc_callbacks.pending[index].body.is_none());
                    assert!(matches!(
                        owner.service_action(&proof, raw),
                        Err(KernelStartupError::Graphics(
                            crate::gpu::driver::Failure::Poisoned
                        ))
                    ));
                } else {
                    assert!(
                        matches!(result, Ok(NativeServiceAction::Returned(value)) if value == up)
                    );
                }
                let mut actual = [0; 24];
                owner.pause().unwrap().read(up, &mut actual).unwrap();
                assert_eq!(
                    u32::from_le_bytes(actual[..4].try_into().unwrap()),
                    0xc004_3700
                );
                assert_eq!(
                    &actual[16..],
                    if bad_payload {
                        &[0xa5; 8]
                    } else {
                        &[0x5c, 0x4a, 0x4a, 0x4a, 0x4a, 0x4a, 0x4a, 0x4a]
                    }
                );
                let mut cursor = [0; 8];
                owner.pause().unwrap().read(buf + 16, &mut cursor).unwrap();
                assert_eq!(u64::from_le_bytes(cursor), up + 24);
                assert_eq!(owner.graphics_usage().calls, 1);
                assert!(
                    owner
                        .complete_agc_command_buffer_call(&proof, start, &returned, nested, &engine)
                        .is_err()
                );
                engine
                    .complete_syscall_call(&mut owner.pause().unwrap(), nested, 0)
                    .unwrap();
                assert!(
                    owner
                        .complete_agc_command_buffer_call(&proof, start, &returned, nested, &engine)
                        .is_err()
                );
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    }
}

#[test]
fn grow_target_requires_actual_initialized_immutable_source_and_failed_admission_has_no_prefix() {
    for kind in 0..4 {
        let mut primary = crate::process::hle::unavailable::tests::original_input(0);
        primary[104..112].copy_from_slice(&(2 * crate::memory::PAGE_SIZE).to_le_bytes());
        with_native_image_session(
            &[&primary],
            &NativeImageLimits::default(),
            |owner, layout| {
                let proof = owner
                    .prepare_native_source(&BlockLimits::default())
                    .unwrap();
                let buf = layout.entry.stack.address + 0x100;
                let up = buf + 0x100;
                let module = &proof.materialized_source().source_plan().modules()[0];
                let segment = &module.runtime_plan().segments[0];
                let target = match kind {
                    0 => u64::MAX,
                    1 => module.load_bias() + segment.file_bytes.len() as u64,
                    2 => up,
                    _ => {
                        owner
                            .pause()
                            .unwrap()
                            .apply_transaction(
                                NativeMemoryTransaction {
                                    effect: NativeMemoryEffect::Protect {
                                        address: module.load_bias(),
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
                descriptor(owner, buf, up, target, 0);
                let parent_rsp = layout.entry.return_slot - 64;
                owner
                    .pause()
                    .unwrap()
                    .write(parent_rsp + 8, &[0; 16])
                    .unwrap();
                let raw = request(owner, [buf, 1, 0, 0, up, 2], parent_rsp);
                assert!(owner.service_action(&proof, raw).is_err());
                assert!(owner.agc_callbacks.pending.is_empty());
                let mut cursor = [0; 8];
                owner.pause().unwrap().read(buf + 16, &mut cursor).unwrap();
                assert_eq!(u64::from_le_bytes(cursor), up);
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    }
}
