// SPDX-License-Identifier: GPL-2.0-only

//! Service isolation over two actual owned TLS instances, without executing code.

use super::*;

fn call(session: &mut PreparedNativeKernelStartup<'_, '_>, service: Service, args: &[u64]) -> u64 {
    let mut registers = [0; 6];
    registers[..args.len()].copy_from_slice(args);
    session
        .kernel_stop(stop(session.plan, service, registers))
        .unwrap()
}

#[test]
fn thread_context_switch_isolates_keys_errno_identity_and_mutex_ownership() {
    let mut limits = KernelStartupLimits::default();
    limits.startup.static_tls.thread_control =
        crate::process::tls::ThreadControlBlockPolicy::KytyObservedSelfPointerV1;
    limits.kernel.max_thread_contexts = 2;
    with_plan(limits, |plan, owner, base, pointers| {
        let mut native_limits = load_limits(plan);
        native_limits.session.resident_limit = initial_mapped_bytes(plan, 2) + 6 * PAGE_SIZE;
        native_limits.session.mapped_limit = native_limits.session.resident_limit;
        let mut session = prepare_in(
            plan,
            &StaticTlsStorage {
                thread_pointers: &pointers,
            },
            &native_limits,
            identity(),
            vec![],
            Some(owner),
        )
        .unwrap();
        let main = session.prepare_main_thread().unwrap();
        let other = session.prepare_thread_context(1).unwrap();
        assert!(session.prepare_thread_context(2).is_err());
        let output = base + OUTPUT;
        session.pause().unwrap().write(output, &[0; 16]).unwrap();
        assert_eq!(call(&mut session, Service::PosixKeyCreate, &[output, 0]), 0);
        let key = word(&mut session, output);
        let mutex = output + 8;
        assert_eq!(
            call(&mut session, Service::PosixMutexInit, &[mutex, 0, 0]),
            0
        );
        assert_eq!(call(&mut session, Service::PosixMutexLock, &[mutex]), 0);
        assert_eq!(call(&mut session, Service::SetSpecific, &[key, 0x1234]), 0);
        let main_errno = call(&mut session, Service::ErrnoLocation, &[]);
        session
            .pause()
            .unwrap()
            .write(main_errno, &37_u32.to_le_bytes())
            .unwrap();
        let main_self = call(&mut session, Service::PthreadSelf, &[]);
        assert_eq!(word(&mut session, main_self), 73);

        session.activate_thread_context(&other).unwrap();
        assert_eq!(call(&mut session, Service::PthreadGetthreadid, &[]), 74);
        assert_eq!(call(&mut session, Service::GetSpecific, &[key]), 0);
        assert_eq!(call(&mut session, Service::SetSpecific, &[key, 0x5678]), 0);
        let other_errno = call(&mut session, Service::ErrnoLocation, &[]);
        let other_self = call(&mut session, Service::PthreadSelf, &[]);
        assert_ne!(main_errno, other_errno);
        assert_ne!(main_self, other_self);
        assert_eq!(word(&mut session, other_errno), 0);
        assert_eq!(word(&mut session, other_self), 74);
        session
            .pause()
            .unwrap()
            .write(other_errno, &19_u32.to_le_bytes())
            .unwrap();
        assert_eq!(call(&mut session, Service::PosixMutexTryLock, &[mutex]), 16);
        assert_eq!(call(&mut session, Service::PosixMutexUnlock, &[mutex]), 1);

        session.activate_thread_context(&main).unwrap();
        assert_eq!(call(&mut session, Service::PthreadGetthreadid, &[]), 73);
        assert_eq!(call(&mut session, Service::PthreadSelf, &[]), main_self);
        assert_eq!(call(&mut session, Service::ErrnoLocation, &[]), main_errno);
        assert_eq!(call(&mut session, Service::GetSpecific, &[key]), 0x1234);
        assert_eq!(word(&mut session, main_errno), 37);
        assert_eq!(call(&mut session, Service::PosixMutexUnlock, &[mutex]), 0);

        // A stale token must not change the active identity or its service state.
        session
            .pause()
            .unwrap()
            .write(pointers[1], &[0; 8])
            .unwrap();
        assert!(session.activate_thread_context(&other).is_err());
        assert_eq!(call(&mut session, Service::PthreadGetthreadid, &[]), 73);
        session
            .pause()
            .unwrap()
            .write(pointers[1], &pointers[1].to_le_bytes())
            .unwrap();
        session.activate_thread_context(&other).unwrap();
        assert_eq!(call(&mut session, Service::GetSpecific, &[key]), 0x5678);
        assert_eq!(word(&mut session, other_errno), 19);
        assert_eq!(call(&mut session, Service::PosixMutexTryLock, &[mutex]), 0);
        assert_eq!(call(&mut session, Service::PosixMutexUnlock, &[mutex]), 0);

        // Keys are process-wide. Reusing one must hide both threads' old values.
        assert_eq!(call(&mut session, Service::PosixKeyDelete, &[key]), 0);
        assert_eq!(call(&mut session, Service::PosixKeyCreate, &[output, 0]), 0);
        assert_eq!(word(&mut session, output), key);
        assert_eq!(call(&mut session, Service::GetSpecific, &[key]), 0);
        session.activate_thread_context(&main).unwrap();
        assert_eq!(call(&mut session, Service::GetSpecific, &[key]), 0);
    });
}
