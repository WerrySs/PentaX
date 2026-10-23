// SPDX-License-Identifier: GPL-2.0-only
//! Original actual submit token, owned command closure and CPU-stable cooperative progress.
use super::super::KernelServiceStop;
use super::*;
use crate::process::hle::{
    NativeEntryLayout, NativeImageLimits, PreparedNativeSource, unavailable::tests::original_input,
    with_native_image_session,
};
use pentax_codegen::{BlockLimits, TranslationPolicy, plan_materialized_block};
use pentax_kernel::{CallArgs, Service};
use pentax_platform::native_dispatch::{
    NativeBlockStepper, NativeBlockStop, NativeCpuState, NativeStackFrame, PreSyscallToken,
};

fn request(
    session: &PreparedNativeKernelStartup<'_, '_>,
    function: driver::Function,
    args: [u64; 6],
    stack: Option<u64>,
) -> NativeServiceRequest {
    let binding = session.plan.graphics.unwrap();
    let marker = binding
        .factory
        .markers()
        .find(|m| m.function == function)
        .unwrap();
    NativeServiceRequest {
        owner_id: session.owner_id(),
        syscall_pc: binding.bias + marker.rva() + 5,
        next_pc: binding.bias + marker.rva() + 7,
        service_id: marker.id().into(),
        args,
        vector_args: None,
        stack_pointer: stack,
    }
}
fn write(session: &mut PreparedNativeKernelStartup<'_, '_>, at: u64, bytes: &[u8]) {
    session.pause().unwrap().write(at, bytes).unwrap();
}
fn words(session: &mut PreparedNativeKernelStartup<'_, '_>, at: u64, words: &[u32]) {
    write(
        session,
        at,
        &words
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect::<Vec<_>>(),
    );
}
fn read(session: &mut PreparedNativeKernelStartup<'_, '_>, at: u64) -> u32 {
    let mut bytes = [0; 4];
    session.pause().unwrap().read(at, &mut bytes).unwrap();
    u32::from_le_bytes(bytes)
}
fn stopped<'view, 'plan, 'image>(
    session: &mut PreparedNativeKernelStartup<'plan, 'image>,
    proof: &'view PreparedNativeSource<'plan, 'image>,
    request: NativeServiceRequest,
    layout: NativeEntryLayout,
) -> (NativeBlockStepper<'view, 'plan, 'image>, PreSyscallToken) {
    let stack = NativeStackFrame {
        stack: layout.stack,
        guards: Some(layout.guards),
        return_slot: layout.return_slot,
        return_marker: layout.return_marker,
    };
    let mut state = NativeCpuState::new(request.syscall_pc, layout.return_slot);
    for (i, value) in state.gpr.iter_mut().enumerate() {
        if i != 4 {
            *value = 0x1357_0000 + i as u64;
        }
    }
    state.gpr[0] = request.service_id;
    for (register, value) in [7, 6, 2, 1, 8, 9].into_iter().zip(request.args) {
        state.gpr[register] = value;
    }
    state.xmm.fill(0xa5);
    state.ymm_upper.fill(0x79);
    state.mxcsr = 0x5f85;
    state.rflags = 0xed7;
    state.fs_base = Some(session.tls().instances()[session.active_tls].thread_pointer);
    let mut engine = NativeBlockStepper::new(
        proof.materialized_source(),
        &mut session.pause().unwrap(),
        state,
        stack,
        &Default::default(),
    )
    .unwrap();
    let plan = plan_materialized_block(
        proof.materialized_source(),
        session.plan.graphics.unwrap().index,
        request.syscall_pc,
        &[0x0f, 0x05],
        engine.placement(),
        &TranslationPolicy::x64_sse2(),
        &BlockLimits::default(),
    )
    .unwrap();
    let NativeBlockStop::PreSyscall(token) = engine
        .step_syscall_exit(&mut session.pause().unwrap(), &plan)
        .unwrap()
    else {
        panic!("real exit-only token missing")
    };
    (engine, token)
}
fn native_request(token: PreSyscallToken) -> NativeServiceRequest {
    NativeServiceRequest {
        owner_id: token.owner_id(),
        syscall_pc: token.syscall_pc(),
        next_pc: token.next_pc(),
        service_id: token.service_id(),
        args: token.args(),
        vector_args: Some(token.vector_args()),
        stack_pointer: Some(token.stack_pointer()),
    }
}

#[test]
fn compute_submission_returns_one_real_token_and_background_progress_preserves_cpu_and_captured_prefix()
 {
    let input = original_input(0);
    with_native_image_session(
        &[&input],
        &NativeImageLimits::default(),
        |session, layout| {
            let scratch = layout.entry.stack.address + 0x2000;
            let marker = *session
                .plan
                .factory
                .markers()
                .iter()
                .find(|m| m.registration.service == Service::MapFlexible)
                .unwrap();
            assert_eq!(
                session
                    .kernel_stop(KernelServiceStop {
                        syscall_pc: session.plan.entry(marker) + 5,
                        service_id: marker.service_id.into(),
                        args: CallArgs([scratch, 0x10000, 0x33, 0, 0, 0]),
                    })
                    .unwrap(),
                0
            );
            let mut pointer = [0; 8];
            session
                .pause()
                .unwrap()
                .read(scratch, &mut pointer)
                .unwrap();
            let base = u64::from_le_bytes(pointer);
            let prefix = base + 0x3000;
            let gate = base + 0x3010;
            let output = base + 0x3020;
            words(session, prefix, &[0, 0x1357_9bdf]);
            words(session, gate, &[0, 0x2468_ace0]);
            words(session, output, &[0, 0xa5a5_5a5a]);
            let stream = [
                0xc003_3700,
                0x100,
                prefix as u32,
                (prefix >> 32) as u32,
                17,
                0xc005_3c00,
                0x13,
                gate as u32,
                (gate >> 32) as u32,
                7,
                u32::MAX,
                0,
                0xc004_4000,
                0x101,
                prefix as u32,
                (prefix >> 32) as u32,
                output as u32,
                (output >> 32) as u32,
                0xc000_4200,
                0,
            ];
            words(session, base + 0x1000, &stream);
            write(session, base + 0x200, &(base + 0x1000).to_le_bytes());
            words(session, base + 0x208, &[stream.len() as u32, 0]);
            let request = request(
                session,
                driver::Function::Acb,
                [0x20, base + 0x200, 0, 0, 0, 0],
                None,
            );
            let proof = session
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            let (mut engine, token) = stopped(session, &proof, request, layout.entry);
            let before = *engine.state();
            let calls = session.graphics_usage().calls;
            let action = session
                .service_action(&proof, native_request(token))
                .unwrap();
            assert!(matches!(action, NativeServiceAction::Returned(0)));
            assert_eq!(*engine.state(), before);
            // The worker completes this submitting CPU exactly once. No GPU wait
            // ticket replaces it or retains its original native callback token.
            engine.complete_syscall(token, 0).unwrap();
            let completed = *engine.state();
            assert!(engine.validate_pending_syscall(token).is_err());
            assert_eq!(session.graphics_usage().calls, calls + 1);
            let captured = session.graphics_usage().command_bytes;
            let mut prefix_seen = false;
            for _ in 0..64 {
                assert!(session.pump_compute_queues().unwrap());
                assert_eq!(*engine.state(), completed);
                if read(session, prefix) == 17 {
                    prefix_seen = true;
                    break;
                }
            }
            assert!(
                prefix_seen,
                "actual WRITE prefix did not progress while gate remained zero"
            );
            assert_eq!(read(session, output), 0);
            // Corrupting the caller's root must not replace the admitted closure.
            // Changing the committed prefix detects replay when the wait resumes.
            words(session, base + 0x1000, &[0x8000_0000; 20]);
            words(session, prefix, &[99]);
            words(session, gate, &[7]);
            let mut finished = false;
            for _ in 0..64 {
                let pending = session.pump_compute_queues().unwrap();
                assert_eq!(*engine.state(), completed);
                if !pending {
                    finished = true;
                    break;
                }
            }
            assert!(
                finished,
                "real current gate store did not release the retained wait"
            );
            assert_eq!(read(session, prefix), 99);
            assert_eq!(read(session, output), 99);
            assert_eq!(read(session, prefix + 4), 0x1357_9bdf);
            assert_eq!(read(session, gate + 4), 0x2468_ace0);
            assert_eq!(read(session, output + 4), 0xa5a5_5a5a);
            assert_eq!(session.graphics_usage().command_bytes, captured);
            assert_eq!(session.graphics_usage().calls, calls + 1);
            assert!(!session.pump_compute_queues().unwrap());
            assert!(!session.revoke_graphics_continuations_on_teardown().unwrap());
        },
    )
    .unwrap();
}
