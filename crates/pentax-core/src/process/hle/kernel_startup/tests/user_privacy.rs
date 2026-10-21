// SPDX-License-Identifier: GPL-2.0-only
//! The actual native owner retains full-span write refusal and current rights.
use super::*;

fn read12(session: &mut PreparedNativeKernelStartup<'_, '_>, address: u64) -> [u8; 12] {
    let mut bytes = [0; 12];
    session.pause().unwrap().read(address, &mut bytes).unwrap();
    bytes
}

#[test]
fn privacy_ws1_actual_backing_writes_four_bytes_and_refuses_late_pages_atomically() {
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
                .write(output - 4, &[0xa5; 12])
                .unwrap();
            assert_eq!(
                session.kernel_stop(stop(
                    plan,
                    Service::UserGetPlatformPrivacyWs1,
                    [u64::MAX, 0, u64::MAX, u64::MAX, u64::MAX, u64::MAX]
                )),
                Ok(0x8096_0005)
            );
            assert_eq!(read12(&mut session, output - 4), [0xa5; 12]);
            assert_eq!(
                session.kernel_stop(stop(
                    plan,
                    Service::UserGetPlatformPrivacyWs1,
                    [
                        0xffff_ffff_0000_0001,
                        output,
                        u64::MAX,
                        u64::MAX,
                        u64::MAX,
                        u64::MAX
                    ]
                )),
                Ok(0)
            );
            assert_eq!(
                read12(&mut session, output - 4),
                [0xa5, 0xa5, 0xa5, 0xa5, 0, 0, 0, 0, 0xa5, 0xa5, 0xa5, 0xa5]
            );

            session
                .pause()
                .unwrap()
                .write(output - 4, &[0xa5; 12])
                .unwrap();
            session
                .kernel_stop(stop(
                    plan,
                    Service::Mprotect,
                    [mapped + PAGE_SIZE, PAGE_SIZE, 1, 0, 0, 0],
                ))
                .unwrap();
            assert!(matches!(
                session.kernel_stop(stop(
                    plan,
                    Service::UserGetPlatformPrivacyWs1,
                    [1, output, 0, 0, 0, 0]
                )),
                Err(KernelStartupError::Kernel(Failure::Guest(_)))
            ));
            assert_eq!(read12(&mut session, output - 4), [0xa5; 12]);
            session
                .kernel_stop(stop(
                    plan,
                    Service::Mprotect,
                    [mapped + PAGE_SIZE, PAGE_SIZE, 3, 0, 0, 0],
                ))
                .unwrap();
            assert_eq!(
                session.kernel_stop(stop(
                    plan,
                    Service::UserGetPlatformPrivacyWs1,
                    [0xffff_ffff, output, 0, 0, 0, 0]
                )),
                Ok(0)
            );

            let tail = mapped + 2 * PAGE_SIZE - 8;
            session.pause().unwrap().write(tail, &[0x5a; 8]).unwrap();
            for address in [tail + 6, tail + 8, u64::MAX - 1] {
                assert!(matches!(
                    session.kernel_stop(stop(
                        plan,
                        Service::UserGetPlatformPrivacyWs1,
                        [1, address, 0, 0, 0, 0]
                    )),
                    Err(KernelStartupError::Kernel(Failure::Guest(_)))
                ));
                assert_eq!(word(&mut session, tail), 0x5a5a_5a5a_5a5a_5a5a);
            }
            session
                .pause()
                .unwrap()
                .write(output - 4, &[0xa5; 12])
                .unwrap();
            let before = session.kernel_usage();
            let mut forged = stop(
                plan,
                Service::UserGetPlatformPrivacyWs1,
                [1, output, 0, 0, 0, 0],
            );
            forged.syscall_pc += 1;
            assert!(matches!(
                session.kernel_stop(forged),
                Err(KernelStartupError::SiteMismatch { .. })
            ));
            assert_eq!(session.kernel_usage().attempts, before.attempts);
            assert_eq!(read12(&mut session, output - 4), [0xa5; 12]);
            assert_eq!(word(&mut session, tail), 0x5a5a_5a5a_5a5a_5a5a);
        },
    );
}
