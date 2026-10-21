// SPDX-License-Identifier: GPL-2.0-only

//! Original exit-only syscall plans: no emitted instruction enters the bridge.

use super::*;
use pentax_codegen::{BlockLimits, TranslationPolicy, plan_materialized_block};
use pentax_platform::native_dispatch::{
    NativeBlockStepper, NativeBlockStop, NativeCpuState, NativeStackFrame, PreSyscallToken,
};
use pentax_platform::native_memory::NativeRange;

fn call(session: &mut PreparedNativeKernelStartup<'_, '_>, service: Service, args: &[u64]) -> u64 {
    let mut registers = [0; 6];
    registers[..args.len()].copy_from_slice(args);
    session
        .kernel_stop(stop(session.plan, service, registers))
        .unwrap()
}

fn request(token: PreSyscallToken) -> NativeServiceRequest {
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

fn stopped<'view, 'plan, 'image>(
    session: &mut PreparedNativeKernelStartup<'plan, 'image>,
    proof: &'view PreparedNativeSource<'plan, 'image>,
    stop: KernelServiceStop,
    base: u64,
) -> (NativeBlockStepper<'view, 'plan, 'image>, PreSyscallToken) {
    let return_slot = base + OUTPUT + PAGE_SIZE - 8;
    let stack = NativeStackFrame {
        stack: NativeRange {
            address: base + OUTPUT,
            size: PAGE_SIZE,
        },
        guards: None,
        return_slot,
        return_marker: 0x5058_5052_4554_5552,
    };
    session
        .pause()
        .unwrap()
        .write(return_slot, &stack.return_marker.to_le_bytes())
        .unwrap();
    let mut state = NativeCpuState::new(stop.syscall_pc, return_slot);
    state.gpr[0] = stop.service_id;
    for (register, value) in [7, 6, 2, 1, 8, 9].into_iter().zip(stop.args.0) {
        state.gpr[register] = value;
    }
    let mut engine = NativeBlockStepper::new(
        proof.materialized_source(),
        &mut session.pause().unwrap(),
        state,
        stack,
        &Default::default(),
    )
    .unwrap();
    let mut bytes = [0; 2];
    session
        .pause()
        .unwrap()
        .fetch_source(stop.syscall_pc, &mut bytes)
        .unwrap();
    assert_eq!(bytes, [0x0f, 0x05]);
    let block = plan_materialized_block(
        proof.materialized_source(),
        session.plan.kernel_provider_index(),
        stop.syscall_pc,
        &bytes,
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
    // The platform admits only this single actual syscall exit; its checked
    // UD2-only plan cannot enter the native instruction bridge.
    let NativeBlockStop::PreSyscall(token) = engine
        .step_syscall_exit(&mut session.pause().unwrap(), &block)
        .unwrap()
    else {
        panic!("expected exit-only PRE-SYSCALL");
    };
    assert_eq!(engine.usage().native_entries, 0);
    (engine, token)
}

fn limits() -> KernelStartupLimits {
    let mut limits = KernelStartupLimits::default();
    limits.startup.static_tls.thread_control =
        crate::process::tls::ThreadControlBlockPolicy::KytyObservedSelfPointerV1;
    limits.kernel.max_thread_contexts = 2;
    limits
}

#[test]
fn usleep_and_equeue_waits_check_selected_tls_and_current_executor_before_any_ticket_completion() {
    let mut bounds = limits();
    // These original exit-only waits never sleep or enter the bridge. The
    // distant sleep deadline and empty equeue retain genuine pending tickets.
    bounds.kernel.max_sleep_nanoseconds = 60_000_000_000;
    for service in [Service::Usleep, Service::WaitEqueue] {
        with_plan(bounds, |plan, owner, base, pointers| {
            let mut session = session(plan, owner, &pointers);
            let main = session.prepare_main_thread().unwrap();
            let child = session.prepare_thread_context(1).unwrap();
            session.activate_thread_context(&child).unwrap();
            let proof = session
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            let args = if service == Service::WaitEqueue {
                session
                    .pause()
                    .unwrap()
                    .write(base + NAME, b"original-wait-observation\0")
                    .unwrap();
                assert_eq!(
                    call(
                        &mut session,
                        Service::CreateEqueue,
                        &[base + OUTPUT, base + NAME]
                    ),
                    0
                );
                let queue = word(&mut session, base + OUTPUT);
                [queue, base + OUTPUT + 128, 1, base + OUTPUT + 192, 0, 0]
            } else {
                [60_000_000, 0, 0, 0, 0, 0]
            };
            let syscall = stop(plan, service, args);
            let (mut engine, token) = stopped(&mut session, &proof, syscall, base);
            let (foreign, _) = stopped(&mut session, &proof, syscall, base);
            let NativeServiceAction::WaitThread(pending) =
                session.service_action(&proof, request(token)).unwrap()
            else {
                panic!("positive native wait must park");
            };
            assert_eq!(pending.is_event_queue(), service == Service::WaitEqueue);
            let attempts = session.kernel_usage().attempts;
            let services = session.native_service_requests();
            assert!(!session.thread_wait_ready(&pending).unwrap());
            assert_eq!(
                session.thread_wait_delay(&pending).unwrap().is_some(),
                service == Service::Usleep
            );
            assert!(matches!(
                session.complete_thread_wait(&pending, token, &proof, &foreign),
                Err(KernelStartupError::InvalidProvider(
                    "thread wait executor continuation"
                ))
            ));
            session.activate_thread_context(&main).unwrap();
            // Readiness is an observation, allowed while another TLS is selected.
            assert!(!session.thread_wait_ready(&pending).unwrap());
            assert!(matches!(
                session.complete_thread_wait(&pending, token, &proof, &engine),
                Err(KernelStartupError::InvalidProvider(
                    "thread wait continuation"
                ))
            ));
            session.activate_thread_context(&child).unwrap();
            assert_eq!(
                session
                    .complete_thread_wait(&pending, token, &proof, &engine)
                    .unwrap(),
                None
            );
            // A consumed platform token is not authority to complete the still-live
            // kernel wait, even when all public PC/ID/argument fields still match.
            engine.complete_syscall(token, 0).unwrap();
            assert!(matches!(
                session.complete_thread_wait(&pending, token, &proof, &engine),
                Err(KernelStartupError::InvalidProvider(
                    "thread wait executor continuation"
                ))
            ));
            assert!(!session.thread_wait_ready(&pending).unwrap());
            assert_eq!(session.kernel_usage().attempts, attempts);
            assert_eq!(session.native_service_requests(), services);
            assert_eq!(engine.usage().native_entries, 0);
        });
    }
}

fn session<'plan, 'image>(
    plan: &'plan KernelStartupPlan<'image>,
    owner: NativeAddressSpace,
    pointers: &[u64],
) -> PreparedNativeKernelStartup<'plan, 'image> {
    let mut native_limits = load_limits(plan);
    native_limits.session.resident_limit = initial_mapped_bytes(plan, 2) + 6 * PAGE_SIZE;
    native_limits.session.mapped_limit = native_limits.session.resident_limit;
    prepare_in(
        plan,
        &StaticTlsStorage {
            thread_pointers: pointers,
        },
        &native_limits,
        identity(),
        vec![],
        Some(owner),
    )
    .unwrap()
}

#[test]
fn all_mutex_wait_aliases_park_and_transfer_only_on_the_selected_exact_executor() {
    for service in [
        Service::MutexLock,
        Service::PosixMutexLock,
        Service::MutexTimedLock,
        Service::PosixMutexTimedLock,
    ] {
        with_plan(limits(), |plan, owner, base, pointers| {
            let mut session = session(plan, owner, &pointers);
            let main = session.prepare_main_thread().unwrap();
            let child = session.prepare_thread_context(1).unwrap();
            let mutex = base + OUTPUT;
            session.pause().unwrap().write(mutex, &[0; 8]).unwrap();
            assert_eq!(
                call(&mut session, Service::PosixMutexInit, &[mutex, 0, 0]),
                0
            );
            assert_eq!(call(&mut session, Service::PosixMutexLock, &[mutex]), 0);
            let proof = session
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            let timeout = if service == Service::PosixMutexTimedLock {
                let seconds = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    + 3600;
                let mut bytes = [0; 16];
                bytes[..8].copy_from_slice(&seconds.to_le_bytes());
                session.pause().unwrap().write(mutex + 64, &bytes).unwrap();
                mutex + 64
            } else {
                60_000_000
            };
            session.activate_thread_context(&child).unwrap();
            let syscall = stop(plan, service, [mutex, timeout, 0, 0, 0, 0]);
            let (mut engine, token) = stopped(&mut session, &proof, syscall, base);
            let (foreign, _) = stopped(&mut session, &proof, syscall, base);
            let NativeServiceAction::WaitThread(pending) =
                session.service_action(&proof, request(token)).unwrap()
            else {
                panic!("contended lock must park");
            };
            let attempts = session.native_service_requests();
            let kernel_attempts = session.kernel_usage().attempts;
            assert!(!session.thread_wait_ready(&pending).unwrap());
            assert_eq!(
                session.thread_wait_delay(&pending).unwrap().is_some(),
                matches!(
                    service,
                    Service::MutexTimedLock | Service::PosixMutexTimedLock
                )
            );
            assert!(matches!(
                session.complete_thread_wait(&pending, token, &proof, &foreign),
                Err(KernelStartupError::InvalidProvider(
                    "thread wait executor continuation"
                ))
            ));
            assert_eq!(session.kernel_usage().attempts, kernel_attempts);
            session.activate_thread_context(&main).unwrap();
            assert_eq!(call(&mut session, Service::PosixMutexUnlock, &[mutex]), 0);
            assert!(session.thread_wait_ready(&pending).unwrap());
            assert!(matches!(
                session.complete_thread_wait(&pending, token, &proof, &engine),
                Err(KernelStartupError::InvalidProvider(
                    "thread wait continuation"
                ))
            ));
            // The wrong TLS/executor checks did not consume membership or grant
            // ownership: the released mutex remains free and destruction busy.
            assert_eq!(call(&mut session, Service::PosixMutexDestroy, &[mutex]), 16);
            assert_eq!(call(&mut session, Service::PosixMutexTryLock, &[mutex]), 0);
            assert_eq!(call(&mut session, Service::PosixMutexUnlock, &[mutex]), 0);
            session.activate_thread_context(&child).unwrap();
            let before_completion = session.kernel_usage().attempts;
            assert_eq!(
                session
                    .complete_thread_wait(&pending, token, &proof, &engine)
                    .unwrap(),
                Some(0)
            );
            engine.complete_syscall(token, 0).unwrap();
            assert!(
                session
                    .complete_thread_wait(&pending, token, &proof, &engine)
                    .is_err()
            );
            assert_eq!(session.kernel_usage().attempts, before_completion);
            assert_eq!(session.native_service_requests(), attempts);
            session.activate_thread_context(&main).unwrap();
            assert_eq!(call(&mut session, Service::PosixMutexTryLock, &[mutex]), 16);
            assert_eq!(call(&mut session, Service::PosixMutexUnlock, &[mutex]), 1);
            session.activate_thread_context(&child).unwrap();
            assert_eq!(call(&mut session, Service::PosixMutexUnlock, &[mutex]), 0);
            assert_eq!(call(&mut session, Service::PosixMutexDestroy, &[mutex]), 0);
        });
    }
}

#[test]
fn consumed_executor_token_refuses_before_a_ready_mutex_changes_owner() {
    with_plan(limits(), |plan, owner, base, pointers| {
        let mut session = session(plan, owner, &pointers);
        let main = session.prepare_main_thread().unwrap();
        let child = session.prepare_thread_context(1).unwrap();
        let mutex = base + OUTPUT;
        session.pause().unwrap().write(mutex, &[0; 8]).unwrap();
        assert_eq!(
            call(&mut session, Service::PosixMutexInit, &[mutex, 0, 0]),
            0
        );
        assert_eq!(call(&mut session, Service::PosixMutexLock, &[mutex]), 0);
        let proof = session
            .prepare_native_source(&BlockLimits::default())
            .unwrap();
        session.activate_thread_context(&child).unwrap();
        let (mut engine, token) = stopped(
            &mut session,
            &proof,
            stop(plan, Service::PosixMutexLock, [mutex, 0, 0, 0, 0, 0]),
            base,
        );
        let NativeServiceAction::WaitThread(pending) =
            session.service_action(&proof, request(token)).unwrap()
        else {
            panic!("expected parked child");
        };
        // An external invalid continuation cannot turn a stale public token
        // into an irreversible ownership transfer inside the core adapter.
        engine.complete_syscall(token, 99).unwrap();
        session.activate_thread_context(&main).unwrap();
        assert_eq!(call(&mut session, Service::PosixMutexUnlock, &[mutex]), 0);
        session.activate_thread_context(&child).unwrap();
        assert!(session.thread_wait_ready(&pending).unwrap());
        assert!(matches!(
            session.complete_thread_wait(&pending, token, &proof, &engine),
            Err(KernelStartupError::InvalidProvider(
                "thread wait executor continuation"
            ))
        ));
        assert!(session.thread_wait_ready(&pending).unwrap());
        session.activate_thread_context(&main).unwrap();
        assert_eq!(call(&mut session, Service::PosixMutexDestroy, &[mutex]), 16);
        assert_eq!(call(&mut session, Service::PosixMutexTryLock, &[mutex]), 0);
        assert_eq!(call(&mut session, Service::PosixMutexUnlock, &[mutex]), 0);
    });
}
