// SPDX-License-Identifier: GPL-2.0-only
//! Real owned cross-page output rights and authenticated keyboard service sites.
use super::*;

#[test]
fn keyboard_type_cross_page_refusal_preserves_bytes_then_uses_current_writable_backing() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            session
                .kernel_stop(stop(
                    plan,
                    Service::MapFlexible,
                    [base + OUTPUT, 2 * PAGE_SIZE, 3, 0, 0, 0],
                ))
                .unwrap();
            let mapped = word(&mut session, base + OUTPUT);
            let output = mapped + PAGE_SIZE - 2;
            session
                .pause()
                .unwrap()
                .write(output - 1, &[0xa5; 6])
                .unwrap();
            session
                .kernel_stop(stop(
                    plan,
                    Service::Mprotect,
                    [mapped + PAGE_SIZE, PAGE_SIZE, 1, 0, 0, 0],
                ))
                .unwrap();
            let request = stop(
                plan,
                Service::ConvertKeycodeGetImeKeyboardType,
                [1, output, u64::MAX, u64::MAX, u64::MAX, u64::MAX],
            );
            assert!(matches!(
                session.kernel_stop(request),
                Err(KernelStartupError::Kernel(Failure::Guest(_)))
            ));
            let mut bytes = [0; 6];
            session
                .pause()
                .unwrap()
                .read(output - 1, &mut bytes)
                .unwrap();
            assert_eq!(bytes, [0xa5; 6]);
            session
                .kernel_stop(stop(
                    plan,
                    Service::Mprotect,
                    [mapped + PAGE_SIZE, PAGE_SIZE, 3, 0, 0, 0],
                ))
                .unwrap();
            assert_eq!(session.kernel_stop(request), Ok(0));
            session
                .pause()
                .unwrap()
                .read(output - 1, &mut bytes)
                .unwrap();
            assert_eq!(bytes, [0xa5, 0, 0, 0, 0, 0xa5]);
            let attempts = session.kernel_usage().attempts;
            let mut forged = request;
            forged.syscall_pc += 1;
            assert!(matches!(
                session.kernel_stop(forged),
                Err(KernelStartupError::SiteMismatch { .. })
            ));
            assert_eq!(session.kernel_usage().attempts, attempts);
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}
