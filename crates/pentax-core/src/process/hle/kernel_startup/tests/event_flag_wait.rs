// SPDX-License-Identifier: GPL-2.0-only

//! Original event-flag continuation proof using genuine exit-only syscall tokens.

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
fn event_flag_resume_requires_exact_tls_executor_and_committed_live_outputs() {
    with_plan(limits(), |plan, owner, base, pointers| {
        let mut session = session(plan, owner, &pointers);
        let main = session.prepare_main_thread().unwrap();
        let child = session.prepare_thread_context(1).unwrap();
        let slot = base + OUTPUT + 0x80;
        assert_eq!(
            call(
                &mut session,
                Service::CreateEventFlag,
                &[slot, base + NAME, 0x20, 0, 0]
            ),
            0
        );
        let flag = word(&mut session, slot);
        let output = base + OUTPUT + 0x100;
        session.pause().unwrap().write(output, &[0xa5; 8]).unwrap();
        let proof = session
            .prepare_native_source(&BlockLimits::default())
            .unwrap();
        session.activate_thread_context(&child).unwrap();
        let syscall = stop(plan, Service::WaitEventFlag, [flag, 1, 0x21, output, 0, 0]);
        let (mut engine, token) = stopped(&mut session, &proof, syscall, base);
        let (foreign, _) = stopped(&mut session, &proof, syscall, base);
        let NativeServiceAction::WaitThread(pending) =
            session.service_action(&proof, request(token)).unwrap()
        else {
            panic!("unsatisfied event flag must park");
        };
        assert!(!session.thread_wait_ready(&pending).unwrap());
        assert_eq!(session.thread_wait_delay(&pending).unwrap(), None);
        assert!(matches!(
            session.complete_thread_wait(&pending, token, &proof, &foreign),
            Err(KernelStartupError::InvalidProvider(
                "thread wait executor continuation"
            ))
        ));
        session.activate_thread_context(&main).unwrap();
        assert_eq!(call(&mut session, Service::SetEventFlag, &[flag, 1]), 0);
        assert!(session.thread_wait_ready(&pending).unwrap());
        assert!(matches!(
            session.complete_thread_wait(&pending, token, &proof, &engine),
            Err(KernelStartupError::InvalidProvider(
                "thread wait continuation"
            ))
        ));
        // Refuse the complete real output span after signal, preserving the bit,
        // enrolled ticket and exact pending machine syscall for a valid retry.
        assert_eq!(
            call(
                &mut session,
                Service::Mprotect,
                &[base + OUTPUT, PAGE_SIZE, 1]
            ),
            0
        );
        session.activate_thread_context(&child).unwrap();
        assert!(matches!(
            session.complete_thread_wait(&pending, token, &proof, &engine),
            Err(KernelStartupError::Kernel(Failure::Guest(
                GuestError::WriteDenied
            )))
        ));
        assert_eq!(word(&mut session, output), 0xa5a5_a5a5_a5a5_a5a5);
        assert!(session.thread_wait_ready(&pending).unwrap());
        session.activate_thread_context(&main).unwrap();
        assert_eq!(
            call(
                &mut session,
                Service::Mprotect,
                &[base + OUTPUT, PAGE_SIZE, 3]
            ),
            0
        );
        session.activate_thread_context(&child).unwrap();
        let attempts = session.kernel_usage().attempts;
        assert_eq!(
            session
                .complete_thread_wait(&pending, token, &proof, &engine)
                .unwrap(),
            Some(0)
        );
        assert_eq!(word(&mut session, output), 1);
        assert_eq!(session.kernel_usage().attempts, attempts);
        engine.complete_syscall(token, 0).unwrap();
        assert!(matches!(
            session.complete_thread_wait(&pending, token, &proof, &engine),
            Err(KernelStartupError::InvalidProvider(
                "thread wait executor continuation"
            ))
        ));
        assert_eq!(
            call(
                &mut session,
                Service::PollEventFlag,
                &[flag, 1, 1, output + 8]
            ),
            0x8002_0010
        );
        assert_eq!(word(&mut session, output + 8), 0);
        assert_eq!(engine.usage().native_entries, 0);
    });
}
