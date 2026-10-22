// SPDX-License-Identifier: GPL-2.0-only
//! Original actual PRE-SYSCALL tickets; no native guest instruction executes.
use super::mutex_wait::{limits, request, session, stopped};
use super::*;
use pentax_codegen::BlockLimits;

fn call(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    service: Service,
    args: [u64; 6],
) -> u64 {
    session
        .kernel_stop(stop(session.plan, service, args))
        .unwrap()
}

#[test]
fn semaphore_pending_resume_requires_selected_tls_and_exact_live_executor_without_re_admission() {
    for service in [Service::PosixSemWait, Service::PthreadSemWait] {
        with_plan(limits(), |plan, owner, base, pointers| {
            let mut session = session(plan, owner, &pointers);
            let main = session.prepare_main_thread().unwrap();
            let child = session.prepare_thread_context(1).unwrap();
            let semaphore = base + OUTPUT + 256;
            assert_eq!(
                call(
                    &mut session,
                    Service::PosixSemInit,
                    [semaphore, 0x1234, 0, 0, 0, 0]
                ),
                0
            );
            let proof = session
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            session.activate_thread_context(&child).unwrap();
            let syscall = stop(
                plan,
                service,
                [semaphore, u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX],
            );
            let (mut engine, token) = stopped(&mut session, &proof, syscall, base);
            let (foreign, _) = stopped(&mut session, &proof, syscall, base);
            let NativeServiceAction::WaitThread(pending) =
                session.service_action(&proof, request(token)).unwrap()
            else {
                panic!("indefinite zero-count wait must park");
            };
            assert!(!pending.is_event_queue());
            assert_eq!(session.thread_wait_delay(&pending).unwrap(), None);
            assert!(!session.thread_wait_ready(&pending).unwrap());
            assert_eq!(word(&mut session, semaphore), 0x0000_0001_0000_09fa);
            assert!(matches!(
                session.complete_thread_wait(&pending, token, &proof, &foreign),
                Err(KernelStartupError::InvalidProvider(
                    "thread wait executor continuation"
                ))
            ));
            session.activate_thread_context(&main).unwrap();
            assert_eq!(
                call(
                    &mut session,
                    Service::PthreadSemPost,
                    [semaphore, 0, 0, 0, 0, 0]
                ),
                0
            );
            let attempts = session.kernel_usage().attempts;
            let services = session.native_service_requests();
            assert!(session.thread_wait_ready(&pending).unwrap());
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
                Some(0)
            );
            assert_eq!(word(&mut session, semaphore), 0x09fa);
            assert_eq!(session.kernel_usage().attempts, attempts);
            assert_eq!(session.native_service_requests(), services);
            engine.complete_syscall(token, 0).unwrap();
            assert!(matches!(
                session.complete_thread_wait(&pending, token, &proof, &engine),
                Err(KernelStartupError::InvalidProvider(
                    "thread wait executor continuation"
                ))
            ));
            assert_eq!(engine.usage().native_entries, 0);
        });
    }
}
