// SPDX-License-Identifier: GPL-2.0-only
//! Original exit-only native tokens and real owned CPU Frames; no OS display.
use super::super::{KernelServiceStop, ThreadWaitAdvance};
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
fn call(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    function: driver::Function,
    args: [u64; 6],
    stack: Option<u64>,
) -> u64 {
    let request = request(session, function, args, stack);
    session.service_request(request).unwrap()
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
fn setup(session: &mut PreparedNativeKernelStartup<'_, '_>, scratch: u64) -> u64 {
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
                args: CallArgs([scratch, 0x40000, 0x33, 0, 0, 0])
            })
            .unwrap(),
        0
    );
    let mut bytes = [0; 8];
    session.pause().unwrap().read(scratch, &mut bytes).unwrap();
    let base = u64::from_le_bytes(bytes);
    let target = (base + 65535) & !65535;
    assert_eq!(
        call(
            session,
            driver::Function::VideoOpen,
            [255, 0, 0, 0, 0, 0],
            None
        ),
        1
    );
    let mut attribute = [0; 80];
    attribute[12..16].copy_from_slice(&2u32.to_le_bytes());
    attribute[16..20].copy_from_slice(&2u32.to_le_bytes());
    attribute[32..40].copy_from_slice(&0x8000_0000_2200_0000u64.to_le_bytes());
    let mut buffers = [0; 64];
    buffers[..8].copy_from_slice(&target.to_le_bytes());
    buffers[32..40].copy_from_slice(&(target + 65536).to_le_bytes());
    write(session, scratch + 0x100, &attribute);
    write(session, scratch + 0x200, &buffers);
    write(session, scratch + 0x300, &[0; 24]);
    assert_eq!(
        call(
            session,
            driver::Function::VideoRegisterBuffers2,
            [1, 0, 0, scratch + 0x200, 2, scratch + 0x100],
            Some(scratch + 0x300)
        ),
        0
    );
    for buffer in [0, 1] {
        assert_eq!(
            call(
                session,
                driver::Function::VideoSubmitFlip,
                [1, buffer, 1, 0, 0, 0],
                None
            ),
            0
        );
    }
    base
}
fn stream(session: &mut PreparedNativeKernelStartup<'_, '_>, base: u64) -> [u64; 6] {
    let mut code = Vec::new();
    for (offset, value, buffer) in [(0, 11, Some(0)), (4, 22, Some(1)), (8, 33, None)] {
        let address = base + 0x3000 + offset;
        code.extend([
            0xc003_3700,
            0x100,
            address as u32,
            (address >> 32) as u32,
            value,
        ]);
        if let Some(buffer) = buffer {
            code.extend([0xc005_1018, 1, buffer, 0, 0, 0, 0]);
        }
    }
    words(session, base + 0x1000, &code);
    write(session, base + 0x100, &[0; 8]); // Actual context queue selector zero.
    write(session, base + 0x200, &(base + 0x1000).to_le_bytes());
    words(session, base + 0x208, &[code.len() as u32, 0]);
    words(session, base + 0x3000, &[0, 0, 0]);
    [base + 0x100, base + 0x200, 0, 0, 0, 0]
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
fn ack(session: &mut PreparedNativeKernelStartup<'_, '_>, tick: u64) {
    let frame = session.graphics_video_take_frame(tick).unwrap().unwrap();
    session.graphics_video_acknowledge_frame(&frame).unwrap();
}

#[test]
fn graphics_wait_replacements_keep_one_real_syscall_token_full_cpu_and_original_captured_prefix() {
    let input = original_input(0);
    with_native_image_session(
        &[&input],
        &NativeImageLimits::default(),
        |session, layout| {
            let scratch = layout.entry.stack.address + 0x2000;
            let base = setup(session, scratch);
            let args = stream(session, base);
            let request = request(session, driver::Function::CommandBuffer, args, None);
            assert!(matches!(
                session.service_request(request),
                Err(KernelStartupError::Graphics(
                    driver::Failure::UnsupportedQueue
                ))
            ));
            assert_eq!(
                (
                    read(session, base + 0x3000),
                    read(session, base + 0x3004),
                    read(session, base + 0x3008)
                ),
                (0, 0, 0)
            );
            let proof = session
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            let (mut engine, token) = stopped(session, &proof, request, layout.entry);
            let before = *engine.state();
            let calls = session.graphics_usage().calls;
            let NativeServiceAction::WaitThread(first) = session
                .service_action(&proof, native_request(token))
                .unwrap()
            else {
                panic!("first actual selected wait missing")
            };
            assert_eq!(session.graphics_usage().calls, calls + 1);
            let captured = session.graphics_usage().command_bytes;
            assert_eq!(
                (read(session, base + 0x3000), read(session, base + 0x3004)),
                (11, 0)
            );
            assert!(!session.thread_wait_ready(&first).unwrap());
            assert!(matches!(
                session.complete_thread_wait(&first, token, &proof, &engine),
                Err(KernelStartupError::InvalidProvider(
                    "graphics wait requires replacement transport"
                ))
            ));
            // Guest writes cannot replace the already-owned command closure or
            // cause its committed prefix to be interpreted a second time.
            words(session, base + 0x1000, &[0x8000_0000; 29]);
            words(session, base + 0x3000, &[99]);
            let frame = session.graphics_video_take_frame(1).unwrap().unwrap();
            assert!(!session.thread_wait_ready(&first).unwrap());
            session.graphics_video_acknowledge_frame(&frame).unwrap();
            let ThreadWaitAdvance::Replace(second) = session
                .advance_thread_wait(&first, token, &proof, &engine)
                .unwrap()
            else {
                panic!("successor wait must explicitly replace its ticket")
            };
            assert_eq!(*engine.state(), before);
            assert_eq!(
                (
                    read(session, base + 0x3000),
                    read(session, base + 0x3004),
                    read(session, base + 0x3008)
                ),
                (99, 22, 0)
            );
            assert!(session.thread_wait_ready(&first).is_err());
            assert!(!session.thread_wait_ready(&second).unwrap());
            words(session, base + 0x3004, &[88]);
            ack(session, 2);
            let ThreadWaitAdvance::Returned(value) = session
                .advance_thread_wait(&second, token, &proof, &engine)
                .unwrap()
            else {
                panic!("whole original submission did not complete")
            };
            assert_eq!(*engine.state(), before);
            assert_eq!(session.graphics_usage().command_bytes, captured);
            assert_eq!(session.graphics_usage().calls, calls + 1);
            assert_eq!(
                (
                    read(session, base + 0x3000),
                    read(session, base + 0x3004),
                    read(session, base + 0x3008)
                ),
                (99, 88, 33)
            );
            engine.complete_syscall(token, value).unwrap();
            let mut expected = before;
            expected.gpr[0] = 0;
            expected.gpr[1] = token.next_pc();
            expected.gpr[11] = before.rflags;
            expected.guest_pc = token.next_pc();
            assert_eq!(*engine.state(), expected);
            assert!(
                session
                    .advance_thread_wait(&second, token, &proof, &engine)
                    .is_err()
            );
            assert_eq!(engine.usage().native_entries, 0);
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}

#[test]
fn graphics_wait_guards_live_tls_private_executor_arguments_site_and_generation_before_resume() {
    let input = original_input(0);
    let mut limits = NativeImageLimits::default();
    limits.startup.kernel.max_thread_contexts = 2;
    with_native_image_session(&[&input], &limits, |session, layout| {
        let main = session.prepare_main_thread().unwrap();
        // NativeImage supplies one real initial TLS instance. Admit the second
        // through the normal pthread constructor; a context quota is not storage.
        let scratch = layout.entry.stack.address + 0x2000;
        let thread_source = session
            .prepare_native_source(&BlockLimits::default())
            .unwrap();
        let marker = *session
            .plan
            .factory
            .markers()
            .iter()
            .find(|marker| marker.registration.service == Service::PthreadCreate)
            .unwrap();
        let pc = session.plan.entry(marker) + 5;
        let NativeServiceAction::CreateThread(pending) = session
            .service_action(
                &thread_source,
                NativeServiceRequest {
                    owner_id: session.owner_id(),
                    syscall_pc: pc,
                    next_pc: pc + 2,
                    service_id: marker.service_id.into(),
                    args: [scratch + 0x700, 0, layout.entry.entry, 0, 0, 0],
                    vector_args: None,
                    stack_pointer: None,
                },
            )
            .unwrap()
        else {
            panic!("actual pthread creation admission missing");
        };
        let child = session
            .prepare_created_thread(&thread_source, pending)
            .unwrap();
        assert_eq!(session.tls().instances().len(), 2);
        let base = setup(session, scratch);
        let args = stream(session, base);
        let request = request(session, driver::Function::CommandBuffer, args, None);
        let proof = session
            .prepare_native_source(&BlockLimits::default())
            .unwrap();
        let (engine, token) = stopped(session, &proof, request, layout.entry);
        let NativeServiceAction::WaitThread(pending) = session
            .service_action(&proof, native_request(token))
            .unwrap()
        else {
            panic!("real wait missing")
        };
        let before = *engine.state();
        let calls = session.graphics_usage().calls;
        // A real second prepared owner cannot advance the first owner's token.
        with_native_image_session(&[&input], &limits, |foreign, _| {
            assert!(matches!(
                foreign.advance_thread_wait(&pending, token, &proof, &engine),
                Err(KernelStartupError::InvalidProvider(
                    "thread wait continuation"
                ))
            ));
            Ok::<_, ()>(())
        })
        .unwrap();
        let (other, _) = stopped(session, &proof, request, layout.entry);
        assert!(matches!(
            session.advance_thread_wait(&pending, token, &proof, &other),
            Err(KernelStartupError::InvalidProvider(
                "thread wait executor continuation"
            ))
        ));
        let mut wrong = request;
        wrong.args[5] = 1;
        let (wrong_engine, wrong_token) = stopped(session, &proof, wrong, layout.entry);
        assert!(matches!(
            session.advance_thread_wait(&pending, wrong_token, &proof, &wrong_engine),
            Err(KernelStartupError::InvalidProvider(
                "thread wait continuation"
            ))
        ));
        let wrong_site = self::request(session, driver::Function::SuspendPoint, [0; 6], None);
        let (wrong_engine, wrong_token) = stopped(session, &proof, wrong_site, layout.entry);
        assert!(
            session
                .advance_thread_wait(&pending, wrong_token, &proof, &wrong_engine)
                .is_err()
        );
        session.activate_thread_context(child.thread()).unwrap();
        assert!(!session.thread_wait_ready(&pending).unwrap()); // A different selected TLS may observe only.
        assert!(matches!(
            session.advance_thread_wait(&pending, token, &proof, &engine),
            Err(KernelStartupError::InvalidProvider(
                "thread wait continuation"
            ))
        ));
        session.activate_thread_context(&main).unwrap();
        assert_eq!(
            call(
                session,
                driver::Function::VideoClose,
                [1, 0, 0, 0, 0, 0],
                None
            ),
            0
        );
        assert_eq!(
            call(
                session,
                driver::Function::VideoOpen,
                [255, 0, 0, 0, 0, 0],
                None
            ),
            1
        );
        assert!(matches!(
            session.advance_thread_wait(&pending, token, &proof, &engine),
            Err(KernelStartupError::Graphics(
                driver::Failure::UnsupportedVideo
            ))
        ));
        assert_eq!(*engine.state(), before);
        assert_eq!(session.graphics_usage().calls, calls + 2);
        assert_eq!(
            (read(session, base + 0x3004), read(session, base + 0x3008)),
            (0, 0)
        );
        assert!(
            session
                .graphics
                .as_ref()
                .unwrap()
                .has_pending_graphics_continuation()
        );
        session.provider_work = session.plan.limits.max_provider_work;
        assert!(session.revoke_graphics_continuations_on_teardown().unwrap());
        assert!(!session.revoke_graphics_continuations_on_teardown().unwrap());
        assert!(
            !session
                .graphics
                .as_ref()
                .unwrap()
                .has_pending_graphics_continuation()
        );
        assert!(session.thread_wait_ready(&pending).is_err());
        Ok::<_, ()>(())
    })
    .unwrap();
}
