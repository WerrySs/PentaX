// SPDX-License-Identifier: GPL-2.0-only
//! Actual owned memory and authenticated Pad sites, without host input devices.
use super::*;

fn read(session: &mut PreparedNativeKernelStartup<'_, '_>, address: u64, length: usize) -> Vec<u8> {
    let mut bytes = vec![0; length];
    session.pause().unwrap().read(address, &mut bytes).unwrap();
    bytes
}

#[test]
fn pad_outputs_and_requests_use_current_whole_spans_and_keep_state_on_backing_refusal() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            session
                .kernel_stop(stop(plan, Service::PadInit, [0; 6]))
                .unwrap();
            session
                .kernel_stop(stop(
                    plan,
                    Service::MapFlexible,
                    [base + OUTPUT, 2 * PAGE_SIZE, 3, 0, 0, 0],
                ))
                .unwrap();
            let mapped = word(&mut session, base + OUTPUT);
            for (index, (service, length)) in [
                (Service::PadGetTriggerEffectState, 8),
                (Service::PadGetControllerInformation, 28),
                (Service::PadRead, 120),
            ]
            .into_iter()
            .enumerate()
            {
                let output = mapped + PAGE_SIZE - 3;
                // Isolate one real queued event so failed Read followed by retry
                // must retain that exact event, not merely another neutral packet.
                session
                    .kernel_stop(stop(plan, Service::PadRead, [1, mapped, 64, 0, 0, 0]))
                    .unwrap();
                let timestamp = session.kernel.pad_monotonic_microseconds().unwrap() + 1;
                let buttons = 0x1234 + index as u32;
                session
                    .kernel
                    .publish_pad_snapshot(pentax_kernel::PadInputSnapshot {
                        buttons,
                        timestamp,
                        ..Default::default()
                    })
                    .unwrap();
                session
                    .pause()
                    .unwrap()
                    .write(output - 1, &vec![0xa5; length + 2])
                    .unwrap();
                session
                    .kernel_stop(stop(
                        plan,
                        Service::Mprotect,
                        [mapped + PAGE_SIZE, PAGE_SIZE, 1, 0, 0, 0],
                    ))
                    .unwrap();
                assert!(matches!(
                    session.kernel_stop(stop(plan, service, [1, output, 1, 0, 0, 0])),
                    Err(KernelStartupError::Kernel(Failure::Guest(_)))
                ));
                assert_eq!(
                    read(&mut session, output - 1, length + 2),
                    vec![0xa5; length + 2]
                );
                session
                    .kernel_stop(stop(
                        plan,
                        Service::Mprotect,
                        [mapped + PAGE_SIZE, PAGE_SIZE, 3, 0, 0, 0],
                    ))
                    .unwrap();
                assert_eq!(
                    session.kernel_stop(stop(plan, service, [1, output, 1, 0, 0, 0])),
                    Ok(u64::from(service == Service::PadRead))
                );
                let result = read(&mut session, output - 1, length + 2);
                assert_eq!((result[0], result[length + 1]), (0xa5, 0xa5));
                if service == Service::PadRead {
                    assert_eq!(&result[1..5], &buttons.to_le_bytes());
                    assert_eq!(
                        u64::from_le_bytes(result[81..89].try_into().unwrap()),
                        timestamp
                    );
                }
                let before = session.kernel_usage();
                let mut forged = stop(plan, service, [1, output, 1, 0, 0, 0]);
                forged.syscall_pc += 1;
                assert!(matches!(
                    session.kernel_stop(forged),
                    Err(KernelStartupError::SiteMismatch { .. })
                ));
                assert_eq!(session.kernel_usage().attempts, before.attempts);
                assert_eq!(read(&mut session, output - 1, length + 2), result);
            }
            for (service, length) in [
                (Service::PadSetVibration, 2),
                (Service::PadSetLightBar, 3),
                (Service::PadSetTriggerEffect, 120),
            ] {
                let address = mapped + PAGE_SIZE - 1;
                session
                    .pause()
                    .unwrap()
                    .write(address, &vec![0; length])
                    .unwrap();
                session
                    .kernel_stop(stop(
                        plan,
                        Service::Mprotect,
                        [mapped + PAGE_SIZE, PAGE_SIZE, 0, 0, 0, 0],
                    ))
                    .unwrap();
                let before = session.kernel.pad_output();
                assert!(matches!(
                    session.kernel_stop(stop(plan, service, [1, address, 0, 0, 0, 0])),
                    Err(KernelStartupError::Kernel(Failure::Guest(_)))
                ));
                assert_eq!(session.kernel.pad_output(), before);
                session
                    .kernel_stop(stop(
                        plan,
                        Service::Mprotect,
                        [mapped + PAGE_SIZE, PAGE_SIZE, 3, 0, 0, 0],
                    ))
                    .unwrap();
                assert_eq!(
                    session.kernel_stop(stop(plan, service, [1, address, 0, 0, 0, 0])),
                    Ok(0)
                );
            }
            let before = session.kernel.pad_output();
            assert_eq!(
                session.kernel_stop(stop(
                    plan,
                    Service::PadSetTiltCorrectionState,
                    [2, 0, 0, 0, 0, 0]
                )),
                Ok(0x8092_0003)
            );
            assert_eq!(session.kernel.pad_output(), before);
            assert_eq!(
                session.kernel_stop(stop(
                    plan,
                    Service::PadSetTiltCorrectionState,
                    [1, 0, 0, 0, 0, 0]
                )),
                Ok(0)
            );
            assert!(!session.kernel.pad_output().tilt_correction);
            assert_eq!(
                session.kernel_stop(stop(
                    plan,
                    Service::PadGetTriggerEffectState,
                    [1, 0, 0, 0, 0, 0]
                )),
                Ok(0x8092_0001)
            );
            let tail = mapped + 2 * PAGE_SIZE - 1;
            session.pause().unwrap().write(tail, &[0x5a]).unwrap();
            for address in [tail, tail + 1, u64::MAX - 2] {
                assert!(matches!(
                    session.kernel_stop(stop(plan, Service::PadRead, [1, address, 1, 0, 0, 0])),
                    Err(KernelStartupError::Kernel(Failure::Guest(_)))
                ));
                assert_eq!(read(&mut session, tail, 1), [0x5a]);
            }
        },
    );
}
