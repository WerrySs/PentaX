// SPDX-License-Identifier: GPL-2.0-only
//! Original real PRE-SYSCALL tokens, without entering a native body.
use super::mutex_wait::{limits, request, session, stopped};
use super::*;
use pentax_codegen::BlockLimits;

fn call(session: &mut PreparedNativeKernelStartup<'_, '_>, service: Service, args: &[u64]) -> u64 {
    let mut values = [0; 6];
    values[..args.len()].copy_from_slice(args);
    session
        .kernel_stop(stop(session.plan, service, values))
        .unwrap()
}

#[test]
fn rwlock_waits_keep_actual_native_ticket_tls_source_and_owner_before_transfer() {
    for service in [
        Service::RwlockReadLock,
        Service::RwlockWriteLock,
        Service::PosixRwlockWriteLock,
        Service::RwlockTimedReadLock,
        Service::RwlockTimedWriteLock,
    ] {
        with_plan(limits(), |plan, owner, base, pointers| {
            let mut session = session(plan, owner, &pointers);
            let main = session.prepare_main_thread().unwrap();
            let child = session.prepare_thread_context(1).unwrap();
            let slot = base + OUTPUT;
            session.pause().unwrap().write(slot, &[0; 8]).unwrap();
            let held = if matches!(
                service,
                Service::RwlockReadLock | Service::RwlockTimedReadLock
            ) {
                Service::RwlockWriteLock
            } else {
                Service::RwlockReadLock
            };
            assert_eq!(call(&mut session, held, &[slot]), 0);
            let proof = session
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            session.activate_thread_context(&child).unwrap();
            let syscall = stop(plan, service, [slot, 60_000_000, 0, 0, 0, 0]);
            let (mut engine, token) = stopped(&mut session, &proof, syscall, base);
            let (foreign, _) = stopped(&mut session, &proof, syscall, base);
            let NativeServiceAction::WaitThread(pending) =
                session.service_action(&proof, request(token)).unwrap()
            else {
                panic!("contended rwlock must retain a real wait");
            };
            let attempts = session.kernel_usage().attempts;
            let services = session.native_service_requests();
            assert!(!session.thread_wait_ready(&pending).unwrap());
            assert_eq!(
                session.thread_wait_delay(&pending).unwrap().is_some(),
                matches!(
                    service,
                    Service::RwlockTimedReadLock | Service::RwlockTimedWriteLock
                )
            );
            assert!(matches!(
                session.complete_thread_wait(&pending, token, &proof, &foreign),
                Err(KernelStartupError::InvalidProvider(
                    "thread wait executor continuation"
                ))
            ));
            session.activate_thread_context(&main).unwrap();
            assert!(matches!(
                session.complete_thread_wait(&pending, token, &proof, &engine),
                Err(KernelStartupError::InvalidProvider(
                    "thread wait continuation"
                ))
            ));
            assert_eq!(call(&mut session, Service::RwlockUnlock, &[slot]), 0);
            assert!(session.thread_wait_ready(&pending).unwrap());
            session.activate_thread_context(&child).unwrap();
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
            assert!(session.thread_wait_ready(&pending).is_err());
            assert_eq!(session.kernel_usage().attempts, attempts + 1); // Only main's real unlock.
            assert_eq!(session.native_service_requests(), services);
            assert_eq!(engine.usage().native_entries, 0);
            assert_eq!(call(&mut session, Service::RwlockUnlock, &[slot]), 0);
            assert_eq!(call(&mut session, Service::RwlockDestroy, &[slot]), 0);
        });
    }
}
